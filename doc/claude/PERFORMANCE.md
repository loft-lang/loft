
# Performance

How to measure loft's speed, where the current numbers are, and which optimisations are still
open.  The record of past measurements and designs is [PERFORMANCE-history.md](PERFORMANCE-history.md).

**The speed contract itself is formal:** [formal/performance.md](formal/performance.md) —
`(Perf-Like)` a comparison is admissible only between lanes proven output-hash-equal;
`(Perf-Weight)` every shipped routine (stdlib + libraries) is measured per release against
an industry-language reference twin, because drift against a previous loft release compares
to nothing outside the project; `(Perf-Twin)` where no natural counterpart exists a twin is
WRITTEN the moment a hit is expected, never waived; `(Perf-Cure)` the twin MEASURES and
never ships — a failed bar is closed in the engine (or the loft algorithm), because the
libraries stay readable loft (the teaching corpus), and a native rewrite is a recorded
per-routine edge case, not a habit; `(Perf-Gap)` an advantage the twin holds by a
construction loft cannot express is a MISSING ABSTRACTION in loft — routed to the language,
never a twin rewritten down or a row excused, because loft's standard implementation
optimises one way of working without excluding the others.  `(Perf-Teach)` a loft rewrite for speed is the last resort after the
code generation, and only behind an `advice` that names the faster API to a user.  The model harness is the
drawing library's `bench/` (loft#1426); @PLN158 generalizes it into the per-library standard
read by the release checklist's `M-perf-pass`.

