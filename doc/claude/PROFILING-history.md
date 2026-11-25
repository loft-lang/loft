<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# PROFILING-history.md — what PROFILING.md no longer describes

The record behind [PROFILING.md](PROFILING.md): switches that were retired, as they stood.

## `LOFT_NATIVE_CHECKPOINTS` (native, wasm) — retired with the rustc writer (@PLN225, 2026-11-24)

The rustc writer wrote this instrument's probes; the C backend writes none (`@C170`).  The
section as it stood in PROFILING.md:

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

## A first `--native` run's resident size (loft#1723)

`/usr/bin/time -f %M` on a first `loft --native p.loft` read about 220 MB while the rustc writer
built the program, whatever the program did; loft#1723 filed that peak as a retention of 1 MiB
bridge answers.  Measured, the program peaked at 11 MB, below the interpreter's 22 MB.
