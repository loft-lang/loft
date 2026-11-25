#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @C3 — headless gate for browser threading: a `loft --html` page, which embeds
# its wasm as base64 in a plain <script>, runs `par` on real Web Worker threads.
# The page is the C backend's: its module is built for wasm32-wasip1-threads and
# runs `par` on a pool of workers the page starts before the program
# (doc/claude/plans/227-par-on-c-pages.md).
#
# Five claims, each measured rather than assumed:
#   1. `loft --html` on a par-using program emits a threaded bundle and the page
#      dispatches `par` across >= 2 distinct workers.
#   2. It computes the same value as the interpreter.
#   3. The SAME bundle on a host without cross-origin isolation falls back to one
#      worker and still computes that value (arc D — never breaks).
#   4. `--no-threads` produces an unthreaded bundle that also computes it.
#   5. On the threaded page, each shape the pool handles apart answers as the
#      interpreter does: records back from the workers, a body that prints (its
#      lines before the program's next one), a `par` inside a worker, a
#      `parallel` block, text-building bodies, fifty `par`s in a row, and more
#      threads asked for than the pool has.
#
# Requires: a built loft, python3, a headless-capable chromium, and a WASI
# sysroot for wasm32-wasip1-threads (`make wasi-threads-sysroot`).  Without the
# sysroot the page builds unthreaded: a SKIP here, a FAIL on CI, which installs
# it.  Exits 0 on pass, 1 on fail.
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

cat > "$WORK/matrix.loft" <<'LOFT'
struct List { v: integer, next: List? }
struct Job { n: integer }
fn build(j: const Job) -> List {
  h = List { v: 0 };
  for i in 1..j.n { h = List { v: i, next: h }; }
  h
}
fn total(l: List?) -> integer {
  if !l { return 0; }
  l.v + total(l.next)
}
fn words(x: integer) -> text {
  s = "";
  for i in 0..200 { s += "w{(x * 31 + i) % 97},"; }
  s
}
fn noisy(x: integer) -> integer {
  println("in-body {x}");
  x * 3
}
fn sq(a: integer) -> integer { a * a }
fn addr(a: integer, r: integer) -> integer { a + r }
fn dbl(a: integer) -> integer { a * 2 }
fn inner_sum(x: integer) -> integer {
  d = [x, x + 1, x + 2, x + 3];
  s = 0;
  for a in d par(b = sq(a), 4) { s += b; }
  s
}
fn main() {
  jobs = [Job { n: 300 }, Job { n: 2 }, Job { n: 100 }, Job { n: 1 }, Job { n: 50 }];
  s1 = "";
  for jb in jobs par(l = build(jb), 3) { s1 += "{total(l)}/"; }
  println("c1={s1}");
  d2 = [1, 2, 3, 4];
  t2 = 0;
  for a in d2 par(b = noisy(a), 4) { t2 += b; }
  println("c2={t2}");
  d3 = [1, 10, 100, 1000];
  t3 = 0;
  for a in d3 par(b = inner_sum(a), 4) { t3 += b; }
  println("c3={t3}");
  parallel {
    println("c4=arm");
    println("c4=arm");
  }
  d5 = [];
  for i in 0..64 { d5 += [i]; }
  n5 = 0;
  for a in d5 par(w = words(a), 8) { n5 += len(w); }
  println("c5={n5}");
  t6 = 0;
  for r in 0..50 {
    for a in d2 par(b = addr(a, r), 4) { t6 += b; }
  }
  println("c6={t6}");
  d7 = [];
  for i in 0..200 { d7 += [i]; }
  t7 = 0;
  for a in d7 par(b = dbl(a), 64) { t7 += b; }
  println("c7={t7}");
}
LOFT

# The reference: what this program means, decided by the interpreter.
REF="$(LOFT_TIMEOUT=$(( 120 * ${WAIT_SCALE:-1} )) "$LOFT" "$WORK/par.loft" | grep -oP 'par_sum=\K\d+')"
echo "== reference (interpreter) par_sum=$REF =="
[ -n "$REF" ] || { echo "FAIL: interpreter produced no par_sum"; exit 1; }

