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

### An advantage the twin's language holds is a missing abstraction, not a caveat

```
  (Perf-Gap)     an advantage a reference twin holds by a CONSTRUCTION loft
                 cannot express — a lazy walk where loft collects, a borrowed
                 slice where loft copies, a struct in registers where loft
                 claims a store, a container shape the stdlib lacks — is
                 neither an unfair twin to be rewritten down to loft's form nor
                 a row to be excused: it names a MISSING ABSTRACTION in loft,
                 and the finding is routed to the language (a rewrite rule, a
                 stdlib form, a type), never to the twin.
```

**In words** (owner, 2026-09-28).  loft is not a fundamentally different language from the
reference.  Its STANDARD implementation optimises one way of working — records in stores,
checked arithmetic, values that carry their null — and that choice excludes none of the
others; so whatever the twin reaches for and loft cannot is exactly an abstraction loft
owes its users, and *"the reference does X, which loft has no way to say"* is a FINDING,
not a caveat on the row.  It goes into the class analysis as a priced unit, and the twin
stays as written: rewriting the twin down to loft's limitation would make the row pass by
measuring less, which `(Perf-Like)`'s hash cannot see because both lanes would still
agree.  The distinction from `(Perf-Cure)`: Cure says a failed bar is closed in the engine
rather than in the library; Gap says WHICH engine work a twin's win points at — the
abstraction the twin used.  Measured, three times over: `parse`'s twin split its text
lazily where loft built a `vector<text>`, and the row closed with `(R-LazySplit)` rather
than a collecting twin; a struct the twin returned in registers became `(R-ValueRecord)`;
a `&str` parameter became `(R-TextBorrow)`.  A twin that wins by computing LESS is a
different matter and `(Perf-Like)`'s.

## Deviations

OPEN: **1**

- **D-perf-2 (OPEN, loft#1743, @PLN158)** — violates (Perf-Weight): the shipped routines as a population
  are over both bars.  Measured 2026-09-29 by `make perf-portal` at 639aaa2c3 (every in-repo lane
  and every library bench with a Rust twin, hash-validated under (Perf-Like)): 205 routines,
  median 2.79×, 140 over 3×, 29 of them at 10× or more — `pluginabi` `check_request` 59.9×,
  `mesh3d` `mat4_mul` 39.0× and `mesh_to_floats` 32.1×, `cbor` `encode_bytes` 31.7×.  The
  table and its classes are [PERF_PORTAL.md](../PERF_PORTAL.md); the per-routine row is
  `bench/portal/results/<host>.tsv`, whose git history `make perf-trend` reads.  The entry
  closes when the portal's population meets both bars; `drawing`, the first library judged
  (D-perf-1, now in [performance-history.md](performance-history.md)), already does.
