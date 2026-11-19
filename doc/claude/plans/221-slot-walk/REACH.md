<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk: reach, measured against past defects

The slot matrix ([the plan](README.md)) finds what its tables span and nothing else.  This
file is how that span is measured and widened: the defects already found by other means are
walked back through the matrix, and each one it cannot express says which table or axis to
add.  Two instruments, one mechanical and one read.

## The time walk — the matrix on an older compiler

```bash
python3 scripts/slot_matrix.py walk --old <an older loft binary> --backend both
```

The same cells run on the older binary and on this one.  A cell that failed then and passes
now is a defect the matrix would have caught before it was reported; a cell that passed
then and fails now is a regression.  No issue is read: the compiler is the witness.

Against the release of 2026-11-03 (`v2026.11.0`, built in a worktree), 2,893 cells, both
backends:

| Group | Cells | What they are |
|---|---|---|
| failed then, pass now | 55 | eight folds: the `for` and filtered-`for` source run once per step (loft#2573), the three forms of the collection append through a path (loft#2571), a `??` over a keyed collection, and three leaks through a field of a call |
| pass then, fail now | 17 | three regressions since the release: an element write whose index is a function-value call is lost (the fix of loft#2503), a lookup in a tuple member built from a literal frees a store twice (loft#2576), and `for n in held(c, h).v[i..j]` frees a store twice |
| fail on both | 48 | nine folds the release already had |
| refused then, accepted now | 0 | the prelude and every cell compile on the release |

Against the release of 2026-10-01 (`v2026.10.0`), 3,657 cells, the interpreter:

| Group | Cells | What they are |
|---|---|---|
| failed then, pass now | 109 | chiefly ONE order defect that was in twelve slots: a field of a call's record as the SECOND operand of a binary operator or the second argument of a call ran before the first.  The right side of an element write (found.md's row 12) is the one instance of it still open |
| pass then, fail now | 45 | the lost element write and the two double frees that the November walk also shows, plus an order and a leak fold that October did not have |
| fail on both | 59 | folds two months old: the comprehension source, the place of a collection assignment, the one-statement block, the generator and `parallel` crashes |
| refused then, accepted now | 89 | syntax October did not have (`**`, among others) |

**How far back the walk reaches is set by the prelude.**  October's compiler wanted enum
variants qualified (`Col.Green`), so the prelude spells them that way; two slots are vacuous
there because their syntax is newer.  A walk further back needs the prelude checked on that
build first — `slot_matrix.py run` on it says at once whether it compiles.

The older binary is any build: `git worktree add --detach <dir> <tag>`, then `cargo build
--release --bin loft` there.  Run the walk at each release against the one before it.

## The issue walk — past reports judged against the tables

Every `silent-wrong` issue filed since that release, 78 of them, was read against the
tables as they stood on 2026-11-19 and given one verdict.  The ledger is
[slot_reach.tsv](../../../../scripts/slot_reach.tsv), one row an issue.

| Verdict | Issues | Meaning |
|---|---|---|
| `fits-now` | 3 | the tables already held the slot and the filler (loft#2573, loft#2571, loft#2577) |
| `fits-rows` | 23 | one statement with one operand form, lacking a slot row (21), a filler row (15), a prelude helper (11) or an operand type (6) |
| `needs-axis` | 43 | it turns on something the tool did not vary |
| `outside` | 9 | not about evaluating an operand: a missing warning, a library binding, lexing |

So a third of what was found by hand is table work, and over half needs an axis.  The axes
the 43 name:

| Axis | Issues | State |
|---|---|---|
| statement context — in a loop, in a branch, in a closure, after an early return | 10 | **added**: `scripts/syntax_contexts.tsv` (`plain`, `loop`, `branch`, `else`); a closure and an early return are not in it yet |
| a sequence of statements — bind a view, disturb its source, read the view | 10 | **tried, and it found nothing**: the prelude's `views` frame binds four views before the statement and reads them after it, and no cell fails there that does not fail without them.  These defects are wrong with a plain variable too, so they want sequences and expected values written by hand |
| the shape of a declaration — a field modifier, a nullable field, a linked group, a generic | 7 | open: needs more than one prelude |
| two operands of one statement interacting — an index that writes the vector it indexes | 7 | open: needs a second counted operand and a twin that binds every operand in source order |
| how the receiver was built — a parameter, a `&` parameter, a literal, a tuple member | 5 | **partly added**: the prelude's `params` frame; a `&` frame is refused wherever the statement does not write through it |
| the element type — a narrow integer, a text, a nested vector | 2 | open |
| the rustc writer; a buffer lent down two frames | 2 | not planned |

**The judged verdicts are a reading, not a measurement.**  Three readers each took a third
of the issues, from the issue text alone and without running anything.  A `fits-rows`
verdict becomes a fact only when the row is written and the cell fails on a build that has
the defect — as was done for loft#2577 and loft#2576, with eight rows.

**Where the twin cannot be the oracle.**  The readers marked eight issues in which the
statement is as wrong with a plain variable as with any other operand, or the bound-first
form is itself the failing one.  The matrix compares a statement with its twin, so it sees
a defect only where the FORM of the operand changes the answer.  A defect that is wrong for
every form needs an expected value written by hand, which is what a suite cell is for.

## Calibration — how far a judged verdict can be trusted

Five of the 23 `fits-rows` verdicts were written as cells with `slot_matrix.py cell` and run
on the release of 2026-11-03, the build of 2026-11-18 and the branch.

| Issue | Outcome |
|---|---|
| loft#2517 | confirmed: the cell fails on the release and passes after; now the slot `op-concat-tail` |
| loft#2409 | confirmed the same way; now the slot `ct-if-empty-arm` |
| loft#2407 | confirmed as a refusal: the release refuses the cell, later builds accept it; now the filler `vint-if-empty` |
| loft#2470 | not reachable: `e[5] = e[99]` is as wrong with the lookup bound first, so the twin agrees with the defect.  Its reader had said so in the note |
| loft#2561 | not rows: it turns on the record also being the element of a keyed collection, which is the declaration-shape axis |

Three of five held, and the two that did not were the ones whose notes already carried a
doubt.  So a `fits-rows` verdict with an unqualified note is a fair guide, and one with a
caveat should be read as `needs-axis`.  The other 18 are not calibrated.

Writing those five also found two faults in the tool, since fixed — a function declared by
a cell collided with the same declaration in the next cell, and the collection kind was not
substituted inside a declaration — and one missing feature, now there: `<BIND>` in a slot's
declaration marks where the twin binds the operand, which gives a slot inside a function
its bound-first twin.

## The two axes this added

Both are tables or prelude sections, off unless asked for (`--context all --frame all`).

| Run | Cells | Time, both backends |
|---|---|---|
| the matrix, one context and one frame | 2,893 | 39 s |
| four contexts and two frames | 23,144 | 5 min 33 s |

Of the 23,144 cells, 22,016 pass, 580 are refused and 548 fail, in 17 folds.  The folds the
bare statement does not show:

- a `for`, a filtered `for` and a `??` over a keyed collection that is a field of a call or
  a `?? []` leave a store behind when the statement stands in an `if` or an `else` branch,
  on both backends, with a plain variable as well as with any filler;
- the member of a call's tuple under `??` is a wrong value where its record is a local and
  a store freed twice where it is a parameter.

## What the walk asked for

Its steps are G1 to G9 of [queue.md](queue.md), with what each built; what none of them
finished is a row of [SLOT_MATRIX.md § Open work](../../SLOT_MATRIX.md#open-work).

## The standing practice — every new defect is a row or a ledger line

The ledger is not redone in batches.  From here on each compiler defect is held against the
matrix when it is filed or fixed, by whoever does that: reached, reached with new rows
(which must fail on the build that has the defect), or a ledger line naming the axis it
needs.  The rule and its four cases are in
[ISSUE_TRACKING.md § Every defect is held against the slot matrix](../../ISSUE_TRACKING.md#every-defect-is-held-against-the-slot-matrix).

Two numbers come out of that, read at each release: the share of the cycle's defects the
matrix reached as it stood (its reach on defects it was not built from), and the axis with
the most ledger lines, which is the next one to build.
