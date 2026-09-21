#!/usr/bin/env bash
# native_ratio.sh — how far loft-native is from plain Rust, per routine, with
# the output hash as the like-for-like precondition (@PLN157 P0, loft#1426).
#
# For every bench dir named in bench/ratio_oracle.tsv: build the loft program
# `--native-emit` + `rustc -O` (the same lane bench/run_bench.sh builds) and
# the plain-Rust reference `rustc -O`, run both (best of $RUNS), join their TSV
# rows by routine and print  routine · rust ns/op · native ns/op · ratio · bar.
#
# Exit status:
#   - a HASH mismatch between the lanes (or a program failing its own hash
#     assert) is ALWAYS fatal — a faster wrong answer must never pass;
#   - a ratio over its bar is fatal only under --gate.  Timings are noisy and
#     machine-bound, so the report form is the default (`make speed` is the
#     precedent); @PLN157's phases and the release evidence run --gate.
#
# Usage: scripts/native_ratio.sh [--gate] [--n N] [--runs R]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ORACLE="$ROOT/bench/ratio_oracle.tsv"
LOFT="${LOFT_BIN:-$ROOT/target/release/loft}"
LOFT_LIB="${LOFT_LIB_DIR:-$ROOT/target/release}"

GATE=0
N=5
RUNS=3
while [[ $# -gt 0 ]]; do
  case "$1" in
    --gate) GATE=1 ;;
    --n) N="$2"; shift ;;
    --runs) RUNS="$2"; shift ;;
    -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
    *) echo "unknown flag: $1"; exit 1 ;;
  esac
  shift
done

[[ -x "$LOFT" ]] || { echo "error: loft binary not found at $LOFT (build with: cargo build --release)"; exit 1; }
[[ -f "$LOFT_LIB/libloft.rlib" ]] || { echo "error: libloft.rlib not found in $LOFT_LIB"; exit 1; }
[[ -f "$ORACLE" ]] || { echo "error: $ORACLE not found"; exit 1; }

# ns_op and hash per routine from a program's TSV output, keeping the MEDIAN
# ns_op over $RUNS runs (a loaded box makes any single run — and min-of-runs —
# swing ~3×; the median is what moves least) and requiring the hash stable
# across them.  Writes `$LANE_DIR/<prefix>.tsv`, one `routine<TAB>median_ns<TAB>hash`
# row per routine, read back by `lane_ns` / `lane_hash`.
#
# A FILE rather than an associative array on purpose: `declare -A` is bash 4, and
# stock macOS ships bash 3.2, where this script died at its `declare` line with
# `declare: -A: invalid option` and produced no measurement at all — on the box
# the owner works from.  Keying by name in awk needs no shell feature at all.
run_lane() {
  local bin="$1" prefix="$2" run
  local raw="$LANE_DIR/$prefix.raw"
  : > "$raw"
  for ((run = 0; run < RUNS; run++)); do
    "$bin" --n "$N" >> "$raw"
  done
  awk -F'\t' -v bin="$bin" '
    $1 == "routine" || $7 == "" { next }
    {
      name = $1
      if (name in seen && hash[name] != $7) {
        # stderr, not stdout: the stdout of this awk IS the lane TSV, so a FAIL
        # printed there is swallowed into the file and the run dies with no message.
        # (No apostrophe in this comment: it would close the awk quote.)
        printf "FAIL %s: hash unstable across runs of %s (%s vs %s)\n", name, bin, hash[name], $7 > "/dev/stderr"
        bad = 1
        exit 1
      }
      seen[name] = 1
      hash[name] = $7
      n[name]++
      ns[name, n[name]] = $4
      if (!(name in order)) { order[name] = ++count; byrank[count] = name }
    }
    END {
      if (bad) { exit 1 }
      for (r = 1; r <= count; r++) {
        name = byrank[r]
        # median of this routine sample, insertion sort over a handful of runs
        for (i = 1; i <= n[name]; i++) { v[i] = ns[name, i] + 0 }
        for (i = 2; i <= n[name]; i++) {
          key = v[i]; j = i - 1
          while (j > 0 && v[j] > key) { v[j + 1] = v[j]; j-- }
          v[j + 1] = key
        }
        printf "%s\t%s\t%s\n", name, v[int((n[name] + 1) / 2)], hash[name]
      }
    }
  ' "$raw" > "$LANE_DIR/$prefix.tsv" || exit 1
}

# The median ns_op / the hash for one routine of one lane, empty when the lane has
# no such row (a hash assert inside the program aborts its rows).
lane_ns()   { awk -F'\t' -v r="$2" '$1 == r { print $2 }' "$LANE_DIR/$1.tsv"; }
lane_hash() { awk -F'\t' -v r="$2" '$1 == r { print $3 }' "$LANE_DIR/$1.tsv"; }

fail=0
current_dir=""
LANE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/loft_native_ratio.XXXXXX")"
trap 'rm -rf "$LANE_DIR"' EXIT

build_and_run() {
  local dir="$ROOT/bench/$1" build
  build="$dir/.loft"
  mkdir -p "$build"
  # loft-native lane — what a SHIPPED binary gets (`--native-release`): the lean tier
  # and full optimisation.  A performance lane measures the optimised build; the
  # semantics runs are the test runner's.
  "$LOFT" --native-emit "$build/ratio_native.rs" --lean --path "$ROOT/" "$dir/bench.loft" > /dev/null
  rustc -C opt-level=3 -C codegen-units=1 --edition=2024 \
    --extern "loft=$LOFT_LIB/libloft.rlib" -L "$LOFT_LIB/deps" \
    -o "$build/ratio_native" "$build/ratio_native.rs"
  rm -f "$build/ratio_native.rs"
  # Rust reference lane.
  rustc -O -o "$build/ratio_rs" "$dir/bench.rs"
  run_lane "$build/ratio_native" nat
  run_lane "$build/ratio_rs" rs
}

printf '%-12s %-12s %12s %12s %8s %6s\n' "bench" "routine" "rust ns/op" "native ns/op" "ratio" "bar"
while IFS=$'\t' read -r dir routine bar; do
  [[ "$dir" =~ ^#.*$ || -z "$dir" ]] && continue
  if [[ "$dir" != "$current_dir" ]]; then
    build_and_run "$dir"
    current_dir="$dir"
  fi
  n_ns="$(lane_ns nat "$routine")"
  r_ns="$(lane_ns rs "$routine")"
  if [[ -z "$n_ns" || -z "$r_ns" ]]; then
    echo "FAIL $dir/$routine: missing row (native='${n_ns:-none}' rust='${r_ns:-none}') — a hash assert inside the program aborts its rows"
    fail=1
    continue
  fi
  n_hash="$(lane_hash nat "$routine")"
  r_hash="$(lane_hash rs "$routine")"
  if [[ "$n_hash" != "$r_hash" ]]; then
    echo "FAIL $dir/$routine: hash mismatch — native $n_hash vs rust $r_hash"
    fail=1
    continue
  fi
  ratio=$(awk -v a="$n_ns" -v b="$r_ns" 'BEGIN { printf "%.1f", a / b }')
  over=$(awk -v r="$ratio" -v bar="$bar" 'BEGIN { print (r > bar) ? 1 : 0 }')
  mark=""
  if [[ "$over" == "1" ]]; then
    mark="  OVER BAR"
    [[ $GATE -eq 1 ]] && fail=1
  fi
  printf '%-12s %-12s %12s %12s %8s %6s%s\n' "$dir" "$routine" "$r_ns" "$n_ns" "$ratio" "$bar" "$mark"
done < "$ORACLE"

exit $fail
