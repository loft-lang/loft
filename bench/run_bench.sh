#!/usr/bin/env bash
# Run the loft benchmark suite.
# NOT a CI suite — run manually for performance comparison.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Auto-detect loft binary: env override → project target/release → PATH
if [[ -n "${LOFT_BIN:-}" ]]; then
  LOFT="$LOFT_BIN"
elif [[ -x "$PROJECT_ROOT/target/release/loft" ]]; then
  LOFT="$PROJECT_ROOT/target/release/loft"
else
  LOFT="loft"
fi

# Auto-detect stdlib: env override → sibling default/ directory (inside the project)
if [[ -n "${LOFT_STDLIB:-}" ]]; then
  STDLIB_PATH="$LOFT_STDLIB"
elif [[ -d "$PROJECT_ROOT/default" ]]; then
  STDLIB_PATH="$PROJECT_ROOT/"
else
  STDLIB_PATH=""
fi

# Auto-detect libloft.rlib for native compilation: project target/release → installed share/loft
if [[ -n "${LOFT_LIB_DIR:-}" ]]; then
  LOFT_LIB="$LOFT_LIB_DIR"
elif [[ -f "$PROJECT_ROOT/target/release/libloft.rlib" ]]; then
  LOFT_LIB="$PROJECT_ROOT/target/release"
elif [[ -f "/usr/local/share/loft/libloft.rlib" ]]; then
  LOFT_LIB="/usr/local/share/loft"
else
  LOFT_LIB=""
fi

