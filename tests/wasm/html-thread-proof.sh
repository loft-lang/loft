#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN117 (@C3) — headless gate for a `loft --html` page that uses `par`: the page,
# which embeds its wasm as base64 in a plain <script> and knows nothing about
# wasm-bindgen, computes what the interpreter computes.
#
# Two claims, each measured rather than assumed:
#   1. `loft --html` on a par-using program builds a page that computes the same
#      value as the interpreter, also on a host without cross-origin isolation.
#   2. `--no-threads` builds a page that computes it too.
#
# RETIRED: the threaded half (the page dispatching `par` over >= 2 Web Workers, and
# falling back to one on a host without isolation).  The page is built through the C
# backend, whose wasm build is unthreaded; the threaded page existed only on the
# rustc writer's route, which rebuilt std with atomics on nightly, and that route is
# gone.  Threaded `par` in the browser is still proven on the wasm-bindgen bundle
# by the other four gates (par-thread-proof, par-memory-proof, par-scaling-bench,
# par-ui-responsive).
#
# Requires: a built loft, the C backend's wasm toolchain, python3 and a
# headless-capable chromium.  Exits 0 on pass, 1 on fail.
#
#   tests/wasm/html-thread-proof.sh
set -u
cd "$(dirname "$0")/../.."
. tests/wasm/gate-lib.sh
LOFT=target/release/loft
PORT="${PORT:-8794}"
PLAIN_PORT="${PLAIN_PORT:-8795}"
WORK="$(mktemp -d)"
REPORT="$(mktemp)"
CHROME="$(command -v chromium || command -v chromium-browser || command -v google-chrome || echo chromium)"
trap 'rm -rf "$WORK" "$REPORT"' EXIT
if [ ! -x "$LOFT" ]; then echo "SKIP: no $LOFT — run 'cargo build --release' first"; exit 0; fi
if ! command -v "$CHROME" >/dev/null 2>&1; then echo "SKIP: no headless chromium"; exit 0; fi

cp tests/wasm/html-thread-proof.html "$WORK/driver.html"
cat > "$WORK/par.loft" <<'LOFT'
fn heavy(x: integer) -> integer {
    acc = 0;
    for i in 0..40000 { acc += (x * i + i) % 7; }
    acc % 1000 + x
}

fn main() {
    data = [0];
    for i in 1..48 { data += [i]; }
    sum = 0;
    for a in data par(b = heavy(a), 8) { sum += b; }
    println("par_sum={sum}");
}
LOFT

# The reference: what this program means, decided by the interpreter.
REF="$(LOFT_TIMEOUT=$(( 120 * ${WAIT_SCALE:-1} )) "$LOFT" "$WORK/par.loft" | grep -oP 'par_sum=\K\d+')"
echo "== reference (interpreter) par_sum=$REF =="
[ -n "$REF" ] || { echo "FAIL: interpreter produced no par_sum"; exit 1; }

echo "== building pages =="
# A page build is a C compile for wasm32; the bound rides WAIT_SCALE like every wait.
BUILD_TIMEOUT=$(( 300 * ${WAIT_SCALE:-1} ))
# Keep the build output.  Discarding both streams cost a nightly diagnosis on
# 2026-07-31: the gate reported only "loft --html (threaded) failed" and threw
# away the `error[E0514]: found crate `loft` compiled by an incompatible version
# of rustc` underneath it, so the CI log named the symptom and nothing else.  A
# gate that hides the error it exists to detect is worse than no gate: it reports
# a regression with no way to act on it.  Quiet on success, the whole thing on
# failure — the last 40 lines, because a compiler failure's useful part is the end.
build_page() {  # $1 = label, $2 = output html, and any further args
  local label="$1" out="$2"; shift 2
  local log="$WORK/build-$label.log"
  if ! LOFT_TIMEOUT=$BUILD_TIMEOUT "$LOFT" --html "$out" "$@" > "$log" 2>&1; then
    echo "FAIL: loft --html ($label) failed"
    echo "---- last 40 lines of $log ----"
    tail -40 "$log"
    echo "---- end ----"
    exit 1
  fi
}
build_page default "$WORK/default.html" "$WORK/par.loft"
build_page no-threads "$WORK/sequential.html" --no-threads "$WORK/par.loft"

run() {  # $1 = server script, $2 = port, $3 = page, $4 = seconds to wait
  : > "$REPORT"
  python3 "$1" "$2" "$WORK" "$REPORT" >/dev/null 2>&1 &
  local srv=$!
  sleep 1
  timeout $(( 60 * ${WAIT_SCALE:-1} )) "$CHROME" --headless=new --no-sandbox --disable-gpu \
    "http://127.0.0.1:$2/driver.html?page=$3" >/dev/null 2>&1 &
  local ch=$!
  await_report "$REPORT" "$4"
  stop_browser $ch
  kill $srv 2>/dev/null
  wait $srv 2>/dev/null
  head -1 "$REPORT"
}

echo "== default page, cross-origin-isolated host =="
T="$(run tests/wasm/coi-server.py "$PORT" default.html 20)"; echo "  $T"
echo "== same page, host WITHOUT cross-origin isolation =="
F="$(run tests/wasm/html-plain-server.py "$PLAIN_PORT" default.html 20)"; echo "  $F"
echo "== --no-threads page =="
S="$(run tests/wasm/coi-server.py "$PORT" sequential.html 18)"; echo "  $S"

fail=0
value_of() { echo "$1" | grep -oP 'par_sum=\K\d+'; }

[ "$(value_of "$T")" = "$REF" ] \
  || { echo "FAIL: default page value $(value_of "$T") != interpreter $REF"; fail=1; }
[ "$(value_of "$F")" = "$REF" ] \
  || { echo "FAIL: non-isolated host value $(value_of "$F") != interpreter $REF"; fail=1; }
[ "$(value_of "$S")" = "$REF" ] \
  || { echo "FAIL: --no-threads value $(value_of "$S") != interpreter $REF"; fail=1; }

[ $fail -eq 0 ] && echo "PASS: loft --html par page correct on both hosts; --no-threads correct; par_sum=$REF"
exit $fail
