
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
- [What is judged, and the run that checks it](#what-is-judged-and-the-run-that-checks-it)
- [Where the numbers are](#where-the-numbers-are)
- [How to optimise — the checklist](#how-to-optimise--the-checklist)
- [Profiling a run](#profiling-a-run)
- [Measuring native code](#measuring-native-code)
- [How the interpreter executes](#how-the-interpreter-executes)
- [Front-end speed — how it is measured and guarded](#front-end-speed--how-it-is-measured-and-guarded)
- [What makes a slow routine a bug](#what-makes-a-slow-routine-a-bug)
- [Open performance work](#open-performance-work)
- The startup cache — what is cached, how to turn it off, which loft a benchmark measures:
  [STARTUP_CACHE.md](STARTUP_CACHE.md)

---

## What is judged, and the run that checks it

The question every performance number here answers is `(Perf-Weight)`'s: how long a routine
takes in the build a RELEASED program runs, against the same routine written by hand in Rust.

| | |
|---|---|
| **judged** | the C backend's SHIP lane (`cship`) ÷ the Rust twin (`rustc -O`), per routine, under clang and under gcc, each compiler on its own.  `cship` is the program as `--ship` builds it: the release-pass probe, `-O3`, every fault check the generator may drop at a ship site dropped. |
| **reported beside it** | the checked C lane (`c`): the same backend with every fault check kept.  Its excess over `cship` is what the guarantees cost — worked (range proofs, guarded copies, fault flags), never a defect of the bar. |
| **not judged** | the Rust generator (`native`, being shelved) and the interpreter (§ How the interpreter executes). |
| **population** | every routine of every CORE bench (`bench/NN_*/bench.loft`) and of every LIBRARY bench (the packages `bench/portal/libs.tsv` names, checked out by `bench/portal/checkout_libs.sh` under `$LOFT_PERF_LIBS`, default `../loft-bench-libs`).  The per-routine bar holds for all of them; the median is taken over the routines loft ships (stdlib and libraries) — the synthetic engine benches are measured and read, but kept out of it. |
| **bars** | 3× per routine is the bug line, 2× the median; a stable release cuts at 5× ([formal/performance.md](formal/performance.md)). |

The run — both C lanes, every core and library bench, one compiler:

```bash
make perf-portal                                             # records this host's rows
make perf-portal ARGS="--cc gcc-16"                          # the gcc half (a real gcc: on macOS `gcc` is clang)
python3 bench/stats.py --lanes rust,c,cship --no-local \
  $(python3 -c "import sys; sys.path.insert(0, 'bench/portal'); import portal; \
                print(' '.join(portal.library_packages()))") \
  --loft "$PWD/target/release/loft"                         # the same run, nothing recorded
```

`stats.py`'s default lanes are these three.  Compare only like with like — a checked-lane number is not a ship-lane
number, and neither is a number from another host: the machine is part of every row, and the
Rust twin itself runs at a different speed on each, which moves every ratio.

Before a full run, `bash bench/portal/checkout_libs.sh --status` must show every library
checkout on the branch `libs.tsv` names and 0 behind it.  The script never switches a branch
itself, and a checkout left on another branch measures another version of the library: the
rows look normal and are not comparable.

Matching hashes vouch for the shapes the benches hold, not for a rewrite's correctness.  The
population holds few routines that rebuild recursive structures (`21_tree`, `22_expr`, the
`markdown` and `html` libraries' trees); a rewrite over such structures — an in-place literal
rebuild, a rebind through a view — is checked by its language-suite cells and by the loft-in-loft
elaborator (@PLN91), the workload that rebuilds tree nodes in place on every pass.

## Where the numbers are

A number written into prose is stale the moment it is committed, so this doc carries none.
Ask the tool that measures:

| question | command |
|---|---|
| where native stands against Rust, by mechanism class | `make perf-portal` → [PERF_PORTAL.md](PERF_PORTAL.md) (generated) |
| a lane vs the Rust reference per routine, as statistics | `python3 bench/stats.py --lanes rust,c,cship` (§ What is judged; [bench/README.md](../../bench/README.md)) |
| what got slower or faster than on `main` | `make speed` (a report); `make speed-gate` is the one speed gate |
| the classic benchmark suite, every lane | `bench/run_bench.sh` |
| which fn, line or path burns the time | `make profile` — § Profiling a run |
| the interpreter against native, every routine with its censuses | `make interp-gap` → `target/interp-gap/report.md` |
| how far each REWRITE reaches — the sites it admitted against the sites it refused, the refusals by reason | `make decline-census` (`scripts/decline_census.py`, a report; `ARGS="--record"` keeps it in `bench/portal/decline_census.tsv`).  A routine over its bar is a shape a rewrite declined; the reason at the top of a rule that matters is the next arc, picked by yield |
| did this change move the WORST routines — interpreter/native and native/Rust, re-timed in seconds to minutes | `make worst` (`scripts/worst.py`): the working set is taken from the last complete `make interp-gap` and `make perf-portal`, so a full run is what redefines it |

The rendered benchmark page for users is [doc/00-performance.html](../00-performance.html).

---

## How to optimise — the checklist

Both backends, in this order.  Each step's detail lives where it points; the order is the rule.

1. **Pick the row by measurement.**  `make interp-gap` (interpreter against native, the ~100×
   cliff) or `make perf-portal` (native against Rust); rank by [§ The clear case
   first](#the-clear-case-first), within [§ Wide before deep](#wide-before-deep).  A routine
   over 3× its Rust twin is a SYMPTOM: loft is not inherently slower than rustc, so the excess
   is a mechanism in loft costing more than it should.  Group such routines by mechanism class
   (`bench/portal/classes.tsv`) and work the class; a slow routine inside a class already
   under the bar is an ordinary bug.
2. **Measure real time on the tier that ships**, never the semantics build (CLAUDE.md § three
   optimisation tiers), pinned to one core (`taskset`, `perf stat -e instructions,cycles`).
   `LOFT_PROFILE` samples by OPERATION COUNT, not time: a line full of cheap ops reads hot and
   is not — confirm with `perf` before believing it.  Run nothing beside a gate: it perturbs
   the timing and can starve the gate of memory.
3. **Name the work the slow form does that the fast one does not** — a copy, a record claimed,
   a conversion, a call frame, a store grown.  [§ Count before you
   time](#count-before-you-time) for an asymptotic question; `LOFT_STORE_CENSUS` for data that
   leaves the cache ([INTERPRETER_PERFORMANCE.md § What to
   optimise](INTERPRETER_PERFORMANCE.md#what-to-optimise-the-data-that-leaves-the-cache)).
   Name it from the CURRENT emission: a profile expires as rules land (a store share measured
   before the work-buffer rules priced a fix that then measured level).  A step of 5–10 % on a
   routine still a factor off its twin is working on the wrong row: find where the rest of the
   gap lives before picking a fix.  For now only the null-integer cost is left alone
   (worklist § Focus).
4. **Hand-write the efficient form first** — in loft, or as the IR the rewrite would produce —
   for that one case, read what it compiles to on BOTH backends (`--interpret --dump`,
   `--emit-c`, `bench/portal/hand_price_c.sh`) and time it.  That is the prize, and the
   proof the target shape is fast.  If the hand-written form is not faster, the named work was
   not the cost: go back to step 3.  The hand form changes only what the rule would change —
   one that also drops checked arithmetic prices a rewrite no rule may build (`slope_path`:
   −27.5 % priced, −15 % available) — and a gain made of two pieces is split by pricing each
   alone.  Time that moves without instructions is a stall (a reference written to memory and
   read back, an alias): `perf stat`'s stall counters name it faster than elimination does.
5. **Only then build the rewrite that removes the unneeded work**, turning the natural spelling
   (the canonical one) into that form — after checking its admission holds AT the target (its
   gate or trace on that shape): a clause whose gate declines the very loop it was written for
   was built, measured and removed.  Where both backends gain, in the IR phase — a native
   rule moved there ([INTERPRETER_PERFORMANCE.md § Why the interpreter is optimised at
   all](INTERPRETER_PERFORMANCE.md#why-the-interpreter-is-optimised-at-all)); a runtime lever
   before a generator rewrite ([§ Wide before deep](#wide-before-deep)).  **No combined opcode
   or kernel built for one row**: it speeds one spelling and nothing next to it ([KERNELS.md](KERNELS.md)); an
   operator comes after the IR work, from broad evidence (formal/performance.md `(Perf-Order)`).
   **Fix the mechanism, not the shape**: a rule that speeds one shape is a STOPGAP — never
   built for the bench alone, only for a shape real programs use (grep the consumers and the
   libraries) while the root fix is far off — recorded with the root fix that retires it, and the measure of a fix is what it moved that nobody
   targeted (class medians, `make perf-portal`).  A representation of the compiler's own —
   a stack array, a tuple in registers — is allowed wherever a program cannot observe it,
   which is everywhere except a package's native code and a store bound to a file
   ([@C135](DESIGN_DECISIONS_PLATFORM.md), [APART_VALUES.md](APART_VALUES.md)).  Only proven situations
   ([@C120](DESIGN_DECISIONS_VALUES.md)), the contract is semantics, not representation
   ([@C122](DESIGN_DECISIONS_PLATFORM.md)), remove the object rather than complicate the memory
   model ([@C125](DESIGN_DECISIONS_OWNERSHIP.md)).
6. **Verify it cannot be wrong**: a guard with hand-computed values on both backends whose
   planted defects go red (`@falsified-at`, [GUARDS.md](GUARDS.md)), its switch A/B
   ([formal/rewrites.md § Every rewrite is switchable and
   falsifiable](formal/rewrites.md#every-rewrite-is-switchable-and-falsifiable)),
   `make rewrite-census`, and native's signatures unchanged — no change may make native or any
   routine's ratio worse.  Regenerate what it derives: `make surface-gen` after a new op or
   builtin.
7. **Measure the routines the optimisation was scheduled against, and only those**:
   `python3 bench/stats.py --routine <bench/routine,…>`, or `make perf-check ARGS="--routine
   <bench/routine> --record"` to compare them with this machine's committed rows and keep the
   new ones.  One optimisation never starts the full list (`make perf-portal`).  A row that
   reads slower than its last committed measurement (`make perf-trend` lists them) is the
   exception that needs no schedule: re-measure it alone, at any time, before explaining it.
   A row that moved while every function it runs is byte-identical between the two builds
   (diff the two emissions) moved by code placement, not by the change.
   The measured before/after goes in the commit and in the plan or ledger that owns the row.
8. **Re-analyse after every few optimisations — never work a list of fixes to its end.**  A
   fix moves more than its own row: it changes what LLVM inlines and what other rewrites see,
   so it can add a cost somewhere else (a rewrite that gave a cold caller access to a hot
   helper changed how that helper was inlined into its hot caller, +30 % on an unrelated
   routine) and remove one nobody priced (two fixes aimed at one row lowered a neighbour by
   19 %).  So after a GROUP of steps — not after each one — run the full list once
   (`make perf-portal`), compare per row against the previous run, bisect any rise with the
   rewrites' switches, and re-derive the remaining analysis from the CURRENT emitted code — an
   analysis written before the fixes describes a program the fixes have replaced.

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
pipeline and the C compiler builds a binary it then does not run (the build-share guard says
so, and names the missing flag).  The report has two tables: self time by SYMBOL, and the same
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

* **Live stores, not leaked ones.** `LOFT_LEAK_SITES` (on a C build too) groups by the same
  key but over what was never freed, so a program that frees everything gets an empty report
  however much memory it used.
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
read the percentages: at fifty samples a 2 % row is one sample. `--annotate` refuses a
symbol whose share rests on fewer than ~50 samples and says why, because a symbol that won a
coin toss would be annotated as if it were the hot one. A short run wants a higher `--freq`,
or more work.

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
generates C, shells out to the C compiler, and runs the binary it built. `perf` follows forks, so
recording that command records the **build** — the C compiler and the linker take the entire
top of the profile, and the few samples that are your program come back as bare hex, because the
binary is stripped. So a native run is built **once, unprofiled**, and only then recorded;
`--no-warm` opts out. The warm-up really does run your program, side effects and all, which
is why the script announces it.

The lever that symbolizes the binary is the `--native-debug` flag, and it is the only one:
the binary cache key hashes *that flag*, not the environment (`src/main.rs`). Set
`LOFT_NATIVE_KEEP_SYMBOLS=1` on its own and an already-cached **stripped** binary is handed
straight back — the setting applies and nothing changes. `--native-debug` keeps symbols
and emits DWARF line tables from the generated C's `#line` markers, which name the loft source,
so `--annotate` lands on the `.loft` line.

What survives is a profile that names your code:

```
25.27%  hot_a-d780ac7f0  [.] n_slow_part
13.86%  loft             [.] loft::use_analysis::first_arg_write_ops   ← front end, not your program
 5.53%  hot_a-d780ac7f0  [.] n_fast_part
```

The `loft` rows are the front end (parse, IR, codegen, cache lookup); the other command is
your compiled program, its functions named `n_<yours>`. When the build still takes 20 % or more,
the script says so rather than letting a plausible-looking compiler profile pass for a hot
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

The per-operator instrument the rustc writer wrote into a program (`LOFT_NATIVE_CHECKPOINTS`)
left with that writer; its section is in [PERFORMANCE-history.md](PERFORMANCE-history.md).

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

The same holds across ROWS: the slowest routines carry the cleaner upgrades, and a row
already near its Rust twin (≲ 2.5×) is hard to analyse — its cost is spread thin and a
10–20 % move there is as often the machine's library or layout as the code.  Such a row is
not taken as a unit of its own; it moves when a slow row's fix lands the shared mechanism
(the allocator, the copy, the text paths) it runs on.

### Wide before deep

**The owner's direction for the perf stream: WIDE, not deep.** Optimisation stays
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
definition — @C122, a rewrite is free only where its conditions hold.  The landing rule is in
[formal/rewrites.md § Every rewrite is switchable and
falsifiable](formal/rewrites.md#every-rewrite-is-switchable-and-falsifiable).

## Trends, and the check before a commit

The portal (`make perf-portal`) writes ONE row per routine into
`bench/portal/results/<host>.tsv` — the machine's latest measurement — and the page reads
that row.  A laptop's node name changes with its network; `LOFT_PERF_HOST=<name>` keeps its
rows in one file.  The history is the file's git history: every commit that touched it is one
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
  compares each routine with this machine's last committed row.  A routine moved when its
  ratio to Rust AND loft's own time (`native_ns`, saved in every row) both moved 15 % or more
  the same way — the ratio alone moves with a noisy twin, the time alone with the machine —
  and a slowdown exits 1; a ratio that moved over a level native time is listed apart, as
  the Rust lane moving.  `--record` makes the run the
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
> On a 20 000-call probe whose whole hot loop is inside `drawing`, the baseline and
> `LOFT_NO_SCALAR_HOIST=1` time 394 ms and 393 ms. Both numbers are also dominated by the native
> compile the run pays, which is the second way that shape of timing misleads.
>
> To A/B a library's own codegen, the cdylib has to be rebuilt under the switch — clear
> `native-auto/` (in a SCRATCH COPY of the package, never the consumer's tree) or run the lane
> with `LOFT_NO_NATIVE_LIBS=1`, which interprets the library and measures a different thing
> again. Attribute inside a library with `LOFT_PROFILE` under `LOFT_NO_NATIVE_LIBS=1` — the
> sampler cannot enter a cdylib at all, so without it the library's time lands on the calling
> line and a library doing the work reads as a hot caller.
>
> The standard library is compiled the same way for an interpreted program (@PLN181): the
> functions `native_gate::export_set` picks — the ones whose loft body loops — run as a C unit
> the binary builds once in the user cache, and the sampler sees each as one call.
> `LOFT_NO_COMPILED_STDLIB=1` interprets them instead, which is how to attribute inside one and
> the A/B for anything the compiled bodies are suspected of.  `LOFT_TIMING=1` prints how many
> dispatched; 0 means the unit could not be built here (no C compiler — `LOFT_HOSTED_VERIFY=1`
> says why).  That state is correct and 5–23× slower on text routines, so **`bench/stats.py`**
> refuses an interpreter lane where none dispatched (`--allow-declined-stdlib` measures that
> state on purpose).

## How the interpreter executes

The interpreter's execution model is [INTERPRETER_PERFORMANCE.md](INTERPRETER_PERFORMANCE.md):
why it is optimised at all, the dispatch loop, stack access, operand fusion, its store work
against native, calls, and how to measure an interpreter change (pin the layout first).

---

## Front-end speed — how it is measured and guarded

loft's own compile (parse, scope pass, lints) is measured by phase, attributed by module,
and guarded by a COUNT, never by a time (@PLN166).

- **By phase.**  `LOFT_TIMING=1` prints `parse_default`, `parse_user`, `scopes`, `lints` and
  `front_end`; the four phases sum to `front_end` (pinned by
  `tests/compile_scaling.rs::loft_timing_phases_sum_to_the_front_end`).  On a warm start a
  `warm_load` line splits `parse_default` into the stdlib fingerprint (`stdlib_key`, a stamp
  of the sources' sizes and mtimes — `cache::stdlib_fingerprint` — never a read of them while
  nothing moved), the drift manifest (`manifest`) and the bundle's materialisation (`bundle`:
  the mmap'd store copied into the definition table, the one cost a warm start still pays
  per definition), and `compiled stdlib: … (N us)` is the dispatch mark.  Past the front end,
  `close_program` prints one line naming each whole-program rewrite pass's time, and
  `value_records` one naming its analysis steps (the site gate summed over the fixpoint's
  rounds, the callers it walked, and the five costliest).  Under `loft test` each file prints
  its own `parse` / `scopes` / `lints` line, and whether it started from a shared base.
- **Where a slower test's time went.**  With `LOFT_TIMING_LEDGER=<dir>` every loft process
  appends rows to `<dir>/timing-<pid>.tsv`: its phases (`parse_default`, `parse_user`,
  `scopes`, `lints`, `byte_code`, `native_emit`, `run`), the program's own native compile
  (`binary-c <program> hit|miss <secs>` — the C compile, or the cache that spared it),
  each external build step (`exec`) and each library build, every row ending with the test
  that started that loft (`LOFT_TIMING_TEST`, passed by `platform::process::harness_command`).
  `python3 scripts/timing_compare.py <before> <after>` compares two ledgers — a directory, or
  `run:<id>[:<leg>]` for a CI run, which uploads one as `timing-ledger-<os>-<shard>` — by
  category and by test, largest growth first, so a slower test reads as the phase that grew
  instead of a commit range to bisect.
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
  `tests/frontend_counts.rs` counts the front end's heap allocations in-process against
  `bench/frontend/allocations.tsv`, keyed by OS and cargo profile.  It is ADVISORY: ci.yml's
  `front-end allocation pins (ubuntu, advisory)` job runs it (`-- --ignored`), red there and
  never failing a test leg or the merge.  A count that GROWS fails it; one that falls passes and
  names the re-pin (`LOFT_FRONTEND_REPIN=1 cargo test [--release] --test frontend_counts --
  --ignored`).  One uncounted run goes first — the first compile
  in a process pays one-time setup that differs by platform — and the counted runs must
  agree wherever a pin is read or written; a build with no pin reports and passes.
- **Two pins, because they are two costs.**  `stdlib` is the COLD stdlib parse alone —
  what an installed `loft` pays once per stdlib change, since a run loads the stdlib from
  the startup cache (`warm_load_stdlib`, [STARTUP_CACHE.md](STARTUP_CACHE.md)).  `tiny` and
  `medium` are the bench's corpus compiled on top of the loaded stdlib — what a warm run
  pays.  The scope pass and the lints walk every definition on every run, the stdlib's
  too, so a new stdlib declaration moves its own row AND both program rows by one constant;
  a delta that grows with the corpus is the front end itself.  The gate reports both halves
  together and says which reading applies.  Accept a stdlib growth with the declarations
  named in the commit (`join.py rederive --accept frontend-allocations`); chase a front-end
  one.
- **The count reads nothing the checkout accumulates.**  The stdlib is parsed from a
  pristine copy of `default/`'s `.loft` files, made outside the counting window, because a
  directory listing costs allocations per entry; and the loader skips hidden entries, so a
  `.loft/` cache a run left beside the sources is never read as part of the stdlib.
- **The edit loop reuses the stdlib.**  A program-cache miss takes the stdlib from its own
  cache, and the program manifest pins the stdlib it was built against with a `stdk` line,
  so an edited, added or removed stdlib file is never served stale
  (`tests/arc_e_program_cache.rs`).  A warm `--native` run goes further and execs its binary
  off the source key before any parse — § N6.

What each cut measured, and the leads left, are [PERFORMANCE-history.md § F1](PERFORMANCE-history.md).

---

## What makes a slow routine a bug

A routine over **3× its Rust twin** (@C136) is a defect.  Who owns it depends on its mechanism
class (`bench/portal/classes.tsv`):

- **an OUTLIER** — over the bar in a class whose median is at or under it: an ordinary
  `performance` issue, filed and fixed like any bug.  One program is slow where its kind is not.
- **SYSTEMIC** — the class median itself is over the bar: the mechanism is slow, and the class's
  plan owns every routine in it.  Filing them one by one would bury the one fix under dozens of
  symptoms.

`bench/portal/outliers.py <results.tsv>` splits a portal run this way and lists the outliers.
File against the results of the host the work runs on, and call a bar met only when every
host's latest run agrees ([bench/README.md § Which host's numbers
count](../../bench/README.md#which-hosts-numbers-count)).  Before filing, the twin must have been audited like-for-like: a twin doing different
work makes the ratio meaningless.

## Open performance work

Each row was checked against the tree: the evidence column says what is absent.  The full
design of each is in the record; the delivered items are listed below the table.

| Item | Backend | Open because | Design |
|---|---|---|---|
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
| **A library's scalar function is not inlined across units** — random `get`, the one C ship routine at the 2.7× line | C | the library's function is emitted in its own unit, so the caller's loop keeps the generator's state in memory where the twin holds it in registers | [@PLN219 § Left to the performance loop](plans/219-rust-slot-links/README.md#left-to-the-performance-loop) |
| **`out += f(…)` for a text `f`** — `(R-AppendTwin)` has no text clause (zttext `materialise`) | C | the rule's clauses name vector buffers only | the same section |
| **A keyed removal shifts its bucket run back** — `lru_mixed` | both | `loft_h_remove_at` shifts; `(Col-RemoveKeyed)` asks only that every other key stays reachable.  A tombstone removal changes the table in both runtimes and in the image: the owner's decision | the same section |
| **Pixels are `vector<integer>`, 8 bytes each** — graphics `canvas`, `clear` | both | `u32` cannot hold opaque white (`(N-Reserve)` keeps its top code), so the width is a language question, not a rewrite | the same section |

**Delivered, for reference:** P1 (operand fusion — [INTERPRETER_PERFORMANCE.md § Operand fusion](INTERPRETER_PERFORMANCE.md#operand-fusion--superinstructions)), N4 (`cr_call_push` suppressed on `#pure` leaves), N6 (no compiler
probe on a cache hit), F1 (the front end's hot spots), F2 (the allocation gate's two pins), BUILD1 (no lib/bin double compile),
BUILD2 (the native-test binary cache), P5/P6 (amortised vector growth, free-block coalescing),
P7's `reserve`, O8's `const_eval`, pre-allocated vector literals and constant range
comprehensions, P2's native half and its interpreter half (formal/rewrites.md `(R-StackBase)`),
and the startup cache.

The two measurement notes that invalidate naïve numbers: `loft --native` compiles a
SEMANTICS build — measure what ships with `--ship` or a performance lane (CLAUDE.md
§ Conventions, three optimisation tiers); and a warm run of a binary copied out of `target/`
has the program cache on (`LOFT_NO_CACHE=1` for an exact before/after).

---

## See also
- [PERFORMANCE-history.md](PERFORMANCE-history.md) — the record: benchmark tables, designs, analyses
- [PROFILING.md](PROFILING.md) — which switch arms which profiler
- [PROFILE_ORACLE.md](PROFILE_ORACLE.md) — the profilers' known answers
- [NATIVE.md](NATIVE.md) · [NATIVE_SWITCHES.md](NATIVE_SWITCHES.md) — native codegen and its rewrites
- [INTERNALS.md](INTERNALS.md) — `src/fill.rs`, `src/state/`
