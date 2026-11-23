<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 225 — The rewrites only the rustc writer applies

**The criterion (owner, 2026-11-21):** a rewrite the rustc writer applies and nothing outside it
does is removed with the writer, everywhere its decision lives — the IR pass, the interpreter
and the C backend included.  A rewrite the C backend or the interpreter also applies stays, and
loses only its rust half.

**The instrument:** `python3 scripts/rule_phases.py` over the rewrite census
(`bench/portal/rewrite_census.tsv`, 61 bench programs on each writer).  `decided in` is where a
rule's decision code lives; `applied by` is which phases admitted it at least once.

## Remove: applied by the writer alone (22)

| Rule | Decided in | Programs / admissions | What it does |
|---|---|---|---|
| `R-Leaf` | rust | 61 / 10 962 | A function that calls no user function or fn-ref is emitted without a shadow-stack frame or fn-ref guard. |
| `R-LitDiv` | rust | 55 / 2 762 | A division or remainder by a literal other than 0 and -1 is one sentinel test and the plain operator. |
| `R-LeafChain` | ir, rust | 61 / 6 540 | In the lean tier, a function whose whole call tree is frameless is emitted as a leaf. |
| `R-TextClear` | interp, rust | 61 / 3 545 | `x = ""` on a text local whose native slot is a `String` empties it in place, keeping its capacity. |
| `R-Header/kept` | ir, interp, c, rust | 30 / 3 610 | A vector's header held across a loop in a kept clause. |
| `R-Header/function` | ir, interp, c, rust | 24 / 141 | The same, held for a whole function. |
| `R-RefillBuffer` | ir, interp, rust | 38 / 378 | On `--native`, the store an `H-SwapRebind` releases is kept for the next refill instead of freed. |
| `R-RecPtr/nested` | ir, c, rust | 25 / 228 | A record view's address taken once from a held base, through nested records. |
| `R-RecPtr/index` | ir, c, rust | 5 / 40 | The same, from the held base by index. |
| `R-GroupPush` | ir, rust | 12 / 173 | A mint group of n new records on one plain vector reserves once. |
| `R-PushFill/record` | ir, interp, c, rust | 16 / 65 | A counted loop pushing records to one path reserves its pushes once. |
| `R-BoundedNest` | ir, interp, c, rust | 4 / 20 | An innermost counted loop whose body is one accumulate over `?`-discharged reads runs raw inside a guard. |
| `R-Refresh` | ir, c, rust | 4 / 16 | A held fact changed inside a loop is refreshed at the admitted op's own site. |
| `R-RefillText` | ir, rust | 6 / 13 | A pooled call buffer's release before a text refill is skipped. |
| `R-FnRefValue` | ir, c, rust | 1 / 11 | A call through a fn-ref answering a value record returns its tuple when every candidate is admitted. |
| `R-FaultFlag` | ir, interp, rust | 5 / 7 | A `for` loop whose nodes cannot fault carries one fault flag instead of a check per node. |
| `R-StreamWalk` | ir, rust | 1 / 2 | `v = P(…); for i in 0..len(v)` runs as P's stream twin with the loop body as its closure. |
| `R-FoldCompare` | ir, rust | 1 / 1 | A text predicate over a case fold compares without building the folded text. |
| `R-LitHoist` | ir, rust | 1 / 1 | An invariant loop-body vector literal builds once per activation. |
| `R-MoveAppend` | ir, rust | 1 / 1 | `for f in call() { V += [f] }` moves each minted element instead of copying it. |
| `R-ViewReturn` | ir, rust | 1 / 1 | A function returning a materialised view gets a twin answering (address, owned). |
| `R-RebindBuffer` | rust | 0 / 0 | A rebind `x = f(…)` in a loop adopts the fresh result and hands `x`'s old store to the call's buffer.  Admitted on no bench program, so the census does not list it; the C and the interpreter make and free the same stores without it (STORE_MODEL D33). |

**Where the C backend has code for them** (`decided in` names `c`, yet the C backend admitted
none on 61 programs): `R-Header/*`, `R-RecPtr/*`, `R-PushFill/record`, `R-BoundedNest`,
`R-Refresh`, `R-FnRefValue`.  That C code is dead on the benches.  R5 removes it after the
corpus-wide check below shows it dead there too.

**Not kept for a future C version.**  Several of these would matter to the C backend
(`R-StreamWalk`, `R-ViewReturn`, `R-MoveAppend` and `R-FnRefValue` avoid an allocation or a copy).
A C version is a new rule, designed against the C census and justified by a C measurement —
not this code kept alive without a backend that applies it.

## Keep, losing only the rust half

- **45 rules applied by `c,native`** (and the 2 `parse,c,native`, 2 `interp,c,native`,
  1 `ir,c,native`): the C backend applies them.  Their rust half disappears with
  `src/generation/` in R7; their formal definitions lose the native wording in R8.
- **`R-SliceBorrow`** (`interp,native`): the interpreter applies it.
- **`R-ExitVector`, `R-ExitVector/record`, `R-ForwardResult`, `R-MoveChild`**: the IR pass or
  the parser applies them (`decided in` reads `-` only because no census site names a phase);
  untouched.

## The check before deletion (R5)

The census covers the 61 bench programs, not every program.  Before a rule's IR, interpreter
or C code is deleted, two exact comparisons hold on the whole corpus:

1. **No C admission anywhere.**  `scripts/c_sweep.py` over the corpus with the census on
   (`LOFT_REWRITE_CENSUS`) admits none of the 22 in the `c` phase.  Positive control: a kept rule
   in the same run (`R-Base`) shows admissions, so a silent census is not a blind one.
2. **The C emission does not move.**  The corpus's emitted C is byte-identical before and after
   removing each rule's IR, interpreter and C code: removing a decision the C backend never
   acted on changes no unit.

A rule that fails either check is not writer-only after all, and moves to the keep list with
the measurement that put it there.
