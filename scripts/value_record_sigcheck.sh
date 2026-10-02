#!/bin/bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN180 — does the IR-phase value-record pass cost --native a tuple anywhere?
#
# For each program: the native Rust signatures `loft introspect` prints with the pass off
# (LOFT_NO_IR_VALUE_RECORD=1) and on.  A function whose signature carries FEWER scalar tuples
# (in its parameters or its return) with the pass on lost a register form native had — the
# regression the first materialisation step caused on three bench functions.  Silent when
# nothing is lost; exit 1 with the pairs otherwise.
#
#   scripts/value_record_sigcheck.sh bench/*/bench.loft
set -u
LOFT=${LOFT:-target/release/loft}
lost=0
for f in "$@"; do
  off=$(LOFT_NO_IR_VALUE_RECORD=1 timeout 120 "$LOFT" introspect "$f" 2>/dev/null | grep -E '^fn [a-z_0-9]+\(cell')
  on=$(timeout 120 "$LOFT" introspect "$f" 2>/dev/null | grep -E '^fn [a-z_0-9]+\(cell')
  out=$(OFF="$off" ON="$on" python3 - "$f" <<'PY'
import os, re, sys
def sigs(text):
    out = {}
    for line in text.splitlines():
        m = re.match(r'fn (\w+)\(', line)
        if m:
            out[m.group(1)] = line.split('{')[0]
    return out
tuple_re = re.compile(r'\((?:f64|i64|f32|u8|bool)(?:, (?:f64|i64|f32|u8|bool))+\)')
off, on = sigs(os.environ['OFF']), sigs(os.environ['ON'])
for name, sig in off.items():
    if name in on and len(tuple_re.findall(on[name])) < len(tuple_re.findall(sig)):
        print(f"{sys.argv[1]}: {name}\n  off: {sig}\n  on:  {on[name]}")
PY
)
  if [ -n "$out" ]; then echo "$out"; lost=1; fi
done
exit $lost
