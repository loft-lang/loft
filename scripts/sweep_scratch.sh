#!/bin/bash
# sweep_scratch.sh — reclaim loft's own scratch from the directories given.
#
#   scripts/sweep_scratch.sh [--sessions] [--days N] [--falsify-days N]
#                            [--enough-gb N [--on PATH]] [--deadline EPOCH] <dir>...
#
# What loft writes to a temp directory, and the rule that removes each (RUN_BOUNDS.md § Scratch
# hygiene):
#
#   loft_native_bin_<pid>, loft_native_<pid>.rs   a `--native` run's artefacts; a run that ends
#                                                  normally removes them, one killed from outside
#                                                  cannot — removed when <pid> is DEAD
#   loft_native_cache_<checkout>/                  the native test harness's per-checkout cache
#                                                  (tests/native.rs): the harness itself drops
#                                                  the entries of an older loft build at the
#                                                  start of a run; here the dead-pid rule runs
#                                                  inside it, and the whole directory goes when
#                                                  nothing in it moved for --days
#   loft_native_<stem>*, loft_test_native_<stem>*  the native suites' per-file caches —
#                                                  removed when older than --days (default 1)
#   loft_*, loft-*                                 the html/probe/rebuild/serve scratch of the
#                                                  test suites — removed when older than --days
#                                                  (`loft-falsify` apart: see the next line)
#   loft-falsify/<ref>{,-target}                   `make falsify` control builds (~2 GB each),
#                                                  here and in falsify's own cache home:
#                                                  a control not USED for --falsify-days
#                                                  (default 7; falsify stamps the one it runs)
#                                                  is removed with its worktree — its own LRU
#                                                  runs only after a SUCCESSFUL build, so a
#                                                  failed one was left behind
#   <any>/.loft/cache/<entry>                      the program cache a test wrote beside its
#                                                  probe (every probe has a fresh name, so the
#                                                  cache only grows) — entries older than --days
#   --sessions: claude-<uid>/<project>/<session>   the agent harness's per-session scratch, next
#                                                  to the directories given — when NOTHING in
#                                                  it changed for 14 days, or for 2 days where
#                                                  the directory is a RAM tmpfs (a live session
#                                                  writes its task output there on every call)
#
# Never another program's files, never a live process's, never a sibling checkout's gate
# scratch (pass only your own).  Prints one line when something was removed, nothing when
# nothing was.
#
# A caller making room for a run (scripts/disk_headroom.sh) bounds it: `--enough-gb N` stops
# once N GB are free on PATH's filesystem (`--on`, default the working directory) and
# `--deadline EPOCH` once the clock passes it — checked between steps and between batches, so
# what is left is left whole for the next sweep.  Removal is BATCHED, one `rm` per few hundred
# entries: one process pair per entry (`du` + `rm`) took 14 minutes over 27 000 entries of a
# 51 000-entry $TMPDIR (2026-10-31), and the gate it ran for died on its 20-minute budget
# before running a test.
set -u
days=1; fdays=7; sessions=0; dirs=(); enough_gb=0; on=.; deadline=0
while [ $# -gt 0 ]; do
  case "$1" in
    --sessions) sessions=1;;
    --days) days="$2"; shift;;
    --falsify-days) fdays="$2"; shift;;
    --enough-gb) enough_gb="$2"; shift;;
    --on) on="$2"; shift;;
    --deadline) deadline="$2"; shift;;
    -h|--help) sed -n '2,43p' "$0"; exit 0;;
    *) dirs+=("$1");;
  esac
  shift
