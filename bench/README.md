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
  unpinned run moves between core kinds, a ±10 % swing by itself.  A threaded bench — one whose `bench.loft` declares
  `// bench-threads: N` (§ The row protocol) — gets one thread of each of the N fastest cores.
- **Reports the median, the spread (interquartile range, % of the median), and the ratio
  with its RANGE** — native's first quartile over the reference's third, and the reverse.
  The verdict reads the range: `ok` only when all of it is inside `--bar` (2.0), `OVER` only
  when all of it is outside, `unclear` otherwise.  A row the tool cannot decide says so.
- **Requires the hashes to agree** across lanes.  A disagreement is fatal: the lanes are not
  computing the same thing and their times do not compare.
- **Builds before it measures, and never beside a measurement.**  The programs go in sets of
  `--batch` (20): a set is compiled `--build-jobs` (5) at a time — each worker takes the next
  program the moment its last one is built, at `nice 10`, waiting while the machine has less
  than `--build-mem-reserve-gb` (3) available — and only then measured, one program at a
  time on the quiet machine.  A program that does not build is listed at the end (exit 1)
  and the others are still measured.
- **Measures `--measure-jobs` programs at once (3).**  Each is pinned to its own cores of the
  machine's FASTEST tier (a hybrid CPU's efficiency cores never measure: a ratio taken there
  does not compare), waits while too few are free, and prints its rows together in the
  batch's order.  A THREADED program is always measured alone: it waits for the others to
  finish and nothing starts beside it.  Measured 2026-09-29 on 86 routines (the cache- and
  memory-bound lanes 09, 10, 13–18 and four library benches): at 2, 3, 4 and 6 at once the
  median ratio moved 0.6–0.8 % from a serial run and the 90th percentile 3.0–4.2 % —
  inside the 0.8 % / 5.5 % that two SERIAL runs differ by — while the wall time fell from
  234 s to 121, 89, 74 and 68 s.  3 is the default: most of the saving, and half the fast
  cores left free.  The rows that did move were the noisy and coarse ones, which move
  between serial runs too, and `16_consumer_shapes` as a whole, which read about 35 %
  faster against its twin in one parallel run — a lane that may depend on WHICH core it
  lands on.  The value is recorded in the run's metadata (`measure_jobs`).
- **Records what each build step cost**: wall-clock and CPU seconds and peak memory, per
  program and step (`native-emit`, `native-rustc`, `rust-rustc`, a package's
  `native-release`), printed as it lands and written by `--build-tsv`; the portal keeps them
  in `results/<host>-builds.tsv`.  Compare the CPU figure across runs — the wall-clock one
  depends on what built beside it.  Measured 2026-09-29: a cold package build is 2.5–8.6 s
  and under 350 MB, an in-repo bench about a second, so a full portal run is spent almost
  entirely MEASURING — a package is calibrated to `--package-target-ms` (4 s) per run, about
  80 s per package over its calibration, warm-up and seven samples in two lanes.

Measured on a quiet x86-64 laptop the spreads are 0.0–2.7 % and two full runs reproduce
every ratio to within about 1 % (the threaded row to about 4 %).  A spread above `--noisy`
(5 %) is flagged on the row: the box was busy, and that row wants re-running.

That reproducibility holds for ONE build.  Any change to the runtime the native lane links
moves some small routines by code layout alone, and a different change moves different
routines.  Measured 2026-09-29 on lanes 13 and 14, a base build and the same build with
code added that never runs, alternated twice each, every row within 1 % run to run:
`replace` moved +18 %, `find_contains` +7 %, `grid` +6 %, `comprehension` +5 %.  A
variant of the same change moved `byte_walk` +34 % and `join` +24 % instead, and left those
four alone.  A one-routine move after a runtime change is therefore not evidence of a
regression until a second, unrelated rebuild reproduces it.  `record_append` is BIMODAL on
top of that (35.6 or 56.7 µs, stable within a run), and it changes state between builds.

`scripts/native_ratio.sh` (`make native-ratio`) reads the same rows against the per-routine
bars in `ratio_oracle.tsv`; `--gate` (`make native-ratio-gate`, and the last step of the
local `make ci`) fails a ratio over its bar.  The bars are ratcheted DOWN as the ratios fall.
A ratio is machine-bound, and the bars are the owner's x86-64 laptop's: on another CPU a row
can sit over its bar repeatably (a cloud Xeon read `sum_loop` at 3.1–3.5 against 2.3 and
`collatz` at 3.7–4.1 against 2.5, twice, while `sieve` and `mandelbrot` read as on the
laptop), which is a calibration question to raise, never a reason to re-bless the oracle
on that box.

## The portal: where we stand, by class

`bench/portal/` turns the measurements into ONE page —
[doc/claude/PERF_PORTAL.md](../doc/claude/PERF_PORTAL.md) — that leads with the question a
performance pass starts from: **which KINDS of routine are still too slow?**

```bash
make perf-portal                                                  # measure here, then render
make perf-portal PACKAGES="--package <scratch clone>/drawing=drawing"
make perf-portal-render                                           # render from the saved runs
```

