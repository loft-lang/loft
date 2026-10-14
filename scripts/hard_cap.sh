#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The hard cap on a test run: 20 minutes, then the run ENDS.
#
#   scripts/hard_cap.sh <run-pid> <seconds> <report-file>
#
# Started beside a run, it polls.  When the run outlives <seconds> it writes why to <report-file>
# and to stderr, then stops every process DESCENDED from <run-pid> — leaves first, so a parent
# cannot spawn more work while its children go — and leaves <run-pid> itself alive, so the run's
# own shell can still summarise what finished.  It exits as soon as the run ends on its own.
#
# The owner's rule: no single run exceeds 20 minutes.  What does not fit is split
# (`find_problems.sh --subject` / `--changed`), parallelised, or moved to a nightly or PR leg —
# never given a longer limit (CI_BUDGET.md).  There is deliberately no override.
#
# Never signals by NAME: every kill is addressed to a pid descended from <run-pid>, so a sibling
# checkout's run on the same box is untouchable.
set -u

run=${1:?usage: hard_cap.sh <run-pid> <seconds> <report-file>}
secs=${2:?usage: hard_cap.sh <run-pid> <seconds> <report-file>}
report=${3:?usage: hard_cap.sh <run-pid> <seconds> <report-file>}
interval=5
start=$(date +%s)

# Every descendant of $1, parents before children; this watchdog's own subtree excluded.
descendants() {
  local p=$1 kid
  for kid in $(pgrep -P "$p" 2>/dev/null); do
    [ "$kid" = "$$" ] && continue
    echo "$kid"
    descendants "$kid"
  done
}

while sleep "$interval"; do
  kill -0 "$run" 2>/dev/null || exit 0
  [ $(( $(date +%s) - start )) -ge "$secs" ] || continue
  if [ "$secs" -ge 60 ]; then lim="$(( secs / 60 )) minutes"; else lim="${secs}s"; fi
  msg="HARD CAP: this run passed $lim and was ended.  What finished is
summarised below; everything else did not run.  Split the selection
(find_problems.sh --subject <name> / --changed) or move slow tests to a nightly or
PR leg — the limit is not raised (CI_BUDGET.md)."
  printf '\n%s\n' "$msg" >> "$report"
  printf '\n%s\n' "$msg" >&2
  tree=$(descendants "$run")
  # Leaves first: reverse the parents-before-children order.
  rev=$(printf '%s\n' $tree | awk '{ l[NR] = $0 } END { for (i = NR; i > 0; i--) print l[i] }')
  for p in $rev; do kill -TERM "$p" 2>/dev/null; done
  sleep 3
  for p in $rev; do kill -KILL "$p" 2>/dev/null; done
  exit 0
done
