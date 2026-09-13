#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The local gate's budget watchdog: cancel `make ci` once it passes CI_BUDGET_SECS.
#
# Why a hard cancel and not a report — a gate that grows is a gate that stops being run.
# Measured 2026-09-12, a local `make ci` on macOS passed 45 minutes, which is long enough that
# the honest response is to skip it, and a gate nobody runs protects nothing.  A red here means
# CUT THE WORK, never "raise the number": raising it is how it got to 45 minutes.
#
# ── The two things that make this safe ──────────────────────────────────────
#
# 1. IT POLLS, so it exits within one interval of the gate ending.  A watchdog that outlived its
#    own run would wake against a RECYCLED pid and cancel somebody else's gate.  Three facts must
#    all still hold before it fires: `.ci-running` exists, still names THIS gate, and that pid is
#    alive.
#
# 2. IT KILLS THE WHOLE TREE, breadth-first, deepest LAST.  Killing `make` alone is not a cancel:
#    measured, the recipe's `cargo nextest` kept running to completion after the verdict was
#    written, because it is a GRANDCHILD and `pkill -P` only reaches direct children.  Signalling
#    the process GROUP would cover it, but only when the gate leads its own group — true for the
#    `setsid` detached path (scripts/ci-run.sh) and an interactive shell's job, false when `make
#    ci` is a child of some other tool's shell, which is exactly where this was first measured.
#    So the group is used when the gate leads one and the descendant walk is the fallback.
#
# Never signals by NAME: every kill here is addressed to a pid descended from this gate, so a
# sibling checkout's gate on the same box is untouchable.
set -u

gate=${1:?usage: ci_budget.sh <gate-pid> <budget-secs> [interval]}
budget=${2:?usage: ci_budget.sh <gate-pid> <budget-secs> [interval]}
interval=${3:-15}
[ "$budget" = 0 ] && exit 0

# Every descendant of $1, parents BEFORE children; the caller reverses to kill leaves first.
descendants() {
  local p=$1 kid
  for kid in $(pgrep -P "$p" 2>/dev/null); do
    echo "$kid"
    descendants "$kid"
  done
}

start=$(date +%s)
while sleep "$interval"; do
  read -r rec < .ci-running 2>/dev/null || exit 0   # gate finished and tidied up
  [ "$rec" = "$gate" ] || exit 0                    # a different run owns the file now
  kill -0 "$gate" 2>/dev/null || exit 0             # gate died on its own
  [ $(( $(date +%s) - start )) -ge "$budget" ] || continue

  if [ "$budget" -ge 60 ]; then over="$(( budget / 60 ))m"; else over="${budget}s"; fi
  {
    printf '\nCI-BUDGET: the gate passed %s — cancelling it (pid %s)\n' "$over" "$gate"
    printf 'CI-RESULT: CANCELLED — over the %s budget\n' "$over"
    printf '  CUT THE WORK: be critical about what the gate runs.  Raising the number is how\n'
    printf '  this became a 45-minute gate, and a gate that long stops being run at all.\n'
    printf '  While iterating use scripts/find_problems.sh --changed (or --subject <name>) —\n'
    printf '  seconds, not minutes.  make ci is the ONE run before committing, and the PR\n'
    printf '  re-runs the same gate on the same sha anyway.\n'
    printf '  A sibling checkout running its own gate roughly doubles this one; for that case\n'
    printf '  only, CI_BUDGET_SECS=... make ci raises it for a single run (CI_BUDGET.md).\n'
    printf '  WARNING: the next cargo test may fail with `undefined symbol: anon.*.llvm.*` — a\n'
    printf '  torn libloft.rlib from the mid-run kill.  Recover with `cargo clean -p loft`.\n'
  } >> result.txt
  rm -f .ci-running

  # Leaves first, so a parent cannot spawn more work while its children are being taken down.
  tree=$(descendants "$gate" | tail -r 2>/dev/null || descendants "$gate" | tac)
  for p in $tree; do kill -TERM "$p" 2>/dev/null; done
  pgid=$(ps -o pgid= -p "$gate" 2>/dev/null | tr -d ' ')
  if [ -n "$pgid" ] && [ "$pgid" = "$gate" ]; then kill -TERM "-$pgid" 2>/dev/null; fi
  kill -TERM "$gate" 2>/dev/null

  sleep 3
  for p in $tree $gate; do kill -0 "$p" 2>/dev/null && kill -KILL "$p" 2>/dev/null; done
  exit 0
done
