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

The older binary is any build: `git worktree add --detach <dir> <tag>`, then `cargo build
--release --bin loft` there.  Run the walk at each release against the one before it.

## The issue walk — past reports judged against the tables

Every `silent-wrong` issue filed since that release, 78 of them, was read against the
tables as they stood on 2026-11-19 and given one verdict.  The ledger is
[reach.tsv](reach.tsv), one row an issue.

| Verdict | Issues | Meaning |
|---|---|---|
| `fits-now` | 3 | the tables already hold the slot and the filler (loft#2573, loft#2571, loft#2577) |
| `fits-rows` | 23 | one statement with one operand form, lacking a slot row (21), a filler row (15), a prelude helper (11) or an operand type (6) |
| `needs-axis` | 43 | it turns on something the tool did not vary |
| `outside` | 9 | not about evaluating an operand: a missing warning, a library binding, lexing |

So a third of what was found by hand is table work, and over half needs an axis.  The axes
the 43 name:

| Axis | Issues | State |
|---|---|---|
| statement context — in a loop, in a branch, in a closure, after an early return | 10 | **added**: `scripts/syntax_contexts.tsv` (`plain`, `loop`, `branch`, `else`); a closure and an early return are not in it yet |
| a sequence of statements — bind a view, disturb its source, read the view | 10 | open; see below |
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

## What the walk asks for next

1. **The 23 `fits-rows` issues**, as rows.  Each is a cell that must fail on the build that
   had the defect, which the time walk checks without reading anything.
2. **A sequence axis.**  Ten issues are "bind a view, disturb, read".  The prelude can bind
   views of its values before the statement and show them after it, which reaches the ones
   where the operand's form matters; the rest need hand-written expectations.
3. **A second operand.**  Seven issues need two counted operands, one of which writes what
   the other reads.  That wants fillers that write the prelude's own values, and a twin
   that binds every operand in source order, not only the slot.
4. **More worlds.**  Seven issues turn on a declaration.  A second prelude — nullable
   fields, a struct-enum, a generic — multiplies the matrix once more.

The ledger is redone at each release: the new `silent-wrong` issues are judged, and the
share that `fits-now` is the matrix's reach on defects it was not built from.
