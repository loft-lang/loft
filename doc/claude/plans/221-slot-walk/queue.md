<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk: the queue

The ordered work of [the plan](README.md).  The tooling comes before the fixes: a wider
matrix finds more with every run, and each fix is then checked by cells that already exist.

## The first queue — reach

1. **G1 — the baseline and the gate** (this is phase F, taken first) — **built.**
   `scripts/slot_matrix.baseline` lists the cell lines that do not pass today, and
   `slot_matrix.py gate` fails on a cell that is not ok and not listed (BROKEN), on a
   listed cell that passes again (FIXED), and lists a cell the compiler newly refuses; a
   crash line is marked unstable and may pass.  `tests/slot_matrix.rs` runs it as seven
   tests, one a slot family, with each fold an annotation on its table row on GitHub.
   *What CI taught.*  The baseline holds on Linux.  But compiling every cell costs a tenth
   of a second each, 96% of a run, and on a CI runner the whole matrix on both backends
   ran past the per-test limit both as one test (421 s) and as seven beside one another
   (four at 600 s).  So the gate runs every cell on the interpreter and a SAMPLE on the C
   backend — each slot with its plain variable and its type's first filler — in 15 s here;
   `gate --full` is the whole matrix on both, and is what rewrites the baseline.  The
   full run, the four contexts, the frames and depth two are a nightly leg not yet
   scheduled.  The test had also carried a `#[cfg(unix)]` and a `std::fs` read, which the
   platform census and Clippy refuse.  With all of that changed the gate is green on
   Linux: the seven tests pass on the runner in 16 s to 68 s each, with Clippy, the doc
   hygiene suite, ASan and the stack-alignment sweep.
2. **G2 — the ad-hoc cell** — **built.**  `slot_matrix.py cell '<statement with <S>>'`
   with `--type T` (every filler of a type) or `--filler EXPR --plain EXPR` runs one shape
   on both backends with all four checks, and prints the table rows the shape would be.
   The agent that meets a defect then adds the row at that moment.  *Verified:*
   `cell 'for p in <S> { s += p.k; }' --type coll` fails on the release of 2026-11-03 (the
   operand runs five times) and passes on the branch.
3. **G3 — calibrate the reading** — **begun.**  Five of the 23 `fits-rows` issues of
   [slot_reach.tsv](../../../../scripts/slot_reach.tsv) were written as cells: three fail on the build that had the
   defect and are rows now, two do not fit after all ([REACH.md § Calibration](REACH.md)).
   Eighteen are left; each is a `cell` command first and a row only when it fails on the
   older build.
4. **G4 — fillers built from fillers** (this is phase D) — **built.**  A row of
   `syntax_fillers.tsv` with a `<I:type>` hole is a wrapper, generated at `--depth 2` once
   around every other filler of that type: 18 wrappers make 15,058 cells, 189 s on both
   backends.  A wrapper around a filler that fails the same way folds into that filler's
   line, and a wrapper that fails around most fillers is a line of its own.  Depth two is
   not in the gate.  What it showed: the two order defects are about an ELEMENT READ or a
   FIELD READ as the operand, whatever stands inside it; the slice of a call's field leaks
   in six slots, not one.
