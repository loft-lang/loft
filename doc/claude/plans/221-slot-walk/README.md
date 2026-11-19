<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk

Tracker: [@PLN221](https://github.com/loft-lang/plans/issues/221).

## Status

The generator is built and gating.  `scripts/slot_matrix.py` makes 4,772 cells from five
tables, runs them on the interpreter and the C backend in a minute, and
`tests/slot_matrix.rs` holds them against a baseline in seven tests, one a slot family.
Built so far: the register with its completeness guard (A), the runner (B), depth two (D),
the gate (F), the reach arc's two walks back over past defects (G), and of the second queue
the anchor and the tools for debugging a failing cell.  The thirteen defects it reports are
handed to the agent that fixes them ([found.md](found.md)).  Open: the rules for the ten
slots that cite none (H), breadth of operand type, and the rest of [queue.md](queue.md).
On Linux the baseline holds (the gate passed in CI at 3,661 cells); the seven-test split
and everything after it have not run there yet.

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
  slot × filler cell IS a pair of syntax constructs.  Every filler but a type's plain
  variable bumps a counter once and writes `S` into a log.
- **Wrapper** — a filler with a hole, generated at depth two around every other filler of
  the hole's type.
- **Kind** — the collection type a cell is generated for: `vector`, `sorted`, `index`, `hash`.
- **Count class** — what the rules say about how often a slot's operand runs: `once`;
  `first` (once, and before the statement reads anything else); `cond` (zero or one,
  decided by another operand); `step` (once per iteration).
- **Twin** — the same statement with the filler first bound to a local.  A local is
  evaluated once by construction, so the twin says what the statement must leave behind.
- **Anchor** — the fields a slot's PLAIN statement must leave changed in the prelude's
  world, computed by hand.  It is what makes a green run more than "cell and twin agree".
- **Context** — where the statement stands: bare, in a loop, in a branch.
- **Frame** — where its values come from: locals, parameters, or locals with views bound
  before the statement and read after it.
- **Fold** — failing cells gathered toward a cause: a slot that fails with most fillers, a
  filler that fails in several slots, a wrapper that fails around most fillers.

## The registers

The matrix is generated; the tables are what is kept and reviewed, never the cells.

| File under `scripts/` | Rows | What it holds |
|---|---|---|
| `syntax_slots.tsv` | 175 slots in 7 families | id, family, the rule that states the count, count class, expected runs, operand type, the statement, an optional declaration |
| `syntax_fillers.tsv` | 96 fillers over 13 types | id, type, the expression; 19 are wrappers, 4 write what the statement works on and go only into `first` slots |
| `syntax_contexts.tsv` | 4 contexts | id, how often the statement runs, the template around it |
| `syntax_expect.tsv` | 174 of the 175 slots | the anchor: slot, kind, the fields the plain statement must leave changed |
| `syntax_outside.tsv` | 40 lexemes | every lexeme in no slot, with why: it takes no operand, it is a declaration, or the matrix cannot reach it yet (12) |
| `syntax_slots_prelude.loft` | 3 frames | the world a cell runs in, and the functions a filler calls |
| `slot_matrix.baseline` | 318 lines | the cells that do not pass today; the gate lets it only shrink |

The lexicon is read from the lexer's tables and the parser's own lexer calls, so syntax
added without a slot row or a reason fails `doc_hygiene`.

## What a cell asserts

On each backend:

1. **Count** — the operand ran as often as its slot says.
2. **Order** — in an order cell every plain variable of the statement is a call that writes
   a letter; the letters and the operand's `S` come in source order.  Made only where the
   operand runs exactly once and nothing in the statement is conditional or repeated.
3. **Value** — the cell leaves behind what its twin does; where the operand does not run
   exactly once, what the same statement leaves with a plain variable.
4. **Leak** — the test runner's `LOFT_LEAK_CHECK` reports nothing, and no store is freed twice.
5. **Anchor** — the plain cell leaves exactly the fields its slot's row in
   `syntax_expect.tsv` names, and nothing else changed.

A cell the compiler refuses is REFUSED, listed with its diagnostic, neither a pass nor a
failure; a slot none of whose cells compile fails as vacuous.  A cell that loops is stopped
by loft's own `LOFT_TIMEOUT` and named.

**What it cannot see.**  A defect that is wrong for every form of operand, where cell and
twin agree and the anchor's hand value was computed from the same misreading.  Eight of the
78 issues walked back were of that kind; they stay hand-written suite cells.

## How big, how long

| Run | Cells | Both backends |
|---|---|---|
| the gate: every filler, one context, one frame | 4,772 | 59 s here; 421 s on a CI runner when it was 3,661 cells in one test |
| four contexts and two frames | about 23,000 | 5 min 33 s |
| depth two | about 15,000 | 3 min 9 s |
| depth three, whole | over 60,000 | not run; a sample is planned |

The cells of one collection kind run as the test functions of one generated program, so a
run is a handful of processes a backend.  **Every cell is run and none is stored**: a NAMED
suite cell is written only for a distinct cause, under the rule it broke.

## The commands

`python3 scripts/slot_matrix.py <command>`; `--bg FILE` before the command runs it in the
background and ends FILE with a line `scripts/wait_for.sh` waits for.

| Command | What it does |
|---|---|
| `check`, `selftest` | the registers are well formed and complete; the check can fail |
| `list`, `expect` | the slots per family, those without a rule, those without an anchor |
| `run` | cells, by `--slot`, `--filler`, `--kind`, `--context`, `--frame`, `--depth`, `--backend` |
| `gate [--family F]` | the matrix against the baseline; `--update` rewrites it |
| `cell '<statement>'` | one statement tried with every filler of a type |
| `expr '<expression>'` | one expression tried in every slot of its type and inside every wrapper |
| `explain <cell>` | one failing cell: its program kept, loft's diagnostics on, its IR diffed with its twin's |
| `walk --old <binary>` | the matrix on an older compiler beside this one |

## What is built

- **[found.md](found.md)** — the thirteen defects the matrix reports, each with the command
  that shows it.  The fixes are another agent's work; this plan's own work is the tooling.
- **[queue.md](queue.md)** — the ordered steps, each with what it built or still owes.
- **[REACH.md](REACH.md)** — the walks back over two releases and over 78 past issues, the
  ledger [reach.tsv](reach.tsv), and what they asked the tool for.
- **[built.md](built.md)** — phases A and B as first built, with their measurements.

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
| **C** — the fixes | each failing cause fixed at the lowering that copies the operand, one named suite cell per cause under its rule | the cause's cells pass and leave the baseline; the named cell fails on the build before the fix (`make falsify`) | Handed off — [found.md](found.md) |
| **D** — depth two | fillers nested once (`--depth 2`, wrapper rows with a `<I:type>` hole) | `fillers --depth 2`: every composed filler bumps its counter once and gives its type's plain value | Shipped (G4) |
| **E** — the cells as lockstep input | every generated cell fed to the Rust-against-loft comparison of results and debug layers | a difference in any layer on any cell | Blocked on the port's comparison harness taking generated input |
| **F** — the gate | `tests/slot_matrix.rs`: seven tests, one a slot family, against a baseline that may only shrink; the wider axes nightly | a new failing cell on a PR (BROKEN); a baseline entry that passes again and was not removed (FIXED) | Shipped for one context and one frame; the nightly leg is open |
| **G** — reach | the matrix walked back over past defects: `slot_matrix.py walk --old <binary>` against the release before, and the `silent-wrong` issues since it judged in [reach.tsv](reach.tsv); every axis and row it asks for | the time walk: a row added for a past issue must FAIL on the build that had the defect; the share of new issues that `fits-now` at the next release | Running — both time walks done, contexts, frames and operands that touch the statement added; [queue.md](queue.md) has what is left |
| **H** — a rule for every slot | the formal text evaluated wherever a slot can cite no rule, or a run raises a question the text does not answer: the rule found and cited, or written | `slot_matrix.py list` names no slot without a rule; `rule_tags.py check` resolves every citation; the slot is the new rule's guard | Open — ten slots; the `??` fallback is settled and written into `(E-Coalesce)` |

## Phase ordering

The ordered steps, each with what it built or what it still owes, are in
[queue.md](queue.md).  In short: the reach queue G1 to G9 (the gate, the ad-hoc cell, the
calibration, depth two, operands that touch the statement's own values, views, a second
prelude, the older walks, the IR probe), arc H (a rule for every slot), then the second
queue K1 to K10, which starts with an anchor — one hand-computed expectation per slot — and
breadth of operand type.  The fixes (C) are handed off through [found.md](found.md).

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

What this plan changes outside itself, and where each stands.

- `STABILITY_METHOD.md` § The slot matrix — what it is, when to run it, how it sits beside
  the rule-led walk.  Done.
- `TESTING.md` § The slot matrix — a new operator or keyword adds a row; the anchor; the
  gate.  Done.
- `COMMANDS.md` — `make slot-matrix` and the script's commands.  Done.
- `GUARDS.md` — the matrix as a guard: what makes its green mean something, and what it
  cannot see.  Done.
- `CI_BUDGET.md` — the cost of the gate and of the wider runs.  Done.
- `ISSUE_TRACKING.md` and `CLAUDE.md` — every defect is held against the matrix.  Done.
- `formal/operational.md` — the `??` fallback, evaluated only when needed.  Done.  The
  count of each of the ten slots that cite no rule is arc H.
- The `loft-test` and `loft-debug` skills — where a slot row goes, and `cell`, `expr` and
  `explain` as instruments.  Done.
- `CHANGELOG.md` — each fixed cause under Bug fixes, with its fix.

## See also

- [STABILITY_METHOD.md § The rule-led walk](../../STABILITY_METHOD.md) — the pass this one
  sits beside.
- [formal/operational.md](../../formal/operational.md) § Evaluation order, § Assignment;
  [formal/iteration.md](../../formal/iteration.md) § The source is evaluated ONCE.
- loft#2573, loft#2571, loft#2575 — the defects of this class found by the port.
