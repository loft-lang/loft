#!/bin/bash
# disk_headroom.sh — make room for a gate BEFORE it runs, so a full disk never fakes a red.
#
#   scripts/disk_headroom.sh [--min-gb N] [--floor-gb M] [--scratch DIR]
#
# A gate that starts on a nearly full disk fails as CODE: truncated object files,
# `ld: cannot find lib<dep>-<hash>.rlib`, `FAIL unknown-mode` after `low space` lines.
# Measured 2026-09-28: 71 of 5312 tests red on a 96 GB disk with 2 MB free — one run's
# fixtures (23 GB) and cargo's incremental cache (9 GB) had filled it, and every native
# compile in the run linked against files being truncated under it.  This reclaims, in the
# order of what it costs to lose, until --min-gb (20) is free, re-measuring after each step:
#
#   1. loft's own dead or aged scratch      scripts/sweep_scratch.sh — the standing rules
#                                           (RUN_BOUNDS.md § Scratch hygiene), nothing newer
#   2. cargo's incremental caches           target/*/incremental — costs a rebuild's time
#   3. THIS checkout's gate scratch, whole   only when no gate of this checkout is ALIVE (the
#                                           gate's pid file and `.ci-running`, liveness-tested,
#                                           never the file's mere existence): a finished run's
#                                           fixtures are garbage and the run about to start
#                                           writes fresh ones — what is lost is the native
#                                           test cache, so this step waits its turn
#   4. cargo artefacts no build used in 14 days   `cargo sweep --time 14`, when installed
#
# Prints one line when it acted or is still short, nothing when nothing needed doing; exits 1
# only below --floor-gb (2), so the gate stops HERE instead of running to a fiction.  Never
# another program's files, never a sibling checkout's scratch (the tag names this one).
set -u
min_gb=20; floor_gb=2; scratch=""
while [ $# -gt 0 ]; do
  case "$1" in
    --min-gb) min_gb="$2"; shift;;
    --floor-gb) floor_gb="$2"; shift;;
    --scratch) scratch="$2"; shift;;
    *) echo "usage: $0 [--min-gb N] [--floor-gb M] [--scratch DIR]" >&2; exit 2;;
  esac
  shift
done
REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$REPO_ROOT" || exit 2
# The same tag find_problems.sh names its scratch, log and pid file by.
REPO_HASH=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
REPO_SLUG=$(printf '%s' "$(basename "$REPO_ROOT")" | tr -c 'A-Za-z0-9._-' '_')
REPO_TAG="$REPO_SLUG.$REPO_HASH"
[ -n "$scratch" ] || scratch="/var/tmp/loft-test-scratch-$REPO_TAG"
PID_FILE="/tmp/loft_test.$REPO_TAG.pid"

# `-Pk` is POSIX (GNU and BSD df both take it); `-B1G` is GNU-only and answered nothing on
# macOS, so every comparison below failed and the guard could neither sweep nor stop a gate.
free_gb() { df -Pk "$REPO_ROOT" | awk 'NR == 2 { print int($4 / 1048576) }'; }
alive() { local p; p=$(cat "$1" 2>/dev/null); [ -n "$p" ] && kill -0 "$p" 2>/dev/null; }

start=$(free_gb)
[ "$start" -ge "$min_gb" ] && exit 0
steps=()
# 1. the standing rules: dead pids, aged entries, the session prune.
scripts/sweep_scratch.sh --sessions "$scratch" "${TMPDIR:-$HOME/.cache/tmp}" /tmp >/dev/null 2>&1
steps+=("scratch swept")
if [ "$(free_gb)" -lt "$min_gb" ]; then
  # 2. incremental caches: a rebuild's time, nothing else.
  for d in target/*/incremental; do [ -d "$d" ] && rm -rf "$d"; done
  steps+=("incremental caches dropped")
fi
if [ "$(free_gb)" -lt "$min_gb" ] && [ -d "$scratch" ]; then
  # 3. this checkout's gate scratch, whole — only with no gate of this checkout alive.
  if alive "$PID_FILE" || alive .ci-running; then
    steps+=("gate scratch kept: a gate of this checkout is running")
  else
    find "$scratch" -mindepth 1 -maxdepth 1 -exec rm -rf {} + 2>/dev/null
    steps+=("gate scratch emptied")
  fi
fi
if [ "$(free_gb)" -lt "$min_gb" ] && cargo sweep --version >/dev/null 2>&1; then
  # 4. cargo artefacts no build touched in two weeks.
  cargo sweep --time 14 >/dev/null 2>&1
  steps+=("cargo swept")
fi
end=$(free_gb)
line="disk: ${end} GB free (was ${start} GB; $(IFS=,; echo "${steps[*]}"))"
if [ "$end" -lt "$floor_gb" ]; then
  echo "$line — below ${floor_gb} GB, the gate would fake failures; free room by hand:" >&2
  echo "  make sweep-target (stale cargo artefacts), \`loft cache prune\` (the native build cache)," >&2
  echo "  and \`du -sh target /var/tmp/loft-test-scratch-* ~/.loft\` for what is left" >&2
  exit 1
fi
[ "$end" -lt "$min_gb" ] && line="$line — still under ${min_gb} GB"
echo "$line"
exit 0