SKIP_PYTHON=0
SKIP_WASM=0
NO_BUILD=0
WARMUP=0
ONLY=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)
      echo "Available benchmarks:"
      echo ""
      for d in "$SCRIPT_DIR"/*/; do
        local_name="$(basename "$d")"
        local_desc=""
        if [[ -f "$d/bench.loft" ]]; then
          local_desc="$(grep -m1 '^// Benchmark' "$d/bench.loft" | sed 's|^// Benchmark [0-9]*: ||')"
        fi
        printf "  %-20s %s\n" "$local_name" "$local_desc"
      done
      exit 0 ;;
    -h|--help)
      cat <<'EOF'
Usage: run_bench.sh [OPTIONS]

Run the loft benchmark suite and print a comparison table.

Options:
  --skip-python   Skip the Python measurements
  --skip-wasm     Skip the loft-wasm measurements
  --no-build      Skip (re)building native/wasm/rust binaries; use existing ones
  --warmup        Run each benchmark once silently before timing
  --only N        Run only one benchmark: a number (e.g. --only 8) or full name (e.g. --only 08_word_count)
  --list          List available benchmarks with a short description
  -h, --help      Show this help message and exit

Environment:
  LOFT_BIN        Path to the loft binary (default: target/release/loft)
  LOFT_STDLIB     Path to the stdlib root directory (default: project root)
  LOFT_LIB_DIR    Directory containing libloft.rlib (default: target/release or /usr/local/share/loft)

Output columns: bench | python | loft-interp | loft-native | loft-wasm | rust
EOF
      exit 0 ;;
    --skip-python) SKIP_PYTHON=1 ;;
    --skip-wasm)   SKIP_WASM=1 ;;
    --no-build)    NO_BUILD=1 ;;
    --warmup)      WARMUP=1 ;;
    --only)
      if [[ ! "$2" =~ ^[0-9]+$ && ! "$2" =~ ^[0-9]+_ ]]; then
        echo "error: --only requires a number or full bench name (e.g. --only 8 or --only 08_word_count)"; exit 1
      fi
      ONLY="$2"; shift ;;
    *) echo "Unknown flag: $1"; exit 1 ;;
  esac
  shift
done

# Check prerequisites and warn (never abort — just skip missing targets)
HAS_LOFT=0
HAS_PYTHON=0
HAS_RUST=0
HAS_WASMTIME=0

if command -v "$LOFT" > /dev/null 2>&1; then
  HAS_LOFT=1
else
  echo "warning: loft not found (run from inside the project, or set LOFT_BIN=<path>) — loft targets will be skipped"
fi

if command -v python3 > /dev/null 2>&1; then
  HAS_PYTHON=1
else
  echo "warning: python3 not found — python target will be skipped"
fi

if command -v rustc > /dev/null 2>&1; then
  HAS_RUST=1
else
  echo "warning: rustc not found — rust target will be skipped"
fi

WASMTIME=""
for _wt_candidate in "$(command -v wasmtime 2>/dev/null)" "$HOME/.cargo/bin/wasmtime" "$HOME/.wasmtime/bin/wasmtime"; do
  if [[ -x "$_wt_candidate" ]]; then
    WASMTIME="$_wt_candidate"
    break
  fi
done
if [[ -n "$WASMTIME" ]]; then
  HAS_WASMTIME=1
else
  echo "warning: wasmtime not found — loft-wasm column will show '-' (install via: cargo install wasmtime-cli  OR  brew install wasmtime)"
fi

# Build --path flag for loft if STDLIB_PATH is set
LOFT_PATH_FLAG=()
if [[ -n "$STDLIB_PATH" ]]; then
  LOFT_PATH_FLAG=(--path "$STDLIB_PATH")
fi

extract_ms() {
  # Extract trailing "time: Xms" from output
  grep -oE 'time: [0-9]+ms' | tail -1 | grep -oE '[0-9]+'
}

run_bench() {
  local dir="$1"
  local name
  name="$(basename "$dir")"

  if [[ -n "$ONLY" ]]; then
    if [[ "$ONLY" == *_* ]]; then
      # Full name: exact match
      [[ "$name" != "$ONLY" ]] && return
    else
      # Number: match by stripping leading zeros from both sides
      local num="${name%%_*}"
      num="${num#"${num%%[!0]*}"}"
      local only_stripped="${ONLY#"${ONLY%%[!0]*}"}"
      [[ "$num" != "$only_stripped" ]] && return
    fi
  fi

  local py_ms="" li_ms="" ln_ms="" lw_ms="" rs_ms=""

  # Python
  if [[ $SKIP_PYTHON -eq 0 && $HAS_PYTHON -eq 1 && -f "$dir/bench.py" ]]; then
    [[ $WARMUP -eq 1 ]] && python3 "$dir/bench.py" > /dev/null 2>&1 || true
    py_ms=$(python3 "$dir/bench.py" 2>/dev/null | extract_ms || true)
  fi

  # loft interpreter
  if [[ $HAS_LOFT -eq 1 && -f "$dir/bench.loft" ]]; then
    [[ $WARMUP -eq 1 ]] && "$LOFT" "${LOFT_PATH_FLAG[@]}" "$dir/bench.loft" > /dev/null 2>&1 || true
    li_ms=$("$LOFT" "${LOFT_PATH_FLAG[@]}" "$dir/bench.loft" 2>/dev/null | extract_ms || true)
  fi

  local build_dir="$dir/.loft"

  # loft native
  if [[ $HAS_LOFT -eq 1 && $HAS_RUST -eq 1 && -n "$LOFT_LIB" && $NO_BUILD -eq 0 && -f "$dir/bench.loft" ]]; then
    mkdir -p "$build_dir"
    native_rs="$build_dir/bench.rs"
    "$LOFT" --native-emit "$native_rs" --lean "${LOFT_PATH_FLAG[@]}" "$dir/bench.loft" > /dev/null 2>&1 || true
    if [[ -f "$native_rs" ]]; then
      rustc -C opt-level=3 -C codegen-units=1 --edition=2024 \
        --extern "loft=$LOFT_LIB/libloft.rlib" \
        -L "$LOFT_LIB/deps" \
        -o "$build_dir/bench_bin" \
        "$native_rs" > /dev/null 2>&1 || true
      rm -f "$native_rs"
    fi
  fi
  if [[ -f "$build_dir/bench_bin" ]]; then
    [[ $WARMUP -eq 1 ]] && "$build_dir/bench_bin" > /dev/null 2>&1 || true
    ln_ms=$("$build_dir/bench_bin" 2>/dev/null | extract_ms || true)
  fi

  # loft wasm
  if [[ $SKIP_WASM -eq 0 && $HAS_LOFT -eq 1 ]]; then
    if [[ $NO_BUILD -eq 0 && -f "$dir/bench.loft" ]]; then
      mkdir -p "$build_dir"
      "$LOFT" --native-wasm "$build_dir/bench.wasm" "${LOFT_PATH_FLAG[@]}" "$dir/bench.loft" > /dev/null 2>&1 || true
    fi
    if [[ -f "$build_dir/bench.wasm" && $HAS_WASMTIME -eq 1 ]]; then
      [[ $WARMUP -eq 1 ]] && "$WASMTIME" --dir . "$build_dir/bench.wasm" > /dev/null 2>&1 || true
      lw_ms=$("$WASMTIME" --dir . "$build_dir/bench.wasm" 2>/dev/null | extract_ms || true)
    fi
  fi

  # Rust
  if [[ $HAS_RUST -eq 1 && $NO_BUILD -eq 0 && -f "$dir/bench.rs" ]]; then
    mkdir -p "$build_dir"
    rustc -O -o "$build_dir/bench_rs_bin" "$dir/bench.rs" > /dev/null 2>&1 || true
  fi
  if [[ $HAS_RUST -eq 1 && -f "$build_dir/bench_rs_bin" ]]; then
    [[ $WARMUP -eq 1 ]] && "$build_dir/bench_rs_bin" > /dev/null 2>&1 || true
    rs_ms=$("$build_dir/bench_rs_bin" 2>/dev/null | extract_ms || true)
  fi

  ms() { [[ -z "$1" ]] && echo "-" || echo "${1}ms"; }
  printf "%-20s %-12s %-13s %-13s %-13s %-10s\n" \
    "$name" "$(ms "$py_ms")" "$(ms "$li_ms")" \
    "$(ms "$ln_ms")" "$(ms "$lw_ms")" "$(ms "$rs_ms")"
}

printf "%-20s %-12s %-13s %-13s %-13s %-10s\n" \
  "bench" "python" "loft-interp" "loft-native" "loft-wasm" "rust"
printf '%s\n' "$(printf '%-20s' '' | tr ' ' '-')$(printf '%-12s' '' | tr ' ' '-')$(printf '%-13s' '' | tr ' ' '-')$(printf '%-13s' '' | tr ' ' '-')$(printf '%-13s' '' | tr ' ' '-')$(printf '%-10s' '' | tr ' ' '-')"

for d in "$SCRIPT_DIR"/*/; do
  [[ -d "$d" ]] && run_bench "$d"
done
