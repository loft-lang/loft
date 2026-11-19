<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk

Tracker: [@PLN221](https://github.com/loft-lang/plans/issues/221).

## Status

**SHIPPED 2026-11-19.**  The generator, its gate, the rules it guards and the tools around it
are built; what is left is standing work, kept as rows of the reference doc.

- **The reference:** [SLOT_MATRIX.md](../../SLOT_MATRIX.md) — the registers, what a cell
  asserts, the commands, the gate, how to write a row, and § Open work.
- **The method:** [STABILITY_METHOD.md § The slot matrix](../../STABILITY_METHOD.md#the-slot-matrix--the-walk-that-is-generated).
- **The rules written for it:** `(E-Once)`, `(E-Field)`, `(E-Index)`, `(E-Is)`, `(E-Record)`,
  `(E-Assert)` in [formal/operational.md](../../formal/operational.md), `(I-While)` in
  [formal/iteration.md](../../formal/iteration.md).
- **The defects it reported**, for whoever fixes them: [found.md](found.md).  Five are filed
  (loft#2578 to loft#2582) and stand as open deviations of the rules they break; each fix
  removes its lines from `scripts/slot_matrix.baseline`.

This directory is the record of how it was built: [queue.md](queue.md) the steps in order,
[REACH.md](REACH.md) the walks back over two releases and 78 past issues,
[built.md](built.md) the first two phases, [pilot/](pilot/README.md) the probe it started from.

## Goal

A generated matrix that puts every form of expression into every operand position of every
construct, over the type kinds that position accepts, and asserts that each operand runs the
number of times the formal rules say, in the order they say, with the result its bound-first
twin gives — on the interpreter and the C backend.

## Effort + design

- **Effort:** MH (the tooling is S; the walk is as long as its yield)
- **Design:** ✓

## The invariant

One sentence carries the whole plan:

> An operand written once in the source is evaluated once per execution of its construct,
> in source order, whatever form of expression it is and whatever type it has — except
> where a rule names a different count.

The rules already say this, per construct: `(E-Left)`, `(F-Args)`, `(E-Asgn)`,
`(E-Asgn-Compound)` ("`place` is addressed EXACTLY ONCE"), `(I-For)` ("the source is
evaluated ONCE").  What they have never had is a check that crosses them with the FORM of the
operand.  A construct tested with a plain variable in its operand position is evaluated once
by construction; the same construct with a call, a keyed lookup or a `?? default` there is a
different lowering, and it is the one that fails.

The failures give the right answer.  loft#2573's loop visited the right elements and summed
them correctly while its source ran five times.  A value assertion cannot see this class;
only a counter in the operand can.

### Re-assertion sites

The invariant is re-asserted wherever a lowering copies an operand's expression instead of
its value: one site per construct per collection kind per phase.  Omission is silent (a
right answer).  That is the `N × silence` shape [DESIGN_PROTOCOL.md](../../DESIGN_PROTOCOL.md)
names as brittle, and the walk's second product, after the defects, is the list of those
sites — the input to a later decision on ONE place that binds an operand before a lowering
may mention it twice.

## The pilot

The probe the design was written on — 114 hand-listed cells over ten slots, counts only —
is recorded in [`pilot/README.md`](pilot/README.md) with its two scripts.  It reported
loft#2573 and loft#2571 on `main` without being told of them, and two of the causes in [built.md](built.md) on
the branch that carries both fixes.

## Sub-arcs

| Item | What it builds | Verify — what goes red if it is done wrong | Status |
|---|---|---|---|
| **A** — the slot register | `syntax_slots.tsv`, `syntax_fillers.tsv`, `syntax_outside.tsv`, each slot citing the rule that states its count | `doc_hygiene`'s `every_lexeme_has_an_operand_slot_or_a_reason`: every lexeme the lexer and the parser know is in a slot or has a reason; `selftest` removes a row, adds a lexeme and misspells a rule, and each must be reported | Shipped — the eleven rule-less counts and the twelve unreached operands are carried into C |
| **B** — generator and runner | `scripts/slot_matrix.py run --backend both`, `--slot`, `--kind`: the twin, the order trace, the leak channel, the batched run on both backends; `make slot-matrix` | run against a build that has loft#2573: it reports those cells, and none on the fixed build.  Recorded as the tool's `@falsified-at` | Shipped |
| **C** — the fixes | each failing cause fixed at the lowering that copies the operand, one named suite cell per cause under its rule | the cause's cells pass and leave the baseline | Handed off — [found.md](found.md); the fixing agent files and fixes them row by row |
| **D** — depth two | fillers nested once (`--depth 2`, wrapper rows with a `<I:type>` hole) | `fillers --depth 2`: every composed filler bumps its counter once and gives its type's plain value | Shipped (G4) |
| **E** — the cells as lockstep input | every generated cell fed to the Rust-against-loft comparison | a difference in any layer on any cell | Deferred — a row of [SLOT_MATRIX.md § Open work](../../SLOT_MATRIX.md#open-work), due when the port's harness takes generated programs |
| **F** — the gate | `tests/slot_matrix.rs`: seven tests, one a slot family, against a baseline that may only shrink; the wider axes nightly | a new failing cell on a PR (BROKEN); a baseline entry that passes again and was not removed (FIXED) | Shipped — every cell interpreted, a sample compiled; the nightly full run is a row of the reference's open work |
| **G** — reach | the matrix walked back over past defects, and every axis and row that asked for | a row added for a past issue fails on the build that had the defect | Shipped — two time walks, the ledger, contexts, frames, operands that touch the statement, forty typed slots; what is left is rows of the reference's open work |
| **H** — a rule for every slot | the formal text evaluated wherever a slot can cite no rule, or a run raises a question the text does not answer: the rule found and cited, or written | `slot_matrix.py list` names no slot without a rule; `rule_tags.py check` resolves every citation; the slot is the new rule's guard | Shipped — every slot cites a rule.  Written for it: `(E-Once)`, `(E-Field)`, `(E-Index)`, `(E-Is)`, `(E-Record)`, `(I-While)`, `(E-Assert)`, and the `??` fallback in `(E-Coalesce)`; each cited by the lowering that implements it.  Five open deviations name the defects the matrix reports (loft#2578 to loft#2582) |

## Design questions, as they were settled

1. **Which backends.**  The interpreter and `--native-c`; the rustc writer and wasm are a
   row of the reference's open work.
2. **A ledger.**  None for the cells — the run is the record, the baseline its only state —
   and one for reach: `scripts/slot_reach.tsv`.
3. **One binding place.**  Not decided here: `(E-Once)` names where a lowering binds an
   operand today (`scopes::args`), and the five open deviations are the sites that do not.
4. **Syntax the register cannot reach** is listed with its reason in
   `scripts/syntax_outside.tsv`, twelve lexemes of it as owed.

## Cross-arc dependencies

- [@PLN91](https://github.com/loft-lang/plans/issues/91) — the port's comparison harness is
  phase E's consumer, and the port is what found the defects that started this.
- [@PLN193](https://github.com/loft-lang/plans/issues/193) — the named cells of phase C land
  in the language suite, under their rules.
- [@PLN211](https://github.com/loft-lang/plans/issues/211) — a lowering that copies an
  operand is a rewrite; a site phase C fixes is a candidate row of its applied-by register.

## Documentation

Every line is done.

- `SLOT_MATRIX.md` — the reference, new with this plan; indexed in `CLAUDE.md`.
- `STABILITY_METHOD.md` § The slot matrix; `TESTING.md` § The slot matrix; `GUARDS.md`
  § A generated guard, and its corrected passage on `assert`; `COMMANDS.md`; `CI_BUDGET.md`.
- `ISSUE_TRACKING.md` and `CLAUDE.md` — every defect is held against the matrix.
- `formal/operational.md`, `formal/iteration.md`, `formal/README.md` — the seven rules, the
  `??` fallback, five open deviations.
- The `loft-test` and `loft-debug` skills.
- `CHANGELOG.md` — nothing: no behaviour changed; the fixes carry their own entries.

## See also

- [STABILITY_METHOD.md § The rule-led walk](../../STABILITY_METHOD.md) — the pass this one
  sits beside.
- [formal/operational.md](../../formal/operational.md) § Evaluation order, § Assignment;
  [formal/iteration.md](../../formal/iteration.md) § The source is evaluated ONCE.
- loft#2573, loft#2571, loft#2575 — the defects of this class found by the port.
