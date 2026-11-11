<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN211 D2 — an index proven below its vector's length (class 1)

Class 1 of [d1-classes.md](d1-classes.md): `(R-Range)`'s INDEX clause proves an index
non-negative, never below the length, so every read of `v[e]` pays the bounds test (and the
from-the-end wrap where the lower bound is unproved).  Each spelling of the upper bound is a
clause of an existing rule, one phase at a time.

| spelling | rule, clause | state |
|---|---|---|
| a counter of `0..n`, `n` a length local of `v` | `(R-Base)` counter clause (C), `(R-InRange)` (IR) | built 9ca584382 |
| a guard: `if e < len(v)` / `while e < n` | `(R-Base)` guard clause (C), fact in `opt` | built (below) |
| a stepped `while` counter (`i += 4`, `pos -= 1` under `pos > 0`) | `(R-Range)` while-counter clause | not built: no site pays (§ The stepped counter, priced) |
| `i % n`, `n = len(v)` | `(R-Range)` rem clause + the guard fact | open |
| `eb` of `boundary_loops`: pushed in lockstep with `ea` | the length-local answer, widened (§ Lockstep twins) | building |
| `comp` of `boundary_loops`: one push per turn of `0..ne` | — | not built: priced at 0 (§ Lockstep twins) |

## The guard clause

**Invariant.**  At a read or write of `v[e]`, `0 <= e < len(v)` holds when (1) a test
`e < len(v)` passed on every path to it, (2) nothing evaluated between that test and the access
can resize `v` or assign a variable `e` names, and (3) `(R-Range)` proves `e >= 0`.  Then the
access names an element and takes no test: `loft_vh_at_int` / `loft_vh_put_int` and siblings,
each proving the index again under `LOFT_VERIFY`.

**Where the test comes from.**
- `if c { … }`: the facts of `c` hold in the then-arm.  A value `if` is the same node
  (`x = if i + 1 < n { v[i + 1] ?? 0 } else { 0 }`).
- `while c { … }` (a loop whose first statement is `if !c break`): the facts of `c` hold from
  the second statement on, in every turn — the next turn tests again.
