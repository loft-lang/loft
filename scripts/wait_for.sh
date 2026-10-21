#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Wait for a background job by what it WRITES or by its PID — never by its name — and always
# stop: every wait has a deadline, and a wait on a file also stops when the process that was
# to write the line has gone.  The hand-written `until grep …; do sleep; done` and
# `until ! pgrep -f …` loops it replaces had neither, so a producer that died, or a pattern
# that matched the waiting shell itself, left them running for days (CODE.md § shell).
#
#   scripts/wait_for.sh FILE REGEX [--pid N] [--timeout SECS]   # a line of FILE matches REGEX
#   scripts/wait_for.sh --pid N [--timeout SECS]                # process N has exited
#   scripts/wait_for.sh --stale [--min MINUTES]                 # the wait loops that will not stop
#
# FILE REGEX: when it matches, the matching line and the file's last lines are printed.  With
# --pid, the producer exiting first ends the wait too (exit 3, its last lines printed).
# --timeout defaults to 3600; 0 is refused, because a wait without a deadline is the defect.
# --stale lists the shells running an `until`/`while` loop with a `sleep` in it for longer
# than --min minutes (default 30), on this box, with the checkout and the session that owns
# each: a loop that old is waiting on something that is not coming.  Linux and macOS.
#
# Exit: 0 matched / exited / nothing stale; 1 stale loops listed; 3 the producer exited
# without the line; 124 the deadline passed; 2 usage.
set -euo pipefail

usage() {
  sed -n '11,13p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit "${1:-2}"
}

file="" regex="" pid="" timeout=3600 stale=0 min=30
while [ $# -gt 0 ]; do
  case "$1" in
    --pid) shift; pid="${1:-}"; [[ "$pid" =~ ^[0-9]+$ ]] || usage ;;
    --timeout) shift; timeout="${1:-}"; [[ "$timeout" =~ ^[1-9][0-9]*$ ]] || usage ;;
    --stale) stale=1 ;;
    --min) shift; min="${1:-}"; [[ "$min" =~ ^[0-9]+$ ]] || usage ;;
    -h|--help) usage 0 ;;
    -*) usage ;;
    *)
      if [ -z "$file" ]; then file="$1"
      elif [ -z "$regex" ]; then regex="$1"
      else usage
      fi ;;
  esac
  shift
done

alive() { kill -0 "$1" 2>/dev/null; }

if [ "$stale" = 1 ]; then
  # Elapsed seconds per process: `etimes` where ps has it (Linux), else `etime` parsed.
  ps -Ao pid=,ppid=,etime=,command= | awk -v self="$$" -v min="$min" '
    function secs(t,   d, n, p, s, i) {
      d = 0; if (index(t, "-")) { split(t, p, "-"); d = p[1]; t = p[2] }
      n = split(t, p, ":"); s = 0
      for (i = 1; i <= n; i++) s = s * 60 + p[i]
      return d * 86400 + s
    }
    { pid = $1; par[pid] = $2; age[pid] = secs($3); cmd = $0
      sub(/^ *[0-9]+ +[0-9]+ +[^ ]+ /, "", cmd); line[pid] = cmd }
    END {
      # This script and its ancestors are never listed.
      for (p = self; p in par && p > 1; p = par[p]) mine[p] = 1
      for (p in line) {
        c = line[p]
        if (p in mine || age[p] < min * 60) continue
        if (c !~ /(^|[;&|{ \047"])(until|while) / || c !~ /sleep /) continue
        if (c ~ /(^| )ps -Ao/) continue
        # The owning session: the nearest ancestor that is a claude process.
        s = "-"; for (q = par[p]; q in par && q > 1; q = par[q]) if (line[q] ~ /claude/) { s = q; break }
        loop = c; if (match(c, /(until|while) .*/)) loop = substr(c, RSTART, 160)
        printf "%s\t%dm\t%s\t%s\n", p, age[p] / 60, s, loop
      }
    }' | sort -t$'\t' -k2 -rn > "${TMPDIR:-/tmp}/wait_for_stale.$$" || true
  n=0
  while IFS=$'\t' read -r p a s loop; do
    [ -n "$p" ] || continue
    n=$((n + 1))
    cwd=$(readlink "/proc/$p/cwd" 2>/dev/null || lsof -a -p "$p" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -1 || true)
    printf 'pid %s  %s old  session %s  %s\n    %s\n' "$p" "$a" "$s" "${cwd:-?}" "$loop"
  done < "${TMPDIR:-/tmp}/wait_for_stale.$$"
  rm -f "${TMPDIR:-/tmp}/wait_for_stale.$$"
  if [ "$n" -gt 0 ]; then
    echo "$n wait loop(s) running longer than ${min} minutes — each waits on something that is not coming; stop one with: kill <pid>" >&2
    exit 1
  fi
  exit 0
fi

start=$(date +%s)
expired() { [ $(( $(date +%s) - start )) -ge "$timeout" ]; }

if [ -z "$file" ]; then
  [ -n "$pid" ] || usage
  while alive "$pid"; do
    if expired; then echo "wait_for: pid $pid still running after ${timeout}s" >&2; exit 124; fi
    sleep 5
  done
  exit 0
fi
[ -n "$regex" ] || usage

while :; do
  if [ -f "$file" ] && grep -Eq -- "$regex" "$file"; then
    grep -E -- "$regex" "$file" | tail -1
    echo "--- $file (last lines)"
    tail -n 15 "$file"
    exit 0
  fi
  if [ -n "$pid" ] && ! alive "$pid"; then
    echo "wait_for: pid $pid exited without writing /$regex/ to $file" >&2
    [ -f "$file" ] && tail -n 15 "$file" >&2
    exit 3
  fi
  if expired; then
    echo "wait_for: no line matching /$regex/ in $file after ${timeout}s" >&2
    [ -f "$file" ] && tail -n 5 "$file" >&2
    exit 124
  fi
  sleep 5
done