## Contents
- [Where the numbers are](#where-the-numbers-are)
- [Profiling a run](#profiling-a-run)
- [Measuring native code](#measuring-native-code)
- [How the interpreter executes](#how-the-interpreter-executes)
- [Front-end speed — how it is measured and guarded](#front-end-speed--how-it-is-measured-and-guarded)
- [Open performance work](#open-performance-work)
- The startup cache — what is cached, how to turn it off, which loft a benchmark measures:
  [STARTUP_CACHE.md](STARTUP_CACHE.md)

---

## Where the numbers are

A number written into prose is stale the moment it is committed, so this doc carries none.
Ask the tool that measures:

| question | command |
|---|---|
| where native stands against Rust, by mechanism class | `make perf-portal` → [PERF_PORTAL.md](PERF_PORTAL.md) (generated) |
| native vs the Rust reference per routine, as statistics | `python3 bench/stats.py` ([bench/README.md](../../bench/README.md)) |
| what got slower or faster than on `main` | `make speed` (a report); `make speed-gate` is the one speed gate |
| the classic benchmark suite, every lane | `bench/run_bench.sh` |
| which fn, line or path burns the time | `make profile` — § Profiling a run |

The rendered benchmark page for users is [doc/00-performance.html](../00-performance.html).

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
names the missing flag).  The report has two tables: self time by SYMBOL, and the same
samples by MODULE — every symbol folded onto the loft module it belongs to, the runtime
below loft onto `allocator` / `mem` / `hashing` / `vec/string`.  Read a compile off the
second: its top symbol is 5–8 % and ties with the allocator's, so the symbol table names
a coin toss, while a module's share is the sum over all of its symbols and holds run to
run (a compile of the front-end corpus: `data` 32–34 %, the allocator 20–21 %).  The
oracle row for it is `frontend_large` in [PROFILE_ORACLE.md](PROFILE_ORACLE.md) § Engine.

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

### Attributing a bench ROW — `--names` on the native build (@PLN158)

`perf` names `n_<yours>` only while the function still exists as a symbol.  A shipped
build (`--native-release`) is lean and fully optimised, and rustc INLINES a small routine
into the loop that calls it — a bench lane's `main` — so `perf annotate` shows the lane's
float math with no way to say which row's it is.  Two rows of the consumer lane
(`mesh_aabb`, `enum_match`) were measured unattributable that way (`round-3.md`), and the
whole record class sat behind them.

`loft --native-release --names` is the instrument: every generated loft function carries
`#[inline(never)]` (loft#954's attribute, which the browser build uses so a trap's frames
resolve), so each row has a symbol and `perf` attributes its samples to it.  A MEASUREMENT
build, never one that ships — the attribute costs exactly the inlining it names.  The
recipe, on one core:

```bash
loft --native-release --names --native-emit /tmp/lane.rs bench/16_consumer_shapes/bench.loft
bench/portal/hand_price.sh /tmp/lane.rs /tmp/lane          # the shipped flags
perf record -F 15000 -e cycles:u -o /tmp/lane.data -- taskset -c 0 /tmp/lane --n 200
perf report -i /tmp/lane.data --stdio --sort symbol | grep n_c_mesh_aabb   # the row's share
perf annotate -i /tmp/lane.data --stdio -s consumer::n_c_mesh_aabb__inv    # its instructions
```

Read the row's TWIN when one exists (`n_<row>__inv`, `@FR-R-Callee`): the lane calls the
twin, and the plain function carries no samples.  Check first that the named build times
the row the same as the plain one (it did, both rows, to 0.3 %) — an attribute that moved
the row would be measuring something else.  What it found on its first use: `mesh_aabb`'s
5.3 ns a vertex was six null-aware float compares spelled as four NaN tests and a
short-circuit each, which LLVM cannot make branchless; respelled as one null test over
operands bound first (`OpLtFloat`'s template), the row went 38.7 → 20.9 µs, 4.81× → 2.73×.

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

Sampling a user process needs `perf_event_paranoid <= 2`; the script refuses with the exact
command when it is higher.

```bash
echo 'kernel.perf_event_paranoid = 2' | sudo tee /etc/sysctl.d/99-perf.conf
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

### `LOFT_NATIVE_CHECKPOINTS` — when the sampler cannot reach (the second instrument)

**This is not the normal way to profile.** `scripts/profile.sh` is, and it perturbs the
program not at all. Reach for checkpoints only where that route is closed: a stripped
release binary, a machine with no `perf` (every macOS box), or **wasm**, where there is no
sampler of any kind.

The generator writes a probe into the emitted Rust at the one call/op chokepoint
(`output_call_inner`), so every OPERATOR the program executes is counted at its own loft
`file:line`. Two tiers:

```bash
LOFT_NATIVE_CHECKPOINTS=count loft --native-release p.loft   # counts; every target
LOFT_NATIVE_CHECKPOINTS=time  loft --native-release p.loft   # counts + inclusive ticks
```

The report prints the hot operator sites and then a **by-function rollup**, which is
exclusive by construction — a user CALL is deliberately not a site, because timing it
would make its ticks inclusive of its callees and one table would then mix inclusive and
exclusive rows.

**Why counts are the portable tier, and ticks are not.** Measured on an Apple M-series:
reading the cycle counter (`mrs cntvct_el0`) costs **0.72 ns** — cheaper than
the `AtomicU64` increment at 1.43 ns, and 20× cheaper than `Instant::now()` at 14.7 ns, so
the instrument uses the instruction and never `Instant`. But the counter only *advances*
every **~41.7 ns**: 196 728 of 200 000 back-to-back reads returned the same value, while an
operator runs in ~1–3 ns. No single operator's duration is measurable. The tick column
survives at all only because the counter is asynchronous to the code, so the quantised
deltas dither and their SUM converges — which is why every row publishes its execution
count beside its ticks. **A row with few executions is noise wearing a number's clothes**;
read the count first.

⚠ **And read the tick column as a HINT, never as a time profile — it is biased toward
operator-dense functions.** Validated against the pure-Rust reference, instrumented the
same way (`bench/bench.rs` with an exclusive self-time guard per function, same clock, the
same `lock_curved` workload — both produce `sink 122400`, so the work is identical):

| function | rust self% | loft ticks% | loft calls% |
|---|---:|---:|---:|
| `lock_layer` | **30.30** | 14.96 | 24.99 |
| `brush_sample` | **28.16** | 21.58 | 9.16 |
| `raster_segment` | **22.68** | **49.24** | 52.54 |
| `chan` | 10.34 | 3.30 | 6.37 |

The two largest rows disagree by about 2×, in opposite directions, for two reasons that are
both real. The probe is charged per OPERATOR, so a function with many cheap operators is
inflated: `raster_segment`'s tick share (49.2) sits on top of its call share (52.5), which
means that row's ticks carry almost nothing the counts did not already say. And the
reference's own guards cost it 4× (0.12 s → 0.48 s) by stopping `chan`, `clampf` and
`clampi` being inlined, so ~15 % of its self-time is time that belongs to their callers in
the real build. **Neither instrumented distribution is the truth.** The ticks are not
noise — normalised cost per operator varies 5.5× across functions (15.7 → 86.2 ticks% per
G-execution) and both instruments agree `brush_sample` is expensive per operation — but a
tick share must not be quoted as "where the time goes".

**The counts, by contrast, validate exactly.** Against the reference's call counts they
give whole-number operators per call, each matching what the loft source says: `chan`
**3.00** (`((c >> sh) & 255) as float` — shift, and, conv), `color_g` **2.00**, `ramp`
**9.00**, with `clampi` 1.99 and `floor_i` 3.05 where a branch varies the body. That is the
column to build an argument on. Counts need no clock at all, which is why they behave identically on
native, wasip2 and in a browser, where `performance.now()` is deliberately clamped as a
Spectre mitigation. On a target with no userspace counter the report says so rather than
printing zeros that read as "this operator took no time".

**Instrument less with a scope filter.** `mode:<filter>` instruments only the operators
whose enclosing loft function or source file contains `<filter>`, so a second pass over one
subsystem costs a fraction of the first pass over everything:

```bash
LOFT_NATIVE_CHECKPOINTS=time:raster_segment loft --native-release p.loft   # one function
LOFT_NATIVE_CHECKPOINTS=count:brush.loft    loft --native-release p.loft   # one module
```

Measured on `lock_curved`: **1155 sites → 126**, and the run drops from 4.98 s to 3.32 s
(overhead 5.4× → 3.6×). Note what that ratio does *not* track — site count fell by 89 % and
cost by only a third, because the filtered function alone is 52 % of the program's operator
EXECUTIONS. The filter is paid for in executions, not in sites, so filter to the hot thing
and expect a modest saving; filter to a cold one and the run is nearly free. The second
benefit is accuracy: instrumenting less perturbs rustc's inlining less, so a filtered run's
shares sit closer to the shipped build's.

**What it costs, and what that means.** Measured on `lock_curved` (400 iterations, 3.3 G
operator executions): plain 0.93 s, `count` **2.75 s (3.0×)**, `time` **4.98 s (5.4×)**.
More important than the slowdown is what the probe does to rustc: an operator wrapped in a
macro is still inline, but the wrapping **changes what may be inlined across it**, so the
distribution you read is the instrumented program's, not the shipped one's. Use it to find
*which* code runs and roughly where the time concentrates; confirm a ratio with
`compare.py` or `profile.sh`.

That difference is not theoretical, and it is what the instrument is FOR. `lock_curved`'s
`perf` profile read `n_lock_layer` at 77.6 % self and concluded the one target
was `brush_sample`'s record return — which @PLN157 § V-aa then closed, for −3 %. The
checkpoint rollup on the same row says why the prize was small:

```
 calls%  ticks%     executions  function
 52.53%  49.31%     1731264000  n_raster_segment
  9.15%  21.56%      301695600  n_brush_sample
 25.00%  14.93%      823778800  n_lock_layer
  3.41%   9.70%      112471200  n_ramp
```

`raster_segment` is **52 % of the program's operator executions** — 1.73 of 3.30 billion, at
**52 147 operators per call**, against a reference call the plain Rust runs in ~1.2 µs. Note
which column that claim rests on: the validated COUNTS, not the ticks. The release build
inlines `raster_segment` into `lock_layer`, so `perf` credits the caller and cannot see it,
while a checkpoint is keyed to the loft function the operator was WRITTEN in. That is the
one question a sampler on optimised code cannot answer, and the reason to keep this
instrument beside the normal one.

### `scripts/native_attrib.py` — the release binary, charged to loft lines

The third instrument answers what the two above cannot answer together: **on the build that
ships, which loft LINE drives the runtime's cycles?**  A self-time profile of a release
binary names runtime routines (`OpDatabase`, `vector_add`) but not the line that called them,
and it counts a runtime helper inlined into a program function as program.  This tool reads
`perf record --call-graph fp`, expands every frame to its inline chain with
`llvm-symbolizer`, and charges each sample to the innermost frame of the emitted program —
whose `// loft:<file>:<line>` comment names the line.  It prints the runtime share by family
of the entry the program called, by loft function, and by loft line.

It needs frame pointers and line tables in the program AND in the runtime rlib it links, or
every chain stops at the runtime boundary; the build recipe is in the script's header
(`cargo build --profile profiling --lib` with `-Cforce-frame-pointers=yes`, then the release
rustc line with `-Cdebuginfo=1 -Cforce-frame-pointers=yes`).  Frame pointers cost a few per
cent, so quote SHARES from this build and TIMES from the release one.  The worked example is
@PLN164 § P0b: it moved the drawing `parse` row's store family from 9 % (self time) to 20 %
(entry-inclusive) and found half of it minted at function entry on paths that never used it.

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

### The clear case first

Pick the case that shows up in the statistics AND has a visible reason, and build that one.
A large routine always has many small spots that could be better; finding them all is hard,
they may be needed later, and the effort on the first, clear cases is rewarded far better.
So rank candidates by the profile share TIMES a mechanism you can name and falsify — one
routine at 57–82 % of three rows whose taps pay a re-tested flag each, one site that
deep-copies a record at its first bind — build it, re-profile, and rank again.  A profile
share is what a symbol costs, not what its removal saves, so the second half of the product
is not optional.  @PLN157 § V-an measured both sides in one evening: the value form's one
declined shape (≈194 record mints per `parse` call) moved the row, and a field-list clone
at 1–2 %, taken only because the code was already open, did not.  When a row's profile is
FLAT — nothing over about 5 % — that is the finding to record: the row is bound by a class
(DESIGN.md § V-an's table for `parse`), and the unit is the class, not a spot.

### Wide before deep

**The owner's direction for the perf stream (2026-09-23): WIDE, not deep.** Optimisation stays
— it is the usability measure game developers judge a language by — but it is aimed at the
breadth of what programs do, not at the next increment on a row that is already measured.
Before any existing portal row is taken deeper, every mechanism class gets a measured row per
library from [PERF_PORTAL.md](PERF_PORTAL.md)'s waiting table (the routines surveyed and not
yet benched), so the portal says where native stands across the classes a consumer will hit:
keyed, `par`, the call frame and the native (C) boundary before another record or text clause.
"The clear case first" still ranks candidates WITHIN that order.

**Prefer runtime work over new generator rewrites.** A runtime lever — a store's allocator
(`@FR-H-Carve`), the keyed class's fast order and one-probe insert — changes no emitted
program, lands behind one switch and one verify form (`LOFT_KEYED_VERIFY` is the model), and
adds almost no rules.  A generator rewrite adds a `formal/rewrites.md` clause, a switch, pins
and a cell corpus each time, and only rewrites add to the contract's rule count and to the
strained trailers the stability meters read.  Choose the rewrite when the runtime cannot reach
the cost, and say so in the ledger.

**A rewrite shared by BOTH backends lands only with its switch A/B green.** For the
ownership / buffer family — `@FR-O-LazyBuffer`, `@FR-O-Move`'s adopt, `@FR-O-Buffer`'s reuse,
`@FR-R-Place`, `@FR-R-InPlaceLiteral`, `@FR-R-ValueRecord` — there is no in-process checking
form: `LOFT_STRICT_STORES` and `LOFT_POISON` catch a leak or a double free, not a stale VALUE,
and the interpreter runs the same rewrite, so it is no oracle.  loft#1647 is exactly that
hole: a lazily minted buffer met a latent double release and read the first pass's value on
both backends, six days on main, seen by nothing in this suite.  The gate that sees it is the
switch A/B nightly (`.github/workflows/switch-ab.yml`): the script corpus and the consumer
suites run twice per rewrite switch, off and on, and any output difference is a defect by
definition — C122, a rewrite is free only where its conditions hold.  The landing rule is in
[formal/rewrites.md § Every rewrite is switchable and
falsifiable](formal/rewrites.md#every-rewrite-is-switchable-and-falsifiable).

## Trends, and the check before a commit

The portal (`make perf-portal`) writes ONE row per routine into
`bench/portal/results/<host>.tsv` — the machine's latest measurement — and the page reads
that row.  The history is the file's git history: every commit that touched it is one
measurement of the whole lane set on that machine, and a routine that got slower between
two joins is visible nowhere else (the census reads admissions, the speed gate reads test
times).  Two scripts read it:

- **`make perf-trend`** (`scripts/perf_trend.py`) walks that history and prints the MOVERS
  — every routine whose latest measurement differs from the one before by 15 % or more, with
  the two loft commits it sits between (the bisect range) — or, with `ARGS="--routine
  <name>"`, one routine's whole series.  Ratios are compared within one machine only.  Read
  when a row looks wrong: `ease` +41 % between the 24 and 27 September measurements was
  found this way, after the fact.
- **`make perf-check`** (`scripts/perf_check.py`) is the same comparison once per arc, in the
  background (CI_BUDGET.md § What a compiler change needs before its commit — a commit itself
  needs only the matrix, the pins, clippy and one targeted A/B on its own row):
  it measures the lanes the change touched — by default the programs whose rewrite
  admissions the census says moved, or `ARGS="--only 16,17"` / `--package DIR=NAME` — and
  compares each routine's ratio to Rust with this machine's last committed row, listing
  every move of 15 % or more and exiting 1 on a slowdown.  `--record` makes the run the
  machine's baseline (the results file), which is committed with the change, so the trend
  history grows one point per landed compiler change instead of one per portal run.

Both are REPORTS.  A routine on a 200 ns twin swings 10 % between two runs (`bench/stats.py`
flags those rows noisy and coarse), so a move is one that repeats; and the density of the
history is the density of the commits, which is why `--record` exists.

---

## Measuring native code

> ⚠ **A codegen A/B switch cannot measure code inside a `use`d LIBRARY.** Every switch in the
> hoist family (`LOFT_NO_SCALAR_HOIST`, `LOFT_NO_VECTOR_HOIST`, `LOFT_NO_PUSH_HOIST`, …) is read
> at GENERATION time, and a library runs as the cdylib loft built for it once and cached under
> the package's `native-auto/`. Setting the switch on the consumer's run regenerates the
> PROGRAM and leaves the library's machine code exactly as it was, so the A/B reads as a flat
> no-difference and looks like "the optimisation does not matter here".
>
> Measured 2026-09-10 while attributing loft#1426's `smooth` row: baseline and
> `LOFT_NO_SCALAR_HOIST=1` timed 394 ms and 393 ms on a 20 000-call probe whose whole hot loop
> is inside `drawing`. Both numbers were also dominated by the rustc compile the run pays, which
> is the second way that shape of timing misleads.
>
> To A/B a library's own codegen, the cdylib has to be rebuilt under the switch — clear
> `native-auto/` (in a SCRATCH COPY of the package, never the consumer's tree) or run the lane
> with `LOFT_NO_NATIVE_LIBS=1`, which interprets the library and measures a different thing
> again. Attribute inside a library with `LOFT_PROFILE` under `LOFT_NO_NATIVE_LIBS=1` — the
> sampler cannot enter a cdylib at all, so without it the library's time lands on the calling
> line and a library doing the work reads as a hot caller.

### Validating a codegen change — the single-file Rust-emit harness

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

---

## How the interpreter executes

Understanding the interpreter's execution model is prerequisite to every performance design
below.

### What to optimise: the data that leaves the cache

The interpreter's own bookkeeping — dispatch, stack slots, operand decoding — runs on a hot
frame that lives in L1.  Cutting it makes every run faster by a constant factor, and it has
been cut (below).  What decides how an ALGORITHM scales is the data that flows out of the
caches: records claimed, grown and relocated, blocks copied, stores created.  So the first
question for an interpreter routine is not *how many instructions* but **does the interpreter
do more work on stores than the compiled code does for the same routine?**  `LOFT_STORE_CENSUS`
answers it for both backends with the same counters ([§ Store work](#store-work-interpreter-against-native)),
and a row where the interpreter does more is where a native rule avoids an object or a move —
the candidate to move into the IR phase, where both backends get it.

### Dispatch loop (`src/state/mod.rs`)

The loop fetches one opcode byte and calls the corresponding function from the `OPERATORS`
function-pointer slice (`src/fill.rs`, generated from the `#rust` templates in
`default/*.loft`).  Bytes 0–254 are one-byte opcodes; **byte 255 is an escape prefix** — the
loop reads a second byte `ext` and dispatches `OPERATORS[255 + ext]` (`emit_op`).

There are two loops in `execute_argv`.  The **lean loop** runs whenever nothing watches
individual ops, and does per op only what an ordinary run needs: publish the allocation site
(`alloc_pc`), publish the crash context (`crash_report::set_dispatch`, twelve bytes; the labels
are set once per loop), dispatch, the frame yield, and the halt checks.  The **full loop**
carries every per-op instrument — the debugger and profiler (`debug_check`), live reload, the
stack census, the stack shadow, allocation paths, the UAF scans — and takes over the moment one
is armed, including a debugger attaching mid-run.  Measured 2026-09-29: the full loop's
bookkeeping was 62 of an op's 152 instructions on a vector-writing loop.

### Stack and variable access (`src/state/mod.rs`)

The execution stack is a single flat region inside a `Stores` record, addressed by
`stack_cur: DbRef` and `stack_pos: u32`.  `get_stack`, `put_stack`, `get_var` and `put_var`
have a **direct path** (`State::fast_stack`): the stack store's buffer plus the offset, inlined
into every operator.  The general store path re-checks on every push and pop what the stack
guarantees by construction — its store is live, not foreign, not locked, and `ensure_stack`
grows the buffer and the record together — and cost 43 % of the interpreter's time on that
loop.  The checked path (`*_checked`, out of line) runs whenever an instrument that watches
stack accesses is armed — `verify_on`, `LOFT_STACK_CENSUS`, `LOFT_UAF_GEN`,
`LOFT_STRICT_STORES`, the `stack_align_guard` feature — and in every debug-assertions build.

### Operand fusion — superinstructions

The bytecode generator emits the most frequent operator shapes as ONE op that reads its
operands in place instead of pushing them first.  Each fused op calls the unfused operators'
own functions in the same order, so fusion changes where operands come from and nothing about
what is computed; `LOFT_NO_FUSE=1` emits the unfused form (R-Switch), and
`tests/scripts/an-integer-operator-over-locals-runs-as-one-op.loft` is the guard.

| fused op | replaces | chosen by |
|---|---|---|
| `OpIntVV` / `VC`, `OpCmpIntVV` / `VC` | an integer operator over locals and literals | `fusable_int` |
| `OpIntVVPut` / `VCPut` | `x = a op c`, e.g. `i += 1` | `set_var` |
| `OpCmpIntVVJump` / `VCJump` | an `if` or loop test and its jump | `gen_if_test` |
| `OpTextWalkStep` | the step of `for c in T` | `hoist::char_walks` (native's `(R-CharWalk)` matcher) |
| `OpTextNullJump`, `OpTextEndJump` | a text walk's two end tests | `emit_text_end_test` |
| `OpVecGetInt[Nullable]`, `OpVecSetInt` | an integer element of a local vector at a local index | `emit_fused_vec` |
| `OpVecEndJump` | `for x in v`'s end test | `gen_if_test` |

A position operand is taken at the stack height the op STARTS at, and a fused op whose
generated body pops a value first takes its local positions before that value is pushed;
both are the defects the guard's planted-defect cells catch.  The shapes were chosen from
`LOFT_OP_NGRAMS`, the statically adjacent operator runs over the bench lanes (PROFILING.md).

What these buy is constant-factor speed on work that already runs in cache.  Measured over
the 79 bench routines (`make interp-gap`, 2026-09-30): the fast stack path and the lean loop
2.0× (median), the fusion above another 1.2×, 2.5× together; up to 5.2× on text walks.
Measured the same way, **none of it changed a single store operation** — which is why the
next work is in the section below, not in more fusion.

### Store work: interpreter against native

`LOFT_STORE_CENSUS=<file>` (PROFILING.md) counts, at the chokepoints both backends share,
stores created and freed, records claimed, deleted, grown and relocated, and the bytes block
copies and text writes move — one line per `ticks()` call, so a bench routine's work is the
difference between the two lines around its timed loop.  Build the interpreter and the native
program against the `op-census` feature (the counters are compiled only there):

```bash
cargo build --release --lib --bin loft --features op-census --target-dir target/op-census
LOFT_STORE_CENSUS=i.tsv target/op-census/release/loft --interpret bench.loft --n 2
target/op-census/release/loft --native-emit n.rs --lean bench.loft
rustc -C opt-level=3 --edition=2024 --extern loft=target/op-census/release/libloft.rlib \
      -L target/op-census/release/deps -o n n.rs && LOFT_STORE_CENSUS=n.tsv ./n --n 2
```

Measured on `14_stdlib_vector`, per op (interpreter / native):

| routine | claims | grows | relocations | bytes relocated |
|---|--:|--:|--:|--:|
| `push` | 4 / 1 | 4 / 0 | 4 / 0 | 81,526 / 0 |
| `record_append` | 3 / 2 | 6 / 0 | 1 / 0 | 233,380 / 0 |
| `grid` | 392 / 137 | 260 / 4 | 4 / 4 | 1,210 / 824 |
| `copy`, `remove_front` | equal | equal | equal | equal — shared runtime work |
| element reads and writes | none | none | none | none on either side |

The interpreter's extra store work is **vector growth**: native sizes a vector it fills once
(`(R-Push)`, `(R-PushFill)`, `(R-PushRec)`) where the interpreter grows it step by step and
relocates the data on each step.  Those rules are the first candidates for the IR phase.

### Function calls

`fn_call` pushes the return address onto the stack and jumps `code_pos` to the callee. The
callee's locals live above the caller's on the same flat stack record — there is no frame
allocation. A return slides the return value down with `copy_block`, which the store census
counts as copied bytes (8 per integer return).

---

## Front-end speed — how it is measured and guarded

loft's own compile (parse, scope pass, lints) is measured by phase, attributed by module,
and guarded by a COUNT, never by a time (@PLN166).

- **By phase.**  `LOFT_TIMING=1` prints `parse_default`, `parse_user`, `scopes`, `lints` and
  `front_end`; the four phases sum to `front_end` (pinned by
  `tests/compile_scaling.rs::loft_timing_phases_sum_to_the_front_end`).
- **The bench.**  `python3 bench/frontend/frontend.py` measures the cold, warm and edit-loop
  modes over a generated corpus frozen by `CORPUS_VERSION` (tiny / medium / large), the modes
  INTERLEAVED run by run so load drift hits all of them alike; `--counts` adds instruction
  counts, `--self-test` proves the harness sees a slowdown it is handed
  (`LOFT_TIMING_INJECT_MS`), and an edit run whose input hash repeats is refused.  A report,
  never a gate.
- **By module.**  `scripts/profile.sh --engine` folds the samples onto loft's modules; its
  oracle row is in PROFILE_ORACLE.md § Engine.  For an exact before/after, callgrind on the
  large corpus with `LOFT_NO_CACHE=1` — a binary copied out of `target/` has the program
  cache ON and otherwise measures a cache write.
- **The gate counts, it does not time.**  Wall clock varied ±30 % under load on one box,
  so a time gate teaches people to ignore it; an allocation count is exact.
  `tests/frontend_counts.rs` counts the front end's heap allocations in-process over the
  bench's corpus against `bench/frontend/allocations.tsv`, keyed by OS and cargo profile.
  A count that GROWS fails; one that falls passes and names the re-pin
  (`LOFT_FRONTEND_REPIN=1 cargo test [--release] --test frontend_counts`).  One uncounted
  run goes first — the first compile in a process pays one-time setup that differs by
  platform — and the two counted runs must agree wherever a pin is read or written; a
  build with no pin reports and passes.
- **The edit loop reuses the stdlib.**  A program-cache miss takes the stdlib from its own
  cache, and the program manifest pins the stdlib it was built against with a `stdk` line,
  so an edited, added or removed stdlib file is never served stale
  (`tests/arc_e_program_cache.rs`).  A warm `--native` run goes further and execs its binary
  off the source key before any parse — § N6.

What each cut measured, and the leads left, are [PERFORMANCE-history.md § F1](PERFORMANCE-history.md).

---

## Open performance work

Each row was checked against the tree: the evidence column says what is absent.  The full
design of each is in the record; the delivered items are listed below the table.

| Item | Backend | Open because | Design |
|---|---|---|---|
| **P1** — superinstruction merging | interpreter | no superinstruction ops exist; nothing blocks it — the two-byte opcode escape has room ([INTERMEDIATE.md § Opcode budget](INTERMEDIATE.md#opcode-budget--and-why-the-count-is-never-written-down)) | [PERFORMANCE-history.md § Design: P1](PERFORMANCE-history.md#design-p1--superinstruction-merging) and § O1 Superinstruction Peephole |
| **P2** — stack raw-pointer cache, the interpreter half | interpreter | the native half ships (loft#885); the interpreter half carries memory-model risk and a bounded gain | [§ Design: P2](PERFORMANCE-history.md#design-p2--reduce-store-indirection-on-the-stack) |
| **P3** — integer paths carry no `long` sentinel | both | no audit test exists | [§ Design: P3](PERFORMANCE-history.md#design-p3--confirm-integer-paths-carry-no-long-sentinel) |
| **P4** — block-copy slice materialisation | both | `OpAppendVectorSlice` does not exist | [§ Design: P4](PERFORMANCE-history.md#design-p4--block-copy-slice-materialisation-for-primitive-vectors) |
| **P7** — `shrink_to_fit(v)` | both | `reserve` ships; `shrink_to_fit` does not | [§ Open work](PERFORMANCE-history.md#open-work) |
| **P8** — store-effect classifier | native | no classifier in `src/`; it is the enabling gate for P2's indexed-read hoist and for N2 | [§ Design: P8](PERFORMANCE-history.md#design-p8--store-effect-classifier) |
| **N1** — direct-emit local collections | native | no escape analysis; the largest native gap is data structures | [§ Design: N1](PERFORMANCE-history.md#design-n1--direct-emit-local-collections-in-native-codegen) |
| **N2** — omit `stores` from pure native fns | native | `def.purity` is computed; the stores-less emit is not built | [§ Design: N2](PERFORMANCE-history.md#design-n2--omit-stores-parameter-from-pure-native-functions) |
| **N3 / N5** — unchecked integer arithmetic when operands are non-null | native | the cheap version is unsound (the sentinel is core null propagation).  Counted loops already run their index chains plain behind a guard (`LOFT_NO_GUARDED_CHAIN`, [NATIVE_SWITCHES.md](NATIVE_SWITCHES.md)); straight-line code stays checked | [§ Design: N3](PERFORMANCE-history.md#design-n3--remove-long-null-sentinel-from-generated-code), [§ N5](PERFORMANCE-history.md#design-n5--inline-integer-arithmetic-when-operands-are-provably-non-null) |
| **W1** — wasm string representation | wasm | not built | [§ Design: W1](PERFORMANCE-history.md#design-w1--wasm-string-representation) |
| **O8.1b / O8.2 / O8.3** — packed bytes, bulk struct vectors, zero-fill defaults | both | "not started" in O8's phase table | [§ Design: O8](PERFORMANCE-history.md#design-o8--bulk-initialisation-of-constant-data) |
| **Worker clone** — `Stores::types` / `names` copied per `par` worker | parallel | the fields are not `Arc`-wrapped | [§ Runtime optimisation audit](PERFORMANCE-history.md#runtime-optimisation-audit) |

**Delivered, for reference:** N4 (`cr_call_push` suppressed on `#pure` leaves), N6 (no rustc
probe on a cache hit), F1 (the front end's hot spots), BUILD1 (no lib/bin double compile),
BUILD2 (the native-test binary cache), P5/P6 (amortised vector growth, free-block coalescing),
P7's `reserve`, O8's `const_eval`, pre-allocated vector literals and constant range
comprehensions, P2's native half, and the startup cache.

The two measurement notes that invalidate naïve numbers: `loft --native` compiles a
SEMANTICS build — measure what ships with `--native-release` or a performance lane (CLAUDE.md
§ Conventions, three optimisation tiers); and a warm run of a binary copied out of `target/`
has the program cache on (`LOFT_NO_CACHE=1` for an exact before/after).

---

## See also
- [PERFORMANCE-history.md](PERFORMANCE-history.md) — the record: benchmark tables, designs, analyses
- [PROFILING.md](PROFILING.md) — which switch arms which profiler
- [PROFILE_ORACLE.md](PROFILE_ORACLE.md) — the profilers' known answers
- [NATIVE.md](NATIVE.md) · [NATIVE_SWITCHES.md](NATIVE_SWITCHES.md) — native codegen and its rewrites
- [INTERNALS.md](INTERNALS.md) — `src/fill.rs`, `src/state/`, `src/generation/`
