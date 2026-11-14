#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN91 — the bytecode runner's step loop written by hand (hand_steps.c): the target the
# generator and the C backend are to reach.  Emits run_fill.loft's C, splices the hand loop in
# front of the generated step loop, compiles it with the C runtime shim (plus one accessor for
# the stack store's frame base: the shim keeps its own store table), and times collatz on it,
# on the generated runner built the same way, and on `loft --interpret`.
#
#   selfhost/bench/hand/build.sh [out dir]      (default: a scratch directory)
set -euo pipefail
if [ "${1:-}" = "--help" ]; then
  echo "usage: selfhost/bench/hand/build.sh [out dir] — the hand-written step loop, the generated runner and loft --interpret on collatz"
  exit 0
fi
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
loft="$root/target/release/loft"
out=${1:-$(mktemp -d)}
mkdir -p "$out"
cd "$root/selfhost"
LOFT_NO_WARNINGS=1 "$loft" --native-c --ship --native-emit "$out/run_fill.c" bench/run_fill.loft >/dev/null
"$loft" --emit-image "$out/c.store" kernels/collatz.loft
"$loft" --emit-run-image "$out/c.run" kernels/collatz.loft
sed "s|#include \"../../../src/c/|#include \"$root/src/c/|" crt/shim/crt.c > "$out/crt.c"
printf '\nuint8_t *loft_crt_stack_base(int64_t store) { return loft_crt_slot_at(store, 0); }\n' >> "$out/crt.c"
python3 - "$out/run_fill.c" "$here/hand_steps.c" "$out/run_fill_hand.c" <<'PY'
import re, sys
s = open(sys.argv[1]).read()
hand = open(sys.argv[2]).read()
m = re.search(r'\nstatic [a-z0-9_ ]+ n_call_m[0-9a-f]+__fr\([^;{]*\) \{\n', s)
if not m:
    sys.exit("no generated call body to splice the hand loop in front of")
s = s[:m.start() + 1] + hand + s[m.start() + 1:]
loop = re.search(r'\n    for \(;;\) \{\n        (t\d+) = loft_get_int\((v\d+_s), 0\);\n        if \(\1 != (v\d+_end)\) \{', s)
trace = re.search(r'(v\d+_trace) = \(uint8_t\)', s)
if not loop or not trace:
    sys.exit("the generated step loop changed shape; update build.sh")
s = s[:loop.start()] + f'\n    for (;;) {{\n        if (!({trace.group(1)} == 1)) loft_hand_steps({loop.group(2)}, {loop.group(3)});' + s[loop.start() + len('\n    for (;;) {'):]
open(sys.argv[3], 'w').write(s)
PY
cc -std=c99 -w -O3 -c "$out/crt.c" -o "$out/crt.o"
cc -std=c99 -w -O3 "$out/run_fill_hand.c" "$out/crt.o" -o "$out/run_hand" -lm -lpthread
cc -std=c99 -w -O3 "$out/run_fill.c" "$out/crt.o" -o "$out/run_gen" -lm -lpthread
pin=(); command -v taskset >/dev/null && pin=(taskset -c 0)
echo "hand-coded loop : $("${pin[@]}" "$out/run_hand" "$out/c.store" "$out/c.run" n_longest 1 100000)  (answer, µs)"
echo "generated runner: $("${pin[@]}" "$out/run_gen" "$out/c.store" "$out/c.run" n_longest 1 100000)"
echo "loft --interpret: $("${pin[@]}" "$loft" --interpret kernels/collatz.loft)"
