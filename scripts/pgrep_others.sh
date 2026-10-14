#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# `pgrep -f` that cannot match the caller.  Lists the processes whose COMMAND LINE matches an
# extended regex, leaving out this script, every one of its ANCESTORS and its own children —
# so the waiting shell, whose `bash -c '…'` argv contains the pattern, is never counted
# (`until ! pgrep -f "make ci"` never exits for exactly that reason, CODE.md § shell).
# Exclusion is by process tree, not by pattern: no `[m]ake` bracket to forget on one branch
# of an alternation, no `-x` truncated to 15 characters.
#
#   scripts/pgrep_others.sh 'make ci'               # list: pid, cwd, command; exit 0 if any
#   scripts/pgrep_others.sh --here 'cargo test'      # only processes running in this checkout
#   scripts/pgrep_others.sh --wait 'rustc' [--timeout 600]   # block until none match
#   scripts/pgrep_others.sh --count 'loft --native'  # just the number
#
# Prefer a recorded PID or an artefact when one exists (`scripts/ci-run.sh status`, the pid
# `find_problems.sh --bg` prints, an output file's mtime): they have no pattern at all.  This
# is for the case that has none.  Never feed its output to `kill` on a shared box without
# `--here`: a sibling checkout's run matches the same pattern.
#
# Exit: 0 a match (list/count) or all gone (--wait); 1 none (list/count); 124 --wait timed
# out; 2 usage.
set -euo pipefail

usage() {
  sed -n '12,16p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

mode=list here=0 timeout=0 pattern=""
while [ $# -gt 0 ]; do
  case "$1" in
    --wait) mode=wait ;;
    --count) mode=count ;;
    --here) here=1 ;;
    --timeout) shift; timeout="${1:-}"; [[ "$timeout" =~ ^[0-9]+$ ]] || usage ;;
    -h|--help) usage ;;
    -*) usage ;;
    *) [ -z "$pattern" ] || usage; pattern="$1" ;;
  esac
  shift
done
[ -n "$pattern" ] || usage
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

# The working directory of a process, or "" when it cannot be read (gone, or not ours).
cwd_of() {
  if [ -r "/proc/$1/cwd" ] || [ -e "/proc/$1" ]; then
    readlink "/proc/$1/cwd" 2>/dev/null || true
  else
    lsof -a -p "$1" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -1
  fi
}

# Matching pids, minus this script's ancestors, itself and its descendants.
matches() {
  ps -Ao pid=,ppid=,command= | awk -v self="$$" -v pat="$pattern" '
    { pid = $1; ppid[pid] = $2; cmd = $0; sub(/^ *[0-9]+ +[0-9]+ /, "", cmd); line[pid] = cmd }
    END {
      # self and every ancestor up to init
      p = self; while (p != "" && p != "0" && !(p in skip)) { skip[p] = 1; p = ppid[p] }
      # every descendant of self (the ps and awk of this very pipe among them)
      changed = 1
      while (changed) { changed = 0
        for (q in ppid) if (!(q in skip) && (ppid[q] in skip) && ppid[q] != ppid[self]) {
          a = ppid[q]; d = 0
          while (a != "" && a != "0") { if (a == self) { d = 1; break } ; a = ppid[a] }
          if (d) { skip[q] = 1; changed = 1 }
        }
      }
      for (q in line) if (!(q in skip) && line[q] ~ pat) print q "\t" line[q]
    }'
}

filtered() {
  matches | while IFS=$'\t' read -r pid cmd; do
    cwd="$(cwd_of "$pid")"
    if [ "$here" = 1 ]; then
      case "$cwd" in "$root"|"$root"/*) ;; *) continue ;; esac
    fi
    printf '%s\t%s\t%s\n' "$pid" "${cwd:-?}" "$cmd"
  done
}

case "$mode" in
  list)
    out="$(filtered)"
    [ -n "$out" ] || exit 1
    printf '%s\n' "$out"
    ;;
  count)
    n="$(filtered | grep -c . || true)"
    echo "$n"
    [ "$n" -gt 0 ]
    ;;
  wait)
    start=$(date +%s)
    while [ -n "$(filtered)" ]; do
      if [ "$timeout" -gt 0 ] && [ $(( $(date +%s) - start )) -ge "$timeout" ]; then
        echo "pgrep_others: still running after ${timeout}s:" >&2
        filtered >&2
        exit 124
      fi
      sleep 5
    done
    ;;
esac
