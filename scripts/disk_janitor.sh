#!/bin/bash
# disk_janitor.sh — the builds clean up what the builds create, on every loft checkout of this box.
#
#   scripts/disk_janitor.sh              # act if due (hourly, or now when the disk is low)
#   scripts/disk_janitor.sh --background # the same, detached: returns at once, runs beside
#                                        # whatever started it
#   scripts/disk_janitor.sh --force      # act now
#   scripts/disk_janitor.sh --dry-run    # say what would be swept, remove nothing
#   scripts/disk_janitor.sh --hook       # a Claude Code PreToolUse hook: reads the tool call on
#                                        # stdin; acts, detached, only beside a BUILD-shaped command
#
# Why: measured 2026-09-29, the disk went from 18 GB free to 0 while agents ran builds and
# tests; a test run then failed 19 native tests as CODE.  None of it was data anyone needed:
# 117 GB of cargo artefacts no build had used for two days (stale-hash test binaries in
# `target/*/deps`, across `loft` and `loft2`), 21 GB of finished runs' native binaries in
# `~/.cache/loft-scratch`, 16 GB of `tests/**/.loft/cache` programs.  The cleanup rules already
# existed (sweep_scratch.sh, disk_headroom.sh) but ran only before a gate, only on the checkout
# running it, and kept 14 days of artefacts.  So whatever STARTS a build starts this beside it,
# detached: find_problems.sh, bench/stats.py, and — for a bare `cargo` or `make` an agent types —
# an async PreToolUse hook (`make janitor-install`).  It never delays the build.
#
# Cheap when not due: one stamp stat and one `df`.  Due = the last run is an hour old, or free
# space is under LOW_GB (50).  One janitor at a time (flock); a second one leaves.
#
# Per checkout (every git worktree of ~/workspace/loft, and every ~/workspace/loft* checkout):
#   1. `cargo sweep --time 3` — artefacts no build used in 3 days (--time 1 under LOW_GB)
#   2. incremental sessions older than a day
#   3. `.loft/cache` entries under tests/ older than 2 days
# Steps 1 and 2 run BESIDE builds, so they hold cargo's own build locks while they do
# (`target/<profile>/.cargo-lock`, the flock every cargo build takes): a build that starts
# meanwhile waits the seconds a sweep takes, and a checkout whose lock a build already holds is
# left for the next round.  A process working inside a checkout that holds no lock — a test
# binary, a `loft --native` run linking the rlib — also leaves it for the next round.
# Box-wide: scripts/sweep_scratch.sh over every scratch location loft writes (its own standing
# rules), `loft-tmp-cleanup` for the RAM /tmp when installed, and perf's ~/.debug older than 14
# days.  Never a file outside those, never a source file, never a checkout's working tree.
#
# Silent.  Each run that freed space appends a line to ~/.cache/loft-janitor/log; under CRIT_GB
# (15) after running it says so on the desktop (notify-send) and in that log, naming the
# largest directories — a full disk announced before a build fails on it.
set -u
LOW_GB=${LOFT_JANITOR_LOW_GB:-50}
CRIT_GB=${LOFT_JANITOR_CRIT_GB:-15}
EVERY_MIN=${LOFT_JANITOR_EVERY_MIN:-60}
WORKSPACE=${LOFT_JANITOR_WORKSPACE:-$HOME/workspace}
STATE="$HOME/.cache/loft-janitor"
mode=run; force=0; dry=0; background=0
for a in "$@"; do
  case "$a" in
    --hook) mode=hook; background=1;;
    --background) background=1;;
    --force) force=1;;
    --dry-run) dry=1; force=1;;
    *) echo "usage: $0 [--background] [--hook] [--force] [--dry-run]" >&2; exit 2;;
  esac
done

if [ "$mode" = hook ]; then
  # Only a command that builds, tests or runs the toolchain; everything else costs a grep.
  cmd=$(jq -r '.tool_input.command // empty' 2>/dev/null)
  printf '%s' "$cmd" | grep -Eq '(^|[;&|(]|\s)(cargo|make|nextest|find_problems\.sh|ci-run\.sh|stats\.py|falsify\.sh|loft)(\s|$)' || exit 0
fi
if [ "$background" = 1 ] && [ -z "${LOFT_JANITOR_DETACHED:-}" ]; then
  # Beside the build, never ahead of it: detach and return.
  args=(); [ "$force" = 1 ] && args+=(--force)
  LOFT_JANITOR_DETACHED=1 setsid "$0" ${args[@]+"${args[@]}"} </dev/null >/dev/null 2>&1 &
  exit 0
fi

free_gb() { df -P -B1G "$HOME" | awk 'NR == 2 { print $4 }'; }
mkdir -p "$STATE"
free=$(free_gb)
stamp="$STATE/stamp"
due=$force
[ "$free" -lt "$LOW_GB" ] && due=1
[ -n "$(find "$stamp" -mmin "-$EVERY_MIN" 2>/dev/null)" ] || due=1
[ "$due" = 1 ] || exit 0

exec 9>"$STATE/lock"
flock -n 9 || exit 0
[ "$dry" = 1 ] || touch "$stamp"

