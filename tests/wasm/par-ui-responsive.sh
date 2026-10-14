#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
# @PLN117 E1 (@C3) — off-main-thread UI-responsiveness gate.  A requestAnimationFrame
# loop runs one heavy par() per frame; with the Web Worker pool each frame's par
# finishes faster, so the main thread is BLOCKED for less of every frame.  That
# shorter block IS "the UI is not blocked", and it is what this gate asserts:
# threaded avg_block_ms <= sequential / (MIN_BLOCK_RATIO_X10/10), with the long
# blocks shorter too (p90 of the steady frames) and the value stable.
#
# The long-block half compares p90_block_ms, not the worst block.  The worst block
# was ALWAYS frame 1 in both legs, a one-time cold start: a warm-up compile of an
# empty program absorbs ~100 ms of it in both legs, and the threaded leg adds ~50 ms
# for its workers' first heavy par (loft#1832).  Comparing two cold starts flipped
# from run to run (955 vs 954 ms on main, 2026-09-30) while the average never did.
# The page now reports that frame apart (first_block_ms, never gated) and computes
# max and p90 over the frames after it, as max_gap_ms already skips the first gap.
# p90 rather than the steady max: a single runner stall in either leg is one sample
# and decides a max, but not the 90th percentile, which still says that the long
# blocks of the threaded leg are shorter.
#
# It does NOT gate on the frame count, though it still reports it.  How often
# rAF fires is the browser's decision, not the work's: a page Chrome treats as
# hidden gets rAF at ~1 Hz whatever runs underneath.  On the CI runner that
# throttle landed on one leg at a time and decided the verdict — the threaded
# leg read a frozen `frames=12 max_gap_ms=1017` (1000 ms + one 60 Hz tick,
# identical to the millisecond across five runs on different runners) while
# the sequential leg varied 11..26 with the compute.  Whichever leg it hit lost,
# so the ratio measured the throttle and not par: it failed runs where par was
# fine, and passed runs by throttling the OTHER leg.  Block time is immune —
# a throttled loop delivers fewer frames, each blocking exactly as long.
#
# The sparse-scheduling NOTE is therefore an observation, never a failure: a leg
# whose idle time dwarfs its work was scheduled sparsely by the browser, which
# says nothing about par.
#
# Calibration (env): THREADS (default 8 workers) and MIN_BLOCK_RATIO_X10 (15,
# i.e. 1.5x) describe this 24-core dev box; the CI workflow lowers both for a
# 4-vCPU runner — see .github/workflows/browser-threads.yml.
#
# UI_WORK / UI_WINDOW size the SAMPLE.  They fall back to WORK / WINDOW so
# running this gate by hand still works; the separate names keep them from
# retuning par-scaling-bench, which reads WORK too.  MIN_FRAMES is a liveness
# floor only — it catches a loop that never ran, which is the one thing a frame
# count still says reliably.
# Exit 0 pass / 1 fail.
set -u
cd "$(dirname "$0")/../.."
. tests/wasm/gate-lib.sh
PORT="${PORT:-8764}"; ROOT=tests/wasm; REPORT="$(mktemp)"
WORK="${UI_WORK:-${WORK:-200000}}"; ELEMS="${ELEMS:-48}"; WINDOW="${UI_WINDOW:-${WINDOW:-3000}}"
THREADS="${THREADS:-8}"; MIN_BLOCK_RATIO_X10="${MIN_BLOCK_RATIO_X10:-15}"; MIN_FRAMES="${MIN_FRAMES:-5}"
CHROME="$(command -v chromium || command -v chromium-browser || command -v google-chrome || echo chromium)"
[ -f "$ROOT/pkg-mt/loft.js" ] || { echo "SKIP: no threaded bundle — run 'make wasm-mt'"; exit 0; }
python3 "$ROOT/coi-server.py" "$PORT" "$ROOT" "$REPORT" >/dev/null 2>&1 &
SRV=$!; trap 'kill $SRV 2>/dev/null; rm -f "$REPORT"' EXIT; sleep 1
run() { : > "$REPORT"; timeout $(( 40 * ${WAIT_SCALE:-1} )) "$CHROME" --headless=new --no-sandbox --disable-gpu \
  "http://127.0.0.1:$PORT/par-ui-responsive.html?threads=$1&work=$WORK&elems=$ELEMS&window=$WINDOW" >/dev/null 2>&1 & local ch=$!; await_report "$REPORT" $(( WINDOW/1000 + 6 )); stop_browser $ch; head -1 "$REPORT"; }