5. **G5 — an operand that touches what the statement works on** — **built in part.**
   Seven fillers answer the statement's own record, vector or collection through a call
   (`own(c, r)`, `ident(c, vi)`, `idc(c, xs)`), and four WRITE it and answer 1 (`poke`,
   `grow`, `renum`, `retext`).  A writing filler changes the answer, so it goes only into
   the twenty slots marked `first`: exactly once, and before the statement reads anything
   else, where binding first is what the rules say.  It showed the lost element write for
   every index that writes its vector, not only a function-value call.  *Open:* a twin
   that binds EVERY operand in source order, with collection operands copied where the
   rules read them by value, which is what a writing operand in a READ slot needs
   (loft#2503's own case, `v[set(v)]`).
6. **G6 — views across the statement** — **built, and it found nothing.**  The prelude's
   `views` frame binds an element view, a collection-field view, a slice and a keyed
   element before the statement and shows them after it (`--frame views`): 3,657 cells,
   none failing that does not fail without the views.  The ten "bind, disturb, read"
   issues are wrong for every form of operand, so the matrix is the wrong instrument for
   them; they stay hand-written suite cells.
7. **G7 — a second prelude**: nullable fields, a struct-enum, a generic, for the seven
   declaration-shape issues.
8. **G8 — the older walks** — **the time walk done.**  Against `v2026.10.0` the matrix
   shows 109 cells fixed since, most of them one order defect that stood in twelve slots
   ([REACH.md](REACH.md)).  The prelude had to spell its enum variants qualified to compile
   there.  *Open:* the SAMPLE of about thirty of October's `silent-wrong` issues, to see
   whether the rule walk's findings name an axis November's did not.
9. **G9 — which IR operations the corpus produces and no cell does** — **measured once.**
   `loft introspect` over the 3,275 corpus files and over every accepted cell (all
   contexts and frames, depth two): the corpus uses 267 distinct operations, the cells 106,
   so 161 were never emitted by the matrix.  The largest gaps were float, `single` and
   character arithmetic and comparison, narrow integer field reads and writes, boolean
   and enum pushes, hash and vector removal, `trie` and `spatial`, and files — which is
   what K2 took its slots from.  The probe is a scratch script, not yet a command; two of
   its three runs were invalid (a cell's function not reachable from a `main`; a program
   holding a refused cell), which is the reason to make it one.
10. **H — a rule for every slot** — **done.**  All 191 slots cite a formal rule and have an
    anchor.  Written for it, each probed on both backends before it was stated and cited
    by the lowering that implements it: `(E-Once)` — an operand of any form is reduced
    once, the exceptions named; `(E-Field)`, `(E-Index)` over a text, `(E-Is)`,
    `(E-Record)` — fields in the order WRITTEN; `(I-While)` — the condition before every
    round; `(E-Assert)` — the message only when the assertion fails; and the `??` fallback
    in `(E-Coalesce)`.  One slot had cited the wrong rule: `v[i] = null` on a vector is
    `(N-Store)`, a null stored, not a removal.  The five defects the fixing agent filed
    (loft#2578 to loft#2582) are open deviations of the rules they break, `D-op-23` to
    `D-op-26` and `D-iter-17`.  *Owed:* a cell in the language suite under each new rule;
    today the slot matrix is their guard.
11. **C — the fixes** are handed off: [found.md](found.md) is the list, the lost write
    first, and each fix removes its lines from the gate's baseline.
12. **E — the cells as input to the two-implementation comparison**, when the port's
    harness takes generated programs.

## The second queue — what the generator can claim

An evaluation of the generator as it stood with G1 to G8 built.  The first two are
weaknesses in what a green run means; the rest is breadth.

13. **K1 — an anchor** — **built.**  `scripts/syntax_expect.tsv` holds, for all but one of the
    slots, the fields the slot's PLAIN statement must leave changed in the prelude's world;
    everything else must stay as the untouched world has it.  Each value was computed by
    hand from the statement and checked against what the compiler answers (`expect
    --propose` prints that answer as a proposal, never as the expectation).  Nine
    statements whose plain result could not be told from "nothing happened" were
    strengthened first.  `wr-remove-index` has no row: what `v[i] = null` does to a dense
    vector is a question for arc H.  *Verified:* `op-add-left`'s expectation edited to 5
    fails its plain cell on both backends.
14. **K2 — breadth of type** — **begun.**  Forty slots for `float`, `single`, `character`,
    the narrow integer fields (`u8`, `i16`, `i32`), `boolean` and enum elements, each with a
    hand-computed anchor; eleven fillers, three of which read a narrow field in every
    integer slot.  175 slots and 96 fillers now, 4,772 cells in the gate.  It found no new
    cause: the right side of an element write running before its index (found.md's row 12)
    holds for every element type.  The statements are written out per type; a slot row
    that names several types, with a partner and a result variable per type, is the open
    half.  *Measured after it* (`coverage ops`, depth one, one context): the cells emit
    133 of the corpus's 267 IR operations, half, where 106 were emitted before by a wider
    run.  What is still never emitted: clearing a collection (`OpClear`, `OpClearKeyed`),
    hash and vector removal, text inequality, boolean equality, a `single` field read and
    push, a character read, `trie` and `spatial`, files, casts from text.
15. **K3 — a failing cell laid open** — **built**, as `slot_matrix.py explain <cell>`.  It
    keeps one program holding the cell, its bound-first twin, its order cell and the plain
    variable; prints each statement; runs the cell alone on both backends with loft's own
    diagnostics on (`LOFT_REFREE_FAILS` names the line and variable of each free,
    `LOFT_LEAK_SITES` the leak); shows every field that differs with both values; and diffs
    the IR of the cell against the IR of its twin — the working form beside the broken
    one, where a compiler fix starts.  Every failing line of any run now carries its
    statement and, for a value failure, the field and both values; the summary names the
    build.  *Verified:* on the lost element write the IR diff is the `__lift_1` copy and
    nothing else; on the slice double free the interpreter names both frees.
16. **K4 — sampled depth three.**  A seeded random sample of three-deep compositions, a few
    thousand a night; the whole of depth three is over 60,000 cells.
17. **K5 — coverage, two ways** — **built**, as `slot_matrix.py coverage`.
    `coverage rules` lists, per formal chapter, the evaluation rules no slot cites: 48
    rules are cited by the 175 slots and well over a hundred are not.  Many of those are
    not about an operand, but the list names missing slots outright — truthiness in a
    condition (`E-Truthy`), a null argument (`E-NullArg`), the combinators (`I-Map`,
    `I-Reduce`), an open-ended range, a null source, tuple destructuring and projection
    (`T-Destr`, `T-Proj`), the patterns of `match`, `yield from` (`G-Delegate`), a default
    field expression, `v.remove(i)`.  `coverage ops` is G9's probe as a command: the IR
    operations the corpus makes the compiler emit and no cell does, the corpus read once a
    build.  Its first use on arc H found that an `assert`'s message is evaluated only when
    the assertion fails, which `GUARDS.md` taught the other way round.  Sixteen of the
    missing slots are added since (truthiness, the three combinators, an open-ended range,
    destructuring, two `match` patterns, `v.remove(i)`, text and boolean comparison), and
    eleven slots whose statement reads a collection of the cell's kind now run in every
    kind, with an anchor a kind: 191 slots and 6,061 cells in the gate.  No new cause; the
    gate's report gained a block for cells the compiler newly refuses, which is how the
    combinators' refusal of a slice came up.
18. **K6 — two counted operands** in one statement, with a twin that binds them in the order
    the rules give for that construct.  The costliest step (it is G5's open half).
19. **K7 — the twelve unreached lexemes**: a file in the prelude, and a cell kind that
    expects the program to end (`panic`).
20. **K8 — more contexts**: a closure body, an early return, a match arm.
21. **K9 — more backends at night**: the rustc writer and wasm, too slow for the PR gate.
22. **K10 — a second prelude** for the shape of a declaration (this is G7).

## Built beside the queues

- **`expr`** — the mirror of `cell`: one EXPRESSION tried in every slot of its type, in the
  contexts and frames asked for, and at depth two both ways round (inside every wrapper
  with a hole of its type; around every filler when it has a hole of its own).  It first
  checks the expression is usable — one bump of its counter, its type's plain value.
- **`--bg FILE`** — any run in the background, its output in FILE, ending with the line
  `slot_matrix: done exit=<n>` that `scripts/wait_for.sh` waits for.
- **A cell that loops is stopped by loft itself.**  Every generated program runs under
  `LOFT_TIMEOUT`; the interpreter names the line it was on, which is the cell, and the C
  backend fails that one test.  What is left of the program then runs under a tight limit,
  so a statement that loops for every form of operand costs seconds a form, not a limit
  each.  This is what bounds a random or a depth-three run, whose cells nobody has read.
