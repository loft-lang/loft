#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN91 strand 12 — one port's check: compile-check the loft-written C generator on the
# interpreter, build it with `--ship`, then compare its `bodies` layer with the Rust writer's
# over the whole corpus under the plain switches (cgen_w.loft's PLAIN_SWITCHES) and under the
# default ones.  It prints `N of M units agree`, so a generator that dies on some units cannot
# pass as "0 different".
#
#   selfhost/bench/cgen_cycle.sh [TOP]      TOP: how many refusal reasons to list (default 10)
#
# The built generator and the logs go to $LOFT_CGEN_OUT (default: $TMPDIR/loft-cgen).
set -u
case "${1:-}" in
  -h|--help) sed -n '4,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac
top="${1:-10}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
out="${LOFT_CGEN_OUT:-${TMPDIR:-/tmp}/loft-cgen}"
mkdir -p "$out"
loft="$root/target/release/loft"
cd "$root" || exit 2
errors=$(LOFT_NO_CACHE=1 LOFT_NO_NATIVE_LIBS=1 "$loft" --interpret --lib selfhost/src --lib selfhost/crt/src \
  selfhost/front/cgen.loft 2>&1 | grep -A6 "^error" | head -30)
if [ -n "$errors" ]; then
  printf '%s\n' "$errors"
  exit 1
fi
LOFT_NO_CACHE=1 "$loft" --native-c --ship --out "$out/gen" --lib selfhost/src --lib selfhost/crt/src \
  selfhost/front/cgen.loft 2>&1 | grep -A8 "^error\|rejected" | head -30
if [ ! -x "$out/gen/cgen" ]; then
  echo "cgen_cycle: the generator did not build ($out/gen/cgen)"
  exit 1
fi
export TMPDIR="$out"
python3 selfhost/bench/cgen_survey.py --plain --generator "$out/gen/cgen" --layer bodies --show differ \
  --top "$top" > "$out/plain.log" 2>&1
grep -A"$top" "^layer bodies" "$out/plain.log" | cut -c1-150 | sed -n '1p;3,$p'
grep -A3 "units agree" "$out/plain.log" | cut -c1-160
grep -A3 "^differ" "$out/plain.log" | cut -c1-250 | head -24
python3 selfhost/bench/cgen_survey.py --generator "$out/gen/cgen" --layer bodies --show differ --top 1 \
  > "$out/default.log" 2>&1
grep "layer bodies\|^differ" "$out/default.log" | head -4
