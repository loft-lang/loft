#!/usr/bin/env bash
# @PLN152 step 6 — what does the fused fit test COST where it is live?
#
# A REPORT, never a gate (README § Why this is worth doing).  Three arms, interleaved so a
# drifting box moves all of them together:
#
#   narrow-plain   a `u8` accumulator loop with NO test written — the opt-in claim's timing
#                  half.  Run on BOTH binaries; they must agree, because their emission for
#                  this program is byte-identical.
#   narrow-tested  the same loop with `if !acc { … }` on the line after the store — the
#                  mechanism, live.  AFTER only: the program does not compile the fused way
#                  on the BEFORE binary, it compiles the always-false way.
#   integer-ctrl   the same loop over plain `integer`, which keeps a sentinel and is never
#                  fused.  The CONTROL that must not move; a run where it does is a run
#                  measuring the box, not the change.
#
# Usage: run.sh <before-loft> <after-loft> [reps] [--native]
set -u
here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)"
before="${1:?before binary}"; after="${2:?after binary}"; reps="${3:-9}"; mode="${4:---interpret}"
med() { printf '%s\n' "$@" | sort -n | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}'; }
one() { # binary, file  -> ms
  local out
  out=$(LOFT_TIMEOUT=300 "$1" --path "$root/" "$mode" "$2" 2>/dev/null | sed -n 's/.*time: \([0-9]*\)ms.*/\1/p')
  printf '%s' "${out:-0}"
}
declare -A samples
for _ in $(seq "$reps"); do
  samples[bp]+=" $(one "$before" "$here/narrow_plain.loft")"
  samples[ap]+=" $(one "$after"  "$here/narrow_plain.loft")"
  samples[at]+=" $(one "$after"  "$here/narrow_tested.loft")"
  samples[bc]+=" $(one "$before" "$here/integer_ctrl.loft")"
  samples[ac]+=" $(one "$after"  "$here/integer_ctrl.loft")"
done
printf '%-16s %-10s %-10s %s\n' ARM BEFORE AFTER DELTA
row() { # label, before-samples, after-samples
  local b a d
  b=$(med ${2}); a=$(med ${3})
  d=$(awk -v b="$b" -v a="$a" 'BEGIN{ if (b>0) printf "%+.2f %%", (a-b)*100.0/b; else print "n/a" }')
  printf '%-16s %-10s %-10s %s\n' "$1" "$b" "$a" "$d"
}
# The two rows that answer different questions.  `narrow-plain` and `integer-ctrl` compare
# two BINARIES on one program and must read 0 %: their emission is byte-identical, so
# whatever they show is the box, and it is the noise bound every other number is read
# against.  `mechanism` compares two PROGRAMS on ONE binary, which is the cost itself.
row narrow-plain   "${samples[bp]}" "${samples[ap]}"
row integer-ctrl   "${samples[bc]}" "${samples[ac]}"
row mechanism      "${samples[ap]}" "${samples[at]}"
