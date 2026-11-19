<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk

Tracker: [@PLN221](https://github.com/loft-lang/plans/issues/221).

## Status

Open — design ready, pilot run, nothing built.  The pilot under [`pilot/`](pilot/) is the
probe the design rests on; its result is in § The pilot.

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

## Terms

- **Slot** — one operand position of one construct: the source of a `for`, the index of an
  element write, the right side of `??`.
- **Filler** — a form of expression placed in a slot.  A filler is itself a construct, so a
  slot × filler cell IS a pair of syntax constructs.
- **Kind** — the type kind of the value in the slot.
- **Count class** — what the rules say about how often the slot runs: `once`, `cond`
  (zero or one, decided by another operand), `per-step` (once per iteration).
- **Twin** — the same statement with every filler first bound to a local, in source order.
  A local is evaluated once by construction, so the twin supplies the expected value and
  the expected order without a hand-computed number.

## The three tables

The matrix is generated from three tables.  They are the artefact that is kept and
reviewed; the cells are not.

**Slots**, by family (the register is `scripts/syntax_slots.tsv`, one row per slot: id,
construct, statement template, the type the slot takes, count class, the `@FR-` rule that
states the count):

| Family | Slots | Count |
|---|---|---|
| Iteration | `for` source · filtered-`for` source and filter · comprehension source, filter and body · range low and high, exclusive and inclusive · text source · slice source · `while` condition | 12 |
| Write places | field, element and keyed-record writes (receiver, index, right side) · compound numeric · text append · collection append, one item and several · removal by `= null` · file attributes | 16 |
| Reads | field of an expression · element (receiver, index) · slices, closed and open · keyed and multi-key lookup · text index and slice · negative index · postfix `?` · `??` left and right | 14 |
| Calls | argument · method receiver · default and named argument · `&` argument · function-value callee | 6 |
| Operators | arithmetic · comparison · `&&` and `||` (left, right) · bitwise · `as` · unary | 10 |
| Control | `if` condition · `if`-expression arms · `match` subject, guard and arm · `return` · `assert` | 8 |
| Constructors | record field · vector element · tuple member · enum payload · format hole · format width | 6 |
| Concurrency and generators | `parallel` arm · `par` source · `yield` value | 3 |

About 75 slots.  The count is an estimate until phase A's register exists; the families are
taken from [LOFT.md](../../LOFT.md) and the lexer's token set.

**Fillers** (`scripts/syntax_fillers.tsv`: id, template, result type, the counters it
bumps): a call · a field of a call's result · an element at a computed index · a keyed
lookup at a computed key · `E ?? d` · `E?` · a method on a temporary · a slice · a block
expression · an `if`-expression · a `match`-expression · a function-value call.  Twelve.

**Kinds**: `integer`, `long`, `float`, `single`, `boolean`, `character`, `text`, an enum, a
record, a nullable record, `vector`, `sorted`, `index`, `hash`, a tuple.  Fifteen; the
vocabulary is [`matrix_axes.py`](../../../../scripts/matrix_axes.py)'s A1, A5 and A7
domains, not a second list.

## How many cells

A slot takes only the fillers and kinds that type-check in it.  The pilot measured this:
ten slots gave 114 cells with four fillers, about eleven a slot.

| Depth | What it crosses | Cells | Interpreter run |
|---|---|---|---|
| One | slot × filler × kind | 1,300–1,600 | about half a minute |
| Two | a filler inside a filler | 8,000–10,000 | a few minutes |
| Three | one more level | near 60,000 | not planned |

The pilot ran 114 cells in two seconds, one process per cell.  The C backend compiles, so
there the cells are batched: one program per slot, one function per cell.

**So: every cell is run, and none is stored.**  At these sizes the question "a test for
each, or only for the ones that fail" has a third answer.  The whole matrix is regenerated
and run from the three tables on every run; the tables are a few hundred reviewable lines,
where 1,500 cell files would be unreviewable and would rot.  A NAMED suite cell is written
only for a distinct cause the matrix found, under the rule it broke — the defect-born cell
[STABILITY_METHOD.md](../../STABILITY_METHOD.md) already ranks highest.  The estimate for
those is tens, not hundreds: the pilot's 30 failing cells were four causes.

## What a cell asserts

Each filler carries its own counter and appends its tag to a trace when it runs.  A cell
passes when all four hold, on each backend:

1. **Count** — every filler ran what its slot's count class says.
2. **Order** — the trace equals the twin's.
3. **Value** — the result equals the twin's.
4. **Leak** — `LOFT_STORES=warn` reports nothing.

A cell that does not compile is neither a pass nor a failure: it is a REFUSED cell, listed
with its diagnostic.  A refusal is a claim the rules must back (CLAUDE.md § Debugging
policy), so the refused list is read once per family, and a slot with no accepted cell at
all fails the run as vacuous.

## The pilot

[`pilot/gen.py`](pilot/gen.py) writes the cells and [`pilot/run.sh`](pilot/run.sh) runs
them; counts and hand-written expected values only, no twin, no trace, no leak channel.

| Build | Cells | Failing | Distinct causes |
|---|---|---|---|
| `main` at the release build of 2026-11-18 | 114 | 30 | 4 |
| the branch carrying the fixes for loft#2573 and loft#2571 (`6ec7ddf15`) | 114 | 12 | 2, and 1 question |

- **It found the known defects unaided.**  loft#2573 (24 cells: every loop form over
  `sorted` and `index`, with every filler) and loft#2571 (the place of a collection append
  run twice per item).
