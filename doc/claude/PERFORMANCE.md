
# Performance Analysis

This document records current benchmark results, a root-cause analysis of every
performance gap relative to CPython and hand-written Rust, and a detailed implementation
design for each planned improvement.

**The speed contract itself is formal:** [formal/performance.md](formal/performance.md) —
`(Perf-Like)` a comparison is admissible only between lanes proven output-hash-equal;
`(Perf-Weight)` every shipped routine (stdlib + libraries) is measured per release against
an industry-language reference twin, because drift against a previous loft release compares
to nothing outside the project; `(Perf-Twin)` where no natural counterpart exists a twin is
WRITTEN the moment a hit is expected, never waived; `(Perf-Cure)` the twin MEASURES and
never ships — a failed bar is closed in the engine (or the loft algorithm), because the
libraries stay readable loft (the teaching corpus), and a native rewrite is a recorded
per-routine edge case, not a habit.  The model harness is the drawing
library's `bench/` (loft#1426); @PLN158 generalizes it into the per-library standard read
by the release checklist's `M-perf-pass`.

---

## Contents

- [Profiling a run](#profiling-a-run)
- [Benchmark results](#benchmark-results)
- [How the interpreter executes](#how-the-interpreter-executes)
- [Interpreter vs Python](#interpreter-vs-python)
- [Native vs Rust](#native-vs-rust)
- [wasm vs native](#wasm-vs-native)
- [Design: P1 — Superinstruction merging](#design-p1--superinstruction-merging)
- [Design: P2 — Reduce store indirection on the stack](#design-p2--reduce-store-indirection-on-the-stack)
- [Design: P3 — Confirm integer paths carry no long sentinel](#design-p3--confirm-integer-paths-carry-no-long-sentinel)
- [Design: P4 — Block-copy slice materialisation for primitive vectors](#design-p4--block-copy-slice-materialisation-for-primitive-vectors)
- [Design: P8 — Store-effect classifier](#design-p8--store-effect-classifier) — **the enabling analysis P2, N2, N3, N4 and N5 all want**
- [Design: N1 — Direct-emit local collections in native codegen](#design-n1--direct-emit-local-collections-in-native-codegen)
- [Design: N2 — Omit stores parameter from pure native functions](#design-n2--omit-stores-parameter-from-pure-native-functions)
- [Design: N3 — Remove long null-sentinel from generated code](#design-n3--remove-long-null-sentinel-from-generated-code)
- [Design: N4 — Suppress cr_call_push on `#pure` leaf functions](#design-n4--suppress-cr_call_push-on-pure-leaf-functions)
- [Design: N5 — Inline `integer` arithmetic when operands are provably non-null](#design-n5--inline-integer-arithmetic-when-operands-are-provably-non-null)
- [Design: N6 — Skip the rustc toolchain probe on a native cache hit](#design-n6--skip-the-rustc-toolchain-probe-on-a-native-cache-hit)
- [Design: W1 — wasm string representation](#design-w1--wasm-string-representation)
- [Improvement priority order](#improvement-priority-order)
- [See also](#see-also)
- [Runtime optimisation audit](#runtime-optimisation-audit)
- [String Buffer Allocation and Optimization Opportunities](#string-buffer-allocation-and-optimization-opportunities)
- [Struct Passing, Copies, and Optimization Opportunities](#struct-passing-copies-and-optimization-opportunities)
- [Block Copy Efficiency: Analysis and Recommendations](#block-copy-efficiency-analysis-and-recommendations)
- [Design: O8 — Bulk initialisation of constant data](#design-o8--bulk-initialisation-of-constant-data)
- [Design: BUILD1 — Eliminate the lib/bin double compilation](#design-build1--eliminate-the-libbin-double-compilation)
- [Design: BUILD2 — Persist the native-test binary cache across CI runs](#design-build2--persist-the-native-test-binary-cache-across-ci-runs)
- [See also — bytecode and store internals](#see-also--bytecode-and-store-internals)
- [Startup cache (shipped, default-on)](#startup-cache-shipped-default-on) — **what a rerun actually costs, and which binary you measured**
- [Open work](#open-work)

---

## Profiling a run

`make profile ARGS="--interpret prog.loft"` answers *where did this run spend its time*,
down to the source line — and with `--mem`, *where did its heap go*.
`scripts/profile.sh` is the same thing with flags.

```
make profile ARGS="--interpret prog.loft"
PROFILE_FLAGS="--mem"       # allocation hot spots, by loft LINE, at the peak
PROFILE_FLAGS="--paths"     # + the call paths that reached each allocation
PROFILE_FLAGS="--engine"    # profile LOFT ITSELF with perf, not your program
PROFILE_FLAGS="--annotate"  # the hot function's source LINES
PROFILE_FLAGS="--calls"     # who calls the hot function
PROFILE_FLAGS="--no-cache"  # profile a COMPILE, not a startup-cache reload
PROFILE_FLAGS="--no-warm"   # skip the native pre-build (see "the build is not the run")
```

**The report goes to STDERR.** So `LOFT_PROFILE=1 loft test > out.txt` keeps the test
results and drops the profile, and an empty `out.txt` section reads as *"no profile was
produced"* — which is the same wrong conclusion the two lies below produce, reached a
different way. It cost a consumer one confused measurement. Redirect both (`> out.txt
2>&1`) or keep stderr on the terminal. Stderr is deliberate: the profile must not land in
the program's own output, where it would corrupt whatever reads it.

### A program with no clean shutdown (loft#1089)

The report renders at process **exit**, and a server has none: the operator sends
`SIGTERM` and the process dies. So the one class of program you most want a profile of —
a server under real load — was the class that could not give you one. Three ways in, all
armed only when the profiler is:

```
LOFT_PROFILE=1 LOFT_PROFILE_EVERY=30 loft server.loft   # a report every 30 s, while running
kill -USR1 <pid>      # dump now and KEEP RUNNING — profiles a WINDOW, not a lifetime
kill -TERM <pid>      # dump, then leave (exit 143); SIGINT/Ctrl-C the same way
```

`LOFT_PROFILE_EVERY` is the one to reach for first: it needs no signal, and what has
already been printed survives a hard kill. Each report covers the run **so far**, not the
interval since the last one, so a series reads as a growing picture rather than as slices
to add up. A window is what `SIGUSR1` is for — dump, drive the load you care about, dump
again, and read the difference.

The report is built from the samples on the running interpreter, resolved against the
`Data` they were compiled from, so it can only be rendered from the execute loop: a
signal raises the request and the next operation answers it. A process **idle** in a
blocking read has no operation to answer at, so the report waits for the next request it
serves — and for `SIGINT`/`SIGTERM` a second signal is the ordinary kill, never a hang.
An unprofiled run installs no handlers at all, so ordinary shutdown behaviour is
untouched.

### `LOFT_NET_PROFILE` — what the network did (loft#1088)

The third instrument that reports on a RUNNING program, beside CPU and memory. Its metric
is **margin**, not duration: an operation that COMPLETED, well within its own success
criteria but close enough to a deadline that a slower machine would have missed it, is
what makes a networked test flake, and no other instrument reports it. Every event also
carries wall-clock start and end, so two PROCESSES' streams merge on one timeline — which
is the only way to answer *did the client connect before the server bound?*

```
LOFT_NET_PROFILE=1        # a summary by site at exit
LOFT_NET_PROFILE=trace    # + one line per event, as it happens — reach for this when
                          #   the ORDER of operations is the question
```

**It records at the sockets the RUNTIME owns** — `engine_host`'s kernel, the
`loft debug --serve` browser server, and the wire to a placed library's worker. A
networking LIBRARY opens its own sockets, and arming the switch does not reach them: its
Rust bridge joins this report by calling `loft::net_profile::time(site, budget, || …)`
around its own accept / read / write, which is what puts it on the same timeline with the
same budgets. Armed with nothing recorded, the report says all of this rather than
printing nothing — a silent instrument and a broken one look identical from outside, and
that cost a consumer an investigation.

### Two profilers, and the driver picks

**`perf` measures the engine — loft's own Rust.** For a `--native` run that is also your
program, because your functions were compiled into the binary being sampled and come back
named `n_<yours>`. To profile the front end alone — parse, IR, codegen — use
`--engine -- --interpret --check p.loft`: **`--check` on its own is not front-end-only**,
because the default backend is the compiler, so `check_only` still falls through the native
pipeline and rustc builds a binary it then does not run (the rustc-share guard says so, and
names the missing flag).

**For an interpreted program it is the wrong instrument, structurally.** A loft call
creates no machine frame, so perf's stack walk yields the interpreter's own path —

```
_start → __libc_start_main → main → std::rt::lang_start → loft::main
       → execute_argv → put_stack::<i64>
```

— identical for every program ever run. No sampling frequency fixes that; it is the wrong
stack, not a truncated one. loft keeps the right one itself (`State::call_stack`), so an
interpreted program is sampled over *that*, and the report names loft functions, loft
lines and loft call paths (@PLN140 arc B, `src/profiler.rs`).

So the script decides: a run that executes loft code under the interpreter gets loft's own
sampler, everything else gets perf. `--engine` and `--program` override it.

That includes **test runs** — `loft test` and `--tests` both interpret, and a suite is
usually the biggest interpreted workload a project owns (loft#860). Each test compiles its
own bytecode and runs in its own `State`, so a `pc` means something different in each one;
the samples are therefore resolved to `(function, file:line)` per test and merged on those
labels, never on positions. One report for the whole run, not one per file — 39 banners
rank nothing, and no attribution is lost, because every row already names its file:

```
════ loft CPU profile — 31274 samples over 255 ms across 3 runs ════
── by line (self time) ──
  37.3 %      95 ms  other.loft:3                 other_work
  33.3 %      85 ms  hot.loft:3                   slow_part
```

Two instruments are out of scope there and say so rather than going quiet: `--native` test
runs (nothing to sample — no dispatch loop) and `LOFT_ALLOC_SITES` (it ranks a
*process-wide* peak by bytecode position, and a suite's peak may have been reached in any
of its runs, so those positions have no single `Data` to resolve against).

### The two ways a profile can lie, and what it now says instead

Both were found by pointing the sampler at a real consumer (moros), and both matter more
than a missing feature would, because each ends in something that *looks* like an answer.

**A native run is not sampled at all (loft#865).** The sampler is interpreter-only, and the
DEFAULT backend is native — so `LOFT_PROFILE=1 loft prog.loft`, the command a person
actually types, accepted the variable and exited 0 with an empty terminal. That is
indistinguishable from *"the profiler ran and your program is not the problem"*. A native
run now says so before it starts, naming `--interpret` and `--engine` as the two cures.

**A `use`d library is a cdylib the sampler cannot see into.** This is the sharper one,
because the report is *populated*. A library runs as compiled code, so its functions cannot
be sampled at any rate; their time lands on the loft line that called them. Measured on a
two-function probe where the library loops 150× what the program does:

| | samples | top row |
|---|---|---|
| default | 365 | `100.0 % app_bit` |
| `LOFT_NO_NATIVE_LIBS=1` | 61 824 | `99.5 % lib_grind`, `0.5 % app_bit` |

The ranking is **inverted**, and nothing in the first table hints at it. Note also that
*one* bridge call was enough — the call count measures calls, not work — and that the low
sample count is a symptom, not a sampling-rate problem: the op clock does not tick while
compiled code runs, so a longer run does not help. The CPU report now leads with this
whenever any library call happened, and the "too few samples to rank" line points at the
library rather than at the interval when that is the real cause.

```
════ loft CPU profile — 44339 samples over 1.07 s ════
── by function (self time) ──
  95.2 %     1.02 s  is_prime
   4.8 %      51 ms  main
── by line (self time) ──
  42.5 %     455 ms  bench.loft:7                 is_prime
── hottest paths (innermost 8 frames) ──
  95.2 %     1.02 s  main → is_prime
```

The clock is an **op counter choosing when to sample and the wall clock saying how much**:
each sample carries the nanoseconds since the previous one. That is what keeps a single
heavy native call (a `sort`, a store operation) from counting as one op — the plan's open
question 1, answered in `src/profiler.rs`'s module doc, which also lists what the choice
still cannot do (`par` workers are not sampled; a long op's time lands on the frame at the
*next* sample).

The sampling period is **jittered** around its mean, and that is not a detail. A fixed
period samples one phase of a periodic program: the arc C oracle allocates down two paths
in a known 9:1 ratio, and a fixed every-16th sampler put **100 %** on one and never once
saw the other. Not noisy — confidently wrong, and no sample count would have shown it.

`LOFT_PROFILE=<ops>` sets the mean (default 1024); `LOFT_ALLOC_PATHS=<ops>` the allocation
rate (default 16). `1`, `on` and `yes` all mean "the default rate".

**What it costs.** Nothing when off — the sampler hangs off the dispatch loop's existing
`self.debug.is_some()` branch, and against the pre-@PLN140 binary the benchmark corpus is
unchanged within noise (×0.90–×1.03, in both directions). Armed: **+7–11 %** for CPU
sampling and **+4–7 %** for allocation paths, measured on `bench/02`, `03`, `05` and `10`.

### Where the heap went (`--mem`)

```
════ allocation hot spots — peak 273.4 MiB, captured at 273.4 MiB (100 % of peak) ════
   136.7 MiB       1 store   main_vector<float>    main    bench.loft:5
   136.7 MiB       1 store   main_vector<float>    main    bench.loft:6
```

Three things separate this from the reports it grew out of, and each was a way the old one
answered a question nobody asked:

* **Live stores, not leaked ones.** `LOFT_LEAK_SITES` groups by the same key but over what
  was never freed, so a program that frees everything gets an empty report however much
  memory it used.
* **At the peak, not at exit.** A program that peaks at 1.5 GiB and exits at 10 MB has
  nothing left to report by the time an exit hook runs. The banner names both the peak and
  the total the table was actually captured at, because a table describing a different
  moment than its headline is the plausible-looking wrong answer this exists to refuse.
* **Bytes, not store counts.** `LOFT_ALLOC_REPORT` counts allocations, weighing one 40 MiB
  vector the same as one 32-byte record.

Two blind spots it states rather than hides: **text buffers are Rust `String`s, not
stores**, so they are not counted; and **`--native` has no allocation site at all** —
`alloc_pc` is published by the interpreter's dispatch loop, so a native binary would
report a table of `line 0`. That is a **decline, not a gap** (@PLN140 open question 3):
`--mem` refuses on a native run and points at `--interpret`, which allocates from the same
loft lines.

### The corpus check — `make profile-corpus`

`bench/profile_oracle.tsv` records what each instrument **must** say about a program whose
hot spot is known in advance: `fib`'s time is in `fib`, `02_sum_loop`'s in `main` (the
negative control — four of the five CPU rows would also pass an instrument that just
reported the deepest frame), `09_matrix_mul`'s memory at the two lines that build its
vectors. An instrument that fails a row is **wrong**, so that half is a gate. The share
drift printed beside it never is: shares move with the machine, so the previous local
capture is diffed rather than a committed baseline (@PLN140 open question 5). Rationale
per row: [PROFILE_ORACLE.md](PROFILE_ORACLE.md).

### Sample counts

The perf banner carries the sample count — `self time — 42 samples`. Read it before you
read the percentages: at fifty samples a 2 % row is one sample. `--annotate` used to
annotate whichever symbol won that coin toss — it once printed forty lines of disassembly
about a **one-sample `getenv`** from libc — so it now refuses a symbol whose share rests on
fewer than ~50 samples and says why. A short run wants a higher `--freq`, or more work.

### One-time setup

Sampling a user process needs `perf_event_paranoid <= 1`; the script refuses with the exact
command when it is higher.

```bash
echo 'kernel.perf_event_paranoid = 1' | sudo tee /etc/sysctl.d/99-perf.conf
sudo sysctl --system
```

### The five choices that make a profile honest

These are baked into the script rather than offered as options, because each one is a way a
profile can be confidently wrong.

**The build is not the run.** loft's *default* backend is the compiler: `loft prog.loft`
generates Rust, shells out to rustc, and runs the binary it built. `perf` follows forks, so
recording that command records the **build** — rustc, LLVM and lld take the entire top of
the profile, and the few samples that are your program come back as bare hex, because the
binary is stripped. So a native run is built **once, unprofiled**, and only then recorded;
`--no-warm` opts out. The warm-up really does run your program, side effects and all, which
is why the script announces it.

The lever that symbolizes the binary is the `--native-debug` flag, and it is the only one:
the binary cache key hashes *that flag*, not the environment (`src/main.rs`). Set
`LOFT_NATIVE_KEEP_SYMBOLS=1` on its own and an already-cached **stripped** binary is handed
straight back — the setting applies and nothing changes. `--native-debug` keeps symbols,
emits DWARF line tables, and preserves the generated `.rs`, so `--annotate` lands on
generated Rust carrying its `// loft:<file>:<line>` marker.

What survives is a profile that names your code:

```
25.27%  hot_a-d780ac7f0  [.] loft_native_1057163::n_slow_part
14.66%  hot_a-d780ac7f0  [.] loft::ops::op_add_int
13.86%  loft             [.] loft::use_analysis::first_arg_write_ops   ← front end, not your program
 5.53%  hot_a-d780ac7f0  [.] loft_native_1057163::n_fast_part
```

The `loft` rows are the front end (parse, IR, codegen, cache lookup); the other command is
your compiled program, its functions named `n_<yours>`. When rustc still takes 20 % or more,
the script says so rather than letting a plausible-looking LLVM profile pass for a hot
program.

**Self time, not inclusive.** loft's hot paths are recursive tree walkers — `scopes::scan`,
`use_analysis::collect_defs`, every `for_each_child` descent. Inclusive time hands ~100 % to
the walker at the root and names nothing. Self time names the function actually burning
cycles; `--calls` then tells you who reaches it.

**Frame pointers, not DWARF.** `--call-graph=dwarf` copies stack memory per sample. Against a
walker that recurses hundreds deep that is slow *and* truncates exactly the chains you came
for. The profiling profile is built `-Cforce-frame-pointers=yes`, so `fp` unwinding is both
cheap and complete.

**A separate cargo profile.** `[profile.profiling]` is release plus line tables.
Release itself stays untouched on purpose: RELEASE.md pins a release binary's sha256 and
`make speed` measures release, so adding debug info there changes the artifact both are
about. (The old `make profile` did exactly that — `RUSTFLAGS=-g cargo build --release`.)

**A cache hit is not a compile.** loft answers a second run of an unchanged file from the
startup cache: same command, same output, a tenth of the time, and a flat profile that blames
the store loader. That is the normal result of profiling the same file twice, so the script
detects it and says so — use `--no-cache`, or vary the file's content per measurement.

### Count before you time

For an *asymptotic* question — "why is this quadratic?" — a profiler is the wrong first tool.
It names the hot function but not the exponent, and the hot function is usually innocent. Add
a counter, run it at two sizes, and read the growth.

loft#854 is the worked example. Timing said `scopes::check` was 100 % of an 8 000-element
compile. Counting said `scan` was called 33 832 → 61 832 → 117 832 times for 2 000 → 4 000 →
8 000 elements — *linear*, while time went up 4× per doubling. Two numbers, and the whole
"the walk re-traverses" family of explanations was dead: the walk is linear, so one call had
to be doing O(n) work. Only then is a profiler the right instrument, and it went straight to
`use_analysis::collect_defs`.

## Benchmark results

Wall-clock milliseconds, **best of 3 warm runs**, single core, Linux x86-64, **refreshed
2026-06-25** (v2026.6.0). Run `bench/run_bench.sh` from the project root to reproduce.

> **Read the two measurement notes first — they invalidate naïve numbers:**
> 1. **`native` = optimized (`rustc -O`).** `loft --native` (the *default*) compiles
>    **without `-O`** for fast dev iteration and runs ~6× slower (fib: 2953 ms unoptimized
>    vs 489 ms optimized). The benchmark + this table use the optimized build
>    (`loft --native-release` / the script's `rustc -O`). Quote *that* for "native speed".
> 2. **Run WITHOUT `LOFT_TIMEOUT`.** An armed watchdog makes the NATIVE `checkpoint_fn`
>    do a mutex `try_lock` + deadline check on **every function call** — it inflated fib
>    ~2× (489 → ~1000 ms+) and call-heavy code more. (Side effect worth noting: `loft
>    test` arms the watchdog at 300 s, so the native test path pays this per-call tax — a
>    real follow-up.) The INTERPRETER's per-call breadcrumb (loft#952) is deliberately not
>    built that way and does not carry the tax: two relaxed atomic stores, with the
>    clock-reading deadline test throttled to every 1024th call. Measured at 20 M calls,
>    armed and disarmed are within run-to-run noise (~2 %).

| # | Benchmark | Python | interp | native | Rust | interp/Py | native/Rust |
|---|-----------|-------:|-------:|-------:|-----:|----------:|------------:|
| 01 | fibonacci (recursive, n=38)      | 2445 | 2972 | 489 |  71 | 1.22× | 6.9× |
| 02 | sum loop                         |   67 |   42 |   6 |   4 | **0.63×** | 1.5× |
| 03 | prime sieve                      |   24 |   15 |   4 |   3 | **0.63×** | 1.3× |
| 04 | Collatz lengths                  | 4706 | 1492 | 176 |  88 | **0.32×** | 2.0× |
| 05 | Mandelbrot                       |  141 |   13 |   9 |  10 | **0.09×** | **0.9×** |
| 06 | Newton sqrt                      | 1216 |  657 | 196 | 115 | **0.54×** | 1.7× |
| 07 | string build                     |   51 |   47 |  25 |  20 | **0.92×** | 1.25× |
| 08 | word frequency (hash map)        |   50 |   59 |  41 |   2 | 1.18× | ~~20.5×~~ ⚠ see note |
| 09 | matrix mul (float)               |  131 |  104 |  49 |   2 | **0.79×** | **24.5×** |
| 10 | insertion sort                   |   97 |   73 |  37 |   2 | **0.75×** | **18.5×** |
| 11 | parallel-for                     |   65 |   40 |  17 |   5 | **0.62×** | 3.4× |

> ⚠ **Row 08's `native/Rust` ratio was never like-for-like** (found 2026-08-09). `bench.loft`
> ran `n = 600000` while `bench.rs` ran `n = 100_000` — both added that way in the same commit
> (`df9cb269`), so every published 08 ratio compared loft against a Rust baseline doing one
> sixth of the work. `bench.rs` now uses `600_000` and carries a comment saying the two must
> match. Re-measured like-for-like on one dev machine: Rust `-O` 9 ms, `loft --native-release`
> 50–58 ms — about **6×**, not 20.5×. The absolute numbers are not comparable to this table
> (different hardware), so 08 needs a full `bench/run_bench.sh` re-run before it is quoted
> again. The hash rows are doubly out of date: they also predate **@PLN135**, which measured
> integer-key insert 933 → 505 ms (350 ms with `reserve`) and lookup ~95 → ~75 ms — see
> [plans/135-hash-performance/README.md](plans/135-hash-performance/README.md).

**What changed since the old table (and what it means):**
- **The interpreter now beats CPython on 8 of 11** (interp/Py 0.09–1.22, median ~0.6) — the
  old table had it 1.4–8.85× *slower*. A large, previously-unrecorded interpreter improvement.
  This makes the interpreter optimisations (P1/P2) **low-priority** — it's already fast.
- **Optimized native vs Rust is healthy on compute** (mandelbrot 0.9×, sieve 1.3×, sum 1.5×,
  newton 1.7×, collatz 2.0×) but has **two real gaps**:
  - **Data structures — matrix 24.5×, word-count 20.5×, sort 18.5×** (the `codegen_runtime`/`DbRef`
    indirection). This is **N1** — confirmed the biggest, most consistent native gap.
  - **Recursive calls — fib 6.9×** (was 1.84× in the old table). A **~3× regression** from
    per-call instrumentation (the `cr_call_push` shadow stack + `live_flipped` hot-reload check
    added since). Addressed by **N2/N4** + gating the instrumentation for non-debug builds.
- **wasm not measured** — the script's wasm run failed silently (`|| true` → blank), likely the
  same wasip2 `#native`-symbol gap seen elsewhere. Re-add once that's fixed.

A native/Rust ratio above ~5× signals a structural gap (here: data structures, and recursive
per-call overhead). Single-box, best-of-3 — directional, not a leaderboard.

---

## How the interpreter executes

Understanding the interpreter's execution model is prerequisite to every performance design
below.

### Dispatch loop (`src/state/mod.rs`)

The main execution loop fetches one opcode byte per cycle and calls the corresponding
function from the `OPERATORS` function-pointer **slice** (`src/fill.rs`). Bytes 0–254
are one-byte opcodes; **byte 255 is a two-byte escape prefix** — the loop reads a
second byte `ext` and dispatches `OPERATORS[255 + ext]` (encoding handled by `emit_op`
in `src/state/mod.rs`):

```rust
while self.code_pos < bytecode_len {
    let op = *self.code::<u8>();              // fetch byte, advance code_pos
    if op == 255 {                            // escape: read ext, dispatch beyond 254
        let ext = *self.code::<u8>();
        OPERATORS[255 + ext as usize](self);
    } else {
        OPERATORS[op as usize](self);         // one-byte op — the common case
    }
    if self.code_pos == u32::MAX { break; }
}
```

Each element of `OPERATORS` is a standalone Rust function taking `&mut State`. The slice
currently holds **269 entries**: all 255 one-byte opcodes (0–254) plus 14 escape-range
ops (255–268, reached via the 255 prefix). The escape extends the space to **~511
opcodes** (255 one-byte + up to 256 via `255 + ext`), so **~242 slots are free**.
`emit_op(op_code: u16, …)` hides the encoding from codegen: `< 255` emits one byte,
`≥ 255` emits the `255` prefix followed by `(op_code − 255)`.

There is no `match` at the top level — dispatch is already a hardware indirect branch.
The cost per cycle is: one array index, one indirect branch (potentially mispredicted),
one function-call ABI round-trip, plus the function body itself.

### Stack and variable access (`src/state/mod.rs`)

The execution stack is **not** a `Vec` per call frame. It is a single flat region of
memory inside a `Stores` record, addressed by two fields:

```rust
pub stack_cur: DbRef,   // (store_nr, rec, pos) — the allocated record
pub stack_pos: u32,     // current offset within that record
```

Every `get_stack<T>` and `put_stack<T>` call does:

```rust
pub fn get_stack<T>(&mut self) -> &T {
    self.stack_pos -= size_of::<T>() as u32;
    self.database
        .store(&self.stack_cur)              // lookup by store_nr
        .addr::<T>(self.stack_cur.rec,
                   self.stack_cur.pos + self.stack_pos)
}
pub fn put_stack<T>(&mut self, val: T) {
    let m = self.database
        .store_mut(&self.stack_cur)          // lookup by store_nr (mutable)
        .addr_mut::<T>(self.stack_cur.rec,
                       self.stack_cur.pos + self.stack_pos);
    *m = val;
    self.stack_pos += size_of::<T>() as u32;
}
```

`database.store(&self.stack_cur)` resolves `store_nr` to a `Store` via an indexed
allocation table. This adds one indirection beyond a raw pointer dereference on every
single push and pop, including every arithmetic intermediate value.

### Function calls

`fn_call` pushes the return address (4 bytes) onto the stack and jumps
`code_pos` to the callee. The callee's local variables live above the caller's on the
same flat stack record — there is no frame allocation or deallocation. Return pops
`code_pos` back from the stack.

The overhead per call is: one `put_stack` (store indirection + write), one `code_pos`
update, and the reverse on return. For a million recursive calls this adds up, but the
store-indirection cost on the many arithmetic operations inside the call body dominates.

---

## Interpreter vs Python

> **⚠ Ratios below are SUPERSEDED** by the refreshed table (2026-06-25): the interpreter now
> *beats* CPython on most benchmarks (interp/Py median ~0.6), so the "2–9× slower" framing here
> is stale. The root-cause *mechanisms* (dispatch overhead, store indirection) still hold as the
> per-op cost model; only the headline ratios are out of date. Refresh this section next.

### Summary table

| Group | Benchmarks | Typical ratio | Primary cost |
|---|---|---|---|
| Tight integer loops | 02, 04 | 2–9× | Dispatch overhead per opcode |
| Recursive compute | 01, 06 | 1.4–2.3× | Dispatch × call depth |
| Float loops | 05, 09 | 2.5–2.7× | Same dispatch; FPU hides some |
| Collection-heavy | 08, 10 | 2.2–3.7× | Store indirection on collection access |
| String building | 07 | **0.87×** | loft format-strings beat CPython object churn |

### Root causes (interpreter)

**1. Indirect branch + ABI round-trip per opcode**

The tight inner loop of sum-loop (02) is:

```
var_int  [slot]      → push integer from slot
const_int [1]        → push constant 1
add_int              → pop two, push sum
put_int  [slot]      → pop, store to slot
goto_false [offset]  → pop condition, maybe branch
```

That is 5 `OPERATORS[op](self)` calls per loop iteration, each with a function-call
ABI round-trip (save/restore registers, align stack). CPython's C implementation
executes an equivalent loop body in a single compiled C frame with no function calls.

**2. Store indirection on every push/pop**

Each `get_stack` and `put_stack` resolves `store_nr → Store → raw pointer` before
reading or writing. For sum-loop: 5 opcodes × ~2 stack ops each = ~10 store-indirection
lookups per loop iteration. This competes with CPython which uses a direct C stack
pointer with no extra indirection.

**3. `long` null-sentinel checks**

`long` arithmetic opcodes in `fill.rs` each check whether the operand equals `i64::MIN`
before performing the operation. Collatz (04) uses `long` throughout; this is roughly
one extra conditional branch per arithmetic operation.

**4. Near parity and one win**

String building (07) runs faster in loft (61 ms) than CPython (70 ms) because loft's
format-string concatenation avoids CPython's per-character `PyUnicodeObject` allocation.
This shows the interpreter's overhead is not universal — I/O-bound and allocation-heavy
workloads can favour loft.

---

## Native vs Rust

> **⚠ Ratios below are SUPERSEDED** by the refreshed table (2026-06-25). Current optimized-native
> reality: compute is healthy (1–2×), the big gaps are **data structures** (matrix/word/sort
> 18–25× — N1) and **recursive per-call overhead** (fib 6.9×, regressed from 1.84× — the
> `cr_call_push`/`live_flipped` instrumentation, N2/N4). The "every fn takes `stores`" and
> "`codegen_runtime` indirection" mechanisms below are still the correct root causes.

### Summary table

| Group | Benchmarks | Typical ratio | Primary cost |
|---|---|---|---|
| Pure float compute | 05, 06 | 1.0–1.2× | Near parity — good target |
| Recursive integer | 01, 02, 04 | 1.8–2.2× | `stores` parameter + call overhead |
| Data structures | 08, 09, 10 | 7–16× | `codegen_runtime` vs direct Rust |

### Root causes (native)

**1. Every generated function takes `stores: &mut Stores`**

`src/generation/` emits all loft functions with this signature:

```rust
fn n_fibonacci(stores: &mut Stores, n: i32) -> i32 { … }
```

Even functions that never read or write a store carry this parameter. For recursive
Fibonacci (01, 1.84× gap) with ~39 million recursive calls, `rustc -O` cannot inline
across the `&mut Stores` borrow boundary because `Stores` is a large external type.
The parameter forces a register save/restore on every call frame.

**2. `codegen_runtime` helpers for collection operations**

All vector and hash operations in generated code go through functions in
`src/codegen_runtime.rs`. Each helper:
- takes `stores: &mut Stores`
- decodes a `DbRef` (store_nr, rec, pos) to get to the raw data
- performs bounds and null-sentinel checks
- calls into the underlying `vector::` or `hash::` module

Examples: `OpSortVector(stores, data, db_tp)`, `OpInsertVector(stores, data, …)`,
`OpIterate(stores, …)`, `OpHashRemove(stores, …)`, `OpAppendCopy(stores, …)`.

Hand-written Rust uses `vec.sort()`, `vec.push()`, `map.get()` — zero indirection.
The gaps are word frequency (16×), dot product (12×), insertion sort (7.25×).

**3. `long` null-sentinel in generated code**

Generated code for `long` arithmetic emits the same `i64::MIN` check as the interpreter:

```rust
if v1 == i64::MIN || v2 == i64::MIN { i64::MIN } else { v1 + v2 }
```

For Collatz (04, 2.24×) this appears in every loop iteration. Hand-written Rust uses
plain arithmetic with no sentinel.

**3b. The crate boundary: a runtime helper is only inlinable if it says so**

A `--native` program is a **separate crate** that links loft's runtime rlib, built with
`rustc --edition=2024 -O … --extern loft=libloft.rlib` — **and no LTO**. A plain
non-generic `pub fn` in the runtime is therefore a real call from generated code, and
rustc can never inline it there however small it is. The second cost is the bigger one:
what the helper already computed stays invisible to its caller, so the caller computes it
again.

Indexed reads pay the most, because they are the hottest operation in the language. `v[i]`
emits `vector::get_vector(…)`, then resolves the store a second time in the caller to read
the element — a second resolution that exists only because the first one happened behind a
call rustc cannot see into. Marking that chain inlinable (`vector::get_vector`,
`vector::length_vector`, `keys::store`, `Stores::store`), measured on loft#885's scan over
flat `vector<single>` columns, alternating single runs on a shared machine:

```
before   30.5  31.6  33.4  32.4  33.9  33.7   ns/iter   (median ~33.1)
inlined  21.0  24.2  18.1  18.7  19.7  19.3   ns/iter   (median ~19.5)
```

Same accumulator on every run. About 1.7×, which moves `vector<single>` indexed reads from
~25× hand-written Rust to ~15×.

Two lessons worth keeping:

- **A cross-crate cost cannot be fixed inside the rlib.** `get_vector` looks like it
  resolves the store twice, because it calls `length_vector`, which resolves it again and
  re-reads the same slot. Removing that by hand measured **no change at all**: inside the
  rlib rustc already inlines and folds it. The duplicate that cost real time was the one
  *across* the crate boundary, which no tidying inside can reach.
- **Alternate single RUNS, not builds.** Rebuilding between variants left minutes between
  the two arms. On a shared machine that reported a 35 % gain in one round and none in the
  next, from the same two binaries. Alternating one run at a time separated the two
  distributions with no overlap.

The rule generalises: a runtime helper on a per-element path needs `#[inline]`, and the
reason belongs beside it, because the attribute looks removable and is not.

**3c. Inlining does not buy loop-invariant code motion — the guards prevent it**

Inlining lets the caller *see* the chain; it does not let rustc hoist it. Every store
accessor guards its load — `get_u32_raw` and `get_single` are both
`if rec != 0 && self.valid(rec, fld) { *self.addr(rec, fld) } else { … }` — and LLVM will
not hoist a conditionally-executed load out of a loop, because it cannot prove the load is
safe to speculate on the iterations where the guard is false. So an indexed read re-loads
the vector's record and length on every element even when nothing in the loop can change
them. On the loft#885 kernel that is 12 guarded header loads per iteration against 3 for a
hand-hoisted body.

This is why the remaining half of P2 had to be a **codegen** change and could not be bought
with another attribute: the hoist has to happen where the loop is known, in the emitter.
That half has since shipped — see § Design: P2 → *Shipped: the NATIVE half*, which is where
the gate that makes it sound is written down.

A cheaper hypothesis was worth testing first and turned out to be **wrong**, which is worth
recording so it is not re-run: `Stores::store` calls `strict_stores()`, a `OnceLock` atomic
load guarding a branch that could call `std::env::var_os`, and it sits on the per-element
path while `get_vector`'s own internal resolution uses the plain `keys::store`. That looks
exactly like an optimisation barrier. Replacing those three generated call sites with
`keys::store` measured ~6 % — inside the run-to-run spread on this box, against the 2.5x the
hoist is worth. The barrier is the guards, not the `OnceLock`.

**4. Float near-parity — the target model**

Newton sqrt (06, 1.05×) and Mandelbrot (05, 1.17×) show what the native pipeline
achieves when there are no stores or collections: `rustc -O` sees clean arithmetic and
produces essentially the same machine code as hand-written Rust. This is the quality
target for integer and collection paths after P1–N2 are implemented.

---

## wasm vs native

| Benchmark | native | wasm | ratio | Note |
|---|---:|---:|---:|---|
| fibonacci       | 169 | 257 | 1.52× | Expected wasm overhead |
| sum loop        |  15 |  21 | 1.40× | Expected |
| sieve           |   4 |   6 | 1.50× | Expected |
| Collatz         | 334 | 599 | 1.79× | `long` sentinel amplified by wasm i64 cost |
| Mandelbrot      |   7 |  10 | 1.43× | Expected |
| Newton sqrt     | 159 | 159 | **1.00×** | FPU bound; wasm matches native |
| string build    |  33 |  68 | 2.06× | wasm memory model for strings |
| word frequency  |  32 |  60 | 1.88× | Hash indirection in wasm linear memory |
| dot product     |  36 |  86 | 2.39× | wasm f64 array layout |
| insertion sort  |  29 |  56 | 1.93× | wasm indirect memory for vector ops |

The 1.4–1.8× overhead on compute-bound benchmarks is structural wasm cost (linear memory
model, function-call overhead through wasm module boundary). FPU-bound Newton sqrt
achieves exact parity because the bottleneck is the FPU, not memory access.
The 2× gaps on data structures and strings are design-level issues addressed by W1.

---

## Design: P1 — Superinstruction merging

**Affected benchmarks:** 02 (8.85×), 04 (1.94×), 03 (2.88×), all tight loops
**Expected gain:** 2–4× on integer loops (reduces dispatch cycles by 60–80% in hot paths)
**Cost:** Medium — peephole pass + new opcode entries + new function bodies

### Background

**No longer blocked (corrected 2026-06).** An earlier note here said the opcode table
was nearly full (254/256, "2 slots left") and deferred P1 *"until opcode space is freed,
e.g. by a two-byte escape prefix."* **That escape prefix now exists** — byte 255 escapes
to `OPERATORS[255 + ext]` (see [How the interpreter executes](#how-the-interpreter-executes)),
the table is at **269/511 with ~242 free slots**, and 14 escape-range ops already use it.
So P1 can proceed: its superinstructions land in the escape range, i.e. as **two-byte
opcodes** (`255` prefix + `ext`). The one extra byte-fetch is negligible against the win
— a superinstruction replaces ~4 one-byte ops (4 fetches + 4 indirect calls + the
intermediate stack traffic), so even as an escape op it is a large net reduction. If a
specific superinstruction profiles hot enough to want a one-byte slot, swap it for one of
the 14 cold escape-range ops; `emit_op` makes the encoding transparent either way.

> Encoding note for the operand sections below: the original P1 design assumed one-byte
> slots `240–245`. With the escape in place, superinstructions are **escape-range ops**
> emitted via `emit_op(op_code ≥ 255, …)`; the operand layouts are otherwise unchanged.

The hot-pattern analysis below stands.

### Hot patterns

Profile of a tight integer loop in loft bytecode:

```
var_int   [slot_a]   ; load variable a
var_int   [slot_b]   ; load variable b
add_int              ; a + b
put_int   [slot_c]   ; store to c
```

```
var_int   [slot_i]   ; load loop counter
const_int [limit]    ; load constant upper bound
cmp_lt_int           ; i < limit?
goto_false [offset]  ; exit loop if false
```

```
var_int   [slot_i]   ; load counter
const_int [1]        ; load 1
add_int              ; i + 1
put_int   [slot_i]   ; i = i + 1
```

The 16 available slots cover the following four superinstructions:

| # | Name | Pattern | Operands | Cycles saved |
|---|---|---|---|---|
| 240 | `si_load2_add_store` | `var_int var_int add_int put_int` | a, b, c (3 × u16) | 3 of 4 |
| 241 | `si_load_const_add_store` | `var_int const_int add_int put_int` | a, k, c | 3 of 4 |
| 242 | `si_load_const_cmp_lt_branch` | `var_int const_int cmp_lt_int goto_false` | a, k, offset | 3 of 4 |
| 243 | `si_load2_cmp_lt_branch` | `var_int var_int cmp_lt_int goto_false` | a, b, offset | 3 of 4 |
| 244 | `si_load_const_mul_store` | `var_int const_int mul_int put_int` | a, k, c | 3 of 4 |
| 245 | `si_load2_mul_store` | `var_int var_int mul_int put_int` | a, b, c | 3 of 4 |

Six superinstructions leave 10 slots for future use. Extend to more patterns if profiling
shows additional high-frequency sequences.

### Peephole pass design

**Location:** `src/compile.rs`, after `state.def_code(d_nr, data)`.

The pass operates on the already-emitted bytecode for one function at a time.
It scans from the start of the function's bytecode region and replaces matching windows
in-place. In-place replacement is safe because superinstruction operand encodings are
designed to be at most as wide as the replaced sequence.

```rust
fn peephole(bytecode: &mut Vec<u8>, start: usize) {
    let mut pc = start;
    while pc < bytecode.len() {
        // Peek at next 4 opcodes (each opcode byte is followed by operand bytes).
        // Parse a window: opcode, then however many operand bytes its encoding needs.
        if let Some((si, new_len)) = match_superinstruction(bytecode, pc) {
            rewrite(bytecode, pc, si, new_len);
            // Do not advance pc — try to match again from same position.
        } else {
            pc += instruction_len(bytecode, pc);
        }
    }
}
```

`match_superinstruction` returns `Some((si_opcode_byte, total_bytes_used))` when a
known pattern matches. `rewrite` overwrites the window starting at `pc` with the new
opcode and its merged operands, then fills the remaining bytes with a new `nop` opcode
(or shrinks the Vec if relocation is acceptable — see below).

### Operand encoding

The canonical form for `si_load_const_add_store` (pattern: `var_int a; const_int k;
add_int; put_int c`):

```
[245] [a_lo] [a_hi] [k_b0] [k_b1] [k_b2] [k_b3] [c_lo] [c_hi]
```
- `a` and `c` are u16 slot offsets (same as `var_int` / `put_int`)
- `k` is a i32 constant (same as `const_int`)
- Total: 9 bytes, same as the original 4-instruction sequence:
  `var_int`(3) + `const_int`(5) + `add_int`(1) + `put_int`(3) = 12 bytes → savings 3 bytes

Because the replacement is always ≤ the original sequence length, the bytecode can be
rewritten in-place; excess bytes become `nop` (opcode 0 if `goto` is not 0, or a
dedicated `nop` opcode). This avoids having to relocate any branch targets.

**Alternative: shrink and relocate.** After peephole, walk the bytecode a second time
and update all `goto` / `goto_false` / `goto_word` / `call` target offsets. This
removes `nop` padding but is more complex. Defer until profiling shows the padding
matters.

### Superinstruction bodies (`fill.rs`)

Example for `si_load_const_add_store`:

```rust
fn si_load_const_add_store(s: &mut State) {
    let slot_a = *s.code::<u16>();
    let k      = *s.code::<i32>();
    let slot_c = *s.code::<u16>();
    let a = *s.get_var::<i32>(slot_a);
    let result = ops::op_add_int(a, k);
    s.put_var(slot_c, result);
}
```

This body does no intermediate stack push/pop — it reads both inputs directly from
variables or the constant, computes the result, and writes it directly to a variable.
The store-indirection lookups drop from 5 (`var_int` get + `const_int` push + `add_int`
get×2 + push + `put_int` get + store) to 2 (`get_var` + `put_var`).

### Registration

Add to the end of `OPERATORS` in `fill.rs`:

```rust
pub const OPERATORS: &[fn(&mut State); 246] = &[
    // … existing 240 …
    si_load2_add_store,        // 240
    si_load_const_add_store,   // 241
    si_load_const_cmp_lt_branch, // 242
    si_load2_cmp_lt_branch,    // 243
    si_load_const_mul_store,   // 244
    si_load2_mul_store,        // 245
];
```

### Prerequisite check

Before implementing, confirm that `instruction_len(bytecode, pc)` can be computed from
opcode tables alone (without executing the instruction). Since every opcode's operand
width is fixed and determined by the opcode byte, this is straightforward to add as a
companion to the OPERATORS array (a `static OPCODE_LEN: &[u8; 256]` table).

---

## Design: P2 — Reduce store indirection on the stack

**Affected benchmarks:** 01 (1.42×), 02 (8.85×), 04 (1.94×), 05 (2.55×), 06 (2.32×)
**Expected gain:** 20–50% across all interpreter benchmarks
**Cost:** High — touches `State`, `Store`, and the entire stack-access API

### Shipped: the NATIVE half — a loop-invariant vector header (loft#885)

The indexed-read half of P2 is in. It is native-only; everything below this subsection is
still the interpreter design, which is untouched.

`v[i]` re-derives three facts per element — which store holds the vector, which record its
elements live in, and how long it is. All three are loop-invariant in a loop that writes no
store, and root cause 3c under § Native vs Rust is why `rustc` cannot lift them for us: every
store load is guarded, and LLVM will not speculate a conditional load out of a loop. So the
emitter lifts them, because it is the one that knows where the loop is.

* `vector::VecHeader` is the triple. `vector::vec_header` derives it; a `let __vh_N = …` per
  vector lands immediately before the loop, inside a wrapper block so the prelude is legal
  wherever a loop can appear.
* `vector::get_vector_hoisted` / `Stores::vec_get_hoisted_or_raise_runtime` take the fast
  path **only** for an index in range for the hoisted length. A negative index, an
  out-of-range one, `i64::MIN`, a null or an empty vector all fall back into `get_vector` /
  `vec_get_or_raise_runtime` — so what those answer, and the `IndexOutOfBounds` /
  `NegativeIndex` raise the non-nullable form owes, keep exactly one definition.
* **Stage 2 fuses the element read into the address**, so a scalar read of a hoisted element
  is ONE load. `vector::get_elem_hoisted::<T, VERIFY>` does the bounds test and the typed
  load, and nothing between them: no element `DbRef` built, no `rec == 0` test against the
  null element, no second store resolution from that `DbRef`, and no `rec != 0 && valid(..)`
  re-check inside the getter — the bounds test decided all of it. Covers `OpGetInt` /
  `OpGetSingle` / `OpGetFloat`, whose `Store` bodies are all
  `if rec != 0 && valid(..) { *addr } else { <sentinel> }`; a getter with another shape
  (`OpGetBoolean` masks, `OpGetByte` re-bases, `OpGetCharacter` decodes) is left unfused
  rather than approximated.
* `src/generation/hoist.rs` is the gate, and `src/generation/ops/vector_ops.rs` the three
  emitters. All fall through to the `#rust` template for a read the gate did not cover.
* **`LOFT_NO_ELEM_FUSE=1`** emits stage 2's scalar reads unfused while KEEPING stage 1's
  header — the middle rung of the A/B, and one bisect step finer than the all-or-nothing
  `LOFT_NO_VECTOR_HOIST=1` for a native-only wrong answer in a vector loop. Like its sibling
  it is read at GENERATION time. `Output::fused_element_read` is where all three conditions
  (shape, header present, switch off) are answered, because the emitter and the pre-eval
  collector have to reach the same verdict — the collector hoists an inner `OpGetVector*`
  into a `let _pre_N` that a fused emission ignores, so a disagreement runs the read twice.
* **The pre-eval had to learn about the fusion.** `OpGetVector*` is on `op_uses_stores`, so
  the collector hoists it into a `let _pre_N` before the statement — and a fused emission
  ignores that binding, which would leave the read happening TWICE. `hoist::fused_element_read`
  is the one definition of the fused shape and both sides ask it, so they cannot disagree
  about which reads are folded.

**Measured on the issue's kernel** (three flat `vector<single>` columns, 10M indexed reads),
alternating single runs of one binary with `LOFT_NO_VECTOR_HOIST=1` for the before-half:

```
before  18.0  20.8  18.3  18.8  17.8  18.9   ns/iter   (median ~18.6)
after    8.2   8.1   8.0   8.3   9.3   9.4   ns/iter   (median ~8.3)
```

Identical accumulator (216390.88) on all twelve runs, and the two distributions do not
overlap. **~2.25×** — against the report's hand-written Rust baseline of 1.27 ns/iter, that
takes `vector<single>` indexed reads from ~15× to **~6.5×**.

#### loft#885 stage 2: what the fusion is worth

Three rungs of ONE binary, alternating, six rounds each — `LOFT_NO_VECTOR_HOIST=1` (nothing
hoisted), `LOFT_NO_ELEM_FUSE=1` (header hoisted, element read left as an address then a load),
and the default (both). The middle rung is why `LOFT_NO_ELEM_FUSE` exists: with only the
all-or-nothing switch the two stages can be measured together but never apart, so what stage 2
was worth on top of stage 1 could only be projected — and the projection was wrong by 2×.

Kernel: three flat `vector<single>` columns, 10 000 elements, 3 340 sweeps — 100.2M indexed
reads, self-timed with `ticks()` so the `rustc` compile is outside the number.

```
                ns/read (6 alternating runs)                median   min
pre-885    42.74 43.25 43.80 43.95 44.65 46.13              43.88   42.74
stage 1     30.95 31.17 31.18 31.25 31.35 31.48             31.21   30.95
stage 1+2    9.12  9.22  9.63  9.64  9.69 10.32              9.64    9.12

stage 1 over pre-885   1.41x
stage 2 over stage 1   3.24x     <- projected ~1.4x
stage 1+2 over pre-885 4.55x
```

Identical accumulator (`116879542.5`, an f64 — an f32 one saturates and stops registering the
small terms, so it cannot witness a divergent read) on all eighteen runs, and the three
distributions do not overlap.

**Read the RATIOS, not the absolute ns.** This ran on a box shared with other agents' loft
builds and a browser (load 6.6–9.6) and pinned to four cores, so every absolute figure is
inflated — the alternation is what makes the comparison survive it, since drift lands on all
three rungs equally. The kernel is also not the one the stage-1 table above used (it
accumulates in `float` and coalesces each read), so its numbers are not comparable to that
table's; and because that constant per-read work sits in both halves of every ratio, these are
LOWER bounds on the code-path ratios. Re-bless on a quiet box before quoting an absolute.

**The gate is an ALLOW-list, and that is the load-bearing decision.** § Design: P8 catalogues
five hand-maintained *deny*-lists of "which op mutates" that have already drifted; a hoist
built on one of those would turn an omission into a silent wrong read. So `hoist.rs` names the
ops it can vouch for and assumes every other one writes: an op missing from the list costs the
optimisation, never correctness. Two rules qualify an op —

1. it is named as a constant or a reader (`OpGetVector`, `OpLengthVector`, the typed field
   getters, the null constants); or
2. it declares at least one parameter and **every parameter is a plain runtime scalar**.

Rule 2 turns on `const` not being a value. A `const` parameter is a compile-time slot number,
type id or field offset, and that is exactly the channel the scalar-signature ops that DO touch
state reach it through: `OpDatabase(pos, db_tp)` allocates a store, `OpCoroutineNext(value_size)`
resumes a generator that can append to anything, `OpFreeText(pos)` releases one. By signature
alone those three are indistinguishable from `OpAddInt`; by this rule none of them qualifies.

The walk follows calls, so `for i in 0..len(v)` still hoists — `len` is `t_6vector_len`, whose
whole body is `OpLengthVector`. `CallRef`, `par` and `yield` decline: what runs is not this body.

Two things worth keeping:

* **Where a native op's parameters live decides the whole gate.** A native op has no body and
  therefore no variable table, so `variables().arguments()` answers the EMPTY list — which
  "are they all scalar?" *accepts*. The first cut of this gate read parameters there, and
  every mutator classified as a reader: `v.remove(0)` ran inside a hoisted loop and the kernel
  answered 120 where every other backend said 38. Parameters come from `attributes()`, and
  `tests/hoist_gate.rs::native_op_parameters_are_visible` pins that so an empty list can never
  read as safe again.
* **`LOFT_HOIST_VERIFY=1` is the gate on the gate.** It emits the checking monomorphisation of
  every hoisted read, which re-derives the header and panics on a mismatch — so a hole in the
  allow-list surfaces under one suite run instead of as a corrupted read later. It is a
  generation-time switch, not a runtime one, because the check costs exactly the loads the
  hoist removed; and a const parameter rather than a `debug_assert!`, because loft's own
  library is built with debug assertions off and a guard written that way could never fire.
  Re-classifying `OpRemoveVector` as a reader on purpose confirmed both halves: unarmed the
  program answered 120, armed it panicked naming the stale header.

**Stage 2 landed** — fusing the element read into the address computation (the `DbRef` →
`stores.store(&db)` → `get_single` chain), on the gate stage 1 built. It was projected at a
further ~1.4×; measured, it is worth **~3.2×**, which is more than stage 1 itself. The
projection counted the loads removed and not what they cost: the stage-1 form still BUILDS a
`DbRef`, tests its `rec` against the null element, and then resolves the store a SECOND time
out of it inside the getter — and it is that second resolution, not the arithmetic, that
dominates. See § loft#885 stage 2: what the fusion is worth.

### Background

Every `get_stack<T>` and `put_stack<T>` call currently goes through:

```
database.store(&self.stack_cur)          // HashMap/vec lookup by store_nr
  .addr::<T>(self.stack_cur.rec,         // compute raw pointer from record
             self.stack_cur.pos + self.stack_pos)
```

The `database.store()` lookup is at minimum an array index into `allocations`, but the
raw pointer to the record's memory changes whenever the underlying `Store` reallocates.
This means the pointer cannot be cached across calls.

### Proposed change: cache the raw stack pointer

Add a `stack_base: *mut u8` field to `State` that is refreshed once per function call
(when `stack_pos` changes structurally, not on every push/pop):

```rust
pub struct State {
    // … existing fields …
    stack_base: *mut u8,   // raw pointer to start of stack record
}
```

After every `fn_call` and `op_return`, refresh:

```rust
fn refresh_stack_ptr(&mut self) {
    self.stack_base = self.database
        .store_mut(&self.stack_cur)
        .record_ptr_mut(self.stack_cur.rec, self.stack_cur.pos);
}
```

Then `get_stack` and `put_stack` become pointer arithmetic with no extra lookup:

```rust
pub fn get_stack<T>(&mut self) -> &T {
    self.stack_pos -= size_of::<T>() as u32;
    unsafe { &*(self.stack_base.add(self.stack_pos as usize) as *const T) }
}
pub fn put_stack<T>(&mut self, val: T) {
    unsafe {
        *(self.stack_base.add(self.stack_pos as usize) as *mut T) = val;
    }
    self.stack_pos += size_of::<T>() as u32;
}
```

`get_var` and `put_var` become similarly simple: `stack_base - slot_offset`.

### Safety requirement

`stack_base` must be **invalidated** whenever the underlying store could reallocate:
- When a new record is allocated (`OpNewRecord`, `OpDatabase`)
- When a vector grows (`OpInsertVector`, `OpAppendCopy`)

In those cases, `execute()` must call `refresh_stack_ptr()` after the operation.
The simplest approach: make `OPERATORS` entries that allocate call `refresh_stack_ptr`
unconditionally at their end. Add a helper flag to `State`:

```rust
pub stack_dirty: bool,  // set by any allocation op; checked at top of loop
```

```rust
while self.code_pos < bytecode_len {
    let op = *self.code::<u8>();
    OPERATORS[op as usize](self);
    if self.stack_dirty {
        self.refresh_stack_ptr();
        self.stack_dirty = false;
    }
    if self.code_pos == u32::MAX { break; }
}
```

This adds one branch per loop iteration (cheaply predicted) and eliminates the
`database.store()` lookup on every arithmetic push/pop.

### Risk

The `Store` backing the stack record must not move between `refresh_stack_ptr` and
the next push/pop. This holds as long as no allocation occurs on the stack store itself
between refreshes. The stack store (`stack_cur`) is never modified by collection
operations — those use different stores — so the invariant is maintainable.

### Alternative (lower risk, lower reward)

If the raw-pointer approach is too risky, a smaller improvement: cache
`&self.database.allocations[stack_store_nr]` as a field. This saves the `HashMap`
or `Vec` index lookup but still requires the `rec + pos` offset calculation. Estimated
gain: 10–20% vs 20–50% for the full approach.

---

## Design: P8 — Store-effect classifier

**Wanted by:** P2 (the indexed-read hoist — its enabling gate), N2 (omit `stores` from pure
fns), N4 (`cr_call_push` on pure leaves).
**Cost:** none per definition, per IR node, per call site or per store — the analysis this
needs already exists and is computed on demand.

### The mechanism already exists — use it, do not build beside it

`parser::find_written_vars` walks a body and collects the variables it writes, and
`parser::callee_param_writes` summarises a callee as a **per-parameter** write vector,
memoised in a `HashMap<u32, Vec<bool>>`, breaking recursion with a placeholder entry and a
monotone merge. That is exactly the interprocedural, argument-granular question an
optimiser needs to ask, and it already handles the awkward parts: `Type::RefVar` parameters,
writes reaching a variable through a field spine (`collect_vars_in`, so `c.items += other`
marks `c`), and `OpCopyRecord` writing through its *second* argument.

So P2's gate is a query, not a new subsystem: run `find_written_vars` over the loop body and
ask whether the hoisted vector's variable is in the written set. **Adding a per-definition
effect field would be a fourth producer of a fact that already has a home** — and the count
of producers is precisely this area's problem.

### What is actually missing

**1. One home for the leaf set.** "Which native op mutates, and through which argument" is
today asserted in five places that no test compares:

| producer | shape | note |
|---|---|---|
| `find_written_vars` (`parser/mod.rs`) | 10 exact names + 3 prefixes | the keystone |
| `find_field_written_vars` (same file) | the same list minus `OpAppendStack*` / `OpClearStack*` | a hand-maintained twin; its own comment says *"mirror the find_written_vars set"* |
| `#impure(parent_write)` (`default/01_code.loft`) | 15 declarations | the par-safety analyser's view |
| `op_uses_stores` (`generation/pre_eval.rs`) | 17 names | a **borrow-conflict** list, not a mutation list — `OpGetVector`, a pure read, is on it |
| `&mut s.database` in `#rust` templates | 9 | most mutators have no template at all |

They have already drifted. `OpReplaceVector` writes through its first argument
(`vector_replace(&r, &other, tp)`), is declared `#impure(parent_write)`, and is emitted from
`parser/control.rs` at two sites in the return machinery — yet it appears in **neither**
walker list. `OpCastVectorFromText`, `OpClaimChildRec`, `OpFinishRecord` and `OpHashAdd` are
declared and likewise absent (`OpHashAdd` has no emit site, so it is inert today).

**2. A gate that makes an omission loud.** The declaration count is irreducible — ops
genuinely differ — so the cure has to attack *silence*, not the site count. A test-only
runtime witness records the executing opcode from `Store`'s mutating entry points (`claim`,
`resize`, `delete`, the `set_*` family, `copy_block*`) into a 256-bit bitset; a harness test
runs the suite and asserts **observed ⊆ declared**. A missing declaration then fails the
first time any test exercises the op, instead of surfacing later as a corrupted read. The
gate must also report the ops **no test reached**, or "never observed" reads as "declared
correctly" — the same silence in a new place. 32 bytes, `#[cfg]`'d out of release.

**3. Aliasing, which the existing analysis does not cover.** `find_written_vars` is
*variable*-granular, and loft aliases by default: `w = v; w += [x]` marks `w` written and
leaves `v` clean, while both name the same store. Every consumer of this analysis inherits
that gap; for P2 it is load-bearing, because a hoisted triple survives the check and is then
invalidated through the alias. The conservative first cut is to decline the hoist whenever
any written variable shares a store with the hoisted vector, which costs the numeric kernels
nothing — their loop bodies write nothing at all.

### Why the record/vector granularity needs no extra representation

A hoisted `(store_nr, record, length)` triple stays valid exactly while no write reaches
**that vector's** container slot or backing record — and a store relocates only the record it
is resizing, never a bystander. `Store::resize` grows in place when it can and otherwise
relocates its own record (`claim` → `copy` → `delete`, returning a new record number);
`claim` takes free space, `delete` touches only free blocks, and `resize_store` reallocates
the backing buffer while record numbers, being word offsets into it, survive unchanged. So
appending to one vector cannot invalidate a triple held for another, and the per-argument
answer `callee_param_writes` already returns is sound rather than merely optimistic.

### Ruled out

- **Reusing `Store::generation` as a runtime backstop.** It is bumped in `claim`, `resize`
  and `delete`, the bump in `resize` precedes the grow-in-place early return, and the
  coroutine machinery already consumes it — so it reads as a free witness for "my triple
  moved". It is not: `remove_vector` shifts elements with `copy_block` and writes the new
  length with `set_u32_raw`, calling none of the three, so a `remove` inside the loop changes
  the length with no bump at all. Widening `generation` would also change when suspended
  generators consider themselves invalidated.
- **A new per-definition effect field.** Superseded by `callee_param_writes`, which computes
  the same summary on demand and stores nothing.

### Open

Whether `OpReplaceVector`'s absence from both walkers is reachable as a wrong answer — the
consumers are `check_ref_mutations`, B1.4's `construct_fresh_rewrite`, `use_analysis`'s
`mut_max_pos`, and @PLN101's value-struct copy-elision — is unprobed. The observation gate's
coverage of the op table is unknown until it is built, and it bounds what the gate is worth.

---

## Design: P3 — Confirm integer paths carry no long sentinel

**Affected benchmarks:** 02, 10 (minor — already separated by opcode)
**Expected gain:** 2–5% on pure integer benchmarks
**Cost:** Low — mostly verification + one test

### Background

`integer` (i32) and `long` (i64) already have separate opcode variants in `fill.rs`
(`add_int` vs `add_long`). The question is whether any `integer` path inadvertently
checks `i64::MIN`.

### Design

1. **Grep audit:** Search `fill.rs` for `i64::MIN` and `i32::MIN`. Confirm they appear
   only in `*_long` functions, never in `*_int` functions.

2. **Compile-time enforcement:** Add a `static_assertions` check in `fill.rs` or a
   test that ensures the `op_add_int`, `op_mul_int`, `op_sub_int` functions in
   `src/ops.rs` contain no branch comparing to `i64::MIN`:

   ```rust
   #[test]
   fn integer_ops_have_no_long_sentinel_checks() {
       // Read ops.rs source, assert no "i64::MIN" appears in *_int functions.
       // Achievable via include_str! + string search.
   }
   ```

3. **If violations exist:** Separate the dispatch table into `op_add_int(a: i32, b: i32)
   -> i32` (no sentinel) vs `op_add_long(a: i64, b: i64) -> i64` (sentinel). The
   `integer` opcode calls the `i32` variant exclusively.

This is a verification task that may yield no changes if the separation is already clean.

---

## Design: P4 — Block-copy slice materialisation for primitive vectors

**Affected workloads:** any consumer that does `local = src_vec[a..b]`
or `s.v = src_vec[a..b]` on a vector of primitive elements (i32, u8,
single, float, integer, character).  Discovered alongside @P287 on
2026-05-20 when the audience-demo projector wanted a heat-field ring
buffer trim.
**Expected gain:** roughly 5–10× on primitive-typed slice copies of
length ~1 000 elements.  Larger slices benefit proportionally more.
**Cost:** Medium — one new opcode (`OpAppendVectorSlice`), one
parser-side fast-path dispatch, one IR-shape detector.

### Background

@P287 made `local = src[a..b]` and `s.v = src[a..b]` auto-materialise
(rather than crashing the scope analyser).  The materialisation IR is
shape-correct but element-by-element through the generic record
allocator:

```
loop "Slice materialise" {
  _elm = OpGetInt(OpGetVectorNullable(src, 8, idx), 0)
  _rec = OpNewRecord(tmp, kt, fld)
  OpSetInt(_rec, 0, _elm)
  OpFinishRecord(tmp, _rec, kt, fld)
}
```

For 1 000 i32 elements that's ~5 000 bytecode dispatches + 1 000
record allocations + 1 000 record finishes — every per-element copy
goes through the same record-system path used by struct-typed
vector inserts (where the per-element allocator overhead is
unavoidable because each record has its own fields and deps).

For PRIMITIVE element types (i32, u8, single, float, integer, character)
the record allocator is entirely unnecessary — the destination is
just a contiguous byte buffer of `len * sizeof(elem)` bytes.  The
existing `OpAppendVector` (whole-vector replace) already uses
`copy_block` / `copy_block_between` from `src/database/structures.rs`
for exactly this case; the slice path should too.

### Proposed opcode

```
OpAppendVectorSlice(dst: vector<T>, src: vector<T>, start_idx: i32, end_idx: i32, known_type: u16)
```

Semantics:
- Resolve `dst` and `src` to byte ranges in their respective stores.
- Compute `len = end_idx - start_idx` (or `src.len() - start_idx` for
  open-end `[a..]`); clamp to source length.
- Call `vector_set_size(dst, len, sizeof(elem))` to allocate the
  destination capacity.
- Issue one `copy_block` (same-store) or `copy_block_between`
  (cross-store) for `len * sizeof(elem)` bytes from
  `src.bytes[start_idx * size .. end_idx * size]` to the destination
  range.
- Per-element type-specific fixup ONLY for linked element types
  (text, sub-structs).  For primitive elements (the case this
  optimisation targets) the byte copy is the whole job.

This is the same shape as the existing `vector_add` block-copy fan-out,
generalised over a `[start, end)` window instead of `[0, src.len())`.

### Parser-side dispatch

In `parse_assign_op` (the @P287 branch), when the iterator's source
expression is a plain `OpGetVectorNullable(src_var, …)` over a
primitive-typed vector AND the slice bounds are statically simple
(`Value::Iter` whose `init` sets `_index = lo` and `next` increments
+ tests `idx < src.len()` or `idx <= hi`), emit a single
`OpAppendVectorSlice(dst, src, lo, hi or src.len(), kt)` instead of
the per-element loop.  Fall back to the generic per-element loop when:

- The iter source is not a vector index (e.g. range iter, collection
  iter, coroutine iter — those are the generic-only path).
- The element type is linked (text, sub-struct, nested vector) — the
  per-element path's `OpNewRecord` / `OpFinishRecord` allocates the
  deps correctly; the byte copy alone would leave dangling dep slots.

The fast-path check is a couple of `matches!` on the `Value::Iter`'s
`init` / `next` shape; cheap, no escape analysis needed.

### Validation

Two micro-benchmarks: copy 1 000-element `vector<i32>` slice (a)
to a local, (b) to a struct field.  Compare bytecode-dispatch count
+ wall-clock against the existing per-element loop.  Native codegen
inherits the same op so the same speedup applies once the native
emit path adds an `OpAppendVectorSlice` runtime helper that issues
the same `copy_block`.

### Prerequisites

- @P287 already shipped — defines the IR shape this optimisation
  recognises and replaces.
- Opcode-table headroom: the two-byte escape (byte 255 → `OPERATORS[255 + ext]`,
  see § How the interpreter executes) leaves **~242 free escape-range slots**, so
  P4's one new opcode (`OpAppendVectorSlice`) is trivially available — emit it via
  `emit_op` in the escape range, no retirement needed.

---

## Design: N1 — Direct-emit local collections in native codegen

**Affected benchmarks:** 08 word-count (20.5×), 09 matrix (24.5×), 10 sort (18.5×) — *refreshed 2026-06-25*
**Expected gain:** 5–15× on data-structure benchmarks; closes the native/Rust gap
**Cost:** High — new analysis pass, new emit path, extended type system in codegen

> **Status (investigated 2026-06-25): the LOCAL-ONLY design below is mis-scoped — it would
> not move the target benchmarks, and its real foundation is the @PLN85 ownership model.** Two
> findings:
> 1. **The benchmark vectors ESCAPE.** In `10_sort`, `data` is passed to `insertion_sort(arr)`
>    and `arr` is a `DbRef` *parameter* mutated in place — so neither is "a local used only
>    within one function." The 18.5× gap is the per-access indirection on a vector that
>    **crosses a function boundary** (`stores.vec_get_or_raise_runtime(...)` + `stores.store(&db).get_int(...)`
>    per `arr[j]`). Closing it needs the vector to be `Vec<i64>` **across the call** (caller's
>    `data: Vec`, callee takes `&mut Vec`) — *not* the local-only emit. The local-only slice is
>    sound but moves few real programs.
> 2. **The escape/representation fact N1 needs IS the ownership work.** The existing
>    `scopes.rs::escapes_value`/`guard_escapes` is partial (only "handed out as-is" — return /
>    yield / tuple), does **not** track "passed to a function" as an escape, and is for
>    store-*freeing* decisions, not representation choice — explicitly "left for the fix's full
>    escape analysis." That full analysis is **@PLN85** (ownership as a type fact). Building a
>    separate N1 escape pass would duplicate it and add a soundness surface in loft's #1-priority
>    heap area (a missed escape → a `Vec` where a `DbRef` is expected → corruption).
>
> **Recommendation:** sequence N1 as a **consumer of the @PLN85 ownership/representation fact**,
> not a standalone pass. Once "is this vector locally owned / does it escape (incl. across calls)"
> is a sound type fact, N1 is a clean translation (facts-in-types per CODEGEN_METHOD): read the
> fact → emit `Vec`/`&mut Vec` vs the store path. The widened scope (Vec across boundaries) is
> what actually moves the benchmarks, and it is even more dependent on that fact. Until then, the
> local-only slice is the only sound piece, and it isn't worth the risk for its narrow payoff.

### Background

All vector and hash collection access in generated Rust currently goes through
`codegen_runtime` helpers that take `stores: &mut Stores` and decode `DbRef` pointers.
For a local `vector<integer>` used only within one function, the correct Rust type is
`Vec<i32>` — no stores, no DbRef, no bounds-check beyond Rust's built-in `panic`.

### Escape analysis pass

A new pre-pass over the IR (run once per function definition, before native code
generation) marks each local variable with one of:

```
Local      — declared in this function, never assigned to a store field
             and never passed by reference to another function
Escaping   — passed by &ref to another function, assigned to a struct field,
             or stored in a Store
External   — parameter or return value
```

Only `Local` variables qualify for direct emit. The analysis is conservative: if in
doubt, mark `Escaping`.

**Rules for `Local`:**
- `Value::Var(v)` where `v` is declared in the current function body → start as `Local`
- `Value::Call(_, args)` where arg is `Value::Ref(v)` → mark `v` as `Escaping`
- `Value::Store(field, v)` → mark `v` as `Escaping`
- `Value::Assign(dest, v)` where `dest` is a struct field → mark `v` as `Escaping`

### Direct-emit type mapping

For a `Local` variable of loft type `vector<T>`, generate Rust type:

| loft type | Rust direct type |
|---|---|
| `vector<integer>` | `Vec<i64>` |
| `vector<float>` | `Vec<f64>` |
| `vector<text>` | `Vec<String>` |
| `index<integer, T>` (local hash) | `HashMap<i32, T>` |
| `index<text, T>` (local hash) | `HashMap<String, T>` |

### Operation mapping

When emitting operations on a `Local` variable, bypass codegen_runtime:

| loft operation | current emit | direct emit |
|---|---|---|
| `v[i]` (get) | `vector::get_vector(&v, size, i, &allocations)` | `v[i as usize]` |
| `v[i] = x` (set) | `vector::set_vector(&mut v, size, i, x, &mut alloc)` | `v[i as usize] = x` |
| `v.length` | `OpSizeofRef(stores, v)` | `v.len() as i32` |
| `v.append(x)` | `OpAppendCopy(stores, v, 1, tp)` | `v.push(x)` |
| `v.sort()` | `OpSortVector(stores, v, tp)` | `v.sort()` |
| `h[k]` (get) | hash::find + store decode | `h.get(&k).copied()` |
| `h[k] = v` | hash operations through stores | `h.insert(k, v)` |

### Declaration site

For a `Local` vector, emit its declaration as a `Vec`:

```rust
let mut var_counts: Vec<i32> = Vec::new();
```

instead of the current:

```rust
let mut var_counts: DbRef = stores.null();
```

Its `drop` at end of scope is automatic — no `OpFreeRef` call needed.

### Interaction with function calls

If a `Local` vector must be passed to a function that expects `DbRef`, it is not
`Local` by the escape analysis above — it has `Escaping` status and uses the existing
store-backed path. This ensures correctness without special cases.

### Changes to `src/generation/`

1. Add `fn escape_analysis(def_nr: u32, data: &Data) -> HashMap<u16, Locality>`.
2. In `Output::output_code_inner`, check `locality[var]` before emitting any
   collection operation.
3. Add a `direct_emit_vec_op` and `direct_emit_hash_op` path alongside the existing
   `codegen_runtime` call emitter.

### Verification strategy

Add a new benchmark test (`tests/bench/`) that asserts the generated Rust for
`09_matrix_mul.loft` contains `Vec<f64>` and no `OpAppendCopy`. Run `make ci` to
ensure the native pipeline produces correct output for all 10 benchmarks.

---

## Design: N2 — Omit stores parameter from pure native functions

**Affected benchmarks:** 01 (1.84×), 06 (2.32×)
**Expected gain:** 10–30% on recursive compute benchmarks
**Cost:** ~~High~~ → **Medium** (audited 2026-06-25 — see status).

> **Status (audited 2026-06-25): OPEN, but the purity prerequisite is already done.**
> The design below says to "add `fn is_pure`" — but a full purity analysis already
> exists: `def.purity` is computed in `src/parser/definitions.rs` + `src/scopes.rs`
> (the `Purity::{Pure, Impure(ParentWrite|HostIo)}` lattice, propagated across the call
> graph and consumed by `scopes.rs` for effect analysis). So the costly half is built;
> what remains is purely the **codegen** half — emit the `n_foo_pure` (stores-less)
> inner function for `Purity::Pure` defs and call it from other pure defs. `src/generation/`
> does **not** read `def.purity` today. Cost drops from High to Medium accordingly. (Use
> the existing `def.purity`; do **not** add a second `is_pure` per the design's §"Purity
> analysis implementation" — that pass is redundant now.)

### Background

Every generated function is currently emitted as:

```rust
fn n_fibonacci(stores: &mut Stores, n: i32) -> i32 {
    if n <= 1 { return n; }
    n_fibonacci(stores, n - 1) + n_fibonacci(stores, n - 2)
}
```

The `stores: &mut Stores` parameter is an 8-byte pointer that must be saved and
restored across recursive calls. `rustc -O` cannot eliminate it because `Stores` is an
externally-defined large struct. For Fibonacci this adds roughly one register
save/restore pair per call — measured cost is 1.84× vs hand-written Rust.

### Purity definition

A function is **pure** for native codegen purposes if:
1. It does not read or write any `Store`
2. It does not call any non-pure function
3. It has no `Format`, `IO`, `HashFind`, `NewRecord`, `FreeRef`, or similar operations
   in its IR

Purity is determined by a recursive scan of `def.code: Value` before `generation/`
runs.

### Pure function signature

```rust
fn n_fibonacci_pure(n: i32) -> i32 {
    if n <= 1 { return n; }
    n_fibonacci_pure(n - 1) + n_fibonacci_pure(n - 2)
}
```

`rustc -O` can now inline or tail-call-optimise this freely.

### Entry-point wrapper

The non-pure `n_fibonacci` wrapper (called from stores-using code) delegates:

```rust
fn n_fibonacci(stores: &mut Stores, n: i32) -> i32 {
    n_fibonacci_pure(n)
}
```

This keeps the call interface uniform while giving `rustc` the pure inner function
to optimise.

### Purity analysis implementation

Add `fn is_pure(def_nr: u32, data: &Data, cache: &mut HashMap<u32, bool>) -> bool`
to `src/generation/`. Scan `data.def(def_nr).code` recursively:

```rust
fn is_pure(v: &Value, data: &Data, cache: &mut HashMap<u32, bool>) -> bool {
    match v {
        Value::Call(d_nr, args) => {
            let def = data.def(*d_nr);
            if def.name.starts_with("Op") { return false; }  // codegen_runtime op
            if def.rust.contains("stores") { return false; } // uses stores in template
            let callee_pure = *cache.entry(*d_nr).or_insert_with(|| {
                is_pure(&def.code, data, cache)
            });
            callee_pure && args.iter().all(|a| is_pure(a, data, cache))
        }
        Value::Block(vs) => vs.iter().all(|v| is_pure(v, data, cache)),
        Value::If(c, t, f) => is_pure(c, data, cache) && is_pure(t, data, cache)
                               && is_pure(f, data, cache),
        // Literals, variables, arithmetic — always pure
        Value::Int(_) | Value::Float(_) | Value::Text(_) | Value::Boolean(_)
        | Value::Var(_) | Value::Assign(_, _) => true,
        // Anything involving stores or IO
        Value::Ref(_) | Value::Store(_, _) | Value::Format(_) => false,
        _ => false,  // conservative: unknown nodes are not pure
    }
}
```

Memoise results to avoid exponential recursion on call graphs.

### Changes to `src/generation/`

1. Add `fn is_pure` (above).
2. In `output_native_reachable`, for each pure function, emit both `n_foo_pure`
   (no `stores`) and `n_foo` (wrapper with `stores`).
3. In `output_function`, when emitting a call to a pure function from within another
   pure function, emit `n_foo_pure(…)` directly.

---

## Design: N3 — Remove long null-sentinel from generated code

**Affected benchmarks:** 04 (2.24×)
**Expected gain:** 1.3–1.5× on Collatz and any `long`-heavy generated code
**Cost:** **Medium+ and soundness-critical** (corrected 2026-06-25 — the original "Low" was
wrong; see Status + the full design below). Medium for slices 1–2, Large with range analysis.

> **Status (investigated 2026-06-25): OPEN — and NOT Low-cost; the cheap version is
> UNSOUND.** The `_nn` long ops exist in `src/ops.rs` (`op_add_long_nn`, …, 6 fns) but are
> dead code. The trap is *why* they're dead. The original "Low cost" estimate assumed a
> nullability classifier was easy — it is the opposite:
> - **`op_add_int(v1,v2) { op_add_long(v1,v2) }`** — the common `integer` path *is* the
>   long path, and its `if v1 != i64::MIN && v2 != i64::MIN { … } else { i64::MIN }` guard
>   is **the core null-PROPAGATION semantics for all integer/long arithmetic** (a null
>   operand flows to a null result through it) — **not** a removable backstop.
> - **No definite-non-null analysis exists.** The only nullability machinery is the
>   `OpAddInt → OpAddIntNullable` swap in `parser/operators.rs::rewrite_outer_arith_to_nullable`,
>   which is the **`??` operator** mechanism (swap so a fault-prone op returns its sentinel
>   silently for `??` to discharge) — it marks where nulls *are expected*, the opposite of
>   proving non-null. There is a narrow `expr_not_null` flag (scoped to a `??` LHS) and a
>   field-level `not_null` bit, but no per-operand dataflow.
> - So emitting `_nn` for an operand that *could* be null produces a **silent wrong result**
>   — in loft's #1-priority (heap/value soundness) area. Plus an overflow edge: `_nn` chains
>   diverge from the sentinel form when an inner op overflows to `i64::MIN`.
>
> **Real cost: Medium+, soundness-critical** — it needs a sound definite-non-null dataflow
> (or to restrict `_nn` to operands a type-level `not_null`/`expr_not_null` fact already
> proves non-null — a narrow but sound subset). The 6 op stubs are the trivial 5% done.
> N5 (`integer`) is the same path (`op_add_int == op_add_long`), not a separate item. **Not
> recommended as a near-term pick** — modest native gain (rustc -O predicts the `!= MIN`
> branch well), real soundness risk; prefer N1 (safe, mechanical, larger).

### Background

The current generated code for `long` arithmetic, e.g. addition, is:

```rust
// ops::op_add_long as emitted today
if v1 == i64::MIN || v2 == i64::MIN { i64::MIN } else { v1 + v2 }
```

For Collatz, this pattern appears in every loop body. The two comparisons and the
conditional branch prevent `rustc -O` from auto-vectorising or pipelining the arithmetic.

> The original strategy below ("a definitely-assigned local is never null") was **wrong** —
> the 2026-06-25 investigation (Status above) showed the sentinel is null *propagation* and
> there is no non-null analysis. The sound full design follows; the old sketch is kept struck
> through for context.

### ~~Strategy: sentinel checks only at store boundaries~~ (superseded — unsound)

~~A `long` local variable that has been assigned is never null during arithmetic.~~ False:
`op_add_int`/`op_add_long` returns `i64::MIN` (null) on **overflow**, and a local can hold a
null read from a nullable field or a null-returning call. Definite *assignment* ≠ definite
*non-null*. The real design must prove non-null, not assume it.

### Full implementation design (the sound version)

Done right, N3 is **not** a codegen patch — it is **non-nullability tracked as a type fact**,
with the `_nn` ops as its first consumer. Per [CODEGEN_METHOD.md](CODEGEN_METHOD.md), the fact
is computed once by an analysis and *read* by codegen, never re-derived per emit site. The
payoff is broader than Collatz: a real non-null fact also sharpens `??` codegen and lets other
defensive sentinels drop — it's a type-system improvement that happens to unlock the perf win.

**The one invariant.** Emit a `_nn` op for an int/long binary-arithmetic node **iff both
operands are provably ≠ `i64::MIN` at that program point**; otherwise emit the sentinel op.
Everything below is the machinery that decides "provably non-null," soundly.

**1. The fact — a `NonNull` bit on int/long values.** Add a two-point nullability lattice
carried on the value/expression type: `Nullable` (⊤, default — may be `MIN`) ⊐ `NonNull`
(proven ≠ `MIN`). It **joins conservatively** — `Nullable` wins at any control-flow merge. The
two nullability hooks that already exist become *inputs*, not the whole story: the field-level
`not_null` bit (`src/typedef.rs`) and the `??`-scoped `expr_not_null` flag
(`src/parser/operators.rs`).

**2. Sources of `NonNull`** (where the bit is introduced):
- **Literals** — `Value::Int(k)` with `k != i64::MIN` → `NonNull` (the lone `MIN` literal stays `Nullable`).
- **`not_null` fields** — a record field declared/inferred non-null reads back `NonNull`.
- **Bounded loop induction vars** — `for i in lo..hi` with non-null bounds: `i ∈ [lo,hi)`, never `MIN` → `NonNull` in the body (the `i + 1` / Collatz win).
- **Post-guard** — a value after `x ?? d` or inside an `if x != null` branch (reuse `expr_not_null`).
- **Non-null-by-construction conversions** — e.g. `bool→int`.

**3. Propagation — and the overflow decision (the load-bearing call).** Is `(a + b)` non-null
when `a`, `b` are? **No, not in general:** `op_add_int` is `checked_add(...).unwrap_or(MIN)`, so
a *result* can be `MIN` on overflow even from non-null inputs. Two sound options:
- **(A) Conservative — recommended first.** Arithmetic *results* re-enter as `Nullable`. So
  `_nn` fires on the *operands* (each proven from a source), but a chained result keeps the
  outer op on the sentinel. Captures the common single-op shape (`i + 1`, `3 * cur`) — most of
  the gain — with **zero** overflow risk and no new analysis.
- **(B) Range-aware — later.** Add an interval lattice; when a result's proven range excludes
  `MIN`, mark it `NonNull` so `_nn` *chains* and `rustc -O` fuses the expression. Bigger, but
  unlocks multi-op fusion.

  Name the **semantic** edge explicitly: today overflow silently yields null and propagates; a
  `_nn` over operands that themselves overflowed would not re-propagate. Option A **side-steps
  it** (results stay `Nullable`, so behaviour is identical) — which is exactly why it ships
  first. Option B must either prove no-overflow or declare overflow-to-null unsupported.

**4. Codegen reads the bit (translation, not derivation).** In `src/generation/` (native) and
`src/state/codegen.rs` (interp emit), at an int/long binary-op node: both operand bits `NonNull`
→ emit the `_nn` op; else the sentinel op. The bit was computed in the analysis pass — no
`has_ref_params && …`-style re-derivation at the site. Note `op_add_int_nn` ≡ `op_add_long_nn`
(same path — `op_add_int` *is* `op_add_long`), so **no new integer ops are needed**; the 6
existing `_nn` long ops cover both.

**5. Both backends.** Native picks the `_nn` Rust fn directly. The interpreter today runs
`ops::op_add_long` via its opcode; for the interp win (the debug-loop slice), add escape-range
`add_long_nn` *opcodes* (opcode space is now free — see § How the interpreter executes) wired to
`op_add_long_nn`, emitted under the same bit. Ship native first; interp opcodes are optional.

**6. Validation — the boundary matrix is the soundness gate.** This is a soundness change in
loft's #1-priority area, so the `engineering-rigor` matrix is mandatory, asserting **value +
null-propagation + overflow on BOTH backends**, and **introspecting *which op was emitted*** (a
result can be coincidentally right):
- non-null `lit op lit` → correct value AND `_nn` chosen.
- one operand a **nullable field** (unassigned record) → result MUST be null AND `_nn` MUST NOT
  be chosen (the corruption cell — this is the whole point).
- overflow cell (`MAX + 1`) → matches current behaviour under the chosen option.
- `??`-fed, bounded-loop-var, conversion-source cells. Grow one `tests/oracle/` guard (Goal D).

**7. Staging (each slice shippable):**
1. `NonNull` lattice + literal & `not_null`-field sources; native codegen reads it; matrix.
   (Smallest sound slice: `_nn` on `lit op lit` / `field op lit`.)
2. Bounded-loop-induction-variable source — the `i + 1` win (Collatz 04).
3. Interpreter `_nn` opcodes.
4. *(optional)* Option-B range analysis for chained fusion.

**Cost/risk:** Medium for slices 1–2, Large with range analysis. Soundness-critical — the
matrix + introspect-the-emit gate is non-negotiable. Re-run `bench/04_collatz` after slice 2 to
confirm the gain is real on current `rustc` (which already predicts the `!= MIN` branch well).

### Touched files (full version)

- `src/ops.rs`: the 6 `_nn` ops already exist; add `_nn` comparison/`neg` variants only if a slice needs them.
- *new* nullability analysis (alongside `src/scopes.rs` / the type pass): compute the `NonNull` bit.
- `src/data.rs` / `src/typedef.rs`: carry the `NonNull` bit on the value/type (feed in the existing `not_null` + `expr_not_null`).
- `src/generation/` + `src/state/codegen.rs`: read the bit, pick `_nn` vs sentinel.
- `default/01_code.loft`: `_nn` op declarations + `#rust` templates (so codegen can name them).
- `tests/` + `tests/oracle/`: the boundary matrix + a differential guard.

---

## Design: N4 — Suppress cr_call_push on `#pure` leaf functions

**Affected workload:** Any hot loop with tiny `#pure` helper calls.
The trigger was scan.loft's byte-scanner (`is_digit_b`,
`is_word_char_b`, `is_segment_char_b` called per-byte over ~24 MB).
**Expected gain:** Eliminates one `thread_local!` + `RefCell::borrow_mut`
+ `Vec::push` per leaf call.  Per call cost is ~10-20 ns; on the
scan.loft hot loop that's a meaningful share of the ~165 ms
`-O` baseline.
**Cost:** Small — localised change in `src/generation/mod.rs` +
purity-classifier extension.

> **Status (audited 2026-06-25): OPEN.** `is_leaf_pure` is absent and `cr_call_push` is
> still emitted unconditionally for every `n_` function. But the "purity-classifier
> extension" the cost cites is mostly free: `def.purity` already exists (see N2 status),
> so `is_leaf_pure` = `def.purity == Purity::Pure` AND the body has no `Call`/`Method` —
> only the leaf check is new. Stays Small.

### Background

`src/generation/mod.rs:1888` emits

```rust
cr_call_push("name", "file", 12);
let _call_guard = codegen_runtime::CallGuard;
```

at the entry of every user function whose name starts with `n_`.
The shadow stack exists so `stack_trace()` returns a meaningful trace
at panic time.  For `#pure` leaf functions:

- They cannot panic from internal logic — any panic is a compiler bug.
- A user-visible stack trace formed by the *parent* of a `#pure`
  leaf still gives the right context (the call site is the leaf
  caller, not the leaf itself).
- The push/pop pair costs more than the body it brackets when the
  body is a single arithmetic comparison.

### Design

1. **Classify** each user fn as `is_leaf_pure(def_nr)`:
   `def.purity == Purity::Pure` AND `def.code: Value::Block` contains
   no `Call` / `Method` operator anywhere in its recursive tree.
   Memoise on `def_nr` (same shape as N2's purity analysis).
2. **In `src/generation/mod.rs:1888`**: when `instrument &&
   is_leaf_pure(def_nr)`, skip the `cr_call_push` + `CallGuard`
   emission (keep the `let stores: &mut Stores = …` derivation —
   inner `ops::` templates may still reference it).
3. **Optional**: emit `#[inline]` on the same set so rustc gets a
   strong hint to inline now that the call-barrier bookkeeping is
   gone.

### Risk

- `stack_trace()` invoked from inside a `#pure` leaf body would
  miss its own frame.  This is hypothetical — `#pure` rules out
  most reasons to introspect runtime state — but it is a behaviour
  change, so document as a known limitation.
- Conservative classifier (leaf-only) keeps the impact bounded and
  preserves trace fidelity for any pure function that fans out.

### Validation

The single-file `--native-emit` + `rustc -O` harness used to discover
the `-O` gap (see `## Open work` below) reproduces the per-call
overhead directly: comment out the `cr_call_push`/`CallGuard` lines
in the emitted `.rs` and re-time.  No build-system changes needed.

---

## Design: N5 — Inline `integer` arithmetic when operands are provably non-null

**Affected workload:** Every loop with `i + 1` / `j + 1` index
arithmetic (scanners, parsers, tokenisers).  N3 is the parallel
for `long`.
**Expected gain:** Replaces `ops::op_add_int(a, b)` (function call
with null-sentinel guards) with a direct `a + b` expression.  Lets
rustc -O fuse with surrounding arithmetic and (where applicable)
vectorise.  Empirically tight loops emit several `op_add_int` calls
per byte; even after `-O` inlines them, the null-sentinel branch
prevents further fusion.
**Cost:** Small if folded into N3's nullability analysis (same
classifier, parallel application).  Standalone is also Small.

### Background

Loft's generated Rust for `j + 1` (where `j` is a local i64
counter) is currently:

```rust
ops::op_add_int(var_j, 1_i64)
```

`ops::op_add_int`'s body is the null-sentinel guard
`if v1 == i64::MIN || v2 == i64::MIN { i64::MIN } else { v1 + v2 }`.
For locals that are definitely non-null at the point of use (assigned
a literal then mutated only by arithmetic), the guards are dead code
— but rustc can't always eliminate them across function-call
boundaries when the caller is non-trivial.

### Design

1. Extend N3's nullability classifier to `integer` types (same
   definite-assignment + flow analysis on local variables).
2. When both operands of an `integer` binary op are provably
   non-null, emit the bare expression (`a + b`, `a - b`, `a * b`,
   `a == b`, etc.) instead of `ops::op_add_int(a, b)`.
3. For mixed cases (one operand from a nullable store field), keep
   the sentinel version.

### Relationship to N3

N3 + N5 share the classifier and the policy.  Ship them as one
sequenced commit set (N3 first to validate the approach on `long`,
N5 right after for `integer`) so the analysis lives in one place.

---

## Design: N6 — Skip the rustc toolchain probe on a native cache hit

> **DELIVERED (verified 2026-06).** There is **no up-front `rustc --version` probe**:
> a warm cache hit runs the cached binary directly and spawns **zero** rustc/cargo
> processes (confirmed via `strace -f -e execve` — 0 rustc execve on a warm run), and
> rustc is checked only on a cache *miss* — a cheap `cache::rustc_mismatch()` for the
> default path plus the compile's NotFound-arm lazy fallback (`src/main.rs` ≈5818–6060,
> the `'native` block). Measured warm native startup of a trivial program is **~0–10 ms**
> (vs the ~26 ms below), so the ~18 ms tax is gone. The implementation is *better* than
> the original design (which moved the probe into the cache-miss branch) — it removed
> the probe entirely, letting the compile attempt itself be the rustc-presence check.
> **Residual (separate, not done):** the ~6.6 ms parse+codegen+emit+hash still runs on
> every warm invocation (the cache is keyed on the *generated Rust*, not the `.loft`
> source) — see the Ceiling note below; the source-keyed cache to reach the ~1 ms floor
> is the open follow-up.

**Affected workload:** Native **startup latency** of any short-lived
program — a CLI tool, a test harness, a script invoked repeatedly.
Not a throughput benchmark; it is a fixed per-invocation tax that
dominates wall-clock when the program itself runs in microseconds.
**Expected gain:** Removes a `rustc --version` probe (~18 ms measured,
Linux x86-64) from every warm run.  Drops native warm startup of a
trivial program from ~26 ms to ~7 ms (≈3.7×).
**Cost:** Small — move one `if`-block in `src/main.rs`.

### Background — measured decomposition

`--native` is the **default** backend (`src/main.rs:101,106`).  A
trivial `fn main() { print("hello"); }` measured on this box:

| Path | Startup |
|---|---:|
| `loft --interpret` | 2.15 ms |
| `python3 -S` | 3.57 ms |
| `python3` | 5.84 ms |
| `loft` (native, default, **warm**) | **25.73 ms** |
| `loft` native **cold** (one rustc compile) | 313 ms |

The 25.73 ms warm path decomposes as:

```
~18.6 ms  rustc --version probe   (main.rs:4866–4881)
 ~6.6 ms  parse + codegen + emit + source_hash + cache check
 0.46 ms  exec the cached binary  (main.rs:5092)
```

`strace -f -e execve` confirms the probe fires on every warm run; the
cached binary run directly (no loft wrapper) starts in 0.46 ms — so the
generated machine code already has excellent startup and the entire
tax is launcher overhead.

### The cache PUBLISH is atomic, and must stay that way

The cached binary lives at `<script dir>/.loft/cache/<stem>-<hash>`, a path SHARED by every
concurrent run of that source, so writing it is the one step several processes contend on.
It is published by staging under a private per-process `.<leaf>.<pid>.tmp` name in the SAME
directory, tightening the mode while the file is still invisible under its final name, and
then `rename`-ing it into place — `native_utils::publish_cached_binary`, the one home for
this.

A plain `fs::copy` onto the destination is the thing to never go back to: it truncates in
place and streams ~11 MB, and for that whole window a concurrent run sees a path that
EXISTS and passes `cache_safe_to_execute` — which reads symlink, owner and mode but **never
size** — so it execs a 0-byte-and-growing ELF and dies with **no stdout and no stderr at
all**.  The mode check does not cover it either: once the entry exists at 0700, a later copy
truncates it while the mode stays 0700.  POSIX `rename` is atomic and a process already
exec'ing the previous inode keeps a complete file.  For the same reason the stale-entry
sweep runs AFTER the publish and spares the entry just written — sweeping first deleted the
very binary a concurrent run had already accepted as usable.

Measured 2026-09-06: `make ci` red in two runs of three on
`alias_link_baseline::baseline_leak_clean_native`, whose two native cells compile one source
concurrently.  Guarded deterministically by the destination's INODE changing across a
publish (`native_utils::publish_cached_binary_tests`) — racing the race does not falsify,
see TESTING.md § How a guard reads green while the defect stands.

### Why the probe is skippable

The probe exists only to **fall back to the interpreter when rustc is
absent**.  But it runs *unconditionally* at `main.rs:4866` — before the
cache lookup at `main.rs:5080`.  On a cache hit (`main.rs:5092`) the
cached binary is executed and **rustc is never invoked**.  The
fallback-to-interpreter behaviour is therefore only relevant on a cache
*miss*, where a compile actually happens.

### Design

Move the `rustc --version` probe (`main.rs:4866–4881`) from before the
codegen pipeline into the **cache-miss `else` branch** (`main.rs:5094`),
just before the compile at `main.rs:5114`.  On a hit the probe is
skipped entirely; on a miss the fallback fires exactly as today.

**Verify:** a genuine cache miss with rustc removed from `PATH` still
falls back to the interpreter with the existing warning, and a cache
hit with rustc absent now succeeds (it previously failed the probe and
fell back even though it never needed rustc).

### Ceiling — what this does NOT remove

The cache is keyed on the **generated Rust source**
(`main.rs:5022`, `source_bytes = read(emit_path)`), so parse + codegen +
emit + hash still run on every invocation — the residual ~6.6 ms.
Reaching the ~1 ms floor (bare cached-binary cost) needs a **second,
larger** change: key the cache on the `.loft` source + stdlib/rlib
identity so a hit short-circuits parse+codegen and jumps straight to
exec.  The source-level cache machinery already exists
(`src/startup_cache.rs`); wiring the native-run path to it is the
follow-up.  Ship N6 first (the cheap, safe ~18 ms win), then the
source-keyed cache as its own change.

---

## Design: W1 — wasm string representation

**Affected benchmark:** 07 (2.06× wasm vs native)
**Expected gain:** Reduce the gap to <1.3×
**Cost:** Medium — wasm-target conditional compilation

### Background

The wasm build compiles the same `src/` Rust code as the native build, which means
string operations use Rust `String` — heap-allocated via Rust's allocator inside wasm
linear memory. Each dynamic string operation (append, concatenate, slice) involves a
call to the wasm allocator, which is slower than native `malloc` because it must
operate within the linear memory model with `memory.grow` for expansion.

### Design

Use `wasm-bindgen`'s or `wasm-pack`'s built-in string handling or, for the wasip2
target, use the native `String` representation but optimise the critical format-string
path:

1. **Pre-allocate the result buffer.** In the generated `format!` equivalent for string
   building, compute an estimated capacity from the number of append operations (if
   statically known) and `String::with_capacity(n * avg_element_len)` before the loop.

2. **Avoid intermediate allocations.** Replace `text + other_text` (which allocates
   a new `String`) with `text.push_str(&other_text)` (appends in place). The loft
   compiler already emits format-string concatenation this way in the interpreter; verify
   that `src/generation/` does the same for native/wasm.

3. **Profile first.** Run `bench/run_bench.sh` with wasm and capture a perf trace
   via `wasmtime --profile`. If the 2× gap is allocator overhead, the capacity
   pre-allocation above will close most of it. If it is wasm function-call overhead
   on string operations, a different approach is needed.

This item is lower priority than P1 and N1 because the absolute time difference is
small (35 ms) and the affected benchmark (string building) is already fast in both
modes.

---

## Improvement priority order

**Audited against the code 2026-06-25.** Two items were already delivered but listed open
(the opcode-table two-byte escape that "blocked" P1, and N6); two are cheaper than estimated
because their prerequisite already exists (N2/N4 — purity analysis is built); one is partially
scaffolded (N3 — dead `_nn` op stubs). The `Status` column records what's actually in the tree.
Cost/gain are unchanged estimates (the benchmark table itself predates v2026.6.0 — re-run
`bench/run_bench.sh` before committing to a High-cost item).

| Item | Target benchmarks | Gain | Cost | **Status (2026-06-25)** |
|---|---|---|---|---|
| P1 — Superinstructions | 02, 03, 04, all tight loops | 2–4× | Medium | **Open** — *unblocked* (the escape exists); not implemented (no `si_*`/peephole). Cheap secondary (debug loop). |
| N1 — Direct collection emit | 08, 09, 10 | 5–15× | High | **Open** — no escape-analysis/`Locality`. The big native prize. |
| P2 — Stack raw pointer cache | all interpreter | 20–50% | High | **Open** — `stack_base:u32` is frame bookkeeping, *not* the raw-ptr cache; + memory-model risk. Low priority. |
| N2 — Pure-fn stores omit | 01, 06 native | 10–30% | ~~High~~ **Med** | **Open**, but `def.purity` already computed (scopes.rs) — only the stores-less codegen half remains. |
| N3 — Long sentinel in codegen | 04 native | ~1.5× | ~~Low~~ **Med+** | **Open — cheap version is UNSOUND** (investigated 2026-06-25). The 6 `_nn` ops are dead; the sentinel they'd skip is the *core null-propagation path* for all integer/long arithmetic, and no definite-non-null analysis exists. Needs a sound non-null dataflow (soundness-critical). See N3 section. |
| N5 — Integer sentinel | scanners, parsers, indexed loops | ~N3 | Med+ | **Open — same path as N3** (`op_add_int == op_add_long`); not a separate item. Same soundness blocker. |
| N4 — Suppress cr_call_push on `#pure` leaves | hot loops w/ tiny `#pure` helpers | per-call | Small | **Open** — reuses existing `def.purity`; `is_leaf_pure` absent. |
| P4 — Block-copy slice (`OpAppendVectorSlice`) | primitive-vector slice copies | 5–10× | Medium | **Open** — op absent; opcode slot now trivially free (escape). |
| P3 — Verify integer sentinel | 02, 10 | 2–5% | Low | **Open** — audit test absent. |
| W1 — wasm string path | 07 wasm | <1.3× | Medium | **Open** — no string `with_capacity` in `generation/`. |
| Arc worker-clone (`Stores::types`/`names`) | parallel workers | small | Low–Med | **Open** — fields not `Arc`-wrapped. |
| O8.3 — zero-fill struct defaults | struct construction | small | Small | **Open** — not in `parser/objects.rs`. |
| ✅ **N6 — Skip rustc probe on cache hit** | native warm startup | ~18 ms / ≈3.7× | Small | **DELIVERED** — verified: 0 rustc spawns on a warm run, ~0–10 ms. |
| ✅ **Opcode two-byte escape** (P1's "blocker") | — | — | — | **DELIVERED** — `255 → OPERATORS[255+ext]`, ~242 free slots. |

Suggested order given the audit: **N4** (Small, purity ready, no soundness risk) → **N2** (now
Medium, purity ready) → **N1** (the High-cost native prize) → **P1** (cheap interpreter
secondary). **N3/N5 dropped from "cheap-first"** — the 2026-06-25 investigation found the cheap
version is unsound (the sentinel is core null-propagation; no non-null analysis exists), so it's
Medium+ and soundness-critical, not a quick win. P2 parked (interp-only, bounded, memory-model risk).

**The interpreter's place — and its bounds.** `--native` is the default *shipping*
backend, but the **interpreter is the debug / live-edit backend** — a debug session
forces execution through the interpreter so it can step, inspect, and hot-reload. That
touches the **debug loop**, which is Purpose-level (the fast iterative loop is loft's
whole value). But the impact is **bounded**, for two reasons: a shipped game runs
native (players never touch the interpreter), and **library calls dispatch to the
native cdylib even under `--interpret`** (the C71 / [NATIVE.md § N9](NATIVE.md#n9--native-library-shared-store-dispatch-c71)
shared-store dispatch). So during a debug session only the maker's *own* glue/logic
runs interpreted, while the heavy library work (render, physics, hex-world) stays
native. Interpreter optimisation (P1, P2) is therefore a **real but secondary** win —
worth doing because P1 is now cheap and unblocked, not because it dominates the loop.
The **native** items (N6, N1, N3/N5) are prime: they speed the shipped game, the
library calls that dominate even a debug session, *and* the dev/test path.

**P1 is the highest-impact interpreter change** — it benefits every tight loop the
maker writes by hand, without touching the memory model, and (corrected 2026-06) it is
**no longer blocked**: the two-byte escape gives ~242 free opcode slots, so its
superinstructions land as escape-range ops. P2 (stack pointer cache) is the larger
follow-on but carries memory-model risk (raw-pointer invalidation), and its
interpreter-only gain is bounded by the same reasoning — so P1 is the safer first step
and P2 is low-priority.

---

## See also

- Optimisations section below — runtime optimisation opportunities audit
- [PLANNING.md](PLANNING.md) — priority-ordered enhancement backlog
- [INTERNALS.md](INTERNALS.md) — `src/fill.rs`, `src/state/`, `src/generation/`
- [NATIVE.md](NATIVE.md) — native code generation design and known issues
- [doc/00-performance.html](../00-performance.html) — rendered benchmark page with bar charts

---

## Runtime optimisation audit

This document audits the interpreter runtime for concrete performance improvements,
weighing impact against implementation cost and maintainability.

### Contents
- [Open opportunities](#open-opportunities)
- [Not worth changing](#not-worth-changing)
- [Open — recommended priority order](#open--recommended-priority-order)

Completed optimisations (debug_assert, clone removal, Arc bytecode sharing, LLRB free-list)
are recorded in CHANGELOG.md.

---

### Open opportunities

#### 1. `Stores::types` and `Stores::names` cloned for every worker

**File:** `database.rs:1541-1561`

`clone_for_worker` copies:

- `types: self.types.clone()` — `Vec<Type>`, read-only after compilation
- `names: self.names.clone()` — `HashMap<String, u16>`, read-only after compilation

Both are pure metadata that no worker modifies.  Wrapping them in
`Arc<Vec<Type>>` and `Arc<HashMap<String, u16>>` would reduce the per-worker
clone to two atomic-ref-count increments.

For a program with 200 types and a 500-entry name map the savings are small in
absolute bytes, but the pattern becomes significant if the type system grows or
if hundreds of parallel calls are made.

**Impact:** Low-Medium — mainly prevents future scaling problems
**Cost:** Medium — field types change throughout `database.rs`; some methods need `Arc::make_mut` if mutation is ever needed before `clone_for_worker` is called
**Verdict:** Defer until parallel usage grows; note the shape of the fix here

---

### Not worth changing

| Pattern | Reason |
|---|---|
| `State` HashMap fields (`stack`, `vars`, `calls`, `types`, `line_numbers`) | Only accessed in debug/dump functions, not in the hot execute loop |
| `WorkerProgram` channel + batching in `parallel.rs` | `Vec::with_capacity(end-start)` is already exact; no reallocation |
| `calc.rs` BTreeMap for struct layout | Compile-time only; immeasurable runtime effect |
| `library_names: HashMap<String, u16>` | Queried during compilation, not execution; worker states leave it empty |
| Function pointer dispatch table in `fill.rs` | Already optimal for an interpreter; JIT is the next step |

---

### Open — recommended priority order

| # | Change | File(s) | Effort | Impact |
|---|--------|---------|--------|--------|
| 1 | `Arc` for `Stores::types` / `names` | `database.rs` | Medium | Low–Med |
| 2 | O8.1b: packed bytes in bytecode | `vector.rs`, `state/mod.rs` | Medium | High |
| 3 | O8.3: zero-fill struct defaults | `parser/objects.rs` | Small | Low–Med |

---

### O1 Superinstruction Peephole — Design Notes (deferred)

The infrastructure for superinstructions is in place but the peephole rewriting
pass is deferred to a future release.  This section documents the design for
the implementor.

#### What exists

- **Opcodes registered** in `default/01_code.loft`: `OpSiLoad2AddStore`,
  `OpSiLoadConstAddStore`, `OpSiLoadConstCmpBranch`, `OpSiLoad2CmpBranch`,
  `OpSiLoadConstMulStore`, `OpSiLoad2MulStore`, `OpNop`.
- **State stubs** in `src/state/mod.rs`: delegation methods that call `nop()`.
  Replace these with the real implementations below.
- **`fill.rs` auto-generated** with the opcodes in the OPERATORS array.
- **`build_opcode_len_table()`** in `src/compile.rs`: computes instruction
  byte-lengths from operator definitions — survives renumbering.
- **`opcode_by_name()`** in `src/compile.rs`: resolves opcode numbers by name.
- **`fill_rs_up_to_date`** CI test: asserts `src/fill.rs` matches the generated
  version — prevents drift when `01_code.loft` changes.

#### The stack-relative operand problem

`get_var(pos)` computes `stack_base + stack_pos - pos`.  Each `VarInt` pushes
4 bytes, advancing `stack_pos`.  The superinstruction runs without intermediate
pushes, so the second operand sees the wrong `stack_pos`.

**Arithmetic for `VarInt(a) VarInt(b) AddInt PutInt(c)` at initial SP:**

| Instruction | stack_pos | Address accessed |
|-------------|-----------|-----------------|
| VarInt(a) | SP | base + SP - a |
| VarInt(b) | SP+4 | base + SP + 4 - b |
| AddInt | SP+8→SP+4 | (pops 2, pushes 1) |
| PutInt(c) | SP+4→SP | base + SP + 4 - c |

The superinstruction at SP (no pushes):
- `get_var(a)`: base + SP - a ✓
- `get_var(b)`: base + SP - b ✗ (should be base + SP + 4 - b)
- `put_var(c)`: base + SP + 4 - c ✓ (put_var adds sizeof(T) internally)

**Fix:** adjust `b' = b - 4` in the peephole rewriter.  Then `base + SP - (b-4) = base + SP + 4 - b`. ✓

**Guard:** skip the pattern when `b < 4` (would underflow).

#### Real implementations for State methods

Replace the `nop()` stubs with:

```rust
pub fn si_load2_add_store(&mut self) {
    let a = *self.code::<u16>();
    let b = *self.code::<u16>();  // pre-adjusted: b' = b - 4
    let c = *self.code::<u16>();
    let va = *self.get_var::<i32>(a);
    let vb = *self.get_var::<i32>(b);
    self.put_var(c, crate::ops::op_add_int(va, vb));
}
// Same pattern for si_load2_mul_store.
// For const variants: k is a literal (no adjustment).
// For cmp+branch: si_load2_cmp_branch reads i16 offset, branches if va >= vb.
```

#### Peephole rewriter

Add `PeepholeCtx` to `src/compile.rs` that:
1. Builds opcode-length table via `build_opcode_len_table(data)`
2. Resolves opcodes by name via `opcode_by_name(data, name)`
3. Scans each function's bytecode as a sliding 4-instruction window
4. Matches patterns with exact length guards (l0==3, l1==3, l2==1, l3==3)
5. Rewrites in-place with adjusted operands, fills excess bytes with OpNop
6. **Skips default library functions** (`data.def(d_nr).position.file.starts_with("default/")`)

#### Known issue: default library corruption

Default-library functions (`default/*.loft`) use VarInt operand patterns
that interact with store-relative addressing in ways the simple `b-4`
adjustment doesn't cover, producing "Unknown record" errors on merge-sort
tests.  **Mitigation:** skip default library functions — they're already
fast (hand-optimised `#rust` templates); only user code benefits from
superinstructions.

#### Adjustments per pattern

| Pattern | a | b/k | c/off | Super size |
|---------|---|-----|-------|------------|
| `VarInt VarInt {Add\|Mul}Int PutInt` | a | b-4 | c | 7 bytes |
| `VarInt ConstInt {Add\|Mul}Int PutInt` | a | k | c | 9 bytes |
| `VarInt VarInt LtInt GotoFalse` | a | b-4 | i16 offset | 7 bytes |
| `VarInt ConstInt LtInt GotoFalse` | a | k | i16 offset | 9 bytes |

Branch offset for cmp patterns: original `goto_false` offset is i8 relative
to `pc3+2`.  Super offset is i16 relative to `pc+7` (or `pc+9` for const).
Compute: `new_off = (pc3 + 2 + old_off) - (pc + super_size)`.

---

### See also — backlog and internals
- [Design: P1](#design-p1--superinstruction-merging) onward — the detailed O1–O7 designs (superinstructions, stack pointer cache, native collection emit, purity analysis)
- [PLANNING.md](PLANNING.md) — Priority-ordered backlog
- [INTERNALS.md](INTERNALS.md) — `src/parallel.rs`, `src/store.rs`, `src/state/` implementation details

#### 2. O8: Constant data initialisation (delivered 2026-04-02)

**Files:** `src/const_eval.rs`, `src/vector.rs`, `src/fill.rs`, `src/parser/vectors.rs`

Three optimisations delivered:

- **O8.1a** `OpPreAllocVector`: pre-allocates vector capacity for known-size
  literals, eliminating all `store.resize()` calls.  One new opcode (replaced
  unused `OpNop` slot).
- **O8.5** Constant comprehension unrolling: `[for i in 0..N { expr(i) }]`
  unrolled at compile time when bounds and body are const-evaluable.  10k limit.
  ⚠ **Written but never reached** — see below.
- **`const_eval()`** module: compile-time constant folder for arithmetic, casts,
  comparisons, boolean ops across all numeric types.

**Impact:** For a 20-element constant vector, eliminates 1-2 resize allocations.

**O8.5's unroll has never run.** `parse_block` wraps a comprehension body in a
`Value::Block`, and `const_eval` has no arm for one, so the fold gives up on the
wrapper before it reaches the body — `[for i in 0..3 { 5 }]` emits a runtime loop
today, and so does every shape the section below describes. Teaching `const_eval`
about single-expression blocks does wake it, and that is not a free change: it
also makes it fire in a struct-literal FIELD, where the literal-list path appends
through the enclosing record and builds `Pair { a: …, b: … }` with six elements in
`a` and none in `b` (loft#892's neighbourhood). Waking it therefore needs #892
closed first, plus its own boundary matrix over the shapes above — the filtered
form, the size limit, and the element types.

**O8.5's FILL half is live (loft#884).** A comprehension whose body folds to a
constant *without* binding the loop variable is a fill, and lowers to the repeat
literal's one-template-plus-`OpAppendCopy` — the same machinery `[x; n]` uses, so
`[for _ in 0..n { -1 }]` now costs what `[-1; n]` costs (measured: 74.6 ms → 14.4 ms
for a million elements, `--native`, against `[-1; n]`'s 14.7 ms). This needs no
const bounds, so it covers the runtime count a consumer actually writes, and it
unwraps the body block at its own call site rather than in `const_eval`, precisely
so it does not wake the unroll above. It declines a struct field, a captured
collection and an indexed element for the #892 reason, and declines a filter, a
body that reads the loop variable, and any body with a call — the gate is PURITY,
not loop-invariance, so `[for _ in 0..n { f() }]` still calls `f()` n times.

Full design: the Constant Data section below.

---


## String Buffer Allocation and Optimization Opportunities

### Text type duality

Loft has two runtime representations for text:

| Type | Size | Heap? | Where used |
|---|---|---|---|
| `Str` | 16 bytes (ptr + len + pad) | No — borrows existing buffer | Arguments, temporaries on eval stack |
| `String` | 24 bytes (ptr + capacity + len) | Yes — owns heap buffer | Local variables, work texts |

The split is the primary optimization: text arguments are zero-copy
references into the caller's (or constant pool's) memory.

---

### Allocation lifecycle of a local text variable

```
OpText          →  24B String written to stack, zero heap (String::new())
OpAppendText    →  first append allocates heap buffer
OpClearText     →  .clear() — content gone, heap buffer preserved
OpAppendText    →  reuses existing buffer if it fits
OpFreeText      →  .shrink_to(0) — deallocates heap buffer
```

Key insight: `String::new()` is free (no heap allocation).  The real
cost is the **first `OpAppendText`** which triggers a heap allocation.
Subsequent reassignments via `OpClearText` + `OpAppendText` often
reuse the existing buffer.

---

### Where copies actually happen

| Situation | What happens | Heap alloc? |
|---|---|---|
| Text argument passing | 16B Str reference pushed | **No** |
| `OpVarText` (read local) | Create 16B Str view of 24B String | **No** |
| `OpArgText` (read param) | Read existing 16B Str | **No** |
| **`x = "hello"` (first)** | OpText + OpAppendText | **Yes** — one alloc |
| **`x = y` (text copy)** | OpText + OpVarText + OpAppendText | **Yes** — copy into new buffer |
| **`x = y + z` (concat)** | Work text + 2× OpAppendText | **Yes** — work text buffer grows |
| `x = func()` (text return) | Destination passing via RefVar(Text) | **No extra** — writes into x directly |
| `x = "new"` (reassign) | OpClearText + OpAppendText | **Usually no** — reuses buffer |
| Work text reuse | OpClearText | **No** — keeps capacity |

#### Destination passing (already optimized)

Text-returning functions use `RefVar(Text)`: the caller's String
buffer is passed as an implicit parameter, and the callee writes
directly into it.  No intermediate copy.

```
fn greet(name: text) -> text {
  "hello " + name       // writes directly into caller's buffer
}
result = greet("world"); // result's String IS the buffer
```

This is implemented in `codegen.rs:gen_text_dest_call` (~line 1858)
and `text_return()` in `control.rs`.

---

### Current efficiency

Zero-copy Str references for arguments, destination-passing for
text-returning functions, and `.clear()` + reappend reuse of heap
buffers across reassignments.  The remaining overhead is one heap
allocation per mutable text variable on first content write —
inherent to the owned-buffer design.

---

### Optimization opportunities

#### O-S1. `String::clone()` for `x = y` — **Low value**

Currently `x = y` emits OpText (empty String) + OpVarText (read y) +
OpAppendText (copy into x).  This does: allocate empty → reallocate
to fit → copy.

A dedicated `OpCloneText` could do `String::clone()` directly: one
allocation at the correct size, one memcpy.  Saves one reallocation.

**Impact:** Marginal — `String::clone()` vs empty + append is ~10%
difference in microbenchmarks.  Not worth a new opcode.

#### O-S2. Pre-sized allocation for known lengths — **Low value**

For `x = "long literal string"`, the compiler knows the length at
compile time.  `String::with_capacity(len)` would avoid the realloc
on first append.

**Impact:** Negligible — short strings (< 16 chars) are the common
case, and the allocator typically over-provisions anyway.

#### O-S3. Copy-on-write (Cow) for read-only variables — **Medium value, high complexity**

If a text variable is assigned once and only read thereafter, it
could stay as a borrowed `Str` instead of copying into an owned
`String`.  This requires:
- Mutation analysis in the parser (which variables are never mutated?)
- A third text representation: `Cow<'a, str>` or similar
- Fallback path for variables that are later mutated

This is analogous to the auto-const analysis for struct parameters.
The compiler already knows (via `find_written_vars`) which variables
are mutated.

**Impact:** Eliminates heap allocation for read-only text variables.
Significant for programs that pass text through multiple layers
without modifying it.  But the P115 auto-promotion mechanism shows
that mutation detection is feasible — we could do the inverse: keep
as Str until first mutation, then promote.

**Risk:** Lifetime management.  The borrowed Str points into the
caller's memory.  If the caller's String is freed or reallocated
while the callee still holds a Str, we get use-after-free.  This
is safe today because Str arguments have function-call lifetime.
Extending to local variables requires proving the source outlives
the borrower.

#### O-S4. Small-string optimization (SSO) — **High value, high complexity**

Store strings ≤ 22 bytes inline in the 24-byte stack slot instead
of heap-allocating.  This eliminates heap allocation for the vast
majority of strings in typical programs (names, labels, short
messages).

Requires replacing `String` with a custom `SmallString` type that
stores either inline data or a heap pointer.  Every text operation
(`OpAppendText`, `OpClearText`, `OpVarText`, `OpFreeText`) needs
to handle both representations.

**Impact:** High — eliminates ~80% of text heap allocations in
typical programs.  But the implementation cost is substantial.

---

### Recommendation

The current design is already well-optimized for the common cases.
The `Str`/`String` split, destination passing, and work-text reuse
handle the important paths.

**No immediate action needed.**  If profiling reveals text allocation
as a bottleneck, O-S3 (copy-on-write for read-only variables) is the
most impactful optimization that integrates with the existing
architecture.  O-S4 (SSO) delivers the highest raw improvement but
requires a custom string type that touches every text operation.

---


## Struct Passing, Copies, and Optimization Opportunities

### Loft parameter semantics

Loft passes ALL struct parameters by reference (shared DbRef).  There
is no implicit copy on function calls.  Mutation is the default:

```loft
fn modify(s: Point) { s.x = 99.0; }
fn main() {
  p = Point { x: 1.0, y: 2.0 };
  modify(p);
  // p.x is now 99.0 — caller's struct was mutated
}
```

The three parameter modes:

| Syntax | Semantics | Store locked? |
|---|---|---|
| `param: T` | Mutable reference — callee can mutate caller's data | No |
| `param: &T` | Mutable reference — same as above, explicit | No |
| `param: const T` | Read-only reference — store locked, writes panic | Yes |

---

### Where copies actually happen

Copies are NOT on parameter passing.  They happen on **first local
variable assignment** and **return values**.

#### Copy landscape

| Situation | What happens | Cost |
|---|---|---|
| Parameter passing | DbRef shared (12 bytes) | **Zero** |
| Vector element `v[i]` | DbRef pointer arithmetic | **Zero** |
| For-loop iteration | DbRef per element | **Zero** |
| Local reassignment `x = y` | DbRef overwrite | **Zero** |
| **First assignment `x = func()`** | OpCopyRecord deep copy | **Expensive** |
| **First assignment `x = y`** (same struct type) | OpCopyRecord deep copy | **Expensive** |
| **Return values** | copy_block (byte copy) | **Moderate** |
| Const lock check | Bool assert per write | Negligible |

#### When OpCopyRecord fires

Only three cases in `gen_set_first_at_tos` (codegen.rs):

1. **`x = func_returning_struct()`** — function return assigned to new
   local variable.  Emits OpConvRefFromNull + OpDatabase + OpCopyRecord.
   Deep copies all fields including nested vectors, text, sub-structs.

2. **`x = y`** where both are same struct type and x is uninitialized —
   same deep copy to give x its own independent store.

3. **Tuple destructuring** `(a, b) = expr` where an element is a
   Reference — deep copy for the extracted element.

#### What OpCopyRecord costs

Runtime at `state/io.rs:932`:
```
copy_block(&data, &to, size)     — raw byte copy of struct fields
copy_claims(&data, &to, tp)      — deep copy of nested structures
```

For `Mat4` (16 × f64 + vector wrapper): ~128 bytes + vector record.
For `Scene` with meshes/materials/nodes: hundreds of bytes + all vectors.

#### Return value copy (latent issue)

`state/mod.rs:1032` copies return values with `copy_block` only — no
`copy_claims`.  This is a shallow byte copy.  If a returned struct
contains owned nested references (vectors, text), the returned DbRef
shares them with the callee's about-to-be-freed store.  **Potential
use-after-free for complex return types.**

---

### Optimization 1: Move semantics for return values

#### Problem

`br_mvp = rect_mvp(proj, x, y, w, h)` — called 60×/frame in Brick Buster.
Each call: callee constructs Mat4, returns it, caller OpCopyRecord deep
copies it into `br_mvp`'s store.  The callee's original is immediately
freed.  The copy is wasted — the data could transfer ownership.

#### Fix: return slot pre-allocation (destination passing)

The caller pre-allocates the destination store and passes a DbRef to the
callee.  The callee writes directly into it.  No copy on return.

```
Before:                              After:
  callee: build Mat4 in local store    callee: build Mat4 in caller's store
  return: copy_block to caller         return: nothing (already there)
  caller: OpCopyRecord to br_mvp      caller: nothing (already in br_mvp)
```

This pattern already exists for text-returning functions
(`try_text_dest_pass` in codegen.rs).  Extending it to struct returns
is the natural next step.

#### Implementation

**File:** `src/state/codegen.rs`

1. When generating a function call whose return type is `Reference`:
   - If the result is assigned to a local variable (`x = func(...)`),
     pass `x`'s store DbRef as a hidden first parameter
   - The callee writes into that store instead of its own local
   - Return is a no-op (data already in the right place)

2. Requires the callee to be aware of the destination.  Two options:
   - **Implicit:** codegen detects struct construction and redirects writes
   - **Explicit:** new `__dest` hidden parameter (like text_return)

#### Impact

| Function | Calls/frame | Bytes saved per call |
|---|---|---|
| `rect_mvp()` | 60 | ~128 bytes + vector overhead |
| `mat4_mul()` | 60 | ~128 bytes |
| `mat4_perspective()` | 1 | ~128 bytes |
| `mat4_look_at()` | 1 | ~128 bytes |

**~15 KB/frame** eliminated in Brick Buster.  Proportionally more in the
renderer (PBR pass constructs Mat4 per node).

---

### Optimization 2: Last-use move (elide copy when source dies)

#### Problem

```loft
a = Point { x: 1.0, y: 2.0 };
b = a;       // OpCopyRecord — deep copy
// a is never used again
```

The copy is unnecessary — `a`'s store could be transferred to `b`.

#### Fix: last-use analysis

If `x = y` and `y` is never read again after this point (last use),
transfer `y`'s DbRef to `x` and null out `y`.  No copy needed.

The variable liveness analysis in `src/variables/` already tracks
`first_def` and `last_use`.  If `last_use(y) == current_statement`,
it's safe to move.

#### Implementation

**File:** `src/state/codegen.rs`, in `gen_set_first_at_tos`

Before emitting OpCopyRecord for `x = y`:
```rust
if let Value::Var(src) = value
    && stack.function.last_use(*src) == current_def_position
{
    // Move: transfer src's DbRef to x, no copy
    let src_pos = stack.position - stack.function.stack(*src);
    stack.add_op("OpVarRef", self);
    self.code_add(src_pos);
    // Mark src as moved — OpFreeRef will skip it
    return;
}
```

#### Impact

Eliminates copies for temporary struct results that are immediately
assigned and never reused.  Common in builder patterns:

```loft
m = mat4_translate(1.0, 2.0, 3.0);      // result → m (move, no copy)
mvp = mat4_mul(proj, mat4_mul(view, m)); // inner result → temp (move)
```

---

### Optimization 3: Auto-const inference (safety, not performance)

#### Purpose

Not a performance optimization (parameters aren't copied).  Instead:
auto-lock stores for provably unwritten parameters to catch accidental
mutation bugs at runtime.

#### When to auto-lock

A struct parameter can be auto-locked when:
- Never directly written (`param.field = x`)
- Never appended to (`param.vec += [x]`)
- Never passed as `&T` to another function
- **Never passed as plain `T` to a non-const function** (conservative —
  callee might mutate through the shared reference)

#### Implementation

1. Add `auto_const: bool` to Variable
2. Run `find_written_vars()` at end of first pass
3. Add escape analysis: check if param is passed to any function call
   where the receiving parameter is not `const`
4. Lock store at function entry for auto-const params

#### Compiler warning

When inference succeeds:
```
Warning: parameter 's' is never mutated — consider adding 'const'
```

---

### Test cases

#### Test 1: mutation through plain parameter (current behavior, correct)

```loft
struct S { x: integer not null }
fn modify(s: S) { s.x = 99; }
fn main() {
  p = S { x: 1 };
  modify(p);
  assert(p.x == 99, "mutation visible to caller");
}
```

#### Test 2: const parameter locks store

```loft
struct S { x: integer not null }
fn read(s: const S) -> integer { s.x }
fn main() {
  p = S { x: 1 };
  assert(read(p) == 1, "const read works");
}
```

#### Test 3: const prevents mutation via &T (runtime panic)

```loft
struct S { x: integer not null }
fn mutate_ref(m: &S) { m.x = 99; }
fn bad(s: const S) { mutate_ref(s); }
fn main() { bad(S { x: 1 }); }
// Panics: "Write to locked store"
```

#### Test 4: escape to non-const blocks auto-lock

```loft
struct S { x: integer not null }
fn helper(s: S) { s.x = 42; }
fn caller(s: S) {
  helper(s);  // s escapes to mutable function — cannot auto-lock
}
```

#### Test 5: return value copy (current behavior)

```loft
struct Point { x: float not null, y: float not null }
fn make() -> Point { Point { x: 1.0, y: 2.0 } }
fn main() {
  p = make();       // OpCopyRecord fires here
  q = p;            // OpCopyRecord fires here
  q.x = 99.0;
  assert(p.x == 1.0, "p isolated from q after copy");
}
```

#### Test 6: move optimization target

```loft
fn make() -> Point { Point { x: 1.0, y: 2.0 } }
fn main() {
  p = make();       // Could be a move (no copy) if dest-passing works
  println("{p.x}");
}
```

---

### Priority order

| # | Optimization | Impact | Effort | Risk |
|---|---|---|---|---|
| 1 | Return slot / destination passing | ~15 KB/frame in games | M | Low — text_return already does this |
| 2 | Last-use move for `x = y` | Eliminates temp copies | S | Low — liveness data available |
| 3 | Auto-const inference | Safety, not perf | M | Medium — needs escape analysis |

---

### Related

- [PERFORMANCE.md](PERFORMANCE.md) — benchmark data and optimization plan
- Optimisations section below — planned interpreter optimizations
- [SLOTS.md](SLOTS.md) — stack slot assignment design

---


## Block Copy Efficiency: Analysis and Recommendations

### What's actually expensive

Block copy has two phases:

| Phase | Function | Cost | What it does |
|---|---|---|---|
| 1. `copy_block` | `structures.rs:777` | O(struct_size) memcpy | Raw byte copy of struct fields |
| 2. `copy_claims` | `allocation.rs:642` | O(total_owned_data) + allocations | Deep copy of ALL owned sub-structures |

Phase 1 is cheap — typically 4-128 bytes of memcpy.

Phase 2 is the real cost.  For each field, `copy_claims` recursively:
- **Text fields:** re-interns the string (allocate store record + memcpy)
- **Vectors:** allocates new vector record, copies header + all elements,
  then recursively copies each element's owned data
- **Nested structs:** recurses into each struct field
- **Arrays/Hash/Index:** O(n) allocations, each with recursive traversal

A `Mat4` with a `vector<float>` costs: 1 store allocation + vector
allocation + 16 floats copied.  A `Scene` with meshes/materials/nodes:
dozens of allocations, hundreds of bytes.

### Where deep copies happen today

Deep copies (OpCopyRecord) fire in exactly three codegen paths, all in
`gen_set_first_at_tos` (codegen.rs:931-983):

```
x = func()      →  gen_set_first_ref_copy      →  OpCopyRecord
x = y            →  gen_set_first_ref_var_copy  →  OpCopyRecord
(a, b) = expr    →  gen_set_first_ref_tuple_copy →  OpCopyRecord
```

Each emits: `OpConvRefFromNull` → `OpDatabase` → `OpCopyRecord`.

Return values themselves are cheap (12-byte DbRef shallow copy in
`copy_result`).  The deep copy only fires at first assignment.

### Optimization candidates

#### O-B1. Last-use move — **IMPLEMENT THIS**

**Pattern:**
```loft
temp = compute();
result = temp;     // temp never used again → move DbRef, skip copy
```

**Detection:** At the `x = y` codegen site (`gen_set_first_ref_var_copy`),
check `stack.function.last_use(src) <= stack.function.first_def(v)`.
If true, the source variable is never read after this assignment.
Transfer the DbRef instead of deep copying.

**Implementation:**
```rust
// In gen_set_first_ref_var_copy, before emitting OpCopyRecord:
if stack.function.last_use(src) <= stack.function.first_def(v) {
    // Move: just copy the DbRef (12 bytes), skip deep copy.
    // Mark src as moved so OpFreeRef skips it.
    stack.add_op("OpVarRef", self);
    let src_pos = stack.position - stack.function.stack(src);
    self.code_add(src_pos);
    stack.function.set_skip_free(src);
    return;
}
```

**Impact:** Eliminates ALL deep copies for temporary-to-final patterns.
Common in math-heavy code:
```loft
m = mat4_identity();        // allocate + build
mvp = mat4_mul(proj, m);    // m's last use → move, not copy
```

**Complexity:** S — 5-10 lines in one function.  No new opcodes,
no ABI changes.  Uses existing `last_use`, `first_def`, `skip_free`.

**Risk:** Low.  The liveness analysis is already computed and used
for slot assignment.  `skip_free` already exists for other purposes.
Must verify that `last_use` accounts for implicit frees (OpFreeRef)
— if it does, the check is safe.

---

#### O-B2. Last-use move for function returns — **IMPLEMENT THIS**

**Pattern:**
```loft
result = make_point();   // function returns struct, immediately assigned
// make_point's return value is never aliased
```

Currently: function returns DbRef (shallow), then `gen_set_first_ref_copy`
deep copies it.  The return value's store is freed immediately after.

**Detection:** The RHS is `Value::Call(OpCopyRecord, args)` where args[0]
is a function call.  The intermediate DbRef from the function call is
always a temporary — it has no variable, so it's always "last use".

**Implementation:** In `gen_set_first_ref_copy`, when the inner call
is a user function (not OpCopyRecord itself), the return value is a
one-shot temporary.  Skip the OpCopyRecord and directly adopt the
returned DbRef.

However, there's a subtlety: the returned DbRef points into the
callee's store, which may be freed.  Need to verify store lifetime.
If the callee's store outlives the return (it should — stores are
not freed until explicit OpFreeRef), this is safe.

**Impact:** Eliminates deep copies for `x = func()` patterns.

**Complexity:** M — needs careful store lifetime analysis.

---

#### O-B3. Destination passing for struct returns — **DEFER**

**Pattern:** Extend the text `RefVar(Text)` destination-passing
mechanism to struct-returning functions.  Caller pre-allocates the
destination store, callee writes directly into it.

**Why defer:** Requires ABI changes (hidden parameter), callee
rewriting to use the destination store for all field writes, and
interaction with existing OpCopyRecord codegen.  The last-use move
(O-B1) handles the most common cases with much less complexity.

**When to revisit:** If profiling shows that `x = func()` copies
remain a bottleneck after O-B1/O-B2, destination passing is the
next step.

---

#### O-B4. Shallow copy for immutable borrows — **DEFER**

**Pattern:** `x = y` where x is never mutated — share the DbRef
instead of deep copying.

**Why defer:** Requires copy-on-write or reference counting to
prevent double-free and aliasing bugs.  The auto-const analysis
(CONST_REF.md O3) would need to run first to identify which
variables are immutable.  High complexity for moderate gain.

---

### Status after O-B1

**O-B1 is implemented** (codegen.rs `gen_set_first_ref_var_copy`).
When `x = y` and y has `uses == 1` (only read here), **owns its store**
(empty type dep), not an argument, not captured: emits OpVarRef +
skip_free instead of the full deep copy.

**The ownership precondition is load-bearing, and it was missing until
loft#823.** A move hands `x` the source's store and suppresses the
source's free, so it is sound only where the source HAS a store to give.
`uses == 1` answers *liveness* — nobody reads it after this — which says
nothing about *ownership*. A non-empty type dep says positively that `y`
is a VIEW of another variable, and moving from a view handed `x` an
interior pointer whose scope-exit `OpFreeRef` then released the
**container's** record: `for p in v { g = p; … }` SIGSEGV'd for every
heap element type, struct as readily as tuple. The forms that survived
did so because an earlier loop is what pushes the use count to exactly 1
— an accident of counting, not an ownership fact.

@PLN90's `use_analysis::move_elidable_source` states the same rule for
the move plans it builds ("never a view/projection, which owns no store
to move"); O-B1 is the shortcut that predates it. Any future move
shortcut needs the same two questions, and *liveness* is only the second
one.

#### Remaining deep copy sites

| Site | Codegen function | Pattern | Frequency |
|---|---|---|---|
| 1 | `gen_set_first_ref_copy` | `x = func()` | **Very common** — every struct-returning call |
| 2 | `gen_set_first_ref_var_copy` | `x = y` (uses > 1) | Rare — O-B1 handles uses == 1 |
| 3 | `gen_set_first_ref_tuple_copy` | `(a, b) = expr` | Rare — tuple destructuring |

**Site 1 is the dominant remaining cost.** Every `m = mat4_mul(a, b)`
allocates a fresh store, deep copies, and **leaks the callee's store**.

#### Store leak on struct returns

When a function returns a struct, the callee's store is kept alive
(scopes.rs `in_ret` check skips OpFreeRef).  After the caller deep
copies from it, nobody frees it.  This is a latent store leak that
grows linearly with struct-returning calls.

### Recommendation

#### O-B2: Return store adoption — **IMPLEMENT NEXT**

For `x = func()`, the source is always a temporary on the eval stack
(no variable holds it).  Instead of allocating a NEW store + deep
copy, adopt the returned DbRef directly.  This fixes BOTH the copy
cost AND the store leak.

**Safety concern:** The returned DbRef might point to a parameter's
store (e.g. `fn identity(p: Point) -> Point { p }`).  Adopting it
would cause the caller to free a shared store.

**Safe implementation:** After the function call + OpCopyRecord runs,
free the source store.  The deep copy is still performed, but the
leak is fixed.  Then separately optimize the copy away for provably
fresh returns (callee constructs a new struct, never returns a param).

**Detecting fresh returns:** A function whose return type has empty
dependencies (`dep.is_empty()`) and whose return expression is a
struct constructor or a call to another struct-returning function
(not a Var pointing to a parameter) is safe to adopt.

**Complexity:** M — two-phase: (1) fix the leak (S), (2) skip copy
for fresh returns (M, needs callee analysis).

#### O-B3 and O-B4 — **DEFER**

Destination passing (O-B3) is the clean long-term solution but
requires ABI changes.  Shallow copy for immutables (O-B4) needs
copy-on-write.  Both deferred until the simpler O-B2 is in place.

#### Expected savings after O-B1 + O-B2

| Pattern | Current cost | After O-B1+O-B2 |
|---|---|---|
| `m = mat4_identity(); mvp = m` | Deep copy | 12B DbRef move (O-B1) |
| `result = temp_struct` where temp dies | Deep copy | 12B DbRef move (O-B1) |
| `x = func()` (func builds new struct) | Deep copy + store leak | 12B DbRef adopt (O-B2) |
| `x = func()` (func returns param) | Deep copy + store leak | Deep copy, no leak (O-B2 phase 1) |
| Loop: `acc = transform(acc)` | Deep copy per iter | 12B move per iter (O-B1) |

For Brick Buster (60 rect_mvp/frame): ~15 KB deep copies + 60 leaked
stores/frame → 720B moves + 0 leaks.

---

### Implementation status

| Optimisation | Status | Issue |
|---|---|---|
| O-B1: last-use move `x = y` | **Done** | — |
| O-B2: adoption for no-ref-param functions | **Done** (codegen branch for `n_*` functions) | — |
| O-B2: deep copy for ref-param functions | **Partially done** (`gen_set_first_ref_call_copy`) | P116 |
| Store leak fix (callee store after copy) | **Partial** — O-B2 adoption fixes no-ref-param case | P117 |
| Threading regression | **Blocked** — needs investigation | P118 |

#### Known issues found during optimisation

- **P116**: `x = func(s)` where func has Reference params aliases
  the store.  Codegen branch added but needs regression testing.
- **P117**: Store leak for struct-returning functions.  Fixed for
  no-ref-param functions by O-B2 adoption.  Remaining: ref-param case.
- **P118**: `22-threading.loft` panics "Incomplete record" after
  P64/P66 checked arithmetic changes.  Not yet diagnosed.

---

## Design: O8 — Bulk initialisation of constant data

Design for bulk initialisation of constant data structures, reducing bytecode
size and interpreter dispatch overhead for vector literals, struct defaults,
and repeated-element patterns.

---

### Contents
- [Motivation](#motivation)
- [Current behaviour](#current-behaviour)
- [Constant folding](#constant-folding)
- [Proposed changes](#proposed-changes)
  - [O8.1 — Bulk primitive vector literals](#o81--bulk-primitive-vector-literals)
  - [O8.2 — Bulk struct vector literals](#o82--bulk-struct-vector-literals)
  - [O8.3 — Zero-fill struct defaults](#o83--zero-fill-struct-defaults)
  - [O8.4 — Const text table](#o84--const-text-table)
  - [O8.5 — Constant range comprehensions](#o85--constant-range-comprehensions)
- [Out of scope](#out-of-scope)
- [Implementation order](#implementation-order)

---

### Motivation

A 20-element integer vector literal `[1, 2, ..., 20]` currently emits 60
bytecodes (3 per element: `OpNewRecord` + `OpSetInt` + `OpFinishRecord`)
and performs 20 store-allocation checks plus multiple vector resizes.  Native
codegen produces 60 individual store writes.

For data-heavy programs (lookup tables, configuration, test fixtures), this
overhead dominates both compilation size and startup time.  The store already
has `copy_block()` and `zero_fill()` primitives that can transfer arbitrary
byte ranges in a single call.

⚠ **Measured at the size a real fixture reaches, the cost is not a slowdown —
it is a wall.**  @PLN146 W3 generated a `graphics` test file holding two
Pillow-derived goldens at 100x100 (a 40 000-element source literal and a
43 200-element expectation, 765 KB of `.loft`).  The INTERPRETER parses and runs
it; `--native` ran past a 900 s `LOFT_TIMEOUT` in `phase=parse` and was
hard-killed with SIGABRT.  Three things worth carrying from that:

- the two backends part company entirely, so a fixture can pass locally on
  `--interpret` and kill the `--native` CI leg;
- the failure is a TIMEOUT, so it reports as a hang rather than as "this literal
  is too big", and nothing points at the literal;
- the workaround is to bound the fixture (W3 shrank each golden to the smallest
  shape that still walks its path, 765 KB → 42 KB), which costs coverage a
  design like O8.1 would give back.

---

### Current behaviour

#### Primitive vector literal: `[1, 2, 3, 4, 5]`

Parser IR (per element):
```
OpNewRecord(vec, type_nr, u16::MAX)   // allocate element slot
OpSetInt(elm, field_offset, value)    // write the integer
OpFinishRecord(vec, elm, type_nr, u16::MAX)  // increment length
```

Interpreter: 3 dispatches per element.  `OpNewRecord` calls `vector_new()`
which checks capacity and may call `store.resize()`.

Native: 3 function calls per element.  No batching.

#### Struct literal: `Point { x: 1.0, y: 2.0 }`

Parser IR (per field):
```
OpSetFloat(ref, field_offset, value)
```

After all explicit fields, `object_init()` fills omitted fields with zero
or default values — one `OpSetInt`/`OpSetFloat`/etc. per omitted field.

#### Repeated element: `[Struct { ... }; 100]`

Already optimised: `OpAppendCopy` copies one initialised element N times
using `copy_block()`.  Only the first element is constructed field-by-field.

---

### Constant folding

All O8 phases share a prerequisite: the ability to evaluate pure expressions
at compile time.  `[2*3, 4+1, 10/2]` should be treated as `[6, 5, 5]` and
become eligible for bulk init, not just bare literals like `[6, 5, 5]`.

#### What qualifies as a constant expression

An expression is **const-evaluable** when it contains only:

| Node type | Example | Const? |
|---|---|---|
| Integer / float / single literal | `42`, `3.14`, `1.0f` | Yes |
| Boolean literal | `true`, `false` | Yes |
| Character literal | `'A'` | Yes |
| Text literal (no interpolation) | `"hello"` | Yes — but only for text table (O8.4) |
| Arithmetic on const operands | `2 * 3`, `n + 1` where `n` is const | Yes |
| Comparison on const operands | `x > 0` where `x` is const | Yes |
| Unary ops on const operands | `-x`, `!b` where operand is const | Yes |
| `as` cast between numeric types | `42 as single`, `3.14 as integer` | Yes |
| File-scope `UPPER_CASE` constants | `PI`, `MAX_SIZE` | Yes |
| Conditional with const condition | `if true { 1 } else { 2 }` → `1` | Yes |
| Null literal | `null` | Yes (folds to sentinel) |
| Function calls | `sqrt(2.0)` | **No** — side effects not provable |
| Variable references | `x` (local mutable) | **No** |
| Field access | `p.x` | **No** |
| Format strings | `"val={x}"` | **No** — depends on runtime values |

#### Implementation: `const_eval()`

Add a function `const_eval(val: &Value, data: &Data) -> Option<Value>` in
`src/parser/expressions.rs` (or a new `src/const_eval.rs`):

```rust
/// Evaluate a pure expression at compile time.
/// Returns Some(literal) when fully evaluable, None otherwise.
/// Conservative: unknown patterns return None → runtime fallback.
///
/// Safety invariants (see §Safety S5):
///  - Integer arithmetic uses wrapping_{add,sub,mul} to match interpreter overflow
///  - Division/modulo by zero → None (runtime returns null)
///  - Division/modulo of i32::MIN by -1 → None (wrapping_div panics in debug)
///  - Float NaN propagation: Rust f64 ops handle this naturally
///  - No recursion depth limit needed: IR tree depth is bounded by parser
pub fn const_eval(val: &Value, data: &Data) -> Option<Value> {
    match val {
        Value::Int(_) | Value::Long(_) | Value::Float(_)
        | Value::Single(_) | Value::Boolean(_) => Some(val.clone()),
        Value::Call(op, args) => {
            let folded: Option<Vec<Value>> = args.iter()
                .map(|a| const_eval(a, data))
                .collect();
            let args = folded?;
            let name = &data.def(*op).name;
            match (name.as_str(), args.as_slice()) {
                // --- integer ---
                ("OpAddInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a.wrapping_add(*b))),
                ("OpMinInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a.wrapping_sub(*b))),
                ("OpMulInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a.wrapping_mul(*b))),
                ("OpDivInt", [Value::Int(a), Value::Int(b)])
                    if *b != 0 && !(*a == i32::MIN && *b == -1) =>
                    Some(Value::Int(a / b)),
                ("OpModInt", [Value::Int(a), Value::Int(b)])
                    if *b != 0 && !(*a == i32::MIN && *b == -1) =>
                    Some(Value::Int(a % b)),
                ("OpMinSingleInt", [Value::Int(a)]) =>
                    Some(Value::Int(a.wrapping_neg())),
                // --- long ---
                ("OpAddLong", [Value::Long(a), Value::Long(b)]) =>
                    Some(Value::Long(a.wrapping_add(*b))),
                ("OpMinLong", [Value::Long(a), Value::Long(b)]) =>
                    Some(Value::Long(a.wrapping_sub(*b))),
                ("OpMulLong", [Value::Long(a), Value::Long(b)]) =>
                    Some(Value::Long(a.wrapping_mul(*b))),
                ("OpDivLong", [Value::Long(a), Value::Long(b)])
                    if *b != 0 && !(*a == i64::MIN && *b == -1) =>
                    Some(Value::Long(a / b)),
                // --- float ---
                ("OpAddFloat", [Value::Float(a), Value::Float(b)]) =>
                    Some(Value::Float(a + b)),
                ("OpMinFloat", [Value::Float(a), Value::Float(b)]) =>
                    Some(Value::Float(a - b)),
                ("OpMulFloat", [Value::Float(a), Value::Float(b)]) =>
                    Some(Value::Float(a * b)),
                ("OpDivFloat", [Value::Float(a), Value::Float(b)]) =>
                    Some(Value::Float(a / b)),  // NaN/Inf handled by IEEE 754
                // --- single ---
                ("OpAddSingle", [Value::Single(a), Value::Single(b)]) =>
                    Some(Value::Single(a + b)),
                ("OpMinSingle", [Value::Single(a), Value::Single(b)]) =>
                    Some(Value::Single(a - b)),
                ("OpMulSingle", [Value::Single(a), Value::Single(b)]) =>
                    Some(Value::Single(a * b)),
                ("OpDivSingle", [Value::Single(a), Value::Single(b)]) =>
                    Some(Value::Single(a / b)),
                // --- comparison (integer) ---
                ("OpEqInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Boolean(*a == *b)),
                ("OpNeInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Boolean(*a != *b)),
                ("OpLtInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Boolean(*a < *b)),
                ("OpLeInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Boolean(*a <= *b)),
                // --- bitwise ---
                ("OpAndInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a & b)),
                ("OpOrInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a | b)),
                ("OpXorInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a ^ b)),
                // --- casts ---
                ("OpConvLongFromInt", [Value::Int(a)]) =>
                    Some(Value::Long(i64::from(*a))),
                ("OpConvFloatFromInt", [Value::Int(a)]) =>
                    Some(Value::Float(*a as f64)),
                ("OpConvIntFromLong", [Value::Long(a)]) =>
                    Some(Value::Int(*a as i32)),
                ("OpConvIntFromFloat", [Value::Float(a)]) if a.is_finite() =>
                    Some(Value::Int(*a as i32)),
                // --- boolean ---
                ("OpNot", [Value::Boolean(a)]) =>
                    Some(Value::Boolean(!a)),
                ("OpAndBool", [Value::Boolean(a), Value::Boolean(b)]) =>
                    Some(Value::Boolean(*a && *b)),
                ("OpOrBool", [Value::Boolean(a), Value::Boolean(b)]) =>
                    Some(Value::Boolean(*a || *b)),
                _ => None,
            }
        }
        Value::If(cond, then_val, else_val) => {
            if let Some(Value::Boolean(c)) = const_eval(cond, data) {
                const_eval(if c { then_val } else { else_val }, data)
            } else {
                None
            }
        }
        _ => None,
    }
}
```

The function returns `Some(literal)` when the expression can be fully
evaluated, or `None` when it cannot.  It is conservative: any unknown
pattern returns `None` and falls back to runtime evaluation.

Key safety properties:
- `wrapping_*` for integer arithmetic matches interpreter overflow semantics
- Division by zero → `None` (runtime returns null via sentinel)
- `i32::MIN / -1` → `None` (would panic in Rust debug, wraps in release)
- Float division by zero → `Inf`/`NaN` via IEEE 754 (same as runtime)
- `as i32` cast on non-finite float → `None` (avoids undefined truncation)

#### Where it plugs in

| Phase | Call site | Effect |
|---|---|---|
| O8.1 | `build_vector_list()` after collecting items | Fold each element; if all fold → bulk init |
| O8.2 | Same, for struct field values | Fold each field; if all fold → packed record |
| O8.3 | `object_init()` for default expressions | Fold default; if folds to zero → skip emit |
| O8.5 | `parse_vector_for()` for `[for i in 0..N { expr(i) }]` | Fold body for each i; if all fold → bulk init |
| General | Any `Value::Call` during second pass | Opportunistic: replace with literal when possible |

#### Null sentinel folding

Null sentinels differ by type:

| Type | Null sentinel | Byte representation |
|---|---|---|
| `integer` | `i64::MIN` | `0x0000000000000080` (little-endian) |
| `integer not null` | N/A (0 is valid) | — |
| narrow `i32` integer | `i32::MIN` (`-2147483648`) | `0x00000080` |
| `float` | `NaN` | `0x000000000000F87F` |
| `single` | `NaN (f32)` | `0x0000C07F` |
| `boolean` | `false` | `0x00` |
| `character` | `'\0'` | `0x00000000` |

When folding `null` in a typed context, produce the correct sentinel value
so it can be packed into the bulk data buffer.

#### File-scope constants

Loft `UPPER_CASE` constants at file scope are already evaluated once:

```loft
PI = 3.14159265358979;
SCALE = 100;
data = [PI * SCALE, PI * SCALE * 2];  // should fold to [314.159..., 628.318...]
```

`const_eval` resolves `PI` and `SCALE` by looking up their `Value::Set`
initialiser in the IR.  Only constants that are themselves const-evaluable
qualify; a constant initialised from a function call does not.

---

### Proposed changes

#### O8.1 — Bulk primitive vector literals

**Applies to:** `vector<integer>`, `vector<float>`,
`vector<single>` where ALL elements are const-evaluable (see
[Constant folding](#constant-folding) — includes literals, arithmetic
on literals, file-scope constants, and casts).

**New opcode:** `OpInitVector(vec, count: const u16, elem_size: const u16)`

The opcode reads `count * elem_size` bytes of packed constant data from
the code stream immediately following the operands, then:

1. Allocates a vector record of `(count * elem_size + 8 + 7) / 8` words
2. Writes `count` into the length field (offset 4)
3. Copies the constant bytes into offsets `8..8 + count * elem_size`

**Opcode definition** (`default/01_code.loft`):
```loft
fn OpInitVector(r: vector, count: const u16, elem_size: const u16);
```

The `#rust` body calls a new `vector::init_vector_bulk()` function in
`src/vector.rs` that reads constant data from the code stream.

**Parser detection** (`src/parser/vectors.rs`):
In `build_vector_list()`, after collecting all elements, call
`const_eval()` on each item.  If every element folds to a primitive
literal, pack the folded values into a byte buffer and emit
`OpInitVector` + the raw bytes instead of the per-element loop.

Examples that qualify:
```loft
[1, 2, 3, 4, 5]              // bare literals
[2*3, 4+1, 10/2]             // arithmetic folds to [6, 5, 5]
[PI, PI*2, PI*3]              // constant references fold
[1 as single, 2 as single]   // casts fold
[0; 1000]                     // already optimised via OpAppendCopy
```

**Interpreter** (`src/fill.rs`):
```rust
fn init_vector(s: &mut State) {
    let count = *s.code::<u16>() as u32;
    let elem_size = *s.code::<u16>() as u32;
    // S4: overflow check before allocation
    let total = u64::from(count) * u64::from(elem_size);
    assert!(total <= MAX_STORE_WORDS as u64 * 8, "OpInitVector: {count}×{elem_size} exceeds store limit");
    let total = total as u32;
    // S3: bounds-checked read from code stream
    let src = s.code_ptr(total);
    let db = *s.get_stack::<DbRef>();
    let store = keys::mut_store(&db, &mut s.database.allocations);
    let vec_rec = store.claim((total + 8 + 7) / 8);
    store.set_int(db.rec, db.pos, vec_rec as i32);
    store.set_int(vec_rec, 4, count as i32);
    // S2: data is already at native alignment (8-byte word boundary + 8-byte header)
    store.copy_from_code(vec_rec, 8, src, total);
}
```

**Parser packing** (`src/parser/vectors.rs`):
```rust
// S1: pack in native byte order to match store.set_int / store.set_float
fn pack_const_vector(values: &[Value]) -> Vec<u8> {
    let mut buf = Vec::new();
    for v in values {
        match v {
            Value::Int(n)    => buf.extend_from_slice(&n.to_ne_bytes()),
            Value::Long(n)   => buf.extend_from_slice(&n.to_ne_bytes()),
            Value::Float(n)  => buf.extend_from_slice(&n.to_ne_bytes()),
            Value::Single(n) => buf.extend_from_slice(&n.to_ne_bytes()),
            _ => unreachable!("non-primitive in const vector"),
        }
    }
    buf
}
```

**Native codegen** (`src/generation/`):
Emit a `static INIT_DATA: [u8; N] = [...]` array (bytes in native order)
and a single `store.copy_block_from_slice(vec_rec, 8, &INIT_DATA)` call.

**Bytecode reduction:** `3 * N` opcodes → 1 opcode + `N * elem_size` raw bytes.
For 100 integers: 300 dispatches → 1 dispatch + 400 bytes inline data.

**State method** (`src/state/mod.rs`):
Add `code_ptr(len: u32) -> *const u8` that returns a pointer to the current
code position and advances past `len` bytes.  Panics in debug if
`code_pos + len > code.len()` (S3).  Used only by `OpInitVector`.

---

#### O8.2 — Bulk struct vector literals

**Applies to:** `vector<Struct>` where ALL elements are struct literals with
ALL fields being const-evaluable (integers, floats, booleans, characters;
no text, no nested structs, no reference fields).

**Approach:** Extend O8.1 to structs.  Each struct element is a fixed-size
byte record.  Pack all N records contiguously and use the same
`OpInitVector` opcode with `elem_size = struct_record_size`.

**Parser detection:** In `build_vector_list()`, for each struct element:
1. Call `const_eval()` on every field value
2. If all fields fold, write the folded values at the correct byte offsets
   (from `calc::calculate_positions`)
3. If all elements fold, emit `OpInitVector` with the packed records

```loft
struct Point { x: float not null, y: float not null }
data = [
  Point { x: 1.0, y: 2.0 },
  Point { x: 3.0, y: 4.0 },
  Point { x: 5.0 + 0.5, y: 6.0 * 2.0 },  // folds to { x: 5.5, y: 12.0 }
];
// → single OpInitVector with 3 × 16 = 48 bytes of packed data
```

**Limitation:** Struct elements with text or reference fields fall back to
per-element initialisation.  This is the common case for real-world structs,
so the benefit is primarily for numeric-heavy structs (points, colours,
coordinates, pixel data).

---

#### O8.3 — Zero-fill struct defaults

**Applies to:** Any struct construction where omitted fields use the default
value (null sentinel for the type).

**Current:** `object_init()` emits one `OpSetInt(ref, offset, 0)` per omitted
integer field, one `OpSetFloat(ref, offset, NaN)` per omitted float, etc.

**Optimisation:** The store's `zero_fill(rec)` already zeroes an entire
record.  Use it as a first step, then patch only non-zero sentinels.

**Approach:**
1. After `OpDatabase` allocates the record, emit `OpZeroFill(ref)` once
2. Only emit explicit `OpSetX` for fields with non-zero null sentinels:
   - `integer` (nullable): `i64::MIN` (narrow `i32` fields use `i32::MIN`, `0x00000080`), not zero → explicit
   - `float`: NaN → explicit
   - `single`: NaN → explicit
   - Fields with `default(expr)` or `= expr` → explicit
3. Fields that ARE zero after `zero_fill` (skip `OpSetX`):
   - `boolean` null = `0` ✓
   - `character` null = `0` ✓
   - `vector`/`sorted`/`hash`/`index` null = `0` ✓
   - `reference` null = `0` ✓
   - `text` null = `0` ✓
   - `integer not null` default = `0` ✓

**Benefit:** A struct with 5 boolean fields, 3 vector fields, and 2
integer fields reduces from 10 `OpSetInt(0)` calls to 1 `OpZeroFill` +
2 `OpSetInt(i32::MIN)`.  Structs with mostly non-numeric fields benefit
most.

**Risk:** Low — `zero_fill` is already used by the store for freed records.
See S6 in the safety section for the full null-sentinel analysis.

---

#### O8.4 — Const text table

**Applies to:** Repeated text literals across a program.

**Current:** Each text literal `"hello"` in a format string or assignment
generates an inline `OpText` with the UTF-8 bytes embedded in the bytecode.
If `"hello"` appears 10 times, the bytes are duplicated 10 times.

**Approach:** Deduplicate text constants into a string table at compile time.
Each unique string gets an index.  `OpConstText(index)` looks up the string
from the table instead of reading inline bytes.

**Benefit:** Reduces bytecode size for programs with repeated string literals
(logging format strings, error messages, enum-to-string tables).

**Cost:** Adds an indirection.  Only beneficial when the same string appears
multiple times.  Not worth it for strings that appear once.

**Verdict:** Low priority — most loft programs use format interpolation, not
repeated literals.  Defer unless bytecode size becomes a bottleneck.

---

#### O8.5 — Constant range comprehensions

**Applies to:** `[for i in A..B { expr(i) }]` where `A` and `B` are
const-evaluable integers and `expr(i)` is const-evaluable for every `i`
in the range.

**Current:** A comprehension always generates a runtime loop: init counter,
test bound, evaluate body, append element, increment, branch back.  For
`[for i in 0..100 { i * i }]` this is 100 loop iterations at runtime.

**Optimisation:** At compile time, unroll the loop:
1. Evaluate `A` and `B` via `const_eval` to get concrete integer bounds
2. For each `i` in `A..B`, substitute `i` into the body and call `const_eval`
3. If every iteration folds to a constant, pack the results and emit
   `OpInitVector`

```loft
squares = [for i in 0..10 { i * i }];
// Compiler unrolls: const_eval(0*0)=0, const_eval(1*1)=1, ..., const_eval(9*9)=81
// → OpInitVector with 10 × 4 = 40 bytes: [0, 1, 4, 9, 16, 25, 36, 49, 64, 81]
```

**Filtered comprehensions:** `[for i in A..B if pred(i) { expr(i) }]`
also qualifies when `pred(i)` is const-evaluable.  The compiler evaluates
the predicate for each `i` and only includes elements where it is true:

```loft
evens = [for i in 0..20 if i % 2 == 0 { i }];
// Compiler: const_eval(0%2==0)=true, const_eval(1%2==0)=false, ...
// → OpInitVector with 10 × 4 = 40 bytes: [0, 2, 4, 6, 8, 10, 12, 14, 16, 18]
```

**Size limit (S7):** Do not unroll ranges larger than 10,000 elements.
This is a hard limit enforced in the parser, not configurable.  Ranges
above the limit silently fall back to runtime loops — no error, no
performance regression, just no optimisation.  The limit prevents
adversarial programs from exhausting compiler memory.

```rust
const MAX_CONST_UNROLL: u32 = 10_000;

// In parse_vector_for, before attempting const fold:
let range_size = (end - start) as u32;
if range_size > MAX_CONST_UNROLL {
    // Fall back to runtime loop — range too large for compile-time unroll
    return normal_loop_path();
}
```

**Where it plugs in:** In `parse_vector_for()` (or `build_comprehension_code`),
before emitting the loop IR:
1. Check if range bounds are const-evaluable
2. If so, try to fold the body for each iteration
3. If all fold, emit `OpInitVector` instead of the loop
4. Otherwise fall back to the normal loop path

**Nested comprehensions:** Not supported for const folding.  Only simple
`for i in A..B` with a non-loop body qualifies.

**Dependencies:** O8.1 (provides `OpInitVector`), `const_eval()`.

---

### Safety analysis

#### S1 — Endianness: native byte order only

`store.set_int()` writes via `*addr_mut::<i32>() = val`, which uses the host's
native byte order.  `OpInitVector` must pack constant bytes in the **same
native byte order** — i.e. `val.to_ne_bytes()`, not `to_le_bytes()` or
`to_be_bytes()`.

**Risk:** If the packing uses the wrong byte order, every element reads as
garbage.  All current platforms (x86-64, aarch64) are little-endian, so the
bug would only surface on a big-endian target.

**Mitigation:** Use `i32::to_ne_bytes()` / `f64::to_ne_bytes()` in the
packing loop.  Add a test that round-trips a known value through pack →
`OpInitVector` → `get_vector` → compare.

#### S2 — Alignment: store uses 8-byte-word addressing

The store's `ptr` is `*mut u8` but `addr_mut::<T>` casts to `*mut T` via
`ptr.offset(...).cast::<T>()`.  This is safe because all records are
allocated at 8-byte word boundaries (`claim` returns word indices, addresses
are `rec * 8 + fld`).  Field offsets are computed by `calc.rs` to respect
alignment.

`OpInitVector` bulk-copies bytes starting at offset 8 (past the length
header).  Elements are at `8 + i * elem_size`.  For 4-byte integers this
is always 4-byte aligned.  For 8-byte integers/floats this is always 8-byte
aligned (because the header is 8 bytes).

**Risk:** None for primitive vectors — alignment is inherent.  For O8.2
(struct vectors), the struct record size must be a multiple of the largest
field alignment (guaranteed by `calc::calculate_positions`).

#### S3 — Buffer overflow in code stream

`OpInitVector` reads `count * elem_size` bytes from the bytecode stream.
If the bytecode is malformed (count or elem_size is wrong), the read could
overrun the code buffer.

**Mitigation:** `State::code_ptr(len)` must bounds-check against the code
stream size.  In debug builds, `debug_assert!(self.code_pos + len <= self.code.len())`.
In release builds the code stream is compiler-generated and cannot be
malformed unless the compiler has a bug — same trust model as existing
opcodes that read `code::<u16>()` etc.

#### S4 — Store allocation overflow

`store.claim((total + 8 + 7) / 8)` can overflow if `count * elem_size`
exceeds `u32::MAX - 15`.  For `u16` count and `u16` elem_size, the maximum
`total` is `65535 * 65535 = 4,294,836,225` which exceeds `u32::MAX`.

**Mitigation:** Check `(count as u64) * (elem_size as u64) <= MAX_STORE_WORDS * 8`
before the allocation.  If exceeded, panic with a clear message (same as
the existing `MAX_STORE_WORDS` guard in `store.rs`).

#### S5 — `const_eval` correctness

If `const_eval` produces a wrong value, the bulk-initialised vector silently
contains incorrect data — with no runtime check.

**Mitigations:**
1. `const_eval` is conservative: any unrecognised pattern returns `None` and
   falls back to runtime.  Wrong results can only come from incorrectly
   implemented operator cases.
2. Use `wrapping_add`/`wrapping_sub`/`wrapping_mul` for integer arithmetic
   to match the interpreter's overflow semantics.  Loft integers wrap on
   overflow — they do not trap.
3. Division by zero: `const_eval` must return `None` (not fold), matching
   the runtime behaviour of returning null.  The design already shows
   `if *b != 0` guard.
4. Float NaN propagation: `NaN + x = NaN`, `NaN * x = NaN` etc. must be
   preserved.  Rust's `f64` arithmetic already handles this.
5. Integer null sentinel: `i32::MIN` is the null sentinel.  Folding
   `i32::MIN + 1` should produce `-2147483647`, not null.  `wrapping_add`
   does the right thing.  Folding `-2147483647 - 1` wraps to `i32::MIN`
   which IS the null sentinel — this matches runtime behaviour.
6. **Test strategy:** For each operator in `const_eval`, add a test that
   compares `const_eval(expr)` against `state.execute(expr)` for the same
   inputs.  Any divergence is a bug.

#### S6 — O8.3 zero-fill assumes null sentinels are zero

`zero_fill` writes all-zero bytes.  This is correct for:
- `integer` null = `0` (which IS `i32::MIN`? **No** — `i32::MIN` is
  `0x80000000`, not zero!)

**Correction:** The O8.3 design is partially wrong.  `integer` null
sentinel is `i32::MIN` (`-2147483648` = `0x00000080` in LE), not `0`.
Zero-fill produces `0` which is a valid non-null integer.

For nullable integer fields, `zero_fill` produces the wrong default.
Only `not null` integer fields (where `0` is the intended default) benefit.

**Revised O8.3 rule:** Use `zero_fill` only when ALL omitted fields have
a zero-byte null sentinel:
- `boolean` null = `false` = `0` ✓
- `character` null = `'\0'` = `0` ✓
- `vector`/`sorted`/`hash`/`index` null = `0` (null pointer) ✓
- `reference` null = `0` ✓
- `integer` null = `i64::MIN` (narrow `i32` fields = `i32::MIN` = `0x00000080`) ✗
- `float` null = `NaN` ✗
- `single` null = `NaN` ✗
- `text` null = null pointer = `0` ✓

So `zero_fill` is safe when the struct has no nullable numeric fields.
Otherwise, emit explicit `OpSetInt(i32::MIN)` / `OpSetFloat(NaN)` for
those fields after the zero-fill.

#### S7 — O8.5 compile-time resource exhaustion

Unrolling `[for i in 0..1000000 { i }]` at compile time produces a 4 MB
byte buffer and a 4 MB bytecode segment.  Without a size limit, an
adversarial program can exhaust compiler memory.

**Mitigation:** The design specifies a 10,000-element threshold.  This
should be enforced as a hard limit in the parser, not configurable.
Ranges above the limit silently fall back to runtime loops — no error,
no performance regression, just no optimisation.

#### S8 — Parallel execution

`OpInitVector` writes to a store via `keys::mut_store()`.  In parallel
`for` loops, each worker has its own store set.  The bulk init is safe
because store writes are worker-local.

If a parallel worker constructs a constant vector, the `OpInitVector`
runs on the worker's private store — same as the current per-element
path.  No new concurrency risk.

#### S9 — Native codegen: static data in generated Rust

O8.1 native codegen emits `static INIT_DATA: [u8; N] = [...]`.  Rust
statics are immutable and thread-safe.  The `copy_block_from_slice` call
copies from the static into the mutable store.

**Risk:** None — Rust's type system ensures the static is never mutated.

---

### Out of scope

| Pattern | Why |
|---|---|
| Sorted/index/hash bulk init | Insertion requires key ordering / hashing per element |
| Runtime-dependent comprehensions | Body depends on variables, function calls, or I/O |
| Mutable default sharing (copy-on-write) | Would require reference counting; complexity not justified |
| JIT compilation | Separate design; this document covers interpreter + native AOT only |
| Cross-function inlining for const eval | Calling `fn square(x: integer) -> integer { x*x }` is not const; only operator intrinsics are folded |

---

### Implementation order

| Phase | Item | Status | Effort | Impact |
|---|---|---|---|---|
| 0 | **`const_eval()`** | **Done** | Small | — |
| O8.1a | **Pre-allocate vector capacity** | **Done** | Small | Medium |
| O8.5 | **Constant range comprehensions** | **Done** | Medium | Medium |
| O8.1b | Packed bytes in bytecode | Not started | Medium | High |
| O8.3 | Zero-fill struct defaults | Not started | Small | Low-Medium |
| O8.2 | Bulk struct vectors | Not started | Medium | Medium |

#### Delivered

- **`const_eval()`** — 130-line module with 10 unit tests.  Folds
  arithmetic, casts, comparisons, boolean ops across all numeric types.
- **O8.1a** — `OpPreAllocVector(vec, capacity, elem_size)` eliminates
  all `store.resize()` calls for known-size vector literals.
- **O8.5** — `[for i in 0..N { expr(i) }]` unrolled at compile time when
  bounds and body are const-evaluable.  Filtered comprehensions also
  supported.  10,000-element safety limit.

#### Remaining

- **O8.1b** — embed packed constant bytes in bytecode for one-memcpy
  init.  Needs `Value::Bytes` IR variant and `State::code_ptr()`.
  Would reduce 3N → 1 ops (currently 3N+1 with pre-alloc).
- **O8.3** — `OpZeroFill` after `OpDatabase` to skip per-field zero
  writes.  Low-medium value since most fields are explicitly set.
- **O8.2** — pack numeric struct records for bulk init.  Needs
  `const_eval` on struct field values + field offset layout.

---

### LLVM overlap analysis

The native backend compiles generated Rust through `rustc` → LLVM.  With
`--native-release` (`-O`), LLVM applies constant folding, inlining, and
dead-code elimination.  This section evaluates which O8 optimisations
overlap with what LLVM already does, and which remain uniquely valuable.

#### What LLVM already optimises

**Arithmetic on literal arguments:**
The generated code emits `ops::op_mul_int(2_i32, 3_i32)`.  With `-O`,
LLVM inlines `op_mul_int` (it's `#[inline]`), sees both arguments are
constants, evaluates the null-sentinel checks (`v1 != i32::MIN`), folds
the arithmetic, and replaces the call with a constant `6_i32`.

This means `const_eval` for **simple arithmetic** (`2*3`, `4+1`) is
**redundant in the native-release path** — LLVM already does it.

**Dead branch elimination:**
`if true { 1 } else { 2 }` — LLVM eliminates the dead branch after
constant propagation.  `const_eval` for conditionals is also redundant
in native-release.

#### What LLVM cannot optimise

**Per-element vector construction:**
The generated code calls `OpNewRecord` / `OpFinishRecord` per element.
These are in the `codegen_runtime` module, compiled into `libloft.rlib`.
Without LTO, LLVM treats them as **opaque extern calls with side effects**.
Even with LTO, these functions contain:
- `vector_new()` → capacity check → possible `store.resize()`
- `vector_finish()` → length increment
- Bounds validation in `store.set_int()`

LLVM cannot:
- Batch 20 separate `store.set_int()` calls into one `memcpy`
- Pre-allocate the vector to the known final size (avoiding resizes)
- Eliminate per-element capacity checks
- Merge 20 `OpNewRecord`+`OpFinishRecord` pairs into a single allocation

**This is the core value of O8.1:** it replaces N opaque runtime calls
with one bulk allocation + one `memcpy`.  LLVM cannot derive this
transformation because it cannot see that 20 consecutive `OpNewRecord`
calls target the same vector with known-size elements.

**Comprehension unrolling (O8.5):**
The native codegen does NOT emit a Rust `for` loop for loft comprehensions.
It emits a loft-level loop with `OpStep`/`OpIterate` runtime calls.  LLVM
cannot unroll or eliminate these because they're opaque function calls with
mutable store references.

#### Summary per phase

| Phase | Interpreter value | Native-debug value | Native-release value |
|---|---|---|---|
| **`const_eval`** | High — reduces bytecodes | Medium — fewer runtime calls | **Low** — LLVM already folds arithmetic |
| **O8.1** bulk vectors | High — 1 vs 3N dispatches | High — 1 vs 3N calls | **High** — 1 memcpy vs 3N opaque calls |
| **O8.2** bulk struct vectors | High | High | **High** — same as O8.1 |
| **O8.3** zero-fill defaults | Medium — fewer opcodes | Medium — fewer calls | **Medium** — LLVM can't merge set_int calls |
| **O8.4** text table | Low — smaller bytecode | Low | **None** — text literals are Rust `&str` in native |
| **O8.5** const comprehensions | High — eliminates loop | High — eliminates loop | **High** — eliminates opaque loop |

#### Revised recommendations

1. **O8.1 (bulk vectors) is valuable across ALL backends.**  The
   per-element `OpNewRecord`/`OpFinishRecord` overhead cannot be
   eliminated by LLVM.  This is the highest-priority item.

2. **`const_eval` is still worthwhile** even though LLVM handles
   arithmetic, because:
   - It benefits the interpreter (the default execution mode)
   - It's the prerequisite for O8.1 detection (identifying which vectors
     are all-constant)
   - It enables O8.5 (comprehension unrolling) which LLVM cannot do
   - Cost is small (~80 lines of Rust)

3. **O8.4 (text table) has NO native value** — the native codegen emits
   Rust string literals (`"hello"`) which are deduplicated by the Rust
   compiler and linker automatically.  Only the interpreter benefits.
   **Deprioritise or drop.**

4. **O8.3 (zero-fill) has moderate native value** — even with `-O`, LLVM
   cannot merge multiple `stores.store_mut(&db).set_int(...)` calls into
   a `memset` because each goes through a bounds-checked method with a
   mutable borrow cycle.

5. **O8.5 (const comprehensions) has high native value** — the loop uses
   opaque runtime dispatch that LLVM cannot unroll or vectorise.

---

## Design: BUILD1 — Eliminate the lib/bin double compilation

**Symptom.**  `src/main.rs` declares `mod <name>;` for **41 modules
that `src/lib.rs` already declares as `pub mod`** — `platform`,
`database`, `state`, `parser`, `compile`, `generation`, … the bulk of
the crate.  Because loft is a dual `[lib]` + `[[bin]]` crate, every one
of those modules is compiled **twice**: once into `libloft.rlib`, and
again, from scratch, into the `loft` binary.  That is **~38,520 LOC of
single-file modules recompiled on every build** (plus the large
dir-modules `state/`, `parser/`, `database/`, which the LOC count
excludes — the true figure is higher).  Only **two** modules are
legitimately binary-only: `native_utils` and `test_runner`.

**Why it exists.**  `main.rs` already links the library (`use loft::…`
appears 26 times), so the inline `mod` declarations are redundant — the
code is reachable through the lib.  The duplicate `mod` set is an
accident of how `main.rs` grew, not a design requirement.

**Cost / impact.**
- **Build time:** the binary half recompiles ~the entire crate
  needlessly — roughly a 2× penalty on the bin's share of a clean
  build, and it defeats incremental reuse between `cargo build --lib`
  (e.g. `ensure_rlib_fresh` / the native rlib refresh) and
  `cargo build --bin loft`.
- **Dead-code noise:** the binary's *copy* of a module is linted
  against what the binary calls, not against the lib's public API.  A
  lib function that is real API (used by `tests/*.rs` via `loft::…`)
  shows up as `dead_code` in the binary view, forcing `#[allow(dead_code)]`
  annotations that wouldn't be needed under a single compilation.  The
  tmpfs-safeguard work (`platform::native_worker_count`, etc.) hit
  exactly this.

**Fix (the standard lib+bin split).**  Make `main.rs` a *thin* binary:
remove the 41 redundant `mod` lines, move any bin-only logic that other
code needs into the lib, and rewrite `main.rs`'s bare `foo::` paths to
`loft::foo::` (or `crate::` for the genuinely bin-only `native_utils` /
`test_runner`).  Each module then compiles once; the binary shrinks to
argument parsing + a call into a `loft::run()` entry point; dead-code is
judged only against the lib's public API.

**Effort / risk.**  M–L and crate-wide: 41 `mod` removals, every
affected path reference in a 3,500-line `main.rs` rewritten, and likely
a migration of `main.rs` logic into a new `lib::run` surface.  Touches
the whole module graph, so it wants its own focused change with a green
`make ci` gate — **not** to be folded into an unrelated fix.  Until it
lands, the binary-only dead-code warnings on shared `platform` helpers
are suppressed with `#[allow(dead_code)]` + a comment pointing here.

**Investigation checklist when this unpauses.**
1. Confirm the overlap set is current:
   `comm -12 <(grep -oE '^mod ([a-z_]+)' src/main.rs|awk '{print $2}'|sort) <(grep -oE '^(pub )?mod ([a-z_]+)' src/lib.rs|awk '{print $NF}'|sort)`.
2. Audit `main.rs` for items it defines that the lib needs (must move
   into the lib before the `mod` lines can be dropped).
3. Remove redundant `mod`s, fix paths, add `loft::run()` if main shrinks
   that far.
4. Measure: clean-build wall-time before/after; confirm the
   `#[allow(dead_code)]` suppressions added for BUILD1's sake can be
   removed.

---

## Design: BUILD2 — Persist the native-test binary cache across CI runs

**LANDED 2026-05-30.**  Implemented as designed: both cache-key functions
(`native_utils::native_cache_key`, `tests/native.rs::cache_key`) now fold the
rlib's CONTENT hash instead of its mtime (memoised once per process), and
ci.yml sets `LOFT_TMPDIR=${{ github.workspace }}/target/loft-native-cache` so
`actions/cache` persists the dir, plus a best-effort prune step caps growth.
Verified locally: run → no-op `cargo build --release --lib` (fresh rlib mtime,
identical bytes) → re-run reports `cached` (0.45s → 0.05s) where the old mtime
key forced a full recompile.  rustc rlib output confirmed byte-deterministic
across a touch-and-rebuild (identical sha256).  **Also a LOCAL speedup**, not
just CI: any local workflow that rebuilds the rlib (`make ci`, `make test`,
`cargo build --release --lib`) previously busted the native cache on the mtime
bump; now an unchanged-bytes rlib keeps the cache warm.  The CI `LOFT_TMPDIR`
relocation is the only CI-specific part (cross-*run* persistence — local /tmp
already persists within a machine).  Design notes preserved below.

**Symptom.**  The Windows CI leg is the long pole at ~25 min (Build
311s + Build-release 149s + Test 1010s).  A large slice of the Test step
is native compiles: `native_library_suite` + `native_scripts` + the two
`exit_codes` native cases + `codegen_emitter` + `p244` recompile **every
fixture to a native binary from scratch on every CI run** (~565s of CPU,
on all three OSes).

**Why the existing cache doesn't survive CI.**  An in-run binary cache
already exists — `compile_native_job` skips recompilation when a
content-hash sidecar (`{binary}.key`) matches (`binary_cache_valid` in
tests/native.rs; `native_cache_key` in src/native_utils.rs).  But the
binaries + sidecars are written to `scratch_dir()`, which defaults to
`std::env::temp_dir()`.  `actions/cache` (`.github/workflows/ci.yml`)
persists only `~/.cargo` + `target/`, so the temp dir is **never
cached** and the in-run cache starts empty every run.

**The blocker a naive "just relocate" misses — the key folds rlib
MTIME.**  Both cache-key functions mix in `libloft.rlib`'s *modification
time* (`native_utils::fold_mtime`; the inline mtime block in
tests/native.rs `cache_key`), and the `@P341` extension also folds each
native-PACKAGE rlib's mtime.  ci.yml runs `cargo build --release --lib`
**every run**, which rewrites `libloft.rlib` with a fresh mtime even on a
no-op rebuild.  Fresh mtime → every key misses → every binary recompiles
**even if the dir were cached**.  So relocation alone buys ~nothing.

**Fix — two parts (both required):**
1. **Relocate** the cache under `target/` so `actions/cache` persists it.
   The plumbing already exists: `scratch_dir()` honours `LOFT_TMPDIR`
   (added with the tmpfs safeguards), so this is just
   `LOFT_TMPDIR=target/loft-native-cache` in ci.yml — no code change.
2. **Content-hash the rlib instead of mtime** in BOTH key functions:
   hash the rlib *bytes* (or reuse `LOFT_BUILD_ID`) so an unchanged rlib
   (same bytes, new mtime) is a cache HIT.  This preserves the
   invalidation invariant the mtime was chosen for ("a recompiled rlib
   with a different binary must invalidate") — a different binary has
   different bytes → different hash — while being mtime-stable.  Likewise
   fold each native-package rlib by content, not mtime (keeps @P341's
   guarantee that a cdylib fix invalidates the cached test binary).

**Expected win.**  Warm runs with an unchanged rlib skip the native
recompiles outright — likely several minutes off the Test step, on all
three OSes.  Same tests, same assertions, same binaries — purely faster.

**Effort / risk.**  S–M test-infra change.  Risks to handle:
- **Cross-run correctness:** a persisted cache raises the stakes on key
  completeness — an in-run miss only wastes the current run; a persisted
  wrong key can mask a regression across runs.  The key must cover
  *everything the binary links* (generated Rust + loft rlib + every
  native-package rlib) by content before it's trusted across runs.
- **Parallel-safety:** the suite compiles fixtures concurrently into the
  shared dir; the content-hash sidecar already makes this safe in-run,
  but verify no races on the relocated path.
- **Cache growth / cleanup:** `target/loft-native-cache/` accumulates
  ~1MB (stripped) per fixture across rlib versions; cap it (LRU or
  wipe-on-rlib-change) so it doesn't bloat the `actions/cache` entry.
- **Hashing cost:** hashing a ~14MB rlib per cache check — measure; fold
  once per run (the rlib is constant within a run) rather than per
  fixture.

**Discovered** 2026-05-30 (CI long-pole analysis, after the tmpfs
safeguards landed).  Independent of BUILD1.

---

## See also — bytecode and store internals
- [Runtime optimisation audit](#runtime-optimisation-audit) — the opportunities register
- [INTERMEDIATE.md](INTERMEDIATE.md) — Bytecode layout and State stack model
- [DATABASE.md](DATABASE.md) — Store allocator and `copy_block` API

---

## Startup cache (shipped, default-on)

**Status: SHIPPED as part of @PLN11 G2 Track 1 (commit `77da481`).**

The whole-program startup cache skips ALL parsing — stdlib + lazily-`use`d
libs + user file — on warm runs by writing a binary bundle (content-addressed
`.store` + `.manifest`) to `$XDG_CACHE_HOME/loft/` (or `$HOME/.cache/loft/`).
Warm-run speedup measured at **~3–3.6×**.

### What is cached

The bundle contains the fully-parsed IR (`Data`) for the entire program prefix:
every `default/*.loft` file, every `use`-d library, and the user script.  It is
keyed on a drift manifest holding a SHA-256 of each parsed source's bytes
(`cache::file_hash`), so any source edit invalidates the cache automatically.

### Default-on behaviour and overrides

`cache::program_cache_enabled()` (`src/cache.rs`) implements the precedence
order:

1. `LOFT_NO_CACHE` (non-empty) → **off** — the explicit kill switch for
   production scripts that must never read/write bundles.
2. `LOFT_PROGRAM_CACHE` (non-empty) → **on** — explicit force; used by the
   cache's own tests to override the two dev defaults below.
3. `CARGO_MANIFEST_DIR` present → **off** — auto-disables inside
   `cargo run` / `cargo test`.  The compiler-debug loop and the entire
   integration-test suite never read/write bundles with zero per-test wiring.
4. **the binary lives in a `target/{debug,release}/` tree → off** — the OTHER
   half of the same compiler-debug loop, and the half that surprises people.
   Rule 3 only catches an invocation *Cargo* made; running `target/release/loft`
   by hand sets no variable, and that is what iterating on the compiler actually
   looks like.  Keyed on the binary's own path (`running_a_dev_build`), so it
   also covers `CARGO_TARGET_DIR=target-da`.
5. otherwise → **on** — the default for installed / real invocations.

### Which loft am I measuring? (rule 4 is a trap for benchmarks)

Rule 4 means **a binary you built from source pays the cold cost on every run,
forever** — by design, so a parser change is never answered by a stale bundle.  It
also means the two binaries on your machine have startup costs that differ by
roughly the warm/cold ratio below, and nothing in the output says which one you ran.

That is not hypothetical: loft#864 was filed as *"every invocation pays full
process-spawn + compile overhead, give me a warm session or a daemon"*, on numbers
measured with a from-source build.  The same programs on the installed binary ran
several times faster, because the cache the report needed was already there and
rule 4 had switched it off.  **Benchmark the installed binary, or say which one you
measured.**

| invocation | program cache | a rerun of an unchanged program |
|---|---|---|
| `loft prog.loft` (installed) | on | warm — no parsing at all |
| `target/release/loft prog.loft` | **off** (rule 4) | full stdlib parse, every run |
| `cargo run -- prog.loft` | off (rule 3) | full stdlib parse, every run |
| `LOFT_NO_CACHE=1 loft prog.loft` | off (rule 1) | full stdlib parse, every run |

**How to tell which you got, in one command.**  `LOFT_TIMING=1` prints
`parse_default=<n>ms`: a couple of ms is a warm bundle load, tens of ms is a cold
parse of `default/`.  It is the cheapest hit/miss check there is, and it works on
any build.

**`LOFT_TIMING=1` also reports the EXTERNAL build tools, with a reason each** (loft#1238).
A `--html` or `--native-wasm` build shells out to `cargo`, `rustc` and `wasm-opt`, and until
now the only channel was a per-event stderr line — right for a spawned process, useless for a
reader watching a 25-second `--html` and wanting to know what it is doing.  The report is
per-invocation, slowest first, and it names WHY each ran:

```
[loft-build] --html: 3 external invocation(s), 25.79s total
[loft-build]     25.61s  cargo     libloft.rlib (wasm32-unknown-unknown)  loft's own build fingerprint moved (loft was rebuilt) — rlib is stale
[loft-build]      0.18s  wasm-opt  prog_opt.wasm                          always runs — --asyncify is required for frame yield, not an optimisation
[loft-build]      0.00s  lock      html                                   waiting for the global build lock
```

**What it made visible.**  `loft_build_fingerprint()` is a CONTENT HASH of `libloft.rlib` /
the loft executable, so it moves on every loft rebuild — and it gates loft's own wasm runtime
rlib.  The first `--html` / `--native-wasm` run after ANY loft source change therefore pays a
full `cargo build` of that rlib (25.6 s here); the second is 0.55 s.  That is the price of the
guarantee that a codegen change reaches an already-built artifact, not a defect, but it was
invisible and it is why a wasm-shaped test can look like a hang.

**`loft cache warm` builds that set up front, and `make ci` runs it** (loft#1238).  The cost
above is unavoidable per artifact, but WHO pays it is not: left alone it lands on whichever test
reaches the artifact first, while the rest queue on the global build lock, and the charge shows
up as that test blowing a per-test deadline it has nothing to do with.  `loft cache warm --from
tests` builds them once, before the parallel section, with the same calls a test would make — so
the artifacts are stamped with the same key and every test afterwards takes the ordinary hit
path.  0.3–0.5 s once warm, and it never fails the caller: a package that cannot be warmed is
simply built by the test that needs it, exactly as before.

**Scope is the whole design of that command.**  The first version warmed every stale tree under
`~/.loft/build-cache/`, which on this box is 61 of 71 — generations belonging to OTHER loft
builds, none of which the run would touch — and was still going minutes later.  The set that
costs a test its budget is the INTERSECTION of two smaller ones: packages the corpus actually
`use`s, and trees already on disk under a previous key.  Neither alone is right — used-but-absent
is a first build no warming can avoid, and stale-but-unused is somebody else's cache.

⚠ **Warm with the RELEASE binary.**  The cdylib key is profile-independent (`BUILD_ID` is the
git-HEAD id, so debug/release/test share it), but `loft_build_fingerprint` — which gates the wasm
runtime rlib — is a content hash of the binary, so a debug-warmed rlib leaves a release-driven
test (`html_asyncify` runs `target/release/loft`) still cold.

**The lock is timed apart from the build on purpose.**  Under a parallel runner every
wasm-shaped process reaches `/tmp/loft-native-build.lock` at once after a loft rebuild, so a
single run's wall-clock can be almost entirely SOMEONE ELSE'S build.  Folding the two together
would name `cargo` for time cargo did not spend in this process.

Silent unless armed, and silent when nothing ran — a build that reused every artifact prints
no report at all.

**`LOFT_STDLIB_CACHE` is the narrower fallback, not an addition.**  It caches
`default/` only, and `main.rs` engages it **just when the program cache is off** —
so on an installed binary setting it changes nothing, while on a from-source build
it recovers most (not all) of what rule 4 gave up.  A measurable win from setting it
is therefore itself a signal that the program cache was disabled.

**`loft test` engages it too, and there it pays per FILE** (loft#925).  A suite
builds one parser per test file — deliberately, so one file's definitions cannot
leak into the next — and each of those parsers used to re-parse `default/` from
scratch.  For a directory of N test files that is N cold stdlib parses of a
directory that provably did not change between them.  `src/test_runner.rs` now asks
`warm_load_stdlib` first and falls through to the cold parse on a miss, exactly as
`main.rs` does; on a 21-file synthetic that is 2.33 s → 1.86 s.  The control matters
for reading that number: before the change, setting `LOFT_STDLIB_CACHE=1` on a
`loft test` run did **nothing at all**, because the runner never called the API.

### The shared library base (loft#925)

The larger half was NOT that cache: a `use`d LIBRARY was loaded from source once
per test file — *twice* per file, because both parse passes re-run the use region,
`Data::reset` having cleared the loaded-library map between them.

**Measured before the fix** (best of 3, idle box; the generator is on loft#925).  A
package of M modules behind an aggregator, N test files that all `use` it, beside a
control package whose N test files `use` nothing:

| N (M = 25) | `use` | control |   | M (N = 20) | per file |
|---|---|---|---|---|---|
| 1 | 0.10 s | 0.03 s |   | 10 modules | 0.039 s |
| 5 | 0.35 s | 0.13 s |   | 25 modules | 0.065 s |
| 10 | 0.66 s | 0.23 s |   | 50 modules | 0.122 s |
| 20 | 1.36 s | 0.45 s |   | | |

Dead linear in N with **zero amortization** — marginal 0.068 s/file against the
control's 0.022 s/file — and the per-file cost proportional to library size, so a
suite paid the **product** of the two.  That is what made it superlinear in project
growth: a new module slowed every test file, a new test file re-paid for every module.

**What it does now.**  Files are grouped by their leading `use` region *verbatim* —
the text, not an interpretation of it, so the parser stays the authority on what
those lines mean and a shared key is a shared library set by construction.  The
second file of a group triggers one parse of that region alone (`Parser::parse_as`,
the whole-program path fed a string instead of a file), and every file in the group
then starts from a copy of it: `Data` cloned, the schema re-installed, plus the
`use`-path and native-registration state a `use` would have produced.  `Data`
carries a `preloaded_uses` map that `reset` re-seeds, which is the one mechanism
that makes a `use` of an already-loaded library a no-op instead of a file read.

**After** (same box, same generator):

| N (M = 25) | before | after |   | M (N = 20) | before | after |
|---|---|---|---|---|---|---|
| 1 | 0.07 s | 0.06 s |   | 10 modules | 0.80 s | 0.23 s |
| 20 | 1.32 s | 0.43 s |   | 25 modules | 1.32 s | 0.43 s |
| 40 | 2.68 s | 0.74 s |   | 50 modules | 2.44 s | 0.81 s |

The marginal per-file cost drops from 0.068 s to 0.0155 s and stops tracking the
library — 3.0× at 25 modules, 3.6× at 40 files.  On the consumer that reported it
(dryopea: 81 test files, 1161 tests, one group) the suite went **238 s → 209 s**,
i.e. the ~31 s the issue predicted, with byte-identical output.

Three decisions are load-bearing and easy to undo by accident:

- **A group of one gets no base.**  The base is built when a SECOND file asks for
  the same region; before that the file parses exactly as it always did.  Building
  eagerly doubled `loft test <one-file>` (0.07 s → 0.13 s) — the tight inner loop of
  development, and a group of one by definition.
- **A seeded file skips the stdlib warm load.**  The base already holds the stdlib,
  so loading the bundle per file only decoded it for `seed_from` to discard —
  that redundant decode was most of what a seeded file still paid (it is the
  difference between the 3.0× above and the 1.5× without it).
- **A leading `#cwd` is part of the region, not a reason to refuse one.**  All 81
  of dryopea's test files open with it; a scanner that gave up there measured
  perfectly on the synthetic and saved the reporting consumer nothing.

`LOFT_NO_TEST_BASE=1` turns the sharing off — the control half of an A/B on one
binary, and what the equivalence guard in `tests/test_base_equivalence.rs` compares
against.  `LOFT_TEST_BASE_REPORT=1` says on stderr which regions got a shared base,
which is how that guard knows it is not comparing a run to itself.

Not covered, and still open: repeated `loft test <one-file>` invocations, which
need a keyed on-disk bundle whose key covers every library source plus the resolved
dependency graph (loft#930 is the reminder of what an incomplete key costs).

Two things had made this un-reproducible outside the reporting consumer, both worth
knowing when cutting a suite-shaped benchmark: `loft test` refuses multiple file
arguments (loft#916 — the suite form is a DIRECTORY, which only became nameable when
`resolve_test_target` stopped appending `.loft` to one), and a package needs
`[library] entry = …` or its own `src/` is not a library its tests can `use`.

### Invalidation

`build_signature()` folds together:

- The cache-format version constant, loft version, and git HEAD (`BUILD_ID`).
- The running binary's mtime (`binary_signature_tag()`) so an *uncommitted*
  compiler rebuild invalidates bundles.  `BUILD_ID` (git HEAD) alone does not
  change across uncommitted edits; the mtime addition closes that gap.

### Eviction

`cache::prune_program_cache()` is called after each cold save.  It evicts the
oldest `(.store + .manifest)` pairs until the cache directory is under
`LOFT_CACHE_MAX_MB` (default **512 MiB**).

### See also — the startup-cache design

Full design, E1/E2/E3 arc, and the zero-copy follow-up: see
[`plans/11-data-as-store/README.md`](plans/11-data-as-store/README.md).

---

## Open work

The 9 design entries above (P1, P2, P3, N1, N2, N3, N4, N5, W1)
are all open optimization items.  Tracking table for ROADMAP and
plan-cleanup audits:

| Item | Section | ROADMAP row | Tier | Status |
|---|---|---|---|---|
| **P1** — Superinstruction merging | § Design: P1 | O1 | Interpreter | Open — **unblocked** (corrected 2026-06): the two-byte escape (byte 255 → `OPERATORS[255+ext]`) gives ~242 free slots; superinstructions are escape-range ops. **Secondary** value — the debug loop runs interpreted, but bounded (library calls stay native via C71/N9; players run native), so it speeds only the maker's own glue. Worth doing because it is now cheap + low-risk. |
| **P2** — Reduce store indirection on the stack | § Design: P2 | (cited in PLANNING.md) | Interpreter **+ Native** | **Native half SHIPPED 2026-08-15 (loft#885)** — a loop the emitter proves writes no store derives each vector's `(store_nr, record, length)` ONCE before the loop, and each element read becomes a bounds test plus address arithmetic.  Alternating single runs of ONE binary (`LOFT_NO_VECTOR_HOIST=1` is the before-half) on the issue's kernel: median **18.6 → 8.3 ns/iter**, distributions not overlapping, identical accumulator on all twelve runs — **~2.25×**, taking `vector<single>` indexed reads from ~15× hand-written Rust to ~6.5×.  The full write-up — the allow-list gate and why it is an allow-list, the `attributes()`-vs-`variables()` parameter hole that made the first cut classify every mutator as a reader, and the `LOFT_HOIST_VERIFY=1` / `LOFT_NO_VECTOR_HOIST=1` switches — is in § Design: P2 → *Shipped: the NATIVE half*.  **Still open:** fusing the element read into the address computation (the `DbRef` → `stores.store(&db)` → `get_single` chain), measured at a further ~1.4× and needing the gate that now exists; and the INTERPRETER half below (`stack_base` raw-pointer cache), which is untouched and still low-priority.  The sibling cost in the same report — `??`'s NaN sentinel — measured at ~0, so a non-nullable indexing form buys nothing on its own. |
| **P3** — Confirm integer paths carry no `long` sentinel | § Design: P3 | — | Interpreter | Open — small verification + audit task; verifies the Plan-01 `i32::MIN`-removal stuck. |
| **P4** — Block-copy slice materialisation for primitive vectors | § Design: P4 | — | Interpreter + Native | Open — discovered alongside @P287 (2026-05-20).  Today's slice → vector materialisation is element-by-element through the record allocator (5 000+ dispatches for 1 000 i32 elements); a new `OpAppendVectorSlice` op + parser fast-path reduces this to one `copy_block`.  Affects both backends. |
| **P5** — `vector +=` capacity reservation (amortised growth) | — | — | Interpreter + Native | **LANDED 2026-05-21.**  Discovered via the `store_memory()` builtin while profiling the @PLN6 crystal mesh: single-element `+= [x]` reallocated the backing record on (nearly) every append, fragmenting the store into O(N) freed records (a 12 738-element build → **101 815 free blocks / ~250 MB** vs ~0.8 MB of data).  Fixed by amortised (~×2) growth in `vector::vector_append` (`src/vector.rs`): when the backing record is out of room it grows to ~2× `length+1` instead of exactly `length+1`; `Store::resize` is grow-only so in-room appends are no-ops.  Length lives in a separate field (word 1), so the trailing slack never affects `len()`/indexing/copy (length-based, shrinks to fit)/serialisation.  One shared function → both backends.  Same 12 000-element build now shows **~8 free blocks** (one trailing slack block).  Guards: `tests/scripts/124-vector-amortised-growth.loft` (cross-mode correctness + fragmentation digit-count bound).  `vector_set_size` (bulk `+= [a,b,c]`) and `insert_vector` keep exact sizing (not the hot path); a user-facing `reserve(v, n)` (wrapping the existing `OpPreAllocVector`) remains an optional opt-in follow-up. |
| **P6** — Free-block coalescing in `Store` (merge mergeable neighbours) | — | — | Interpreter + Native | **LANDED 2026-05-21.**  Partner of P5.  `Store::delete` coalesces only FORWARD (`rec + size`) — the header-only block layout has no footer, so a freed block can't find its PREDECESSOR; freeing B while A is already free left A|B uncoalesced, and accumulated small free blocks forced the store to GROW instead of reusing freed space.  **Fixed with NO new index** (no end→start map, no boundary-tag footer — those grow with the free-block count): a lazy `coalesce_free` sweep walks the contiguous block chain (the adjacency info that already exists — the same walk `claim_scan`/`usage`/`fl_rebuild` do), merges every run of adjacent free blocks in place, and rebuilds the ONE existing size-keyed free tree via `fl_rebuild`.  It runs in `claim` only when a best-fit miss would otherwise grow the store (that path already costs O(n) via `resize_store`), guarded by a single `needs_coalesce` bool so alloc-only workloads never sweep.  Backward coalescing falls out of the address-order walk (A+B merge regardless of free order).  Shared `Store` → both backends.  Guards: Rust unit test `coalesce_free_merges_adjacent_and_reuses_space` (`src/store.rs` — mergeable-pairs → 0, merged block reused without growing), cross-mode `tests/scripts/126-store-coalesce.loft`. |
| **P7** — `reserve(v, n)` / `shrink_to_fit(v)` vector builtins | — | — | Interpreter + Native | Open — the deferred P5 follow-up.  P5's amortised ×2 growth leaves a vector at up to ~2× capacity (builds run ~57–73 % utilised).  Expose `reserve(v, n)` (wrapping the existing `OpPreAllocVector` / `vector::pre_alloc_vector`) so a known-size build claims exactly once, and/or `shrink_to_fit(v)` (a length-based re-claim — the deep-copy path `copy_claims_seq_vector` already does this internally) to reclaim slack after building.  Opt-in, benefits every vector consumer; drives @PLN6 mesh memory-reduction item **M2** (`plans/6-audience-generative-art/03-projector-view.md`).  Effort S–M. |
| **N1** — Direct-emit local collections in native codegen | § Design: N1 | **O4** | Native | Open — design ready.  Cooperates with `lib_plans/59-lazy-stdlib/`.  (`plans/finished/21-retire-scratch/` shipped, so the scratch consumer set is now zero — N1 is independent.) |
| **N2** — Omit `stores` parameter from pure native fns | § Design: N2 | **O5** | Native | Open — purity analysis already built (`def.purity`, scopes.rs); only the stores-less codegen remains (audited 2026-06-25 → Cost High→Medium). |
| **N3** — Remove `long` null-sentinel from generated code | § Design: N3 | — | Native | Open — the 6 `_nn` long ops exist in `ops.rs` but are dead/unwired; needs the non-null classifier + emit (audited 2026-06-25). |
| **N4** — Suppress `cr_call_push` on `#pure` leaf functions | § Design: N4 | — | Native | Open — discovered while optimising @PLN42 scan.loft (2026-05-18).  Small change; reuses N2's purity classifier with a leaf check. |
| **N5** — Inline `integer` arithmetic when operands are non-null | § Design: N5 | — | Native | Open — discovered alongside N4 (2026-05-18) while inspecting `--native-emit` output for scan.loft's byte loop.  Folds into N3 (same nullability classifier, parallel application to `integer`). |
| **W1** — wasm string representation | § Design: W1 | — | WASM | Open — design ready, scheduled for wasm-priority workloads (game-client + browser-IDE consumers). |
| **BUILD1** — Eliminate lib/bin double compilation | § Design: BUILD1 | — | Build-time | Open — discovered 2026-05-30 wiring the tmpfs safeguards.  `main.rs` re-declares 41 modules already in `lib.rs` (~38,520 LOC of single-file modules) → whole crate compiled twice (rlib + bin).  Fix = thin-binary lib/bin split.  M–L, crate-wide; needs its own change.  Until then, shared `platform` helpers used only by `tests/` carry `#[allow(dead_code)]` in the binary view. |
| **BUILD2** — Persist native-test binary cache across CI runs | § Design: BUILD2 | — | Build-time (CI + local) | **LANDED 2026-05-30.**  Both cache keys fold rlib CONTENT hash (was mtime) + ci.yml sets `LOFT_TMPDIR=target/loft-native-cache` (persisted by actions/cache) + prune step.  Warm runs skip ~565s/OS of native recompiles; also a local speedup (rlib rebuild no longer busts the native cache on the mtime bump).  NB: `libraries4` `f73e58a0 @P334` fixes the world.loft wasip2 stub at the source — supersedes the harness preopen workaround when it merges. |
| **E2 (zero-copy `read_data`)** — `read_data` profiling breakdown | `§ Startup cache` above | — | Startup / parse | Open but **deprioritised (2026-06-04): the native-library execution model supersedes E2 as the startup-perf lever** — in that model library bodies + variable tables are never materialised at all (libraries are native artifacts loaded via `dlopen`), so the allocation cost E2 eliminates is simply not incurred.  See [DESIGN_DECISIONS.md § C71](DESIGN_DECISIONS.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model) and [BROADENING.md § Native-library execution model](BROADENING.md#native-library-execution-model--the-steady-state-design).  Measurement record: `bench_read_data_breakdown` (`src/ir_read.rs`, `#[ignore]`) on the real stdlib bundle: full `read_data` = **693 µs** = def-fields **453 µs (65%)** + variable tables **98 µs (14%)** + body trees **142 µs (20%)**.  Variable-table decode is **~0.39 µs/variable** (linear in allocation count); each variable rebuilds a `String` name + a boxed `Type` — exactly what E2 eliminates.  No cheap `read_function` optimisation exists; E2 is the only lever **if** this cost matters.  E2 startup prize: **~0.7 ms on the stdlib** (scales with def + var count).  Full arc: `plans/11-data-as-store/README.md`. |

Other ROADMAP rows that conceptually belong here but lack
PERFORMANCE.md design content yet — A12 (lazy work-variable
init), O2 (stack raw pointer cache), A4 (spatial index ops).
Each stays as a PLANNING.md-cited row until design content
lands here.

Suggested order when this work unpauses:
1. **P3 + N3 + N5** — small verification/cleanup; clears the deck.
   N3 + N5 ship together (shared nullability classifier).
2. **N2 + N4** — both touch the purity classifier; N4 reuses it for
   the leaf-pure check and is similarly small.  Ship in sequence.
3. **N1** — direct-emit local collections; cooperates with plan 21.
4. **P1** — biggest interpreter win.  BLOCKED on opcode-table decision.
5. **P2** — store indirection reduction; smaller than P1, architecturally cleaner.
6. **W1** — wasm string representation when wasm becomes priority.

Items are independent — order can shift based on which consumer
(interpreter / native / wasm) needs the win first.

### Validation methodology — single-file Rust-emit harness

N4 and N5 were discovered by emitting `--native-emit` output to a
standalone `.rs` file and compiling it with `rustc --edition=2024
-O --extern loft=…/libloft.rlib -L target/release/deps`.  This
isolates each codegen variant (commenting out `cr_call_push`,
replacing `ops::op_add_int(a, b)` with `a + b`, etc.) and times
the resulting binary directly.  No build-system or loft-compiler
edit needed per variant.  Recommended for validating any future
codegen-side change before landing it in `src/generation/`.

The same harness surfaced the `--native` vs `--native-release` gap
(`-O` missing from the default mode) — a 10× wall-clock difference
that was not a codegen issue at all.  That fix shipped in commit
`ae34bdb1` (Makefile: `make index` uses `--native-release`).
Other consumers of bare `--native` for runtime-heavy work likely
have similar headroom; this is a CLI-UX question, not a codegen
follow-up, so it is not tracked here.
