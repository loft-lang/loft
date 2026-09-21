# Loft Benchmark Suite

Performance comparison across five targets: Python, loft interpreter, loft native, loft wasm, and Rust.

> **Not a CI suite** — run manually to compare performance.

## Benchmarks

| # | Name | Description |
|---|------|-------------|
| 01 | fibonacci | Recursive `fib(30)` / `fib(31)`, alternating |
| 02 | sum_loop | 5,000,000 steps of a loop-carried integer mix (no closed form, not vectorisable) |
| 03 | sieve | Count primes below 300,000 (trial division) |
| 04 | collatz | Collatz sequence lengths for starts below 200,000 |
| 05 | mandelbrot | 200×200 Mandelbrot, 256 max iters |
| 06 | newton_sqrt | Newton's method sqrt, 100,000 calls of 50 steps |
| 07 | string_build | 200,000 formatted appends to one text |
| 08 | word_count | Hash-based word frequency, 300,000 operations on a fresh table |
| 09 | matrix_mul | Float dot product, 2,000,000 elements |
| 10 | sort | Insertion sort of 3,000 integers, built and sorted per op |
| 11 | par | Parallel for-loop, 100K elements × 50-iter Newton's sqrt |
| 12 | drawing | Drawing hot loops: seeded hash (100K calls) + brush-lock raster (~22K px), output hashes asserted — the loft#1426 / @PLN157 workload; per-routine ratios gated by `scripts/native_ratio.sh` against `ratio_oracle.tsv` |

Each workload above is ONE OP.  A program runs its op `--n` times and reports the time per
op, so the numbers do not depend on how long a run was.

## Statistics: `stats.py`

`run_bench.sh` is the quick look — one run per lane.  For numbers that can be compared from
one pass to the next, use:

```bash
python3 bench/stats.py                       # native against the Rust reference, every bench
python3 bench/stats.py --only 01,08          # some of them
python3 bench/stats.py --lanes native,rust,interp,python
python3 bench/stats.py --tsv out.tsv         # keep the run, to diff against the next one
python3 bench/stats.py --show-samples        # every sample under its row
```

```
bench           routine          native ns/op    ±%      rust ns/op    ±%  nat/rust         range  verdict
01_fibonacci    fibonacci          10,362,156   0.7       2,636,753   0.0      3.93     3.92–3.95  OVER
02_sum_loop     sum_loop            5,384,541   0.0       2,975,253   0.0      1.81     1.81–1.81  ok
```

What it does, and why each part is there:

- **Calibrates `--n` per lane** until a run lasts about `--target-ms` (400), so a 1 ms op and
  a 50 ms op are both measured over the same stretch of wall time.
- **Discards one warm-up round.**  The first run of a lane reads 5–8 % slow (caches, the
  frequency ramp); every later one does not.
- **Samples each lane `--samples` times (7), the lanes interleaved**, so drift lands on both.
- **Pins the process to the fastest core** where `taskset` exists.  On a hybrid CPU an
  unpinned run moves between core kinds, a ±10 % swing by itself.  The threaded bench gets
  one thread of each of the four fastest cores.
- **Reports the median, the spread (interquartile range, % of the median), and the ratio
  with its RANGE** — native's first quartile over the reference's third, and the reverse.
  The verdict reads the range: `ok` only when all of it is inside `--bar` (2.0), `OVER` only
  when all of it is outside, `unclear` otherwise.  A row the tool cannot decide says so.
- **Requires the hashes to agree** across lanes.  A disagreement is fatal: the lanes are not
  computing the same thing and their times do not compare.

Measured on a quiet x86-64 laptop the spreads are 0.0–2.7 % and two full runs reproduce
every ratio to within about 1 % (the threaded row to about 4 %).  A spread above `--noisy`
(5 %) is flagged on the row: the box was busy, and that row wants re-running.

`scripts/native_ratio.sh` (`make native-ratio`) reads the same rows against the per-routine
bars in `ratio_oracle.tsv`; `--gate` fails a ratio over its bar.  The bars are ratcheted
DOWN as the ratios fall.

## The row protocol

Every lane of every bench prints one tab-separated row per routine, after a header:

```
routine    iters    us    ns_op    px    ns_px    hash
```

`iters` is `--n`, `us` the timed region, `ns_op` the time per op, `px` the items one op
processes (`ns_px` the time per item), and `hash` the result of ONE canonical op, in hex.
A last line `time: <ms>ms sink=<n>` carries the folded results, so no backend can drop the
work as unused.

Four rules keep a row LIKE-FOR-LIKE, and a bench that breaks one measures something else:

1. **The same algorithm in every lane, and the hash proves it.**  The suite once compared a
   loft insertion sort with a Rust bubble sort; no timer fixes that.
2. **Each repetition's input differs in VALUE, never in the amount of work** (`r & 1` added
   to a start, a salt, an origin).  A pure kernel on a constant input can be hoisted out of
   the repetition loop or folded to a constant, and then the row times nothing.
3. **The Rust reference `black_box`es the op's INPUT and the sink — never anything inside
   the kernel.**  Inside, it blocks the optimisations the reference is there to show.
4. **A kernel an optimiser can collapse is not a benchmark.**  A plain `sum += i` has a
   closed form; the integer loop here carries a dependency from step to step instead.

## Targets

- **python** — CPython interpreter
- **loft-interp** — loft interpreter (`loft --interpret`; a bare `loft prog.loft` runs the
  NATIVE backend, which is what this column measured until it said so)
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

Milliseconds per OP (the slow lanes run 2 ops, the fast ones 10):

```
bench                python       loft-interp   loft-native   loft-wasm     rust
---------------------------------------------------------------------------------
10_sort              94.27ms      987.15ms      2.22ms        -             1.03ms
...
```
