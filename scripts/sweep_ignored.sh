#!/bin/bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# sweep_ignored.sh — reclaim what git IGNORES in a checkout once nothing has written it for days.
#
#   scripts/sweep_ignored.sh [--dry-run] [--days N] [--keep FILE] [--log FILE] <checkout>...
#   scripts/sweep_ignored.sh --self-test
#
# A checkout's working tree fills with data git does not hold: the program cache loft writes
# beside every `.loft` file it runs (`.loft/`), a package's `native-auto/`, a fixture crate's
# `target/`, a scratch `target-*/`, the stores and traces a test leaves, the worktrees an agent
# was given under `.claude/worktrees/`.  Measured on one checkout: 26 MB of tracked plan
# documents under 89 GB of such caches.  `cargo sweep` keeps the main `target/`; nothing kept
# the rest.
#
# The rule: an IGNORED path that nothing wrote for --days (default 3) is removed, unless the
# keep-list names it.  git decides what is ignored (`git status --ignored=matching`), so the
# sweep reads the same `.gitignore` files a person does.
#
#   a file                      removed when older than --days
#   a directory                 removed whole when NOTHING in it is newer than --days
#   `.loft/` still in use       its files older than --days go one by one — every cache entry
#                               is its own file, and a probe directory in daily use otherwise
#                               keeps every binary it ever built
#   a worktree of this checkout through `git worktree remove`, and only when it is clean, its
#                               commits are held by a branch, a tag or a remote, and no live
#                               agent has it locked — a lock naming a dead process is lifted
#
# Never touched: a tracked file; an UNTRACKED file git does not ignore (new work looks exactly
# like that — `--dry-run` lists the old ones); a directory holding tracked files or another
# repository; anything the keep-list names.
#
# The keep-list: `--keep FILE`, else `<checkout>/.janitor-keep`, else
# `<checkout>/scripts/janitor_keep.txt`.  One pattern per line, `#` starts a comment.  A
# pattern is a path from the checkout root in which `*` matches any run of characters,
# slashes included; it protects the path it matches and everything under it, and an ignored
# directory that CONTAINS a kept path (spelled from the root, no leading `*`) stays whole.
# A checkout with no keep-list is not swept: the list is how a project says it takes part.
#
# Prints one line per checkout it removed something from, nothing otherwise; `--log FILE`
# appends every path it removed, dated, with its size.
set -u
days=3; dry=0; keep_arg=; log=; selftest=0; cos=()
usage() { echo "usage: $0 [--dry-run] [--days N] [--keep FILE] [--log FILE] <checkout>... | --self-test" >&2; exit 2; }
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) dry=1;;
    --days) [ $# -ge 2 ] || usage; days=$2; shift;;
    --keep) [ $# -ge 2 ] || usage; keep_arg=$2; shift;;
    --log) [ $# -ge 2 ] || usage; log=$2; shift;;
    --self-test) selftest=1;;
    -h|--help) sed -n '5,/^set -u/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0;;
    -*) usage;;
    *) cos+=("$1");;
  esac
  shift
done
case "$days" in ''|*[!0-9]*) usage;; esac

# Was anything at or under `$1` written within --days?  The NEWEST file decides: a
# directory's own mtime moves only when an entry is added or removed.
fresh() { [ -n "$(find "$1" -mtime "-$days" 2>/dev/null | head -n 1)" ]; }

