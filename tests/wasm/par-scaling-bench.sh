#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
# @PLN117 E2 — par() scaling gate.  A CPU-heavy par() must speed up with the
# wasm Web Worker pool size: par-time (parse overhead subtracted) decreases
# monotonically along the measured pool sizes up to $SPEEDUP_AT and reaches
# $MIN_SPEEDUP_PCT there, every run agrees on the value, and distinct-worker
# count tracks the pool size.  Prints the full curve (incl. the sequential
# fallback) for the record.
#
# Calibration (env): POOLS (default "1 2 4 8", ascending, first entry = the
# single-worker reference), SPEEDUP_AT (4), MIN_SPEEDUP_PCT (250), and the work
# size SCALING_WORK (falls back to WORK, then 2000000: one worker must compute for
# seconds, or the pool's fixed cost decides the curve instead of `par`).  Those
# defaults describe this 24-core dev box; the CI workflow lowers them to what a
# 4-vCPU runner can actually deliver rather than switching the gate off — see
# .github/workflows/browser-threads.yml.
# Exit 0 pass / 1 fail.
set -u
cd "$(dirname "$0")/../.."
. tests/wasm/gate-lib.sh
PORT="${PORT:-8768}"; ROOT=tests/wasm; REPORT="$(mktemp)"
WORK="${SCALING_WORK:-${WORK:-2000000}}"; ELEMS="${ELEMS:-48}"
POOLS="${POOLS:-1 2 4 8}"; SPEEDUP_AT="${SPEEDUP_AT:-4}"; MIN_SPEEDUP_PCT="${MIN_SPEEDUP_PCT:-250}"
BASE="${POOLS%% *}"
CHROME="$(command -v chromium || command -v chromium-browser || command -v google-chrome || echo chromium)"
[ -f "$ROOT/pkg-mt/loft.js" ] || { echo "SKIP: no threaded bundle — run 'make wasm-mt'"; exit 0; }
python3 "$ROOT/coi-server.py" "$PORT" "$ROOT" "$REPORT" >/dev/null 2>&1 &
SRV=$!; trap 'kill $SRV 2>/dev/null; rm -f "$REPORT"' EXIT; sleep 1
runP() { : > "$REPORT"; timeout $(( 70 * ${WAIT_SCALE:-1} )) "$CHROME" --headless=new --no-sandbox --disable-gpu \
  "http://127.0.0.1:$PORT/par-scaling-bench.html?threads=$1&work=$WORK&elems=$ELEMS&trials=2" >/dev/null 2>&1 & local ch=$!; await_report "$REPORT" "${2:-15}"; stop_browser $ch; head -1 "$REPORT"; }
# Speedup as a percentage of the single-worker time; 0 when a time is missing or
# zero, so a sub-millisecond measurement can't abort the gate on a divide.
pct() { [ "${2:-0}" -gt 0 ] 2>/dev/null && echo $(( $1 * 100 / $2 )) || echo 0; }
declare -A PAR DW VAL
echo "== par() scaling (work=$WORK elems=$ELEMS pools='$POOLS') =="
for p in $POOLS 0; do
  r="$(runP $p)"; echo "  $r"
  PAR[$p]="$(echo "$r" | grep -oP 'par_ms=\K\d+')"
  DW[$p]="$(echo "$r" | grep -oP 'dw=\K\d+')"
  VAL[$p]="$(echo "$r" | grep -oP 'value=\K\d+')"
  echo "$r" | grep -q 'success=true' || { echo "FAIL: P=$p unsuccessful"; exit 1; }
done
fail=0
# value stable across all pool sizes
for p in $POOLS 0; do
  [ "$p" = "$BASE" ] && continue
  [ "${VAL[$p]}" = "${VAL[$BASE]}" ] || { echo "FAIL: value drift P=$p (${VAL[$p]} vs ${VAL[$BASE]})"; fail=1; }
done
# distinct workers tracks pool size
for p in $POOLS; do [ "${DW[$p]:-0}" -ge "$p" ] 2>/dev/null || { echo "FAIL: P=$p ran on ${DW[$p]:-?} workers"; fail=1; }; done
# Monotonic along the measured pools up to SPEEDUP_AT.  Pools past that are
# measured and printed but not asserted: beyond the core count more workers stop
# buying wall-clock, which is a property of the machine, not of the dispatch.
prev=""
for p in $POOLS; do
  if [ -n "$prev" ]; then
    [ "${PAR[$p]}" -lt "${PAR[$prev]}" ] 2>/dev/null \
      || { echo "FAIL: no speedup ${prev}->${p} (${PAR[$prev]}->${PAR[$p]})"; fail=1; }
  fi
  prev="$p"
  [ "$p" = "$SPEEDUP_AT" ] && break
done
sp="$(pct "${PAR[$BASE]}" "${PAR[$SPEEDUP_AT]}")"
[ "$sp" -ge "$MIN_SPEEDUP_PCT" ] \
  || { echo "FAIL: ${SPEEDUP_AT}-worker speedup ${sp}% < ${MIN_SPEEDUP_PCT}%"; fail=1; }
if [ $fail -eq 0 ]; then
  curve=""
  for p in $POOLS; do curve="$curve ${p}w=${PAR[$p]}ms($(pct "${PAR[$BASE]}" "${PAR[$p]}")%)"; done
  echo "PASS: par() scales —$curve; value=${VAL[$BASE]} stable; fallback(P0)=${PAR[0]}ms~=${BASE}w"
fi
exit $fail
