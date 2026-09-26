<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Profiling switches

Which environment variable arms which profiler, and what each one can and cannot see.  Start
with `make profile`, which picks the instrument; [PERFORMANCE.md](PERFORMANCE.md) § Profiling a
run is the method, and [PROFILE_ORACLE.md](PROFILE_ORACLE.md) is what `make profile-corpus`
checks the instruments against.

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
compiles its own bytecode, so positions cannot be merged, only labels (loft#860).
`LOFT_ALLOC_SITES` is program-only and says so under a suite instead of going quiet.
**A NATIVE run is not sampled** — and the default backend IS native, so a bare
`LOFT_PROFILE=1 loft p.loft` announces that rather than exiting empty (loft#865).
**A `use`d library is a cdylib the sampler cannot enter**: its functions cannot appear
and their time lands on the CALLING line, so a library doing the work reads as a hot
caller — one probe inverted from `100 % app_bit` to `99.5 % lib_grind` under
`LOFT_NO_NATIVE_LIBS=1`. The report says so whenever a library was called.
Prefer `make profile`, which picks the instrument. Off costs nothing (the
sampler rides the existing per-op debug branch); armed costs +7–11 %. PERFORMANCE.md § Profiling.

## `LOFT_NATIVE_CHECKPOINTS` (native, wasm)

**`LOFT_NATIVE_CHECKPOINTS=count|time[:filter]`** — the SECOND profiler, for where the
sampler cannot reach (a stripped binary, no `perf`, **wasm**). The generator writes a probe
at the one call/op chokepoint, so every OPERATOR is counted at its own loft `file:line`,
with a by-function rollup that is exclusive by construction (a user CALL is deliberately
not a site — timing it would mix inclusive and exclusive rows). `count` needs no clock and
so behaves identically on native, wasip2 and in a browser; `time` adds the cycle counter
where one exists and says so where it does not. **The COUNTS are the trustworthy column; the tick
share is a hint.** Counts validate exactly against the pure-Rust reference (whole-number
operators per call: `chan` 3.00, `color_g` 2.00, `ramp` 9.00). Ticks do not: the counter
advances only every ~41.7 ns against a ~1-3 ns operator, so a total is a sum of dithered
samples, AND the per-operator probe inflates operator-dense functions — measured against
the instrumented reference, the two biggest `lock_curved` rows disagree ~2× in opposite
directions. Never quote a tick share as "where the time goes". Costs 3.0×
(`count`) / 5.4× (`time`), and `:filter` narrows it to one function or module. It changes
what rustc may inline ACROSS a probe, so it tells you WHICH code runs and roughly where
time concentrates — confirm a ratio with `compare.py` or `profile.sh`. It is not the normal
route: `scripts/profile.sh` is, and it perturbs nothing. PERFORMANCE.md §
`LOFT_NATIVE_CHECKPOINTS`.