keep=()
load_keep() { # <file>
  local line
  keep=()
  while IFS= read -r line || [ -n "$line" ]; do
    line=${line%%#*}
    line=${line#"${line%%[![:space:]]*}"}
    line=${line%"${line##*[![:space:]]}"}
    line=${line#/}; line=${line%/}
    [ -n "$line" ] && keep+=("$line")
  done < "$1"
  return 0
}
kept() { # <path from the root, no trailing slash> <1 when a directory>
  local p
  for p in ${keep[@]+"${keep[@]}"}; do
    # shellcheck disable=SC2053  # the keep-list line is a pattern on purpose
    [[ $1 == $p || $1 == $p/* ]] && return 0
    [ "$2" = 1 ] && [[ $p == "$1"/* ]] && return 0
  done
  return 1
}

removed=0; kb_total=0; left=()
size_kb() { local kb; kb=$(du -sk "$1" 2>/dev/null | cut -f1); echo "${kb:-0}"; }
count() { # <kB> [<path>, when it is really gone]
  removed=$((removed + 1)); kb_total=$((kb_total + $1))
  [ -n "$log" ] && [ -n "${2:-}" ] && echo "$(date '+%F %T') removed $(($1 / 1024)) MB $2" >> "$log"
  return 0
}
gone() { # <path> — remove whole, counting
  local kb; kb=$(size_kb "$1")
  if [ "$dry" = 1 ]; then
    printf 'would remove %7s MB  %s\n' $((kb / 1024)) "$1"; count "$kb"
  else
    rm -rf -- "$1" 2>/dev/null && count "$kb" "$1"
  fi
  return 0
}
# The files of a directory still in use that nothing wrote for --days, one by one.
gone_old_files() { # <dir>
  local before after f any=0
  before=$(size_kb "$1")
  if [ "$dry" = 1 ]; then
    after=$before
    while IFS= read -r -d '' f; do
      any=1; after=$((after - $(size_kb "$f")))
    done < <(find "$1" ! -type d ! -mtime "-$days" -print0 2>/dev/null)
    [ "$any" = 1 ] && printf 'would remove %7s MB  %s (its files older than %s days)\n' \
      $(((before - after) / 1024)) "$1" "$days"
  else
    find "$1" ! -type d ! -mtime "-$days" -exec rm -f -- {} + 2>/dev/null
    find "$1" -mindepth 1 -depth -type d -empty -exec rmdir -- {} + 2>/dev/null
    after=$(size_kb "$1")
    [ "$after" -lt "$before" ] && any=1
  fi
  if [ "$any" = 1 ]; then
    if [ "$dry" = 1 ]; then count $((before - after)); else count $((before - after)) "$1 (files older than $days days)"; fi
  fi
  return 0
}

# Is the process that took a worktree lock still there?  A recycled pid reads as alive, which
# only keeps a worktree one more round.
alive() { kill -0 "$1" 2>/dev/null; }

# One worktree of the checkout, registered at a path inside an ignored directory.
sweep_worktree() { # <checkout> <worktree> <head> <branch or empty> <locked: 0/1> <lock reason>
  local co=$1 wt=$2 head=$3 branch=$4 locked=$5 reason=$6 stale_lock=0 st pid kb
  [ -d "$wt" ] || return 0
  fresh "$wt" && return 0
  if [ "$locked" = 1 ]; then
    pid=$(printf '%s' "$reason" | sed -n 's/.*(pid \([0-9][0-9]*\).*/\1/p')
    if [ -z "$pid" ]; then left+=("$wt — locked${reason:+: $reason}"); return 0; fi
    if alive "$pid"; then left+=("$wt — locked by a live process ($pid)"); return 0; fi
    stale_lock=1
  fi
  if ! st=$(git --no-optional-locks -C "$wt" status --porcelain --untracked-files=normal 2>/dev/null); then
    left+=("$wt — git cannot read it"); return 0
  fi
  if [ -n "$st" ]; then left+=("$wt — uncommitted work"); return 0; fi
  if [ -z "$branch" ] && [ -z "$(git -C "$co" for-each-ref --count=1 --contains "$head" \
        refs/heads refs/remotes refs/tags refs/keep 2>/dev/null)" ]; then
    left+=("$wt — its commits are on no branch, tag or remote"); return 0
  fi
  kb=$(size_kb "$wt")
  if [ "$dry" = 1 ]; then
    printf 'would remove %7s MB  %s (git worktree remove)\n' $((kb / 1024)) "$wt"; count "$kb"
    return 0
  fi
  [ "$stale_lock" = 1 ] && git -C "$co" worktree unlock "$wt" >/dev/null 2>&1
  if git -C "$co" worktree remove "$wt" >/dev/null 2>&1; then
    count "$kb" "$wt (git worktree remove)"
  else
    [ "$stale_lock" = 1 ] && git -C "$co" worktree lock --reason "$reason" "$wt" >/dev/null 2>&1
    left+=("$wt — git worktree remove refused it")
  fi
  return 0
}
# Every worktree registered under directory `$2` of the checkout; 1 when there was none.
sweep_worktrees_under() { # <checkout> <top as git spells it> <path from the root>
  local co=$1 dir="$2/$3" line wt='' head='' branch='' locked=0 reason='' found=1 first=1
  while IFS= read -r line || [ -n "$wt" ]; do
    case "$line" in
      'worktree '*) wt=${line#worktree }; head=''; branch=''; locked=0; reason='';;
      'HEAD '*) head=${line#HEAD };;
      'branch '*) branch=${line#branch };;
      locked) locked=1;;
      'locked '*) locked=1; reason=${line#locked };;
      '')
        if [ -n "$wt" ] && [ "$first" = 0 ]; then
          case "$wt" in "$dir"/*) found=0; sweep_worktree "$co" "$wt" "$head" "$branch" "$locked" "$reason";; esac
        fi
        [ -n "$wt" ] && first=0
        wt='';;
    esac
  done <<< "$worktrees"
  return $found
}

sweep_checkout() { # <checkout>
  local co=$1 top keepfile rec xy rel p isdir worktrees old_untracked=()
  removed=0; kb_total=0; left=()
  co=$(cd "$co" 2>/dev/null && pwd) || return 0
  if [ "$(git -C "$co" rev-parse --is-inside-work-tree 2>/dev/null)" != true ] \
      || [ -n "$(git -C "$co" rev-parse --show-cdup 2>/dev/null)" ]; then
    echo "sweep_ignored: $co is not the root of a git checkout" >&2; return 0
  fi
  top=$(git -C "$co" rev-parse --show-toplevel 2>/dev/null)
  keepfile=$keep_arg
  [ -n "$keepfile" ] || { [ -f "$co/.janitor-keep" ] && keepfile="$co/.janitor-keep"; }
  [ -n "$keepfile" ] || { [ -f "$co/scripts/janitor_keep.txt" ] && keepfile="$co/scripts/janitor_keep.txt"; }
  if [ -z "$keepfile" ] || [ ! -f "$keepfile" ]; then
    [ "$dry" = 1 ] && echo "sweep_ignored: $co has no keep-list (.janitor-keep) — not swept"
    return 0
  fi
  load_keep "$keepfile"
  worktrees=$(git -C "$co" worktree list --porcelain 2>/dev/null)
  # `--untracked-files=no` would hide the ignored paths as well.
  while IFS= read -r -d '' -u 3 rec; do
    xy=${rec:0:2}
    case "$xy" in R?|?R|C?|?C) IFS= read -r -d '' -u 3 _;; esac
    rel=${rec:3}
    if [ "$xy" = '??' ]; then
      [ "$dry" = 1 ] && ! fresh "$co/${rel%/}" && old_untracked+=("$rel")
      continue
    fi
    [ "$xy" = '!!' ] || continue
    isdir=0
    case "$rel" in */) isdir=1; rel=${rel%/};; esac
    case "$rel" in ''|/*|..|../*|*/../*|*/..|.git|.git/*) continue;; esac
    p="$co/$rel"
    [ -e "$p" ] || [ -L "$p" ] || continue
    kept "$rel" "$isdir" && continue
    if [ "$isdir" = 1 ] && [ ! -L "$p" ]; then
      if [ -n "$(git -C "$co" ls-files -- "$rel" 2>/dev/null | head -n 1)" ]; then
        left+=("$p — holds tracked files"); continue
      fi
      sweep_worktrees_under "$co" "$top" "$rel" && continue
      if [ -n "$(find "$p" -maxdepth 4 -name .git 2>/dev/null | head -n 1)" ]; then
        left+=("$p — holds another repository"); continue
      fi
      if fresh "$p"; then
        [ "${rel##*/}" = .loft ] && gone_old_files "$p"
        continue
      fi
    else
      fresh "$p" && continue
    fi
    gone "$p"
  done 3< <(git --no-optional-locks -C "$co" status --porcelain=v1 -z --ignored=matching \
              --untracked-files=normal 2>/dev/null)
  if [ "$dry" = 1 ]; then
    for p in ${left[@]+"${left[@]}"}; do echo "left alone: $p"; done
    for p in ${old_untracked[@]+"${old_untracked[@]}"}; do
      echo "left alone: $co/$p — untracked and not ignored (commit it, ignore it, or remove it by hand)"
    done
    echo "sweep_ignored: $co: would remove $removed entries, $((kb_total / 1024)) MB"
  elif [ "$removed" -gt 0 ]; then
    echo "sweep_ignored: $co: removed $removed entries, $((kb_total / 1024)) MB"
  fi
  return 0
}

# ── the self-test: a scratch repository with one cell per rule, swept for real ──────────────
self_test() {
  local me=$1 t r fails=0 live dead out
  t=$(mktemp -d) || exit 1
  sleep 600 & live=$!
  # shellcheck disable=SC2064
  trap "kill $live 2>/dev/null; rm -rf -- '$t'" EXIT
  r="$t/repo"
  g() { git -C "$r" -c user.name=sweep -c user.email=sweep@example.invalid -c commit.gpgsign=false "$@"; }
  old() { find "$@" -exec touch -h -t 202001010000 {} + 2>/dev/null; }
  mkdir -p "$r" && git -C "$r" init -q -b main . || exit 1
  printf '%s\n' '.loft/' 'native-auto/' '*.store' 'keepme/' 'data/' 'vendor/' 'forced/*.bin' \
    'state/*' '!state/.gitkeep' '.claude/worktrees/' 'target/' 'gen/' > "$r/.gitignore"
  mkdir -p "$r"/{src,state,forced,gen,keepme,data/deep,vendor/lib,probes/.loft/cache,live/.loft/cache} \
           "$r"/{pkg/native-auto,used/native-auto,scripts}
  echo code > "$r/src/a.txt"; : > "$r/state/.gitkeep"; echo f > "$r/forced/a.txt"; echo f > "$r/gen/tracked.txt"
  printf '%s\n' '# kept on purpose' 'keepme' 'data/deep/precious.bin  # one file inside an ignored directory' \
    > "$r/scripts/janitor_keep.txt"
  g add -A && g add -f forced/a.txt gen/tracked.txt && g commit -qm base || exit 1
  # ignored data, old and new
  echo x > "$r/old.store"; echo x > "$r/new.store"
  echo x > "$r/probes/.loft/cache/native-1"; echo x > "$r/probes/.loft/log.txt"
  echo x > "$r/live/.loft/cache/native-old"; echo x > "$r/live/.loft/cache/native-new"
  echo x > "$r/pkg/native-auto/lib_old.so"
  echo x > "$r/used/native-auto/lib_old.so"; echo x > "$r/used/native-auto/.fp"
  echo x > "$r/keepme/k.bin"; echo x > "$r/data/deep/precious.bin"; echo x > "$r/data/other.bin"
  echo x > "$r/state/x.json"; echo x > "$r/forced/b.bin"; echo x > "$r/gen/old.bin"
  git -C "$r/vendor/lib" init -q . ; echo x > "$r/vendor/lib/v.txt"
  # not ignored at all
  echo x > "$r/notes.txt"
  old "$r/old.store" "$r/probes" "$r/live/.loft/cache/native-old" "$r/pkg" \
      "$r/used/native-auto/lib_old.so" "$r/keepme" "$r/data" "$r/state/x.json" "$r/forced/b.bin" \
      "$r/vendor" "$r/notes.txt" "$r/src/a.txt" "$r/gen"
  # worktrees: a clean one on a branch, a detached one whose commit no ref holds, a dirty one,
  # one locked by a live process, one locked by a dead one, a clean one still fresh
  mkdir -p "$r/.claude/worktrees"
  for w in a b c d e f; do g worktree add -q -b "wt-$w" ".claude/worktrees/$w" || exit 1; done
  git -C "$r/.claude/worktrees/b" -c user.name=sweep -c user.email=sweep@example.invalid \
      -c commit.gpgsign=false commit -q --allow-empty -m loose
  git -C "$r/.claude/worktrees/b" checkout -q --detach && g branch -qf wt-b main
  mkdir -p "$r/.claude/worktrees/a/target" && echo x > "$r/.claude/worktrees/a/target/built"
  echo x > "$r/.claude/worktrees/c/new-work.txt"
  dead=999999
  while kill -0 "$dead" 2>/dev/null; do dead=$((dead - 1)); done
  g worktree lock --reason "claude agent agent-d (pid $live start 1)" .claude/worktrees/d
  g worktree lock --reason "claude agent agent-e (pid $dead start 1)" .claude/worktrees/e
  old "$r/.claude/worktrees"/{a,b,c,d,e}
  expect() { # <present|absent> <path> <what>
    local have=absent; { [ -e "$r/$2" ] || [ -L "$r/$2" ]; } && have=present
    [ "$have" = "$1" ] || { echo "self-test FAILED: $2 is $have, expected $1 — $3"; fails=$((fails + 1)); }
  }
  check_kept() {
    expect present new.store "an ignored file written today"
    expect present live/.loft/cache/native-new "a fresh entry of a cache in use"
    expect present used/native-auto/lib_old.so "a directory with one fresh file stays whole"
    expect present keepme/k.bin "the keep-list names the directory"
    expect present data/other.bin "an ignored directory holding a kept path stays whole"
    expect present data/deep/precious.bin "the keep-list names the file"
    expect present notes.txt "untracked and not ignored: new work looks like this"
    expect present src/a.txt "tracked"
    expect present forced/a.txt "tracked inside an ignored pattern"
    expect present state/.gitkeep "tracked beside ignored files"
    expect present gen/tracked.txt "tracked inside an ignored directory"
    expect present vendor/lib/v.txt "another repository"
    expect present .claude/worktrees/b "a worktree whose commit no ref holds"
    expect present .claude/worktrees/c/new-work.txt "a worktree with uncommitted work"
    expect present .claude/worktrees/d "a worktree a live process has locked"
    expect present .claude/worktrees/f "a worktree written today"
  }
  # 1. a dry run names what would go and removes nothing
  out=$("$me" --dry-run "$r")
  for p in old.store probes/.loft live/.loft/cache/native-old pkg/native-auto state/x.json forced/b.bin \
           .claude/worktrees/a .claude/worktrees/e; do
    expect present "$p" "a dry run removes nothing"
  done
  for p in old.store probes/.loft live/.loft pkg/native-auto worktrees/a worktrees/e; do
    case "$out" in *"would remove"*"$p"*) ;; *) echo "self-test FAILED: the dry run does not name $p"; fails=$((fails + 1));; esac
  done
  case "$out" in *"left alone"*notes.txt*) ;; *) echo "self-test FAILED: the dry run does not list the old untracked file"; fails=$((fails + 1));; esac
  check_kept
  # 2. the sweep
  "$me" "$r" >/dev/null
  expect absent old.store "an ignored file nothing wrote for days"
  expect absent probes/.loft "a cache directory nothing wrote for days goes whole"
  expect absent live/.loft/cache/native-old "an old entry of a cache in use"
  expect absent pkg/native-auto "an ignored directory nothing wrote for days"
  expect absent state/x.json "an old ignored file beside a tracked one"
  expect absent forced/b.bin "an old ignored file beside a force-added one"
  expect absent gen/old.bin "an old ignored file in a directory that also holds a tracked one"
  expect absent .claude/worktrees/a "a clean worktree on a branch (its ignored build included)"
  expect absent .claude/worktrees/e "a clean worktree locked by a dead process"
  check_kept
  [ -n "$(g branch --list wt-a)" ] || { echo "self-test FAILED: removing worktree a took its branch"; fails=$((fails + 1)); }
  case "$(g worktree list --porcelain)" in
    *worktrees/a*|*worktrees/e*) echo "self-test FAILED: a removed worktree is still registered"; fails=$((fails + 1));;
  esac
  # 3. no keep-list, no sweep
  rm -f "$r/scripts/janitor_keep.txt"; old "$r/new.store"
  "$me" "$r" >/dev/null
  expect present new.store "a checkout with no keep-list is not swept"
  if [ "$fails" -gt 0 ]; then echo "self-test: $fails failed"; exit 1; fi
  echo "self-test ok"
}

if [ "$selftest" = 1 ]; then
  self_test "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
  exit 0
fi
[ ${#cos[@]} -gt 0 ] || usage
for co in "${cos[@]}"; do sweep_checkout "$co"; done
exit 0
