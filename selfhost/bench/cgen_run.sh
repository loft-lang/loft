#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN91 strand 12, U-0 — one program through the loft generator's C, the way
# `cgen_survey.py --run --plain` runs each: under the plain switches, `loft --native-c` with
# LOFT_CGEN set, its output diffed against the interpreter's.
#
#   selfhost/bench/cgen_run.sh [--gen EXE|live] FILE [emit|run|both]
#
#   --gen EXE   a built generator (default: $LOFT_CGEN_OUT/gen/cgen, the one cgen_cycle.sh
#               builds); `--gen live` runs the live sources interpreted (cgen_live.sh)
#   emit        write the loft generator's unit to $LOFT_CGEN_OUT/run/got.c
#   run         run it on the interpreter and through the loft unit, and diff the outputs
#   both        (default) the two
set -u
. "$(dirname "$0")/cgen_env.sh"
gen="$cgen_out/gen/cgen"
if [ "${1:-}" = "--gen" ]; then
  gen="${2:?--gen EXE|live}"
  shift 2
fi
case "${1:-}" in
  -h|--help|"") sed -n '5,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac
[ "$gen" = live ] && gen="$cgen_root/selfhost/bench/cgen_live.sh"
[ -x "$gen" ] || { echo "cgen_run: no generator at $gen (selfhost/bench/cgen_cycle.sh builds one)"; exit 2; }
file="$(realpath "$1")"
mode="${2:-both}"
cgen_plain
export LOFT_NO_WARNINGS=1 LOFT_CGEN="$(realpath "$gen")" LOFT_CGEN_RUNTIME="$cgen_root/src/c" TMPDIR="$cgen_out"
s="$cgen_out/run"
mkdir -p "$s"
cd "$(cgen_cwd "$file")" || exit 2
rc=0
if [ "$mode" != run ]; then
  LOFT_TIMEOUT=120 "$cgen_loft" --native-c --native-emit "$s/got.c" "$file"
  e=$?
  echo "emit rc=$e"
  [ $e -eq 0 ] || rc=1
fi
if [ "$mode" != emit ]; then
  LOFT_CGEN= LOFT_TIMEOUT=120 "$cgen_loft" --interpret "$file" > "$s/interpret.out" 2>&1
  echo "interpret rc=$?"
  LOFT_TIMEOUT=120 "$cgen_loft" --native-c "$file" > "$s/cgen.out" 2>&1
  echo "loft unit rc=$?"
  if ! diff "$s/interpret.out" "$s/cgen.out" | head -20; then rc=1; fi
  cmp -s "$s/interpret.out" "$s/cgen.out" || rc=1
fi
exit $rc