- `a && b` arrives as `if a { b } else { false }`: both sides' facts.
- A fact is `e < L` with `L` either `len(v)` or a local `n` that holds `len(v)`
  (`length_locals::own_length_locals`, the frame's own vector).  `e` is a variable or an
  integer expression over variables and constants, matched structurally at the access.

**What ends a fact (the kill).**  `forward_walk::check_body` for `v` and each variable of `e`:
an assignment of either, `v` handed to an operator that may resize it, a call through a
reference, a yield or a par.  Element writes do not resize.  Applied in evaluation order —
after each argument that is not a bare variable, after each call, after each statement — and
over the WHOLE body of a loop nested inside the guarded region before that loop is entered,
since a later turn runs the kill before the read.  `v` must be the frame's own vector
(`own_vector`): a parameter can share its vector with another name.

**Not covered.**  The else side (`if e >= len(v) { … } else { v[e] }`) and the early exit
(`if e >= len(v) { return }` then `v[e]`); a field vector (`d.f`); a guard on a different but
equal expression (`i + 1 < n` guarding `v[1 + i]`).  Each is a later widening if a routine
needs it.

**Site plan.**  `src/opt/below_len.rs`: one pass over a function's IR answering the set of
index nodes (by address) a guard proves below their vector's length — shared, so another
writer can read it.  The C writer reads it where it reads `counter_index` (the held element
read and write paths in `src/c/body.rs`), together with `index_arg`'s lower bound.  Switch
`LOFT_NO_GUARD_BOUND`; census row `R-Base/guard`.

**Sites it reaches** (D1): crypto `b64_decode` under pluginabi `request` (`while i < n`,
`if i + k < n`; the lower bound is unproved, so the `_lt` form).  Not random `indices`
(`k < len(out) && j < len(out)`): its `out` is a hidden return buffer.  Priced by hand before
it was built (below).

## Hand prices (2026-11-10, gcc 15.3 / clang 22.1.8, min of five interleaved runs)

pluginabi `request` (`b64_decode`'s four guarded reads; µs per op, hash `16387f10`):

| form | gcc | clang |
|---|--:|--:|
| as emitted | 1544.5 | 1494.3 |
| lower bound only (`_nn`) | 1532.7 | 1497.6 |
| upper bound only (wrap and a sign test) | 1495.7 | 1453.1 |
| both (no test) | 1503.3 | 1456.0 |
| **built** (`_lt`) | **1496.1** | **1453.1** |

The upper bound is the whole prize and needs no lower bound, so the clause has two forms: no
test where `(R-Range)` proves `e >= 0`, the `_lt` accessors where it does not.

random `indices` (two reads, two writes under `k < len(out) && j < len(out)`; hash
`39dc7552`): the unchecked writes halve gcc (4382 → 2164 µs; clang 4329 → 4325), but the
counters show why: 12.5 M fewer branch misses, at `get`'s own sign branch before its divide,
which gcc then if-converts.  That is the compiler's luck, not the removed tests' price.  The
clause does not reach this site anyway: `out` there is the hidden return buffer, which it does
not admit.

Found on the way: `own_vector` read a parameter's attribute by its variable number, which is
its place among the parameters only when no local is numbered between two of them; a promoted
work buffer keeps its local's number, so work buffers were never admitted.  Fixed (the place
in `vars.arguments()`).

## Both spellings against the tip before them (2026-11-11)

`bench/stats.py --lanes rust,cship`, `--only 18,20` plus drawing, hex_form, hex_recover,
zttext, pluginabi, crypto and graphics; a0da7073f against ca30d9971; gcc two rounds, clang
three on the programs that moved.  Gains in every round: boundary_loops −17 to −31 %,
base64_to_bytes −6 to −13 %, fill_star −3 to −5 %, pluginabi `request` −3 to −6 %;
fill_circle and wide_line −1 to −12 %.  No gcc row is slower in both rounds.

Four clang rows are slower in all three rounds, and each is layout:

| row | clang | evidence |
|---|--:|---|
| zttext `locate` | +8 to +23 % | its C is identical; zttext changed only `seg` and `main`'s argument read |
| crawler `build_vis` | +12 to +16 % | its C is identical; reverting crawler's guard sites by hand leaves it as slow |
| graphics `fill_polygon` | +6 to +7 % | reverting its one `_lt` read by hand leaves it as slow; reverting `main`'s untimed argument read restores it, at the same instruction count (9.762 G both) |
| crawler `sort_floats` | +5 to +7 % | its `_lt` write runs once per outer turn beside an unchanged n² loop; with it the program runs fewer instructions (6.1283 G against 6.1318 G) and fewer branch misses; gcc −5 to −9 % |

A widening this shows: `sort_floats`' guard is `best >= 0 and best < len(used)`, so the test
itself carries the lower bound, which would give the no-test form instead of `_lt`.

## Lockstep twins

**Price first** (2026-11-11, hex_form ship C, min of nine interleaved runs; hash `6c9375d9`):
after both built spellings, `boundary_loops`' `ea[…]` reads take no test and its `eb[…]` and
`comp[…]` accesses still test the upper bound (`_nn`).

| hand form | gcc | clang |
|---|--:|--:|
| as emitted | 9.24 ms | 12.45 ms |
| `comp` test-free | −0.1 % | +1.6 % |
| `eb` test-free | −20.1 % | −17.3 % |
| both | −20.5 % | −19.0 % |

So `eb` is the prize and `comp` is not built.

**Invariant.**  `len(w) == len(v)` at every statement boundary of the function, outside the
two places where one has moved and the other not yet, when:

1. `v` and `w` are distinct vectors of the frame's own (`own_vector`), neither captured;
2. their starts are ADJACENT statements of one statement list, each making its vector empty:
   the local form (`__vdb = null; OpDatabase(__vdb, …); x = OpGetField(__vdb, …);
   OpSetInt4(__vdb, 0, 0)`) or the work-buffer form (`if OpRefIsNull(x) { … x = … } else {
   OpClearVector(x) }`);
3. every OTHER node that writes either (the write test `stable_after` uses) lies in a PAIR:
   one append to one, then, as the very next statement, one append to the other.  An append is
   an optional `OpPreAllocVector(x, 1, _)` and an `OpPush*(x, e)`.  The second `e` holds no
   `break`, `continue`, `return` or `yield`, so no path leaves between the two pushes.

A length local `n = len(v)` is a statement, so it never runs inside a pair or between the two
starts: `n` holds `len(w)` too, from its assignment for as long as it holds `len(v)`, and while
`w` is `stable_after` it.  `own_length_locals` answers `(n, w)` beside `(n, v)`; switch
`LOFT_NO_LOCKSTEP`.  A start run again (in a loop) resets both; a free ends both.

**Not covered**: an element write to either (`w[i] = x` does not resize, but the write test
counts it), an append of a vector or a slice, a record append (`OpNewRecord` …
`OpFinishRecord`), starts that are not adjacent.  Each is a later widening if a site needs it.

**Readers**: each one mapped a range end to ONE vector (`.find`).  Now each takes every vector the
local is the length of: `peel::range_lens` and `counters_in_range` (C, `@FR-R-Base`),
`below_len::length_of` (one guard fact per vector), and `in_range`'s alias (`rewrite_for` once
per vector, `@FR-R-InRange`).

## The stepped counter, priced (2026-11-11)

Not built: neither site it was meant for pays.

- crypto `b64_decode` (`i += 4`): the lower bound turns its `_lt` reads into the no-test form,
  and the hand price above shows that form is no faster (gcc 1496 → 1503 µs, clang 1453 →
  1456).
- hex_recover `sort_keys` (`pos = len(out) - 1`, `while pos > 0 && … { …; pos = pos - 1 }`):
  its reads have no `< len(out)` guard, so it would need the guard's lower bound AND an upper
  bound carried from `pos = len(out) - 1` through the decrement.  Priced by hand first on the
  ship C (`n_sort_keys__fr`, all eight reads and two writes; min of 11 interleaved runs):

  | routine | form | gcc | clang |
  |---|---|--:|--:|
  | forms_upto | lower only / upper only / both | −0.5 / 0.0 / +0.3 % | +0.1 / +0.2 / +0.1 % |
  | field_digest | lower only / upper only / both | −1.4 / +10.8 / +10.8 % | −14.3 / +16.1 / −2.6 % |

  `forms_upto` never calls `sort_keys` (callgrind, collecting inside `n_forms_upto` only): D1's
  attribution came from a whole-binary profile, where `field_digest`'s samples landed.  In
  `field_digest`, `sort_keys` is 85 % of the instructions and the no-test form cuts them 36 %
  (604 M → 384 M), yet the time moves ±15 % in both directions, which is layout. The sort is
  bound by its data-dependent compare, not by the tests.  `field_digest` is under 2× anyway.

The guard's own lower bound (`best >= 0 and best < len(used)`, crawler `sort_floats`) is not
built either: its one guarded write runs once per outer turn beside an n² loop (§ Both
spellings).
