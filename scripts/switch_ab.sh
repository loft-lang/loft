#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# switch_ab.sh — the falsifier of a rewrite BOTH backends run: every corpus program with the
# rewrite ON and with it OFF, and every output difference reported as a defect.
#
#   scripts/switch_ab.sh LOFT_NO_LAZY_BUFFER                 # the tests/scripts corpus
#   scripts/switch_ab.sh LOFT_NO_LAZY_BUFFER --consumer DIR   # + a consumer package's suite
#   JOBS=4 LOFT=target/release/loft scripts/switch_ab.sh LOFT_NO_CARVE_IN_PLACE
#   ONLY='1647-*' LOFT=<control>/loft LOFT_ARGS="--path $PWD/" scripts/switch_ab.sh …   # a subset
#
# Why it exists (formal/rewrites.md, (R-Switch)'s both-backend clause): a rewrite the
# parser, the scope pass or the runtime applies gives the interpreter and native the SAME
# answer, right or wrong, so their agreement proves nothing, and LOFT_STRICT_STORES /
# LOFT_POISON catch a leak or a double free, not a stale value.  The switch is the only
# second answer, and C122 makes it a complete one: a rewrite is free exactly where its
# conditions hold, so wherever the program's output moves with the switch, the rewrite
# changed a meaning.  loft#1647 read a loop's first pass on both backends for six days;
# `LOFT_NO_LAZY_BUFFER=1` told the difference at once.
#
# A program whose output differs between two runs with the rewrite ON is nondeterministic
# (time, randomness, a temp path) and is reported as NOISE, never as a defect.  Exit 1 when
# any defect is found, 0 otherwise.
#
# A program whose output MEASURES what a switch changes — a hash table's byte size under
# LOFT_NO_HALF_LOAD (a writer's sizing policy no reader assumes), a store's occupancy — moves
# with that switch without any meaning moving.  It says so in its header with
# `// @observes: LOFT_NO_<REWRITE>`, and the difference is reported as OBSERVED, not as a
# defect.  The annotation names ONE switch: under every other switch the same program is
# compared as usual.
set -uo pipefail
if [ $# -lt 1 ] || [[ "$1" != LOFT_NO_* ]]; then
  echo "usage: $0 LOFT_NO_<REWRITE> [--consumer DIR]..." >&2
  exit 2
fi
SWITCH="$1"; shift
CONSUMERS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --consumer) CONSUMERS+=("$2"); shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOFT="${LOFT:-$ROOT/target/release/loft}"
[ -x "$LOFT" ] || { echo "no loft binary at $LOFT — cargo build --release --bin loft" >&2; exit 2; }
JOBS="${JOBS:-$(nproc 2>/dev/null || echo 4)}"
# The scratch parent is created first: a fresh CI runner has no `~/.cache`, `mktemp -d` then
# fails, `WORK` is empty, and every run wrote to `/on/<file>.out` — which the comparison below
# read as 1706 "noisy" programs, a failed harness reported as a finding on every leg.  A
# harness that cannot write its outputs has measured nothing, so it stops.
scratch="${XDG_CACHE_HOME:-$HOME/.cache}"
mkdir -p "$scratch" && WORK="$(mktemp -d "$scratch/loft-switch-ab.XXXXXX")" && [ -n "$WORK" ] \
  || { echo "cannot create a scratch directory under $scratch" >&2; exit 2; }
trap 'rm -rf "$WORK"' EXIT

# One program, one mode: output to $WORK/<mode>/<file>.out.  The `@ARGS:` annotation is the
# runner's own (test_runner.rs); both modes pass it alike, so the comparison is fair.
run_one() {
  local mode="$1" file="$2" base args
  base="$(basename "$file" .loft)"
  args="$(sed -n 's#^// @ARGS: *##p' "$file" | head -1)"
  if [ "$mode" = off ]; then
    env "$SWITCH=1" LOFT_TIMEOUT="$AB_TIMEOUT" "$LOFT" $LOFT_ARGS --interpret $args "$file" >"$WORK/$mode/$base.out" 2>&1
  else
    env -u "$SWITCH" LOFT_TIMEOUT="$AB_TIMEOUT" "$LOFT" $LOFT_ARGS --interpret $args "$file" >"$WORK/$mode/$base.out" 2>&1
  fi
  echo "exit=$?" >>"$WORK/$mode/$base.out"
}
export -f run_one
LOFT_ARGS="${LOFT_ARGS:-}"
# A program cut off by the watchdog stops at a point that depends on the load, so it is
# never compared: it is reported as TIMEOUT.  The corpus's slowest program runs ~80 s.
AB_TIMEOUT="${AB_TIMEOUT:-300}"
export SWITCH LOFT WORK LOFT_ARGS AB_TIMEOUT

mkdir -p "$WORK/on" "$WORK/on2" "$WORK/off" || { echo "cannot create $WORK/{on,on2,off}" >&2; exit 2; }
cd "$ROOT"
files=(tests/scripts/${ONLY:-*}.loft)
for mode in on off; do
  printf '%s\n' "${files[@]}" | xargs -P "$JOBS" -I{} bash -c "run_one $mode {}"
done

defects=0; noise=0; timeouts=0; observed=0
for f in "${files[@]}"; do
  base="$(basename "$f" .loft)"
  if grep -q '^\[timeout\] deadline reached' "$WORK/on/$base.out" "$WORK/off/$base.out"; then
    timeouts=$((timeouts + 1))
    echo "TIMEOUT $f (cut off after ${AB_TIMEOUT}s — not compared)"
    continue
  fi
  if ! cmp -s "$WORK/on/$base.out" "$WORK/off/$base.out"; then
    run_one on2 "$f"
    if ! cmp -s "$WORK/on/$base.out" "$WORK/on2/$base.out"; then
      noise=$((noise + 1))
      echo "NOISE   $f (differs between two runs with the rewrite on)"
    elif grep -qE "^// @observes:.*\b$SWITCH\b" "$f"; then
      observed=$((observed + 1))
      echo "OBSERVED $f (its output measures what $SWITCH changes, as its header declares)"
    else
      defects=$((defects + 1))
      echo "DEFECT  $f — output moves with $SWITCH:"
      diff "$WORK/on/$base.out" "$WORK/off/$base.out" | head -12 | sed 's/^/    /'
    fi
  fi
done

# A consumer package: its suite on the interpreter, the per-test PASS/FAIL lines compared.
for dir in "${CONSUMERS[@]}"; do
  [ -f "$dir/loft.toml" ] || { echo "SKIP    $dir (no loft.toml)"; continue; }
  on="$(cd "$dir" && env -u "$SWITCH" LOFT_TIMEOUT=600 "$LOFT" test --interpret 2>&1 | grep -E 'FAIL|test result' | sort)"
  off="$(cd "$dir" && env "$SWITCH=1" LOFT_TIMEOUT=600 "$LOFT" test --interpret 2>&1 | grep -E 'FAIL|test result' | sort)"
  if [ "$on" != "$off" ]; then
    defects=$((defects + 1))
    echo "DEFECT  consumer $dir — its suite moves with $SWITCH:"
    diff <(echo "$on") <(echo "$off") | head -12 | sed 's/^/    /'
  fi
done

echo "$SWITCH: ${#files[@]} programs, $defects defect(s), $observed observed, $noise noisy, $timeouts timed out"
[ "$defects" -eq 0 ]
