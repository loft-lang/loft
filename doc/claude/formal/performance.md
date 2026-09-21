<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/performance.md — routines pull their weight (strict)

**Catalogue:** GOALS.md § G (the goal this doc is the contract for), @PLN158 (the release performance pass), loft#1426 (the first measurement).

> **Rules then deviations** (see [README](README.md)). Unlike the other docs here, these
> rules bind the DISTRIBUTION rather than a language construct: what "fast enough" MEANS
> for a routine loft ships, and what a performance number must prove before it may be
> compared. They exist because the standing instrument before them (`make speed`) measures
> drift — loft against yesterday's loft — which compares to nothing outside the project and
> so can read green while every routine trails the industry by an order of magnitude.
> Instruments: [PERFORMANCE.md](../PERFORMANCE.md); the model harness is the drawing
> library's `bench/` (loft#1426).

## Rules

### A comparison is admissible only between lanes proven to compute the same thing

```
  (Perf-Like)    speeds of two implementations of a routine are comparable
                 only when both lanes produce the same output hash on the
                 same fixed workload.  Lanes that disagree are not one
                 algorithm, and no conclusion may be drawn from their times.
```

**In words.** A benchmark is a VALIDATION first: the loft interpreter, the loft native
build and any reference twin must print the same hash of their output (the drawing bench
uses FNV-1a-32 over the produced pixels/points), on a workload fixed by construction —
same inputs, same order — never by a seed alone. Only then does the ratio mean anything.
A lane that "wins" by computing less has measured nothing.

### Every shipped routine pulls its weight against an industry reference

```
  (Perf-Weight)  every public routine of the stdlib and of a shipped library,
                 measured per release on a fixed workload, keeps its native
                 time within the stated bar of a reference implementation in
                 an industry-standard language (pure Rust; C# admissible).
                 The `--native` bar is 3× the reference for each routine, and
                 the MEDIAN over every judged routine is held at 2×.
                 Drift against a previous loft release satisfies nothing here.
```

**In words.** The comparison target is the INDUSTRY, not the previous release. In-language
drift (`make speed`) stays useful as a regression tripwire, but it compares loft to
itself — a distribution can hold that line forever while being unusable next to what the
reader would otherwise use. The bar is stated per routine class (a tree-walking
interpreter is not held to compiled-Rust time; `--native` is), and the measured table with
its bar lives with the bench, re-measured each release (`M-perf-pass`).

**The numbers** (owner, 2026-09-17).  The ceiling was 4× — the drawing bench's
`compare.py --bar` default, and @PLN157's end state — and is 3× now, with a median of 2×
as the end goal.  The population is the ROUTINES loft ships — stdlib functions and library
`pub fn`s, each on a workload a real program would run — and grows library by library
through @PLN158's census; the median is taken over that population, never over one
library's table.  A synthetic benchmark program (`bench/01`–`11`) measures the ENGINE and
is read as that — informational, outside the median — because nobody's program is a
recursive Fibonacci.  The drawing library is the first library measured: on that day its
fourteen routines read a median of 2.01×, with six at or over 3×.  A per-routine bar in
`bench/ratio_oracle.tsv` is a ratchet DOWN toward 3×, recorded where a routine starts above
it, and never a licence to stay there.

### A reference twin is created where it matters, not everywhere

```
  (Perf-Twin)    a routine with no natural industry counterpart is measured
                 but not judged — UNTIL a performance hit is expected or
                 measured on it, at which point a twin is WRITTEN (same
                 arithmetic, same order, in the reference language) rather
                 than the judgment waived.
```

**In words.** Not every library has an industry-standard sibling, and (Perf-Weight) does
not demand one up front. But "no twin exists" is never a standing excuse: the twin is a
port of the routine's own algorithm (the drawing bench's `bench.rs` mirrors `bench.loft`
statement for statement), so it can always be created — and must be, the moment a routine
is suspected of not pulling its weight. A routine measured against nothing is a routine
whose performance claim is unfalsifiable.

### The twin is an instrument — the cure goes to the engine, not the library

```
  (Perf-Cure)    a routine that fails (Perf-Weight) is closed in the ENGINE
                 first (the codegen/runtime class the profiler attributes),
                 then in the loft algorithm itself; REPLACING the loft
                 implementation with a native one is an edge case decided
                 per routine and recorded as such — never the default cure.
```

**In words.** The libraries are written in loft ON PURPOSE, twice over: loft has to be a
fast language, and the fix that makes ONE routine fast by rewriting it in Rust removes
the pressure that makes the LANGUAGE fast while leaving every other routine slow.  And
the libraries double as the project's teaching corpus — an open-source distribution whose
libraries a reader can actually comprehend (GOALS.md § B — the teaching corpus) — so the
industry-wide pattern this rule refuses is the "fast pass": readable code shadowed by an
optimized twin nobody can follow.  The reference twin exists to MEASURE, never to ship.
A native rewrite remains available for the edge case that genuinely needs it, taken per
routine, with the reason recorded beside it — an exception with a receipt, not a habit.
loft#1426 is the rule applied: the profiler showed the loft hot loop matching the
reference's, so the deviation is filed against the ENGINE (N1 class), and the drawing
library's source does not change.

## Deviations

OPEN: **1**

- **D-perf-1 (OPEN, loft#1570)** — violates (Perf-Weight) and (Perf-Twin): a drawing routine
  over the 3× bar, and four with no reference twin to judge them against.  Measured 2026-09-21
  on main 9f5cf6a96 (`compare.py --skip-interp --repeat 4 --bar 3.0`, quiet machine, 14/14
  hashes): of the ten routines with a Rust reference only `smooth` is over, at 4.50× (a 200 ns
  reference that swings; 7.50× under load the same afternoon), and the median is 1.41×.
  `parse`, `render_lock`, `render_marks` and `resize` have no reference in the bench, so the
  bench cannot judge them; the 2026-09-17 figures had them at 3.39×, 4.43×, 6.25× and 4.51×.
  The entry closes when every routine has its twin and meets the bar.

  Filed as loft#1426 when the routines ran 10–50× behind their twins on `--native-release`
  (hash-validated lanes, so the comparison is admissible under (Perf-Like)).  Profiler
  attribution showed the loft side's hot loop matching the reference's, placing the cost in
  the value model (`codegen_runtime` / `DbRef` indirection — PERFORMANCE.md's N1 class), not
  the library.  That issue closed when @PLN157 brought every judged row within the then 4×
  bar; @PLN157 and @PLN164 remain the fix streams.