| file | what it holds |
|---|---|
| `portal/classes.tsv` | the mechanism classes: what bounds a routine of each |
| `portal/routines.tsv` | every measured routine -> its class and its POPULATION (`engine`, `stdlib`, `library:<name>`) |
| `portal/census.tsv` | library and consumer routines worth a row and not measured yet, each with its workload and its twin |
| `portal/gaps.tsv` | features the consumer programs lean on that no measured row exercises |
| `portal/libs.tsv` + `portal/checkout_libs.sh` | the library repositories the pass measures from, and the script that checks them out |
| `portal/results/<host>.tsv` | the latest run per machine, stamped with its commit, compiler and settings |

A CLASS names the mechanism a routine's cost is made of — the per-call frame, a checked
integer operator, a record appended to a vector, a hash probe — so a slow class points at one
part of the compiler or the runtime rather than at one program.  The page gives each class
its median, its best and its worst row, then every routine under its class, then coverage:
which libraries have a lane and which surveyed routines are waiting.

**Adding a row.**  Write the routine in a lane's `bench.loft` and its twin in `bench.rs` (the
four rules below), confirm the hashes agree (`python3 bench/stats.py --only <lane>`), and
give it a line in `routines.tsv`; a routine a lane prints and the registry lacks is reported
on the page as UNCLASSIFIED.  If it came off the census, delete its census line.

**Lanes.**  `01`–`12` are ENGINE programs (informational).  `13_stdlib_text`,
`14_stdlib_vector` and `15_stdlib_keyed` are the STANDARD LIBRARY as a program calls it, one
row per routine.  `16_consumer_shapes` MODELS the hot loops of real loft programs (crawler,
moros, dryopea) at their own data layouts and sizes — a tuple returned inside a hot loop,
nested struct fields through a vector, a struct-enum matched, a composite-key hash — because
those programs are never run by the bench and their trees are never written to.  A library's
own bench (the drawing library's `bench/`) joins through `--package`, measured from the
pass's OWN checkout of the library: `make perf-libs` clones the eight library repositories
as normal checkouts beside this one (`$LOFT_PERF_LIBS`, default `../loft-bench-libs`) and
fast-forwards them when they are clean; `portal/libs.tsv` names them and their benches.

## Interpreter against native: `interp_gap.py`

The long-run question is how the interpreter compares with the compiled program. The
nearer question is which native rewrites should move into the IR phase, so that both
backends run them. `make interp-gap` answers both per routine. It is a report you run by
hand, and it is never a gate.

```bash
make interp-gap                                   # every lane: time both backends, census both
make interp-gap ARGS="--only 14,16"               # some lanes
make interp-gap ARGS="--no-timing"                # the censuses only: a minute, no ratio column
make interp-gap ARGS="--timing old.tsv"           # reuse a `stats.py --lanes native,interp --tsv` run
make interp-gap ARGS="--package <scratch clone>/drawing=drawing"
```

Per routine it reports:

- the interp/native ratio (`stats.py --lanes native,interp`);
- the interpreter's operators per op, split into families: `frame` (a value onto the stack
  and back, which native keeps in registers), `store` (field and element reads and writes),
  `records` (records, vectors and texts built, copied, appended, freed), `control`,
  `compute`;
- the bytes it moved (copy / relocate / text);
- the **native-only** rewrites admitted in the functions the routine ran, each with the
  share of the routine's interpreter ops those functions carry.

A second table turns this around, one row per generator rule: the routines and functions
the rule fires in, weighed by the interpreter time those functions carry. These are the
candidates for the IR phase. The weight is the time the functions carry, not a promise of
what porting the rule saves. The per-routine detail (functions, their top operators, their
rewrites) says what the interpreter does there instead.

The report goes to `target/interp-gap/report.md`, and the terminal gets the ranking. The
two censuses behind it are `LOFT_OP_CENSUS` and `LOFT_REWRITE_CENSUS_FN`
([PROFILING.md](../doc/claude/PROFILING.md)). A routine is found by pairing the k-th
row-printing line of `main` with the k-th row the program printed. The region runs from the
last `… = ticks()` before that line up to it. So a new lane needs no annotation as long as
it keeps that shape, and one that does not is reported as not attributed.

## The row protocol

Every lane of every bench prints one tab-separated row per routine, after a header:

```
routine    iters    us    ns_op    px    ns_px    hash
```

`iters` is `--n`, `us` the timed region, `ns_op` the time per op, `px` the items one op
processes (`ns_px` the time per item), and `hash` the result of ONE canonical op, in hex.
A last line `time: <ms>ms sink=<n>` carries the folded results, so no backend can drop the
work as unused.

A bench whose routines run on more than one thread says how many in a header comment of its
`bench.loft` — `// bench-threads: 4` — and `stats.py` pins it to that many cores and measures
it alone.  Undeclared is one thread, pinned to one core: `19_stdlib_par`'s four threads ran on
a single core until it declared them.

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

A twin that wins by a construction loft cannot express — a lazy iterator, a borrowed
slice, a struct in registers — breaks none of these and is NOT rewritten down to loft's
form: it names an abstraction loft is missing, and the finding goes to the language
(`formal/performance.md` `(Perf-Gap)`).

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
