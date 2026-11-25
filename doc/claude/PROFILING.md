<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Profiling switches

Which environment variable arms which profiler, and what each one can and cannot see.  Start
with `make profile`, which picks the instrument; [PERFORMANCE.md](PERFORMANCE.md) § Profiling a
run is the method, and [PROFILE_ORACLE.md](PROFILE_ORACLE.md) is what `make profile-corpus`
checks the instruments against.  The rustc writer's per-operator instrument
(`LOFT_NATIVE_CHECKPOINTS`) left with that writer: [PROFILING-history.md](PROFILING-history.md).

## The sampler and the allocation sites (`--interpret`)

**Profiling (@PLN140, all opt-in, all `--interpret`):** `LOFT_PROFILE=<ops>` samples the loft
call stack — hot FUNCTION, hot LINE, hot PATH (default one sample per 1024 ops; the op counter
picks *when*, a wall clock says *how much*, and the period is JITTERED because a fixed one
samples a single phase of a periodic program and reports it as the whole) ·
`LOFT_ALLOC_SITES=1` ranks live store BYTES by the loft line that allocated them, captured at
the run's PEAK rather than at exit · `LOFT_ALLOC_PATHS=<ops>` adds the call paths that reached
each allocation. **A program whose only exit is a signal — a server — reports through
`LOFT_PROFILE_EVERY=<seconds>` (a report while running, surviving a hard kill), `kill -USR1`
(dump and keep going, which profiles a WINDOW) or `kill -TERM`/Ctrl-C (dump, then leave):
the report used to render at process exit, so the run you most want a profile of was the
one that could not produce one (loft#1089). Handlers are installed only when the profiler
is armed.** `LOFT_PROFILE` / `LOFT_ALLOC_PATHS` also cover **test runs** (`loft test`,
`--tests`), merged into ONE report keyed by resolved `function` + `file:line` — each test
compiles its own bytecode, so positions cannot be merged, only labels.
`LOFT_ALLOC_SITES` is program-only and says so under a suite instead of going quiet.
**A NATIVE run is not sampled** — and the default backend IS native, so a bare
`LOFT_PROFILE=1 loft p.loft` announces that rather than exiting empty (loft#865).
**An op that runs long is timed, not sampled**: a native call, a `use`d library's compiled
code, a sort or a read that takes 50 µs or more is credited to its own line, where the op
clock alone would land it on whatever op the next sample fell on — a later line, often
another function (@PLN179 finding 022; the banner counts them).
**A `use`d library is a cdylib the sampler cannot enter**: its functions cannot appear
and their time lands on the CALLING line, so a library doing the work reads as a hot
caller — `main`'s call line, where `LOFT_NO_NATIVE_LIBS=1` reads `99 % lib_grind`. The
report says so whenever a library was called.
Prefer `make profile`, which picks the instrument. Off costs nothing. **Armed costs
+9–18 %** on `bench/01`–`05`: a run only the CPU profiler watches keeps the lean dispatch
loop, with the profiler's per-op state in the loop's own locals (a tick read and a count);
allocation paths, the op census or a breakpoint take the full loop, which costs +46–116 % on
its own. A frame-driven program (a game loop, `yield_frame()`) is sampled over every frame:
the run continues through the same dispatch after each yield. PERFORMANCE.md § Profiling.

## What the interpreter executes, per routine (`LOFT_OP_CENSUS`)

**`LOFT_OP_CENSUS=<file>`** (`--interpret`) counts every operator exactly. (`=1` prints it to
stderr; a relative name is the directory loft was launched in — as for every census switch.) There is no
sampling. Each op is keyed by the line of the entry function it ran under (its call site
from `main`, or its own line in `main`), by the function it belongs to, and by the operator.
Beside each count go the bytes the op moved through the store's block routes: `copy`
(`copy_block`), `relocate` (a record moved because it outgrew its claim) and `text` (text
written into a store). A bench times each routine between lines of `main`, so a line is a
routine. Only the thread that runs `main` is counted, so a `par` worker's ops are missing.
The per-op hook rides the sampler's debugger branch, so an unarmed interpreter pays nothing
for it. The byte counters are different. They sit in the store code every interpreted run
executes, so their call sites are compiled only with `--features op-census`, and a release
build carries none of them. `make interp-gap` builds that binary in `target/op-census`. Without
the feature, the file's first line says the bytes were not counted. Armed, a run is
1.6–1.8× slower (lanes 01, 14 and 16).

**`LOFT_REWRITE_CENSUS_FN=<file>`** splits `LOFT_REWRITE_CENSUS` by WHERE each rewrite was
admitted, written as `phase<TAB>function<TAB>rule<TAB>count`. The phase is `c` for the C writer
(the interpreter never gets that form), `interp` for the interpreter's bytecode generator
(compiled code never gets that form), `ir` for the scope pass (both backends) and `parse` for everything earlier.  Summed over the benches per rule, the
same split is `bench/portal/rule_phases.tsv`, which `make rewrite-census` keeps
(`scripts/rule_phases.py` reads it). The per-function totals equal the per-rule census.

**`LOFT_DECLINE_CENSUS_FN=<file>`** is the same split for the REFUSALS of
`LOFT_DECLINE_CENSUS`: `function<TAB>rule<TAB>test<TAB>count`.  Read the two together with
`make decline-census ARGS="--functions <prog.loft> [name …]"`
([REWRITE_REASONS.md](REWRITE_REASONS.md) § The per-function listing).

**`LOFT_OP_NGRAMS=<file>`** (with `LOFT_OP_CENSUS`) counts the runs of two to four operators
that execute one after the other within a function, keeping only runs whose control operators
come last — the statically adjacent ones, which a superinstruction can fuse.

**`LOFT_STORE_CENSUS=<file>`** (`--interpret`) counts the work a run does on its stores:
stores created and freed, records claimed, deleted, grown and relocated, and the bytes block
copies and text writes move.  Each `ticks()` call appends one line (the clock value, then the
counters so far), so the work between two calls is the difference of two lines — a bench
routine's timed loop sits between two of them.  The counters sit in the interpreter's store
code, so they are compiled only with `--features op-census`; a `--native` program's C store
carries none.

`make interp-gap` (`scripts/interp_gap.py`) joins the two with a `stats.py` timing of both
backends ([bench/README.md § Interpreter against native](../../bench/README.md)).

## Resident size of a `--native` run

`/usr/bin/time -f %M` on `loft --native p.loft` reports the largest RSS of any process in the
tree, and a first run includes the C compiler building the program, whose peak is not the
program's. Time a second run, which hits the cache under
`.loft/cache/`, or run that cached binary directly. A peak that stays the same with a loop
count of 1 is the compiler, not the program.