echo "== building pages =="
# A page builds in seconds; the bound rides WAIT_SCALE for a slow runner.
BUILD_TIMEOUT=$(( 300 * ${WAIT_SCALE:-1} ))
# Keep the build output.  Discarding both streams cost a nightly diagnosis on
# 2026-07-31: the gate reported only "loft --html (threaded) failed" and threw
# away the `error[E0514]: found crate `loft` compiled by an incompatible version
# of rustc` underneath it, so the CI log named the symptom and nothing else.  A
# gate that hides the error it exists to detect is worse than no gate: it reports
# a regression with no way to act on it.  Quiet on success, the whole thing on
# failure — the last 40 lines, because a rustc failure's useful part is the end.
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
build_page threaded "$WORK/threaded.html" "$WORK/par.loft"
if grep -q "runs \`par\` on the main thread" "$WORK/build-threaded.log"; then
  if [ -n "${CI:-}" ]; then
    echo "FAIL: no wasm32-wasip1-threads sysroot on this runner — the workflow installs one"
    exit 1
  fi
  echo "SKIP: no wasm32-wasip1-threads sysroot — make wasi-threads-sysroot"; exit 0
fi
build_page no-threads "$WORK/sequential.html" --no-threads "$WORK/par.loft"
build_page matrix "$WORK/matrix.html" "$WORK/matrix.loft"
# Every `cN=` value, in order: hand-checked once against the cells' arithmetic
# (c1 the sums n(n-1)/2, c3 4x²+12x+14 summed, c6 500+4·1225, c7 2·19900).
MREF="$(LOFT_TIMEOUT=$(( 120 * ${WAIT_SCALE:-1} )) "$LOFT" "$WORK/matrix.loft" | grep -oP 'c\d=\S+' | tr '\n' ' ')"
[ "$(echo "$MREF" | wc -w)" = 8 ] || { echo "FAIL: the interpreter answered the matrix with <<$MREF>>"; exit 1; }

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

echo "== threaded page, cross-origin-isolated host =="
T="$(run tests/wasm/coi-server.py "$PORT" threaded.html 20)"; echo "  $T"
echo "== same page, host WITHOUT cross-origin isolation =="
F="$(run tests/wasm/html-plain-server.py "$PLAIN_PORT" threaded.html 20)"; echo "  $F"
echo "== --no-threads page =="
S="$(run tests/wasm/coi-server.py "$PORT" sequential.html 18)"; echo "  $S"
echo "== the matrix, threaded page, cross-origin-isolated host =="
M="$(run tests/wasm/coi-server.py "$PORT" matrix.html 20)"; echo "  $M"

fail=0
value_of() { echo "$1" | grep -oP 'par_sum=\K\d+'; }
workers_of() { echo "$1" | grep -oP 'distinct_workers=\K\d+'; }

dw="$(workers_of "$T")"
[ "${dw:-0}" -ge 2 ] 2>/dev/null \
  || { echo "FAIL: threaded page dispatched over ${dw:-no} workers, expected >= 2"; fail=1; }
[ "$(value_of "$T")" = "$REF" ] \
  || { echo "FAIL: threaded value $(value_of "$T") != interpreter $REF"; fail=1; }
[ "$(workers_of "$F")" = "1" ] \
  || echo "WARN: non-isolated host reported distinct_workers=$(workers_of "$F"), expected 1"
[ "$(value_of "$F")" = "$REF" ] \
  || { echo "FAIL: non-isolated fallback value $(value_of "$F") != interpreter $REF"; fail=1; }
[ "$(value_of "$S")" = "$REF" ] \
  || { echo "FAIL: --no-threads value $(value_of "$S") != interpreter $REF"; fail=1; }

mout="$(echo "$M" | grep -oP 'out<<\K.*(?=>>)')"
[ "$(echo "$mout" | grep -oP 'c\d=\S+' | tr '\n' ' ')" = "$MREF" ] \
  || { echo "FAIL: the matrix answered <<$mout>>, the interpreter $MREF"; fail=1; }
[ "$(echo "${mout%%c2=*}" | grep -o 'in-body' | wc -l)" = 4 ] \
  || { echo "FAIL: a par body's lines did not all come before the program's next line: <<$mout>>"; fail=1; }

[ $fail -eq 0 ] && echo "PASS: loft --html threaded par over $dw workers; fallback + --no-threads correct; the matrix as the interpreter; par_sum=$REF"
exit $fail
