<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk: the queue

The ordered work of [the plan](README.md).  The tooling comes before the fixes: a wider
matrix finds more with every run, and each fix is then checked by cells that already exist.

## The first queue — reach

1. **G1 — the baseline and the gate** (this is phase F, taken first) — **built.**
   `scripts/slot_matrix.baseline` lists the 179 cell lines that do not pass today, and
   `slot_matrix.py gate` (run by `tests/slot_matrix.rs`, 41 s) fails on a cell that is not
   ok and not listed, and on a listed cell that passes again; a crash line is marked
   unstable and may pass.  *Verified:* against the release of 2026-11-03 the gate reports
   120 new cells and 41 lines to remove.  The gate is split by slot family
   (`gate --family F`, seven tests): a red test names the family, its output says first
   whether something is BROKEN or only FIXED, the failing cells are folded toward causes,
   and on GitHub each fold is an annotation on its row of the table.  Still to learn:
   whether the baseline is the same on Linux.  The four contexts and two frames are not in the gate; they are a nightly
   candidate at five and a half minutes.
2. **G2 — the ad-hoc cell** — **built.**  `slot_matrix.py cell '<statement with <S>>'`
   with `--type T` (every filler of a type) or `--filler EXPR --plain EXPR` runs one shape
   on both backends with all four checks, and prints the table rows the shape would be.
   The agent that meets a defect then adds the row at that moment.  *Verified:*
   `cell 'for p in <S> { s += p.k; }' --type coll` fails on the release of 2026-11-03 (the
   operand runs five times) and passes on the branch.
3. **G3 — calibrate the reading** — **begun.**  Five of the 23 `fits-rows` issues of
   [reach.tsv](reach.tsv) were written as cells: three fail on the build that had the
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
9. **G9 — a probe, before it is planned:** which IR operations the corpus produces and no
   cell of the matrix does.  If `loft introspect` gives that histogram cheaply, the gaps
   name missing rows without anyone reading an issue.
10. **H — a rule for every slot.**  A slot whose `rule` column is `-` is an operand whose
    evaluation count no formal rule states: the `while` condition, a field read, the three
    text reads, `is`, the two `assert` operands, the two record-literal fields
    (`slot_matrix.py list` names them).  For each: read the chapter it belongs to, and
    either cite the rule that already covers it, or write the rule — how often the operand
    runs, in what order with its neighbours, and under which condition it is skipped — and
    cite that.  The same for every question a run raises that the text does not answer
    (the `??` fallback was the first: settled as evaluated only when needed, and written
    into `(E-Coalesce)`).  A new rule is the owner's to approve; the slot that cites it is
    its guard from the first run.  *Verify:* `slot_matrix.py list` reports no slot without
    a rule, and `rule_tags.py check` resolves every citation.
11. **C — the fixes** are handed off: [found.md](found.md) is the list, the lost write
    first, and each fix removes its lines from the gate's baseline.
12. **E — the cells as input to the two-implementation comparison**, when the port's
    harness takes generated programs.

## The second queue — what the generator can claim

An evaluation of the generator as it stood with G1 to G8 built.  The first two are
weaknesses in what a green run means; the rest is breadth.

13. **K1 — an anchor** — **built.**  `scripts/syntax_expect.tsv` holds, for 134 of the 135
    slots, the fields the slot's PLAIN statement must leave changed in the prelude's world;
    everything else must stay as the untouched world has it.  Each value was computed by
    hand from the statement and checked against what the compiler answers (`expect
    --propose` prints that answer as a proposal, never as the expectation).  Nine
    statements whose plain result could not be told from "nothing happened" were
    strengthened first.  `wr-remove-index` has no row: what `v[i] = null` does to a dense
    vector is a question for arc H.  *Verified:* `op-add-left`'s expectation edited to 5
    fails its plain cell on both backends.
14. **K2 — breadth of type.**  Of 135 slots 80 take an integer, 2 a float, 2 an enum, 1 a
    function value; every operator is tried with integers only.  A slot row that names
    several operand types runs the same statement over float, text and the narrow integer
    fields.  *Verify:* the IR operations of the corpus that no cell emits (G9) fall.
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
17. **K5 — rule coverage.**  The evaluation rules of the formal chapters that no slot cites,
    listed: missing slots found without reading an issue.  It feeds arc H.
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
