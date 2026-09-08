# Loft Benchmark Suite

Performance comparison across five targets: Python, loft interpreter, loft native, loft wasm, and Rust.

> **Not a CI suite** — run manually to compare performance.

## Benchmarks

| # | Name | Description |
|---|------|-------------|
| 01 | fibonacci | Recursive fib(38) |
| 02 | sum_loop | Sum 0..10,000,000 |
| 03 | sieve | Count primes to 100,000 (trial division) |
| 04 | collatz | Collatz sequence lengths 1..1,000,000 |
| 05 | mandelbrot | 200×200 Mandelbrot, 256 max iters |
| 06 | newton_sqrt | Newton's method sqrt, 1M calls |
| 07 | string_build | 500,000 string appends |
| 08 | word_count | Hash-based word frequency, 600K ops |
| 09 | matrix_mul | Float dot product, 5M elements |
| 10 | sort | Insertion sort, 3,000 integers |
| 11 | par | Parallel for-loop, 100K elements × 50-iter Newton's sqrt |
| 12 | drawing | Drawing hot loops: seeded hash (100K calls) + brush-lock raster (~22K px), output hashes asserted — the loft#1426 / @PLN157 workload; per-routine ratios gated by `scripts/native_ratio.sh` against `ratio_oracle.tsv` |

## Targets

- **python** — CPython interpreter
- **loft-interp** — loft interpreter (`loft run`)
- **loft-native** — loft native binary (`loft build --native`)
- **loft-wasm** — loft WASM via wasmtime (`loft build --wasm`)
- **rust** — Rust release build (`rustc -O`)

## Prerequisites

Missing tools produce a warning and that target is skipped — no hard failures.

- `loft` — either in PATH, or set `LOFT_BIN=/path/to/loft`
- `LOFT_STDLIB` — set to the directory containing `default/` if loft can't find its stdlib (e.g. `LOFT_STDLIB=/path/to/loft-repo/`)
- `python3` — in PATH
- `rustc` — in PATH
- `wasmtime` — in PATH (wasm target only)

## Usage

```bash
cd bench
./run_bench.sh                        # run all benchmarks, all targets
./run_bench.sh --list                 # list benchmarks with descriptions
./run_bench.sh --only 8               # single benchmark by number
./run_bench.sh --only 08_word_count   # single benchmark by full name
./run_bench.sh --skip-python          # skip Python
./run_bench.sh --skip-wasm            # skip wasm
./run_bench.sh --no-build             # skip compilation step
./run_bench.sh --warmup               # run once before timing
./run_bench.sh -h                     # show all options
```

## Output

```
bench                python       loft-interp   loft-native   loft-wasm     rust
---------------------------------------------------------------------------------
01_fibonacci         1823ms       612ms         18ms          22ms          8ms
...
```
