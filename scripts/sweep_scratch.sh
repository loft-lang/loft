#!/bin/bash
# sweep_scratch.sh — reclaim loft's own scratch from the directories given.
#
#   scripts/sweep_scratch.sh [--sessions] [--days N] [--falsify-days N] <dir>...
#
# What loft writes to a temp directory, and the rule that removes each (TESTING.md § Scratch
# hygiene):
#
#   loft_native_bin_<pid>, loft_native_<pid>.rs   a `--native` run's artefacts; a run that ends
#                                                  normally removes them, one killed from outside
#                                                  cannot — removed when <pid> is DEAD
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
set -u
days=1; fdays=7; sessions=0; dirs=()
while [ $# -gt 0 ]; do
  case "$1" in
    --sessions) sessions=1;;
    --days) days="$2"; shift;;
    --falsify-days) fdays="$2"; shift;;
    -h|--help) sed -n '2,33p' "$0"; exit 0;;
    *) dirs+=("$1");;
  esac
  shift
done
[ ${#dirs[@]} -gt 0 ] || { echo "usage: $0 [--sessions] [--days N] [--falsify-days N] <dir>..." >&2; exit 2; }
removed=0; bytes=0
gone() { # <path> — remove, counting
  local b
  b=$(du -sb "$1" 2>/dev/null | cut -f1); b=${b:-0}
  rm -rf -- "$1" 2>/dev/null && { removed=$((removed + 1)); bytes=$((bytes + b)); }
}
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
for d in "${dirs[@]}"; do
  [ -d "$d" ] || continue
  # 1. dead-process native artefacts (the pid is the trailing digit run of the stem)
  # Only the two shapes the runtime writes per process; the test suites name theirs by
  # script stem, and a stem ending in digits is not a pid.
  for f in "$d"/loft_native_bin_* "$d"/loft_native_*.rs; do
    [ -e "$f" ] || continue
    name=${f##*/}
    pid=${name#loft_native_bin_}; [ "$pid" = "$name" ] && { pid=${name#loft_native_}; pid=${pid%.rs}; }
    case "$pid" in ''|*[!0-9]*) continue;; esac
    [ -d "/proc/$pid" ] && continue
    gone "$f"
  done
  # 2. aged scratch of loft's families
  while IFS= read -r f; do gone "$f"; done < <(
    find "$d" -mindepth 1 -maxdepth 1 \( -name 'loft_*' \
      -o \( -name 'loft-*' ! -name 'loft-falsify' \) \) \
      -mtime "+$days" 2>/dev/null)
  # 2b. falsify controls
  sweep_falsify "$d/loft-falsify"
  # 3. program-cache entries a test wrote beside a probe (or in the directory itself)
  while IFS= read -r f; do gone "$f"; done < <(
    find "$d" -mindepth 3 -maxdepth 4 -path '*/.loft/cache/*' -mtime "+$days" 2>/dev/null)
  # 4. the harness's per-session scratch beside the directory
  if [ "$sessions" = 1 ]; then
    sdays=14
    [ "$(stat -f -c %T "$d" 2>/dev/null)" = tmpfs ] && sdays=2
    while IFS= read -r s; do
      # the NEWEST file decides: the directory's own mtime moves only when an entry is added
      [ -n "$(find "$s" -mtime "-$sdays" -print -quit 2>/dev/null)" ] || gone "$s"
    done < <(find "$d"/claude-[0-9]* -mindepth 2 -maxdepth 2 -type d 2>/dev/null)
  fi
done
[ "$removed" -gt 0 ] && echo "sweep_scratch: removed $removed entries, $((bytes / 1048576)) MB"
exit 0