done
[ ${#dirs[@]} -gt 0 ] || { echo "usage: $0 [--sessions] [--days N] [--falsify-days N] [--enough-gb N [--on PATH]] [--deadline EPOCH] <dir>..." >&2; exit 2; }
removed=0
free_kb() { df -Pk "$on" 2>/dev/null | awk 'NR == 2 { print $4 }'; }
start_kb=$(free_kb)
# Has the caller's bound been reached: the room it asked for, or its deadline?
done_yet() {
  [ "$deadline" -gt 0 ] && [ "$(date +%s)" -ge "$deadline" ] && return 0
  [ "$enough_gb" -gt 0 ] && [ "$(free_kb)" -ge $((enough_gb * 1048576)) ] && return 0
  return 1
}
gone() { # <path> — remove, counting
  rm -rf -- "$1" 2>/dev/null && removed=$((removed + 1))
}
# Remove the NUL-separated paths on stdin, a batch per `rm`, the bound tested between batches.
remove_all() {
  local batch=() f
  while IFS= read -r -d '' f; do
    batch+=("$f")
    if [ ${#batch[@]} -ge 256 ]; then
      rm -rf -- "${batch[@]}" 2>/dev/null
      removed=$((removed + ${#batch[@]})); batch=()
      done_yet && return 0
    fi
  done
  if [ ${#batch[@]} -gt 0 ]; then
    rm -rf -- "${batch[@]}" 2>/dev/null
    removed=$((removed + ${#batch[@]}))
  fi
  return 0
}
# The live pids, read ONCE: `/proc` exists on Linux only, and testing `/proc/<pid>` on macOS
# called every process dead — a running `--native` program's binary included.  No listing (ps
# failed) means no pid is known dead, so the dead-pid rule removes nothing.
live=" $(ps -A -o pid= 2>/dev/null | tr -s ' \n' '  ') "
is_live() { case "$live" in "  ") return 0;; *" $1 "*) return 0;; esac; return 1; }
# A falsify control not used for --falsify-days: the worktree through git, so the clone's
# worktree list keeps no dangling entry.
sweep_falsify() { # <cache dir>
  [ -d "$1" ] || return 0
  local t w common
  while IFS= read -r t; do
    w=${t%-target}
    if [ -e "$w/.git" ]; then
      common=$(git -C "$w" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)
      [ -n "$common" ] && git --git-dir="$common" worktree remove --force "$w" >/dev/null 2>&1
    fi
    [ -e "$w" ] && gone "$w"
    gone "$t"; rm -f -- "$t.lock"
  done < <(find "$1" -mindepth 1 -maxdepth 1 -name '*-target' -mtime "+$fdays" 2>/dev/null)
}
# The cache's own home (`falsify.sh`), which is not a temp directory anyone passes here.
sweep_falsify "${LOFT_FALSIFY_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/loft-falsify}"
# 1. dead-process native artefacts (the pid is the trailing digit run of the stem).  Only
# the shapes written per process: the runtime's two, and the harness's per-pid temporaries
# inside its cache directory (`<stem>_<pid>_args.txt`, `<stem>_<pid>.rs.tmp`,
# `<stem>_<pid>_bin.key.tmp`).  NOT `<stem>_<pid>_bin`: a script stem ending in digits
# (`text_nul_iteration_755`) spells the same name, so that orphan is left to the age rule.
dead_pids() { # <dir> — prints each dead process's artefact, NUL-terminated
  local f name pid
  for f in "$1"/loft_native_bin_* "$1"/loft_native_*.rs \
           "$1"/loft_native_*_[0-9]*_args.txt "$1"/loft_native_*_[0-9]*.rs.tmp \
           "$1"/loft_native_*_[0-9]*_bin.key.tmp; do
    [ -e "$f" ] || continue
    name=${f##*/}
    case "$name" in
      loft_native_bin_*) pid=${name#loft_native_bin_};;
      *_args.txt) pid=${name%_args.txt}; pid=${pid##*_};;
      *.rs.tmp) pid=${name%.rs.tmp}; pid=${pid##*_};;
      *_bin.key.tmp) pid=${name%_bin.key.tmp}; pid=${pid##*_};;
      *) pid=${name#loft_native_}; pid=${pid%.rs};;
    esac
    case "$pid" in ''|*[!0-9]*) continue;; esac
    is_live "$pid" && continue
    printf '%s\0' "$f"
  done
}
for d in "${dirs[@]}"; do
  [ -d "$d" ] || continue
  done_yet && break
  remove_all < <(dead_pids "$d"; for c in "$d"/loft_native_cache_*; do [ -d "$c" ] && dead_pids "$c"; done)
  # 2. aged scratch of loft's families
  done_yet && break
  remove_all < <(
    find "$d" -mindepth 1 -maxdepth 1 \( -name 'loft_*' \
      -o \( -name 'loft-*' ! -name 'loft-falsify' \) \) \
      -mtime "+$days" -print0 2>/dev/null)
  # 2b. falsify controls
  done_yet && break
  sweep_falsify "$d/loft-falsify"
  # 3. program-cache entries a test wrote beside a probe (or in the directory itself)
  done_yet && break
  remove_all < <(
    find "$d" -mindepth 3 -maxdepth 4 -path '*/.loft/cache/*' -mtime "+$days" -print0 2>/dev/null)
  # 4. the harness's per-session scratch beside the directory
  done_yet && break
  if [ "$sessions" = 1 ]; then
    sdays=14
    [ "$(stat -f -c %T "$d" 2>/dev/null)" = tmpfs ] && sdays=2
    while IFS= read -r s; do
      # the NEWEST file decides: the directory's own mtime moves only when an entry is added
      [ -n "$(find "$s" -mtime "-$sdays" -print -quit 2>/dev/null)" ] || gone "$s"
    done < <(find "$d"/claude-[0-9]* -mindepth 2 -maxdepth 2 -type d 2>/dev/null)
  fi
done
# What the disk gained while the sweep ran (df, so another process's writes count against it).
gained=$(( ($(free_kb) - ${start_kb:-0}) / 1024 ))
[ "$gained" -lt 0 ] && gained=0
[ "$removed" -gt 0 ] && echo "sweep_scratch: removed $removed entries, ${gained} MB freed"
exit 0
