<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# A `par` worker that maps its rows at once (design, 2026-10-24)

**Built 2026-10-24** as `(R-ParRows)` (`formal/rewrites.md`): `11_par/par` 3.49× → 1.80× of
Rust, pinned to one core, hash unchanged — the priced 1.78×.  Two things the design below did
not foresee: the row variants are translated from the IR by a covering subset
(`generation/par_rows.rs`) rather than by a mode threaded through the main emitter, and `+` /
`-` need their overflow found by sign bits — the built form first ran at 433 µs, not
vectorised, because `checked_add` lowers to an overflow intrinsic no loop vectoriser widens.
The report flag turned out to matter only for `OpDivFloat`'s raise: an integer overflow
reports only under `--dev-soft-halt`, where the runtime takes the per-row worker for every row.

Priced in [over-3.5x.md](over-3.5x.md) § par: a worker that maps its contiguous slice of rows,
with the inner constant-count loop unrolled, takes `11_par/par` from 3.49× to 1.78× of Rust with
every NaN test kept (−49 %).  This is the design of the build.

## What changes

Today a `par` over `vector<T>` with a scalar result runs, per row: an out-of-line `get_vector`
(the element's `DbRef`), the worker closure (which reads each field through the store with a
null branch), and `push`.  No row loop around a call vectorises, so two rows never share an
instruction.

After: the emitter hands the runtime TWO closures for such a `par`.

- **exact** — today's per-row closure, unchanged.
- **fast** — maps a contiguous slice of rows: `|base: *const u8, count, out: &mut [i64]| -> bool`.
  It reads each element's fields straight from the slice and calls the worker's ROW variant,
  which answers the same value the exact worker answers and raises a flag wherever the exact
  worker would have REPORTED (an integer overflow, a divide by zero) instead of reporting.

The runtime (`n_parallel_rows_native`) runs `fast` over each worker's batch; a batch whose
flag is raised is run again, whole, through `exact`.  So a batch that reports does exactly what
it does today, and a batch that does not computes the same values in a loop LLVM can
vectorise.

## Which `par` qualifies (decided at generation time, declined with a reason otherwise)

1. The result rides the WIDE scalar lane (`integer`, `float`, `boolean`, a wide `τ?`); the
   narrow lane and every heap result keep their emitters.
2. The input is a `vector<T>` whose element `T` is a struct of scalar fields stored inline
   (no text, no collection, no reference, no nested struct), read through `const`.
3. The worker and everything it calls are ROW-PURE: they read the element's fields, scalar
   extras and locals; compute with operators the row variant has a silent-flagged form for;
   call only row-pure loft functions; write nothing to a store, allocate nothing, touch no text,
   collection, file or global, and do not `yield`.  A function that is not is the decline
   reason, named.

## The row variant

`n_<worker>__row(elem: *const u8, extras…, __bad: &mut bool) -> P`, and `__row` variants of
the callees.  The same body, with:

- a field read of the element: a raw load at the field's offset, the null sentinel as stored;
- every operator that can REPORT replaced by its silent twin that sets `*__bad` exactly where
  the reporting form would report, and answers the same value (`op_add_long` → `add_long_flag`,
  …): a table in `generation/row_variant.rs`, one line per operator, built from the operator
  families the emitter already names;
- no `cell` / `stores` access at all (that is what makes it row-pure to LLVM too).

The silent twins are branch-free selects, so LLVM can vectorise them; `*__bad` is OR-ed.

## The unroll

A `for _ in 0..C` with a constant `C ≤ 64` whose body is straight-line arithmetic (no loop, no
`break` / `continue` / `return`, at most 16 operators) is emitted unrolled INSIDE a row variant
only.  Measured: without it the row loop does not vectorise (3.29×, the NaN select keeps LLVM
from unrolling the 50 rounds itself); with it, 1.78×.

## Failure paths, and the invariant they share

| what could go wrong | answer |
|---|---|
| a row the fast path computes differently | the silent twins are the exact ops' values; pinned per operator against the exact op on every boundary value (null, MIN+1, MAX, 0, ±0.0, NaN, inf) |
| a report lost | a raised flag reruns the batch through `exact`; a guard plants a twin that forgets to flag and fails |
| a report duplicated | the fast path never reports; `exact` runs once per flagged batch |
| a store growing under the slice | qualifying rule 3: nothing in the worker writes a store; the input is `const` |
| an element read past the vector | the slice is `[start, end)` of `length_vector`, taken once before the batches |
| a worker that reads a field of a null element | elements of a `vector<T>` are inline, never null; a null FIELD reads its sentinel exactly as the exact worker does |

The invariant: **the fast path answers a batch only when it can prove the exact path would
have answered the same values and reported nothing**; everything else is the exact path.

## Steps

1. Runtime: `n_parallel_rows_native` (fast + exact, per-batch fallback).  Unit-tested with a
   fast closure that flags a chosen row.
2. Silent-flagged twins for the integer and float operators, with their boundary table test.
3. Emitter: the qualification walk (with decline reasons under `LOFT_TRACE_PAR_ROWS`), the
   `__row` variants, the two-closure call; switch `LOFT_NO_PAR_ROWS`.
4. The unroll inside row variants; switch `LOFT_NO_ROW_UNROLL`.
5. Guards: cells for values (hand-computed), for a report in one row of a batch (overflow,
   divide by zero) being made exactly once, and for each decline; falsified.
6. Measure `11_par/par` and the `19_stdlib_par` lane; the rewrite census.
