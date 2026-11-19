<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk

Tracker: [@PLN221](https://github.com/loft-lang/plans/issues/221).

## Status

Phases A and B are built: the register with its completeness guard, and the runner that
asserts count, order, value and leak on both backends (`scripts/slot_matrix.py`).  Phases C
to F are open, and C starts from ten causes the matrix reports today (§ Phase B as built).
The pilot the design rests on is in § The pilot.

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

## Phase A as built

| Register | Rows | What it holds |
|---|---|---|
| `scripts/syntax_slots.tsv` | 132 slots | id, family, the rule that states the count, count class, expected runs, type, the statement |
| `scripts/syntax_fillers.tsv` | 47 fillers | 9 types, each with its plain variable and 1 to 10 other forms |
| `scripts/syntax_outside.tsv` | 40 lexemes | 12 `none`, 16 `decl`, 12 `unreached` |
| `scripts/syntax_slots_prelude.loft` | — | the world a cell runs in |

- **The lexicon is read from the source.**  `check` takes it from the lexer's two tables
  and from every literal the parser asks the lexer for, 103 lexemes today.  A lexeme in
  no slot, no filler and not listed outside fails `doc_hygiene`.
- **Twelve lexemes take an operand the matrix cannot reach** (`unreached`): the field
  expressions (`init`, `computed`, `default`, `$`), the file attributes (`#next`, `#read`,
  `#format`), `#lock`, `T.parse`, `yield from` and `panic`.  loft#2575 sits in that list
  (`f#next`).  They are owed, and each needs the prelude extended.
- **Eleven slots cite no rule**, because no rule states their count: the `while`
  condition, a field read, the three text reads, `is`, the two `assert` operands and the
  record-literal fields.  `slot_matrix.py list` prints both lists.
- **Every slot compiles with its plain variable, and every filler bumps its counter once
  and gives its type's plain value** (`run`, `fillers`): 158 and 47 cells, none failing.

**The first whole run at depth one**: 1,286 cells, 133 s on the interpreter with one process
a cell; 1,222 pass, 35 are refused, 29 fail.  The failing cells are three causes, on the
branch at `6ec7ddf15`:

1. A comprehension over a `sorted` or `index` source runs the source once per step (8
   cells) — in the pilot.
2. A collection assigned to a field at a computed place addresses the place three times,
   with every form of index (10 cells) — in the pilot.
3. **New.**  A block expression as the index of a compound write runs twice, where a call
   in the same position runs once: `vi[{ c.calls += 1; 1 }] += 3` (11 cells: all five
   compound operators, the text and collection appends through a path, and three runs under
   `?`).  The same on `--native-c` and on the release build of 2026-11-18.
   `(E-Asgn-Compound)` says the place is addressed exactly once.

## Phase B as built

The cells of one collection kind are the test functions of one generated program, run by
`loft --tests` with `LOFT_LEAK_CHECK=1`; a refused cell is taken out by the line its error
names and the program is run again, and a cell that takes the program down is found by
halving.  That makes the whole matrix a handful of processes a backend.

| Run | Cells | Time |
|---|---|---|
| every slot with its plain variable, interpreter | 158 | under a second |
| the whole matrix, interpreter | 2,868 with twins and order cells | 3 s |
| the whole matrix, interpreter and `--native-c` | the same | 39 s |

Of the 2,868 cells 2,746 pass, 61 are refused and 61 fail, 333 of them check order.
**Falsified:** against the release build of 2026-11-18, which has loft#2573, `run --slot
it-for --filler all` fails 8 of 36 cells; on the branch it fails none.

**The ten causes behind the 61 failing cells**, on the branch at `6ec7ddf15`, each
reproduced in a program of its own.  Both backends unless a row says otherwise.

| # | Slot | What happens | Channel |
|---|---|---|---|
| 1 | `it-comp-source` | a comprehension over a `sorted` or `index` source runs the source once per step | count |
| 2 | `wr-path-coll` | `h.ws[e].items = […]` addresses the place three times, with every form of `e` | count |
| 3 | the compound writes and appends | a block of one statement and a value as the index runs twice (`vi[{ c.calls += 1; 1 }] += 3`); a block of two statements runs once | count |
| 4 | `wr-elem-index` and the compound and removal forms | a write to an element whose index is a function-value call is lost: after `vi[fnv(at(c, 0))] = 3` the vector is unchanged | value |
| 5 | `wr-elem-value` | in `vi[i] = box(c, h).n` the right side runs before the index | order |
| 6 | `rd-text-slice` | in `x[h.ones[e]..]` the index runs before the text it indexes | order |
| 7 | `it-slice-source` | `for n in held(c, h).v[i..j]` frees a store twice | leak |
| 8 | `rd-coal-coll-absent` | `for p in h.ws[9].items ?? mk(c)` leaves a store behind, for the three keyed kinds | leak |
| 9 | `ct-yield` | `yield box(c, h).n` in a generator ends the interpreter with SIGSEGV; the C backend answers right | crash, interpreter only |
| 10 | `ct-parallel` | an arm that calls a function writing a record of the enclosing scope: the interpreter's worker panics or the process takes SIGBUS, the C backend runs it and the write lands.  The same write spelled in the arm itself is refused at compile time | crash, and the backends differ |

Cause 4 is a wrong answer with nothing said.  Causes 5 and 6 are `(E-Asgn)` and `(E-Left)`
not held for one form of operand each.  None of the ten is filed or fixed; they are phase
C's first work, in this order of severity: 4, 10, 9, 7, 8, then the counts and the orders.

**What the first runs taught about the tables.**

- A filler must be a primary expression or stand in parentheses.  `tx(c) + ""` in a slot
  followed by `[i]` parsed as another statement than its twin and read as a defect.
- The form of a filler matters below what its name says: a block of one statement is
  lowered differently from a block of two, and only one of them shows cause 3.  Depth two
  will multiply such differences; the fillers table is where they are written down.
- A receiver slot cannot take its expected value from the plain variable, because a call
  that returns `h.ws[0]` writes somewhere else than `r`.  The bound-first twin is the
  oracle wherever the operand runs exactly once.

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
loft#2573 and loft#2571 on `main` without being told of them, and causes 1 and 2 above on
the branch that carries both fixes.

## Sub-arcs

| Item | What it builds | Verify — what goes red if it is done wrong | Status |
|---|---|---|---|
| **A** — the slot register | `syntax_slots.tsv`, `syntax_fillers.tsv`, `syntax_outside.tsv`, each slot citing the rule that states its count | `doc_hygiene`'s `every_lexeme_has_an_operand_slot_or_a_reason`: every lexeme the lexer and the parser know is in a slot or has a reason; `selftest` removes a row, adds a lexeme and misspells a rule, and each must be reported | Shipped — the eleven rule-less counts and the twelve unreached operands are carried into C |
| **B** — generator and runner | `scripts/slot_matrix.py run --backend both`, `--slot`, `--kind`: the twin, the order trace, the leak channel, the batched run on both backends; `make slot-matrix` | run against a build that has loft#2573: it reports those cells, and none on the fixed build.  Recorded as the tool's `@falsified-at` | Shipped |
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