echo "== UI responsiveness (heavy par per rAF frame, window=${WINDOW}ms, threads=$THREADS) =="
TH="$(run "$THREADS")"; echo "  threaded:   $TH"
SQ="$(run 0)"; echo "  sequential: $SQ"
fail=0
echo "$TH" | grep -q '^E1 ' && echo "$SQ" | grep -q '^E1 ' || { echo "FAIL: a run did not report"; exit 1; }
num() { echo "$1" | grep -oP "$2=\K[0-9.]+"; }
# Block time in tenths of a ms, so the ratio stays in integer arithmetic.
# Read back with `10#` — a sub-millisecond block gives a leading zero (`0.8` ->
# `08`), which bash would otherwise parse as octal and reject outright.
tb=$(num "$TH" avg_block_ms); sb=$(num "$SQ" avg_block_ms)
tbx=${tb%.*}$(printf '%s' "${tb#*.}" | cut -c1); sbx=${sb%.*}$(printf '%s' "${sb#*.}" | cut -c1)
tm=$(num "$TH" max_block_ms); sm=$(num "$SQ" max_block_ms)
tp=$(num "$TH" p90_block_ms); sp=$(num "$SQ" p90_block_ms)
t1=$(num "$TH" first_block_ms); s1=$(num "$SQ" first_block_ms)
tf=$(num "$TH" frames); sf=$(num "$SQ" frames)
tw=$(num "$TH" work_ms); sw=$(num "$SQ" work_ms)
tg=$(num "$TH" max_gap_ms); sg=$(num "$SQ" max_gap_ms)
tv=$(num "$TH" value); sv=$(num "$SQ" value)
[ "$tv" = "$sv" ] || { echo "FAIL: value drift threaded=$tv sequential=$sv"; fail=1; }
# Liveness: a loop that never ran cannot be judged on its averages.
for leg in "threaded:$tf" "sequential:$sf"; do
  [ "${leg#*:}" -ge "$MIN_FRAMES" ] 2>/dev/null \
    || { echo "FAIL: ${leg%%:*} delivered ${leg#*:} frames (< $MIN_FRAMES) — the loop never ran"; fail=1; }
done
# The gate: threading must shorten the main-thread block.
[ $(( 10#${tbx:-999999} * MIN_BLOCK_RATIO_X10 )) -le $(( 10#${sbx:-0} * 10 )) ] 2>/dev/null \
  || { echo "FAIL: threaded avg block (${tb}ms) is not ${MIN_BLOCK_RATIO_X10}/10 x shorter than sequential (${sb}ms)"; fail=1; }
[ "${tp:-999999}" -lt "${sp:-0}" ] 2>/dev/null \
  || { echo "FAIL: threaded p90 block (${tp}ms) not < sequential (${sp}ms) — steady frames, the first excluded"; fail=1; }
# Reported, never gated: a leg that spent most of the window NOT computing was
# scheduled sparsely by the browser.  This is the state that used to decide the
# old frame-count verdict; naming it keeps it visible now that it cannot.
note_sparse() {
  [ "$(( ${3%.*} * 2 ))" -lt "$WINDOW" ] \
    && echo "  NOTE: $1 leg computed for ${3}ms of the ${WINDOW}ms window (frames=$2) — rAF scheduled it sparsely"
  return 0
}
note_sparse threaded "$tf" "$tw"
note_sparse sequential "$sf" "$sw"
[ $fail -eq 0 ] && echo "PASS: threading shortens the main-thread block — avg ${tb}ms vs ${sb}ms, p90 ${tp}ms vs ${sp}ms (worst ${tm}ms vs ${sm}ms, first frame ${t1}ms vs ${s1}ms); frames ${tf} vs ${sf}, gap ${tg}ms vs ${sg}ms; value=$tv"
exit $fail
