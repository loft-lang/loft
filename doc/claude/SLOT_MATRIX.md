<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The slot matrix

`scripts/slot_matrix.py` answers one question for every construct of the language alike: is
each operand evaluated once, in order, to the right result, without a leak, whatever FORM of
expression it is?  It keeps the operand positions of the syntax and the forms of expression
that go into them as tables, generates the product, and runs it on the interpreter and the
C backend.  The rule it guards is [formal/operational.md](formal/operational.md) `(E-Once)`.

Why it exists, and what it gives beside a hand-written walk:
[STABILITY_METHOD.md § The slot matrix](STABILITY_METHOD.md#the-slot-matrix--the-walk-that-is-generated).
What a change owes it: [TESTING.md § The slot matrix](TESTING.md).  How it was built, what it
found on the way and the walks back over past defects: the closed plan,
[plans/221-slot-walk](plans/221-slot-walk/README.md).

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
| `syntax_slots.tsv` | 197 slots in 7 families | id, family, the rule that states the count, count class, expected runs, operand type, the statement, an optional declaration |
| `syntax_fillers.tsv` | 100 fillers over 14 types | id, type, the expression; 18 are wrappers, 4 write what the statement works on and go only into `first` slots |
| `syntax_contexts.tsv` | 4 contexts | id, how often the statement runs, the template around it |
| `syntax_expect.tsv` | all 191 slots, a row a kind where the kind changes the answer | the anchor: slot, kind, the fields the plain statement must leave changed |
| `syntax_outside.tsv` | 38 lexemes | every lexeme in no slot, with why: it takes no operand, it is a declaration, or the matrix cannot reach it yet (12) |
| `syntax_slots_prelude.loft` | 3 frames | the world a cell runs in, and the functions a filler calls; its `sm` is a record of scalars only, bound by a call, so the value-record rewrites (`R-ValueRecord`, `R-ValueLocal`) carry it as a tuple |
| `slot_matrix.baseline` | 379 lines | the cells that do not pass today; the gate lets it only shrink |

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
| the PR gate: every cell interpreted, a sample compiled | about 6,100, of which about 900 on C | 15 s here; 16 s to 68 s a test on a CI runner |
| the gate, full (`gate --full`): every cell on both backends | 6,061 | about 100 s here; several times that on a CI runner |
| four contexts and two frames | about 23,000 | 5 min 33 s |
| depth two | about 15,000 | 3 min 9 s |
| depth three, whole | over 60,000 | not run |

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
| `seq …` | the sequence axis: a few statements over one local ([below](#the-sequence-axis)) |

## The gate

`tests/slot_matrix.rs` is seven tests, one a slot family, each `slot_matrix.py gate --family
F`, and an eighth for the sequence axis, `slot_matrix.py seq gate`, against
`scripts/slot_sequence.baseline`.  A failing test says first which of three things it is:

- **`BROKEN`** — a cell fails that is not in `scripts/slot_matrix.baseline`.  The failing
  cells are folded toward causes, each with its statement; on GitHub each fold is an
  annotation on the row of the table it names.
- **`FIXED`** — a baseline line whose cell passes again.  Nothing is broken: run <!-- doc-lint: ok -->
  `slot_matrix.py gate --update` and read the diff, which must only remove lines.
- **`REFUSED`** — a cell the compiler refuses that the baseline does not list, with the
  compiler's words.  A refusal is a claim the rules must back.

The tests interpret every cell and compile a SAMPLE: each slot with its plain variable and
its type's first filler.  Compiling a cell costs about a tenth of a second and is 96% of a
full run, which on a CI runner passed the per-test limit.  `gate --full` compiles every
cell, and `gate --update` is a full run.  A regression only the C backend shows, with a
filler outside the sample, is therefore caught by the full run and not by a PR.

A compiled program that does not build fails every cell in it with one error, so
a compiled backend's batch that fails to compile is halved until the cell that breaks it
stands alone; that cell is the one the run names.

## Debugging a failing cell

`slot_matrix.py explain <cell>` keeps one program holding the cell, its bound-first twin,
its order cell and the plain variable; prints each statement; runs the cell alone on both
backends with `LOFT_REFREE_FAILS` and `LOFT_LEAK_SITES` on; shows every field that differs
with both values; and diffs the IR of the cell against the IR of its twin — the working
form beside the broken one, where a compiler fix starts
([CODEGEN_METHOD.md](CODEGEN_METHOD.md)).

## The sequence axis

A slot cell is ONE statement, and a defect that needs two statements to meet — a first bind
and a rebind of one local, an early return ahead of the bind a later statement renames, a
loop pass that keeps a record and a return before the next mint, a local answered on one
arm of a join — is the same in every form of operand, so no filler and no twin sees it.
`slot_matrix.py seq` (the module `scripts/slot_sequence.py`) generates those programs.

**A cell** is a function `sq_N(p: T, k: integer, h: HoldT) -> T` (or the type of a
projection of `T`) whose body is one to three statements over one local `x`, and `y` bound
from it, and a tail.  The driver calls it with a parameter a callee hands back (`T`
numbered 4) and one a callee replaces (numbered 7), and with `k` from 0 to 4, so that each
branch is taken once and none once; it prints the result, the parameter after the call and
the holder.

**The answer is computed, not compared.**  The generator evaluates the same statements in
its own model of the values: every record is "the record numbered n" (its other fields
follow from n), every bind and every copy into the holder is a copy ([binding.md](formal/binding.md)
`(B-Copy)`, [calls.md](formal/calls.md) `(F-Ret)`), and it knows what each callee of the
prelude answers.  A cell fails when a call's line differs from the model's, or when a check
fails: every program runs under `LOFT_LEAK_CHECK`, `LOFT_NATIVE_LEAK_CHECK`,
`LOFT_STRICT_STORES=1` and `LOFT_POISON=1`, on the interpreter and the C backend.  So a
value that is wrong in every spelling is still wrong here.

**The tables.**

| File under `scripts/` | What it holds |
|---|---|
| `syntax_sequence.tsv` | the shapes, as class, id, types, loft, model: `src` (what a bind takes: the parameter, a literal, a fresh call, a callee handing back its argument or replacing it, one declared below the cells, `??`, a join of two), `isrc` (what a loop pass mints), `ret` (what an early return answers), `stmt` (a bind; a bind of `y`; a copy into a field or an element of the holder; a write through the local; a discarded call with a literal argument; an early return in a branch; a loop that mints, or returns before or after the mint) and `end` (the local, `y`, the parameter, a literal, a fresh call, a call on the local, a `??`, a join, an explicit `return`; with a projection where the row says `<PJ>`) |
| `syntax_sequence_prelude.loft` | four types — a record of one integer, a record holding a record and a vector, a struct-enum, `vector<integer>` — and their callees `mk`, `id`, `w` (a fresh record for an odd argument, the argument for an even one, by an early return), `wt` (the same as a tail join), `alt` (`T?`), `pk` (two parameters), and `lw`, which is written below the cells |
| `slot_sequence.baseline` | the gate's cells that do not pass today |

The first statement binds `x`, from a value that does not read it, or is a loop that mints
it; a statement reads only locals bound before it, as loft's scopes say (a local first
bound in a loop body is gone after the loop).  The product is every type and projection ×
every length × every statement variant in every place: about ten million combinations at
three statements; at two, 160,000, of which 69,000 keep the scope rule.

**A failing cell is shrunk.**  `seq run` takes each failing cell to the simplest cell that
still fails on the same backend through the same channel — a statement left out, the
projection dropped, an earlier row of the table in a hole or the tail — and prints each
simplest cell once, with how many failing cells shrink to it and over which types.  That
list, not the failing cells, is what to read: two thousand failing cells of a run shrink to
about twenty sequences.

| Command (`slot_matrix.py [--jobs N] seq …`) | What it does |
|---|---|
| `list` | the shapes, and the size of the product per type, projection and length |
| `check` | the table is well formed and every one-statement cell has a model that runs; `slot_matrix.py check` runs it too |
| `run` | the product to `--length` statements (default 3), sampled with `--seed` past `--sample` cells (default 2,000; 0 is all of it), on `--backend` (default both); `--cells FILE` runs named cells, `--list FILE` writes the simplest failing ones |
| `cell <statement> … <tail>` | one sequence, its statements as loft with `<t>` for the type's letter (`'x = lw_<t>(p);' 'if k == 0 { return x; }' 'y = w_<t>(x);' 'y'`), as shape ids, or as a cell's name; `--keep DIR` keeps its program.  A statement no row makes is named: a row for it is the change |
| `walk --old <binary>` | the product on an older build beside this one: fixed since, new since, failing on both |
| `gate [--full] [--update]` | a fixed sample (three statements, 1,500 cells, seed 1) against the baseline; every sixth cell compiled unless `--full` |

**Cost.**  An interpreted cell costs a few milliseconds: the whole product at two
statements, 69,000 cells, runs in about nine minutes of CPU on four processes.  A compiled cell costs about
a quarter of a second on a loaded machine, so the C backend gets samples: 6,000 cells is
about fifty minutes of CPU.  The PR gate (`tests/slot_matrix.rs`
`sequences_hold_their_baseline`: 1,500 cells interpreted, 250 compiled) takes 15 s here on
four processes; `seq gate --full`, which the nightly leg runs and `--update` is, several
minutes.  `SLOT_MATRIX_JOBS` (or `--jobs`) bounds how many programs run at once on a shared
machine.

**What it cannot see.**  Views and `&` (a view bound, the local rebound, the view read), text,
closures, `par`, a nullable return type, a match arm as a statement's context, keyed
collections: no row of the table names them.  A defect of one of those is a row or a type to
add, or a ledger line.

## Keeping it honest — every defect is held against it

A compiler defect found by any means is tried against the matrix when it is filed or fixed
([ISSUE_TRACKING.md § Every defect is held against the slot
matrix](ISSUE_TRACKING.md#every-defect-is-held-against-the-slot-matrix)): `cell` for a
statement, `expr` for an expression, `seq cell` for a defect that spans statements.
Reached, or reached with new rows that fail on the build with the defect; or a line in the
reach ledger, `scripts/slot_reach.tsv`, naming the axis the tool lacks.  The ledger's verdicts are `fits-now`, `fits-rows`, `needs-axis` and
`outside`; the axis with the most lines is the next one to build.  Two more instruments
say where it is thin without reading an issue: `coverage rules` (the evaluation rules no
slot cites) and `coverage ops` (the IR operations the corpus makes the compiler emit and no
cell does — half of them, at the last measurement).  `walk --old <binary>` runs the matrix
on an older compiler: what failed then and passes now is what it would have caught.

## Writing a table row

- A **slot** statement uses the names of the prelude's world; `<S>` is the operand.  A
  function the slot declares takes `<ENV>` and is called with `<ARGS>`, the whole world, so
  that a filler finds its names there; `<ENV: c h i>` keeps only some.  The type column may
  say `type@kind,kind` (the kinds the slot has a meaning in) and `type!filler` (a filler
  that is not of the type the statement needs).  The aim is that every cell the compiler
  refuses is refused by the LANGUAGE: a name the cell's world lacks fails the run.  Where
  a `{` follows the slot the operand stands in parentheses, or a block as the operand is
  read as the statement's body.  A statement's plain result must differ from "nothing
  happened", or its anchor cannot see a lost write.
- A **filler** is a primary expression or stands in parentheses; it bumps its counter once
  and gives its type's plain value (`slot_matrix.py fillers` checks both).  A filler that
  WRITES what the statement works on is marked `first` and goes only into slots whose count
  is `first`, where binding the operand first is what the rules say.
- An **anchor** is computed by hand.  `expect --propose` prints the compiler's answer as a
  proposal to check.
- `cell` and `expr` print the row a shape would be.

## Open work

| Item | What it is | When it is due |
|---|---|---|
| More shapes for the sequence axis | views and `&` (a view bound, the local rebound, the view read), a text local, a closure capture, a `-> T?` return, a match arm or a closure body as the statement's context: the shapes the ledger's remaining sequence lines name (`awk -F'\t' '$2=="needs-axis"' scripts/slot_reach.tsv`).  Each is a row of `syntax_sequence.tsv`, or a type of its prelude, with its model | with the first defect of one of those shapes, and before the ledger's sequence lines outnumber the others again |
| A nightly leg | `--context all --frame all` and `--depth 2` scheduled; the night runs `gate --full`, `seq gate --full` and a `seq run` with a new seed each night on the interpreter (the two-statement product whole) | before the next release: a C-only regression outside the sample is otherwise unseen |
| Two counted operands | a second counted operand in one statement, with a twin that binds them in the order the form's rule gives; seven issues of the ledger need it | when a defect of that shape is filed that the matrix cannot express |
| The unreached lexemes | a file in the prelude (`#next`, `#read`, `#format`), field expressions (`init`, `computed`, `default`, `$`), `T.parse`, `yield from`, `#lock`, and a cell kind that expects the program to end (`panic`) | with the first defect in one of them |
| A second prelude | declaration shapes: nullable fields, a struct-enum, a generic, a record that is an element of two collections; seven ledger issues (the record of scalars only is `sm`) | when the ledger's `shape of a declaration` lines outnumber the others |
| More contexts | a closure body, an early return, a match arm | with the second prelude |
| Slot rows naming several types | one statement run over float, text and the rest without a row a type | when the forty typed rows need a second statement shape |
| Sampled depth three | a seeded random sample of three-deep compositions a night | with the nightly leg |
| More backends at night | wasm | with the nightly leg |
| The uncalibrated ledger lines | eighteen `fits-rows` verdicts never written as cells; a sample of October's issues never judged | at the next release's reach reading |
| The cells as lockstep input | every cell fed to the Rust-against-loft comparison | when the port's harness takes generated programs (@PLN91) |
| Suite cells for the new rules | `E-Once`, `E-Field`, `E-Index`, `E-Is`, `E-Record`, `E-Assert`, `I-While`: written and passing, on the suite's branch `slot-walk-rules`, not yet merged | when the branch is pushed |
