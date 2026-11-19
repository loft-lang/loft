<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk

Tracker: [@PLN221](https://github.com/loft-lang/plans/issues/221).

## Status

Phases A and B are built: the register with its completeness guard, and the runner that
asserts count, order, value and leak on both backends (`scripts/slot_matrix.py`).  The reach
arc G is running: the matrix was walked back over the last release and over the 78
`silent-wrong` issues since it, which added two axes and says what to add next
([REACH.md](REACH.md)).  Phases C to F are open; the causes the matrix reports today are in
[built.md](built.md) and REACH.md.

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

This table was the estimate the design started from, about 75 slots.  The register phase A
wrote has 132: every operator token is its own row, and a slot whose count depends on
another operand is two rows, one per outcome.

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
| One | slot × filler × kind | 1,286 measured (1,300–1,600 estimated); 2,868 with twins and order cells | 3 s measured, batched |
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

## What is built

Phases A and B, with their measurements and the ten causes the first runs found, are in
[built.md](built.md).  What the back-walk over past defects added — two axes, the `walk`
command, the reach ledger — is in [REACH.md](REACH.md).

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

The probe the design was written on — 114 hand-listed cells over ten slots, counts only —
is recorded in [`pilot/README.md`](pilot/README.md) with its two scripts.  It reported
loft#2573 and loft#2571 on `main` without being told of them, and two of the causes in [built.md](built.md) on
the branch that carries both fixes.

## Sub-arcs

| Item | What it builds | Verify — what goes red if it is done wrong | Status |
|---|---|---|---|
| **A** — the slot register | `syntax_slots.tsv`, `syntax_fillers.tsv`, `syntax_outside.tsv`, each slot citing the rule that states its count | `doc_hygiene`'s `every_lexeme_has_an_operand_slot_or_a_reason`: every lexeme the lexer and the parser know is in a slot or has a reason; `selftest` removes a row, adds a lexeme and misspells a rule, and each must be reported | Shipped — the eleven rule-less counts and the twelve unreached operands are carried into C |
| **B** — generator and runner | `scripts/slot_matrix.py run --backend both`, `--slot`, `--kind`: the twin, the order trace, the leak channel, the batched run on both backends; `make slot-matrix` | run against a build that has loft#2573: it reports those cells, and none on the fixed build.  Recorded as the tool's `@falsified-at` | Shipped |
| **C** — the walk, one family per step | each failing cause fixed at the lowering that copies the operand, one named suite cell per cause under its rule, the refused list read | the family's cells, both backends; the named cell fails on the build before the fix (`make falsify`) | Open |
| **D** — depth two | fillers nested once | the same run at depth two; a cause found here that depth one missed is recorded with the pair that exposed it | Open |
| **E** — the cells as lockstep input | every generated cell fed to the Rust-against-loft comparison of results and debug layers | a difference in any layer on any cell | Blocked on the port's comparison harness taking generated input |
| **F** — the gate | depth one in the PR gate (`tests/slot_matrix.rs`), a baseline of open cells that may only shrink; the wider axes nightly | a new failing cell on a PR; a baseline entry that passes again and was not removed | Shipped for one context and one frame (G1); the nightly leg is open |
| **G** — reach | the matrix walked back over past defects: `slot_matrix.py walk --old <binary>` against the release before, and the `silent-wrong` issues since it judged in [reach.tsv](reach.tsv); every axis and row it asks for | the time walk: a row added for a past issue must FAIL on the build that had the defect; the share of new issues that `fits-now` at the next release | Running — contexts and frames added; G1 to G9 of § Phase ordering are its steps |

## Phase ordering

The tooling comes before the fixes: a wider matrix finds more with every run, and each
fix is then checked by cells that already exist.

1. **G1 — the baseline and the gate** (this is phase F, taken first) — **built.**
   `scripts/slot_matrix.baseline` lists the 179 cell lines that do not pass today, and
   `slot_matrix.py gate` (run by `tests/slot_matrix.rs`, 41 s) fails on a cell that is not
   ok and not listed, and on a listed cell that passes again; a crash line is marked
   unstable and may pass.  *Verified:* against the release of 2026-11-03 the gate reports
   120 new cells and 41 lines to remove.  Still to learn: whether the baseline is the same
   on Linux.  The four contexts and two frames are not in the gate; they are a nightly
   candidate at five and a half minutes.
2. **G2 — the ad-hoc cell** — **built.**  `slot_matrix.py cell '<statement with <S>>'`
   with `--type T` (every filler of a type) or `--filler EXPR --plain EXPR` runs one shape
   on both backends with all four checks, and prints the table rows the shape would be.
   The agent that meets a defect then adds the row at that moment.  *Verified:*
   `cell 'for p in <S> { s += p.k; }' --type coll` fails on the release of 2026-11-03 (the
   operand runs five times) and passes on the branch.
3. **G3 — calibrate the reading.**  The 23 `fits-rows` issues of [reach.tsv](reach.tsv)
   written as rows; the time walk says how many of them really fail on the build that had
   the defect.  That share is how far a judged verdict can be trusted.
4. **G4 — fillers built from fillers** (this is phase D).  A filler template with a hole of
   its own, filled by the other fillers of that type: depth two without writing each by hand.
5. **G5 — a second counted operand**, and a twin that binds every operand in source order.
   For the seven "two operands interacting" issues; it also replaces the plain-variable
   order check by a real one.
6. **G6 — views across the statement.**  The prelude binds views of its values before the
   statement and shows them after it, for the ten "bind, disturb, read" issues.
7. **G7 — a second prelude**: nullable fields, a struct-enum, a generic, for the seven
   declaration-shape issues.
8. **G8 — the older walks.**  `walk` against `v2026.10.0`, which needs no reading; then a
   SAMPLE of about thirty of October's `silent-wrong` issues, judged as November's were.
   October's finder was the rule walk and not the port, so the sample is there to see
   whether it names an axis November did not — not to fill the ledger.
9. **G9 — a probe, before it is planned:** which IR operations the corpus produces and no
   cell of the matrix does.  If `loft introspect` gives that histogram cheaply, the gaps
   name missing rows without anyone reading an issue.
10. **C — the fixes**, in the order of [built.md](built.md): the lost write first.  Phase
    C's families follow the yield: iteration, write places, reads, the rest.
11. **E — the cells as input to the two-implementation comparison**, when the port's
    harness takes generated programs.

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
- `TESTING.md` — a new construct or operator adds a slot row (done with phase A); the
  `loft-test` skill, and where a cause's named cell goes.
- `COMMANDS.md` — `make slot-matrix` and the script's subcommands (done with phase A).
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