- **It found two more that the fixes did not reach**, the same on the interpreter and
  `--native-c`:
  - a comprehension over a `sorted` or `index` source still runs the source once per step
    (`[for p in mk(c) { p.k }]`: five runs for three elements) — loft#2573's fix covered the
    `for` statement and not the comprehension;
  - a collection assigned to a field at a computed place addresses the place three times
    (`h.ws[at(c, 1)].items = [P { … }]`).
- **It raised a question the rules must answer.**  A compound write through a record a
  function returned (`box(c, h).n += 5`, where `box` returns `h.ws[0]`) leaves `h.ws[0]`
  unchanged, with no diagnostic.  Whether that is a write to a copy that should be refused,
  or a lost write, is for `binding.md` to say.
- **It showed two rule sentences that disagree.**  `(E-Left)`'s prose says `??` "evaluates
  both operands"; `(E-Coalesce)` and both backends evaluate the right side only when the
  left is null.

These are recorded here and not yet filed or fixed; phase C takes them first.

## Sub-arcs

| Item | What it builds | Verify — what goes red if it is done wrong | Status |
|---|---|---|---|
| **A** — the slot register | `syntax_slots.tsv`, `syntax_fillers.tsv`, each slot citing the rule that states its count; the count-class exceptions written into the rule text where a rule is silent | a completeness guard: every operator token, keyword and `#` attribute the lexer knows appears in a slot row or in a short list of forms that take no operand.  Falsified by removing one row | Open |
| **B** — generator and runner | `scripts/slot_matrix.py` (`gen`, `run --backend both`, `--slot`, `--family`), the twin, the trace, the leak channel, the batched C run; `make slot-matrix` | run against a build that has loft#2573: it must report those cells.  The receipt is recorded as the tool's `@falsified-at` | Open |
| **C** — the walk, one family per step | each failing cause fixed at the lowering that copies the operand, one named suite cell per cause under its rule, the refused list read | the family's cells, both backends; the named cell fails on the build before the fix (`make falsify`) | Open |
| **D** — depth two | fillers nested once | the same run at depth two; a cause found here that depth one missed is recorded with the pair that exposed it | Open |
| **E** — the cells as lockstep input | every generated cell fed to the Rust-against-loft comparison of results and debug layers | a difference in any layer on any cell | Blocked on the port's comparison harness taking generated input |
| **F** — the gate | depth one in the changed-subject loop and the PR gate, depth two nightly; a baseline of open cells that may only shrink | a new failing cell on a PR; a baseline entry that passes again and was not removed | Open |

Phase C's order is by the pilot's yield: iteration, then write places, then reads, then the
rest.  Each family is its own step, with its own comparison, and can be put down between
steps.

## Open design questions

1. **Which backends.**  The interpreter and `--native-c` always.  The rustc writer joins a
   family only when a fix touches what it emits.
2. **The ledger.**  The rule-pair walk keeps [RULE_CROSSINGS.md](../../RULE_CROSSINGS.md)
   because its cells are written by hand.  Here the run is the record, so no ledger is
   planned; the baseline of open cells in phase F is the only state.  Revisit if a family's
   refused list needs a recorded judgement per row.
3. **One binding place.**  Whether the fixes of phase C converge on one site that binds an
   operand before a lowering may use it twice is decided from the list of sites phase C
   produces, not before.
4. **Syntax the register cannot reach.**  A form with no operand (a bare `break`, a type
   declaration) is outside the matrix by definition; the completeness guard lists them so
   the exclusion is explicit.

## Cross-arc dependencies

- [@PLN91](https://github.com/loft-lang/plans/issues/91) — the port's comparison harness is
  phase E's consumer, and the port is what found the defects that started this.
- [@PLN193](https://github.com/loft-lang/plans/issues/193) — the named cells of phase C land
  in the language suite, under their rules.
- [@PLN211](https://github.com/loft-lang/plans/issues/211) — a lowering that copies an
  operand is a rewrite; a site phase C fixes is a candidate row of its applied-by register.

## Documentation

- `STABILITY_METHOD.md` — a section beside § The rule-led walk: what the slot walk is, when
  to run it, how a new construct adds its row.
- `TESTING.md` and the `loft-test` skill — a new construct or operator adds a slot row;
  where a cause's named cell goes.
- `COMMANDS.md` — `make slot-matrix` and the script's subcommands.
- `GUARDS.md` — the completeness guard and the runner's falsification receipt.
- `formal/operational.md` — the `(E-Left)` sentence on `??` made to agree with
  `(E-Coalesce)`; the count class of each slot the chapters leave unstated.
- `formal/binding.md` — the answer to the write through a returned record.
- `CI_BUDGET.md` — the cost of phase F's two runs.
- `CHANGELOG.md` — each fixed cause under Bug fixes.

## See also

- [STABILITY_METHOD.md § The rule-led walk](../../STABILITY_METHOD.md) — the pass this one
  sits beside.
- [formal/operational.md](../../formal/operational.md) § Evaluation order, § Assignment;
  [formal/iteration.md](../../formal/iteration.md) § The source is evaluated ONCE.
- loft#2573, loft#2571, loft#2575 — the defects of this class found by the port.
