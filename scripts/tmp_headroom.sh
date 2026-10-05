#!/bin/bash
# tmp_headroom.sh — the turn-end sweep: reclaim loft's temp entries once nothing uses them.
#
#   scripts/tmp_headroom.sh            (the agent harness's Stop / SubagentStop hook)
#
# Two filesystems fill during a session and each kills the box differently: /tmp (a tmpfs on
# some boxes, 16 GB) with the native suites' per-file caches and a session's own scratch,
# where a full one stops the harness itself from capturing a command's output; and the
# disk under the checkout with a gate's fixtures and cargo's caches, where a full one fakes
# a red gate.  Measured 2026-09-30: both filled twice in one session.
#
#   1. the standing sweep over /tmp and $TMPDIR   scripts/sweep_scratch.sh — dead pids, entries
#                                                 aged past a day, dead agent sessions
#   2. /tmp below --tmp-min-gb (4) and no test run alive: the suites' native binary caches
#                                                 (`loft_native_cache_<checkout>/`, the
#                                                 `loft_test_native_*_bin` entries), which the
#                                                 next run rebuilds; skipped while cargo, nextest
#                                                 or rustc runs, since a live run writes into them
#   3. the checkout's disk below 20 GB              scripts/disk_headroom.sh — its own order,
#                                                 never a live gate's scratch
#
# Prints one line per step that acted, nothing when nothing needed doing.  Never another
# program's files (the sweep matches loft's own names only).
set -u
tmp_min_gb=4
while [ $# -gt 0 ]; do
  case "$1" in
    --tmp-min-gb) tmp_min_gb="$2"; shift;;
    *) echo "usage: $0 [--tmp-min-gb N]" >&2; exit 2;;
  esac
  shift
done
REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$REPO_ROOT" || exit 2
free_gb() { df -Pk "$1" 2>/dev/null | awk 'NR == 2 { print int($4 / 1048576) }'; }
dirs=(/tmp)
[ -n "${TMPDIR:-}" ] && [ "$TMPDIR" != /tmp ] && dirs+=("$TMPDIR")
scripts/sweep_scratch.sh --sessions "${dirs[@]}"
if [ "$(free_gb /tmp)" -lt "$tmp_min_gb" ]; then
  if pgrep -x cargo-nextest >/dev/null 2>&1 || pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1; then
    echo "tmp_headroom: /tmp has $(free_gb /tmp) GB free but a build or test run is alive — today's caches kept"
  else
    # The suites' binary caches: the per-checkout native cache (14 GB measured after one
    # direct nextest run) and the per-program `--tests --native` caches.  A cache, so the
    # next run rebuilds what it needs; `--days` cannot reach them (find's day is a whole one).
    for c in "${dirs[@]/%//loft_native_cache_}"*; do [ -d "$c" ] && rm -rf -- "$c"; done
    for c in "${dirs[@]/%//loft_test_native_}"*_bin; do [ -e "$c" ] && rm -rf -- "$c"; done
    echo "tmp_headroom: the suites' native caches dropped, $(free_gb /tmp) GB free"
  fi
fi
scripts/disk_headroom.sh
# When it last ran — the hook is silent when nothing needed doing, so this is its receipt.
date -u +%FT%TZ > /tmp/loft_tmp_headroom.last 2>/dev/null
exit 0