# The checkouts: every worktree of the main clone, and the sibling loft* checkouts.
checkouts() {
  {
    git -C "$WORKSPACE/loft" worktree list --porcelain 2>/dev/null | sed -n 's/^worktree //p'
    for d in "$WORKSPACE"/loft*/; do [ -d "$d/.git" ] || [ -f "$d/.git" ] && echo "${d%/}"; done
  } | sort -u | while read -r d; do [ -f "$d/Cargo.toml" ] && echo "$d"; done
}
# The working directories of every process that is not a shell, an editor or an agent: read
# once, because one pass over /proc costs a second and there are dozens of checkouts.
working=()
for p in /proc/[0-9]*; do
  read -r comm < "$p/comm" 2>/dev/null || continue
  # Shells, agents, editors and their language servers (rust-analyzer lives in the main
  # checkout for good and never links an artefact), and the short-lived text tools a
  # command line pipes through.  Anything else — cargo, rustc, a test binary, loft, make,
  # python — is work.
  case "$comm" in
    bash|sh|zsh|dash|fish|claude|node|git|less|vim|nvim|code|tmux|sleep|flock|disk_janitor.sh) continue;;
    rust-analyzer*|find|readlink|cat|grep|sed|awk|cut|sort|uniq|head|tail|tr|wc|du|df|jq|ps|xargs|tee) continue;;
  esac
  cwd=$(readlink "$p/cwd" 2>/dev/null) && working+=("$cwd/")
done
# Is a build, a test or a run working inside checkout `$1`?
busy() {
  local cwd
  for cwd in "${working[@]}"; do
    case "$cwd" in "$1"/*) return 0;; esac
  done
  return 1
}
run() { if [ "$dry" = 1 ]; then echo "would: $*"; else "$@"; fi; }

sweep_days=3
[ "$free" -lt "$LOW_GB" ] && sweep_days=1
kept=()
# Take every build lock of checkout `$1` without waiting, in a subshell that then runs the
# rest of its arguments; exit 3 when a build holds one.  cargo locks with flock(2), so the
# util-linux `flock` here excludes a cargo build and is excluded by it.
with_build_locks() {
  local co=$1; shift
  (
    for lock in "$co"/target/*/.cargo-lock; do
      [ -e "$lock" ] || continue
      exec {fd}>>"$lock" || exit 3
      flock -n "$fd" || exit 3
    done
    "$@"
  )
}
sweep_target() {
  local co=$1
  if cargo sweep --version >/dev/null 2>&1; then
    (cd "$co" && run cargo sweep --time "$sweep_days" >/dev/null 2>&1)
  fi
  for inc in "$co"/target/*/incremental; do
    [ -d "$inc" ] && run find "$inc" -mindepth 1 -maxdepth 1 -mtime +1 -exec rm -rf {} +
  done
  return 0
}
while read -r co; do
  if [ -d "$co/target" ]; then
    if busy "$co"; then
      kept+=("$(basename "$co")")
    else
      with_build_locks "$co" sweep_target "$co" || kept+=("$(basename "$co")")
    fi
  fi
  [ -d "$co/tests" ] && run find "$co/tests" -xdev -path '*/.loft/cache/*' -mindepth 1 -mtime +2 -delete 2>/dev/null
done < <(checkouts)

# Box-wide scratch, by the standing rules of the one script that owns them.
sweeper="$WORKSPACE/loft/scripts/sweep_scratch.sh"
[ -x "$sweeper" ] || sweeper="$(dirname "${BASH_SOURCE[0]}")/sweep_scratch.sh"
if [ -x "$sweeper" ]; then
  run "$sweeper" --sessions "$HOME/.cache/loft-scratch" "${TMPDIR:-$HOME/.cache/tmp}" "$HOME/.cache/tmp" /tmp >/dev/null 2>&1
fi
command -v loft-tmp-cleanup >/dev/null 2>&1 && run loft-tmp-cleanup -q >/dev/null 2>&1
[ -d "$HOME/.debug" ] && run find "$HOME/.debug" -type f -mtime +14 -delete 2>/dev/null

if [ "$dry" = 1 ]; then
  [ ${#kept[@]} -gt 0 ] && echo "kept the targets of: ${kept[*]} (a build is running there)"
  exit 0
fi
after=$(free_gb)
line="disk janitor: ${after} GB free (was ${free} GB)"
[ ${#kept[@]} -gt 0 ] && line="$line; kept the targets of ${kept[*]} (a build is running there)"
[ "$after" -gt "$free" ] && echo "$(date '+%F %T') $line" >> "$STATE/log"
if [ "$after" -lt "$CRIT_GB" ]; then
  top=$(du -xsh "$WORKSPACE"/* "$HOME"/.cache/* 2>/dev/null | sort -h | tail -4 | awk '{printf "%s %s; ", $1, $2}')
  msg="$line — still under ${CRIT_GB} GB. Largest: $top"
  echo "$(date '+%F %T') $msg" >> "$STATE/log"
  command -v notify-send >/dev/null 2>&1 && notify-send -u critical "Disk nearly full" "$msg" 2>/dev/null
  [ -t 2 ] && echo "$msg" >&2
elif [ "$after" -gt "$free" ] && [ -t 1 ]; then
  echo "$line"
fi
exit 0
