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
# across them.  Populates <prefix>_ns and <prefix>_hash by name.
run_lane() {
  local bin="$1" prefix="$2" run name ns hash key
  local -A samples=()
  for ((run = 0; run < RUNS; run++)); do
    while IFS=$'\t' read -r name _ _ ns _ _ hash; do
      [[ "$name" == "routine" || -z "$hash" ]] && continue
      samples[$name]="${samples[$name]:-} $ns"
      local prev="${prefix}_hash[$name]"
      if [[ -n "${!prev:-}" && "${!prev}" != "$hash" ]]; then
        echo "FAIL $name: hash unstable across runs of $bin (${!prev} vs $hash)"
        exit 1
      fi
      printf -v "${prefix}_hash[$name]" '%s' "$hash"
    done < <("$bin" --n "$N")
  done
  for key in "${!samples[@]}"; do
    ns=$(printf '%s\n' ${samples[$key]} | sort -n | awk '{ a[NR] = $1 } END { print a[int((NR + 1) / 2)] }')
    printf -v "${prefix}_ns[$key]" '%s' "$ns"
  done
}

fail=0
current_dir=""
declare -A nat_ns nat_hash rs_ns rs_hash

build_and_run() {
  local dir="$ROOT/bench/$1" build
  build="$dir/.loft"
  mkdir -p "$build"
  # loft-native lane — the exact shape run_bench.sh and loft#1426 build.
  "$LOFT" --native-emit "$build/ratio_native.rs" --path "$ROOT/" "$dir/bench.loft" > /dev/null
  rustc -O --edition=2024 --extern "loft=$LOFT_LIB/libloft.rlib" -L "$LOFT_LIB/deps" \
    -o "$build/ratio_native" "$build/ratio_native.rs"
  rm -f "$build/ratio_native.rs"
  # Rust reference lane.
  rustc -O -o "$build/ratio_rs" "$dir/bench.rs"
  nat_ns=(); nat_hash=(); rs_ns=(); rs_hash=()
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
  n_ns="${nat_ns[$routine]:-}"
  r_ns="${rs_ns[$routine]:-}"
  if [[ -z "$n_ns" || -z "$r_ns" ]]; then
    echo "FAIL $dir/$routine: missing row (native='${n_ns:-none}' rust='${r_ns:-none}') — a hash assert inside the program aborts its rows"
    fail=1
    continue
  fi
  if [[ "${nat_hash[$routine]}" != "${rs_hash[$routine]}" ]]; then
    echo "FAIL $dir/$routine: hash mismatch — native ${nat_hash[$routine]} vs rust ${rs_hash[$routine]}"
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
