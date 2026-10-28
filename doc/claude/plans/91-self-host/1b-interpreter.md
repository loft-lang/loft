<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 strand 1b — the interpreter in loft, first

## Status

**Open — design, no implementation (owner, 2026-10-27).**  A child document of
[@PLN91](README.md).  It changes the strand order the README records (1a, the front end,
first): the owner starts with the INTERPRETER, written in loft, reading the IR the Rust
front end already writes.  The README's strand table and phase ordering are to be updated
to match.  Every "measured" line below was run on 2026-10-27 with the installed `loft`
2026.10.0 (`origin/main`), on `--interpret`, from a scratch directory.

## Goal

An interpreter written in loft that runs any loft program with the same result as
`loft --interpret`, and that is the **ultimate dogfood project**: every place where loft is
not handy to write it in is recorded, and every bug it meets is fixed in the language before
the interpreter moves on.

It holds four constraints:

1. **The same IR.**  The input is the program image the Rust front end writes today: the
   whole-program bundle `~/.cache/loft/program-<hash>.store`, written after
   `scopes::check` (`src/main.rs`, `startup_cache::save_program`).  So the ownership
   decisions (every free, every copy) are already in the IR. The interpreter carries them
   out and never derives them again.
2. **The same types.**  The interpreter reads the bundle as the records
   `selfhost/src/ir.loft` declares (`Bundle`, `Data`, `Definition`, `Node`, `TypeT`,
   `DbType`, …): the compiler's own IR vocabulary, used verbatim.  There is no second IR,
   so the `lib/code.loft` sketch is not a starting point.
3. **The same syntax.**  The language it runs is loft as it is.  When the loft-written
   front end (strand 1a) lands, it must emit the same `ir.loft` records, byte for byte,
   and this interpreter runs them unchanged.  1a's gate is the IR diff; 1b's gate is the
   program's output.
4. **The formal rules are the specification.**  Each module implements the rules of one
   `doc/claude/formal/` chapter and cites them by tag.  It is measured by the language
   suite's cells, which are filed under those same rules (§ Reusing the formal rules).

## The dogfood rule (governs every step)

This project exists to find out what loft is like for its hardest consumer, a language
implementation.  So the usual "work around it and move on" is not allowed here:

- **A bug is fixed first.**  When the interpreter meets a defect in loft (a wrong answer,
  a backend disagreement, a crash, a refusal a formal rule says should not happen), work on
  the interpreter step stops.  The defect gets a both-backend repro and is fixed on the spot
  under the CLAUDE.md matrix-first protocol; the probe graduates to `tests/scripts/`, and
  then the interpreter step resumes using the plain spelling.  **No workaround is written
  for a bug**, not even a temporary one: a workaround in the interpreter would hide exactly
  what this project exists to find.
- **Friction is recorded, then judged.**  Anything that works but is not handy (a missing
  construct, a long spelling, an error message that names no cure) gets a row in the
  friction register below.  It records the spelling it forced and what a better one would
  be.  A row becomes a language feature when it recurs or when the cost is clear. The
  owner decides; the register is the evidence.
- **A refusal by design is costed, not reopened silently.**  Where loft declines something
  on purpose (@C89 positional variants, @C116 closures in collections, @C63 local recursive
  functions), the register records what the interpreter paid for it.  That cost is the
  input if the owner ever revisits the decision.
- **A finding fixed on the spot needs no issue.**  Its register row and the fixing commit
  are its record (CLAUDE.md: the default is fix, not file).  One that cannot be fixed
  inside the step is filed as loft's own (`hit-by:loft`, `Found-via: @PLN91 1b` in the
  body), and the step waits for its fix.

## The working norm (owner, 2026-10-27)

**This strand matters only if it reaches its goals, so a good design beats a quick build.**
Stopping to ask the owner a design question is welcome here, not a delay to avoid.  Ask it in
prose, with a recommendation and its reasons, so the answer can be discussed rather than
picked from a fixed list.  `#package` (S0.3a) is the example: the question about a package
level grew, over three answers, into a rule none of the first options held.

What does not need asking: a design the formal rules (`doc/claude/formal/`) or a recorded
decision (`DESIGN_DECISIONS*.md`) already answer is used as it stands.  The exception is where
following it means STRUGGLING AGAINST LOFT, where the rule or decision makes the code take a
detour, a workaround or a shape the problem does not have.  That is a finding, and it goes to
the owner as a design question before the code is bent around it.  X2, C140's per-file privacy
costing 167 `pub`, was such a case, and its answer was a language change, not the 167 `pub`.

- **Every next step is designed on paper before it is implemented, and only the next
  one.**  The plan grows as it goes: a step's design is a section under § Steps, written
  and read before its code (what it changes, the failure paths, the invariant that must
  hold, how it is tested), and the design of later work stays an outline until the step
  before it lands.
- **The design is the measure of loft.**  A step's design is written the way one would
  reason about the problem: its types, its functions and their contracts, its cases,
  sketched in the shape the reasoning takes rather than bent to what loft is known to
  accept.  Implementing it should then be a straightforward transcription in the same
  shape.  Where it is not (the code needs a detour, an extra type, a split function, a
  different shape), the gap is a finding: a register row, or a bug fixed first.  So every
  built step ends with a short **design → code** note: what transcribed directly, and each
  place it did not.  The sum of those notes answers whether loft is good enough to write
  itself.
- **Small safe steps.**  A step is just enough code to be tested well: its tests are
  written with it, green on both backends, and the step is committed and pushed before the
  next one starts.  A phase in the table below is several such steps, never one.
- **Every step names its code points**: the files and functions it touches, in this tree
  (`selfhost/src/interp/run.loft: run_frame`) and in the Rust tree where it fixes a finding
  (`src/ir_store.rs: save_bundle`).  So a reviewer can check the step against the code
  before and after.

## Location

In this repository, as a top-level package:

```
selfhost/
  loft.toml              the package (name `selfhost`), tested by `loft test`
  src/ir.loft            MOVED from tools/ir_schema/ir.loft — the one home of the IR
                         vocabulary; tools/ir_schema/ generates the Rust schema from here
  src/interp/
    load.loft            bundle → ir.loft records (S0)
    lower.loft           a function's Node tree → a flat op list with jump targets (S1)
    run.loft             the dispatch loop, frames, the value stack (S1)
    store.loft           the heap: stores, records, claim/free, layouts from DbType (S3)
    ops_*.loft           the operator bodies, one file per formal chapter (S1–S5)
    natives.loft         the stdlib natives, each calling loft's own function of that name (S6)
  src/front/             strand 1a, later: lexer and parser emitting the same records
  tests/                 unit tests per module
```

Why here and not elsewhere:

- **Not a separate repository.**  `ir.loft` has to change in the same commit as
  `src/data.rs` (`cargo test --test ir_schema_roundtrip` holds them together). The oracle
  is this repository's corpus, and the end state of @PLN91 is that this code IS loft's
  core. A consumer repository would always be a step behind on all three.
- **Not `lib/`.**  `lib/` holds libraries a program `use`s.  The older `lib/lexer.loft`,
  `lib/parser.loft` and `lib/code.loft` are a self-hosting sketch with its own IR
  (`code.loft`). They stay where they are until 1a decides whether to retarget or
  retire them.
- **Not `../loft2`.**  It is a second checkout of this repository, not a separate tree.

## Reusing the formal rules

`doc/claude/formal/` defines loft as rules (361 tagged `@FR-…`) plus a list of the places the
Rust implementation deviates from them.  The interpreter reuses them in three ways.

**1. A module per chapter, citing its rules.**  The runtime chapters map onto the
interpreter's modules:

| chapter | rules | the interpreter's part |
|---|---|---|
| `operational.md` (E-*) | 22 | the dispatch loop: evaluation order, `??`, `&&`/`||`, assignment, truthiness, null propagation |
| `calls.md` (F-*) | 15 | frames, argument passing by kind (scalar, heap, `&`), return |
| `heap.md` (H-*) | 36 | `store.loft`: claim, free, copy, move, view, null reads and writes, out-of-bounds |
| `layout.md` (L-*) | 14 | field offsets, enum tags, narrow types, null encodings, read from the bundle's `DbType` |
| `binding.md` (B-*, Const-*) | 26 | copy versus view versus `&`, const |
| `ownership.md` (O-*) | 18 | already decided in the IR; the interpreter executes the frees, and from S3 checks them ([ownership.md](ownership.md) in this directory) |
| `collections.md` (Col-*, Slice-*) | 26 | vector, sorted, index, hash, spatial, trie |
| `iteration.md` (I-*) | 12 | `for`, ranges, text iteration, comprehensions |
| `matching.md` (M-*, P-*) | 23 | match dispatch, PEG patterns |
| `tuples.md` (T-*) | 10 | tuples |
| `formatting.md` (F-Spec…) | 14 | format strings |
| `closures.md`, `coroutines.md`, `concurrency.md` | 22 | lambdas, `yield`, `par` |

`types.md`, `grammar.md` and `interfaces.md` belong to the front end (1a).  `rewrites.md`
holds optimisations, mostly native-only. The interpreter must give the same values
whether a rewrite ran or not, which is exactly what those rules promise.

Each operator body and each store routine cites the rule it carries out
(`// @FR-H-FreeTwice`).  `scripts/rule_tags.py` reads citations from `src/*.rs` only
(`CITE_DIRS`, `CITE_EXTS`).  Extending it to `selfhost/**/*.loft` makes `sites <tag>`
answer for both interpreters, and `coverage` report how much of the rule set the loft
interpreter covers.

**2. The suite is the test.**  `../loft-suite` (pinned by `tests/loft-suite.pin`) holds
2700 cells filed by chapter and rule, run with `--mode interpret|native|both`.  Add a
`--mode selfhost` that runs each cell through the loft interpreter.  Progress is then a
number per rule: how many of the rule's cells hold on the loft interpreter.  That is a
better measure than "how many scripts pass", because it says WHICH promises hold.  The
`tests/scripts` corpus, compared against `loft --interpret` (stdout, exit code, leak
check), is the second instrument.

**3. A disagreement is settled by the rule, not by the Rust interpreter.**  When the two
interpreters disagree, the formal rule decides which one is wrong (formal/README.md: "the
code changes to match the rules").  If the Rust interpreter is the one breaking the rule,
that is a deviation found, and it is fixed in Rust under the dogfood rule.  So the loft
interpreter becomes a second, independent oracle for the formal definition, and that is
worth having on its own.

## Direction (not a design)

Constraints already known, which each step's design must respect.  They are not decided
designs; each is settled in the step that first needs it.

- **No recursive tree-walk at run time.**  loft stops a program at 10 000 call frames
  (`src/limits.rs`), so an interpreter recursing once per IR node would fail on a program
  recursing ~2 000 deep.  The likely shape is a flat op list per function and a `while` +
  `match` loop with explicit frames, as the Rust interpreter has.  Decided in S1.
- **Operators are resolved once per `def_nr`** (an `NdCall` names a definition) into a
  table of plain fn-refs, which loft supports.  Decided in S1.
- **The program's heap is the interpreter's own data**, an arena with layouts from the
  bundle's `types: vector<DbType>`, specified by `(H-*)` and `(L-*)`.  Decided in S3.
- **Natives call loft's own function of the same name**, never a reimplementation.
  Decided in S6.

## Outline

Each phase is several small steps, and only the NEXT step is designed (§ Steps).  A
phase is designed when the one before it closes.  Its gate is fixed now so a phase
cannot quietly shrink.

| phase | scope | gate |
|---|---|---|
| **S0** | the IR bundle opens in loft as `ir.loft` records | a guard script reads a bundle's definitions |
| **S1** | scalars, locals, control flow, calls — @PLN91's **P-perf** | benchmark output identical; speed against `loft --interpret` decides the strand |
| **S2** | text, floats, format strings | suite chapters `operational`, `calls`, `formatting` |
| **S3** | the store, records, vectors, tuples, the IR's frees | `heap`, `layout`, `binding`, `tuples`, `matching`; leak check agrees |
| **S4** | keyed collections, iteration | `collections`, `iteration` |
| **S5** | closures, coroutines, `par` | `closures`, `coroutines`, `concurrency` |
| **S6** | natives, the corpus | `tests/scripts` against `--interpret` |
| **S7** | the interpreter runs itself | S1's benchmarks through the self-run |
| **O** (beside S1–S6) | ownership as its own track, from S0 on: a checker over the bundle's IR, then the authority in 1a | silent on the ownership corpus and in agreement with `ownership_cfg` ([ownership.md](ownership.md)) |

## Steps

Phase S0 (the bundle opens in loft: S0.1 to S0.3d) is built; each step's design and its
design→code note are in [1b-s0.md](1b-s0.md).

### S1 — the speed check (@PLN91 P-perf), through the C backend — DESIGNED, not built

**The question S1 answers.**  Is a loft-written interpreter, compiled to native code, fast
enough to be loft's interpreter?  @PLN91 states the gate: within ~1.5× of the Rust interpreter
is viable, ~3× stops strand 1b (and leaves 1a, the compiler, untouched).  The answer is
measured on the smallest interpreter that runs real kernels, before any more of it is built.

**Built through the best compiler on this machine (owner, 2026-10-28).**
- **The base.**  The other agent's C-backend performance branch, `origin/laptop-c-perf` at
  `45bb2708b` (19 commits on the same `origin/main`), is joined into this branch: a merge,
  clean, 20 files, almost all `src/c/`.  Its author: a sound base with no known-bad C output.
  The full default-mode sweep over all 2,883 programs was clean at `891bbeed6`, and the
  targeted sweeps cover its WIP commit; the full four-mode sweep of the tip was not finished.
  **Consequence (CLAUDE.md branch rule 5):** this branch now merges after `laptop-c-perf`.
- **The build.**  The interpreter is compiled `loft --native-c --ship`: C, gcc, `-O3`.  gcc
  because its checked lane is the better one today (median 1.57× of the Rust twins, against
  clang's 1.74×).  Never `LOFT_RELEASE_PASS_PROBE`, which is a measurement probe and not a
  shipped build.
- **The baseline.**  `loft --interpret` of the SAME joined tree, built release
  (`target/release/loft`), on the same kernel programs.

**The kernels.**  The cores of three bench programs, without their harness (the harness
needs text, arguments and the clock, which S1's subset does not carry):
- `fib(30)` from `bench/01_fibonacci` (calls and recursion);
- `mix(…)` from `bench/02_sum_loop` (a counted loop, bit operators);
- `collatz_len` over a range from `bench/04_collatz` (`for`, `break`, `%`, `/`).

Each kernel file has a `main` printing the result, for the Rust interpreter.  The loft
interpreter runs the same image's entry function with integer arguments and prints the
integer it returns.  Both must print the same number, or the times do not compare.

**The interpreter's shape** (direction for S1's steps, settled step by step):
- `selfhost/src/lower.loft`: a function's `Node` tree becomes a flat `vector<Op>` once, with
  `Op { code, a, b }`, jumps resolved to indices, and the IR's variable numbers as frame
  slots.  Flat, not a recursive walk: the 10,000-frame limit (`src/limits.rs`) and S1's
  speed both ask for it.
- `selfhost/src/run.loft`: one `while` loop that dispatches with `match op.code`, a value
  stack `vector<integer>`, and frames as `(function, pc, base)`.
- **An operator is loft's own operator.**  `OpAddInt` executes as loft's `a + b`, so overflow
  and null (`(N-Prop)`, operational.md) are the Rust interpreter's by definition, with
  nothing re-implemented.  The `def_nr` of each `NdCall` is resolved once, at load, by the
  definition's name.

**Steps** (each designed when the one before it lands):
- **S1.1 — the operator table and the first lowering.**  The `fib` kernel's node set
  (`NdInt`, `NdVar`, `NdIf`, `NdBlock`, `NdCall` of `OpLeInt`/`OpAddInt`/`OpMinInt` and of a
  user function, `NdReturn`, `NdLine`) lowers to ops, and a test compares the ops against a
  hand-written list.  Code points: `selfhost/src/lower.loft` (new),
  `selfhost/tests/lower.loft` (new; under S0.3c it may read `Op`).  First, the join of
  `45bb2708b`.
- **S1.2 — the run loop**: `fib(25)` answers what `--interpret` answers.
- **S1.3 — loops**: `NdLoop`, `NdBreak`, `NdSet` and the iterator form a `for` lowers to,
  for `mix` and `collatz_len`.
- **S1.4 — the measurement**: best of five wall times per kernel, both sides, numbers in
  this section, and the gate read against them.

### S1.1a — a block-bodied match arm needs no comma (fixes X14) — BUILT 2026-10-28

**Found by S1.1.**  The lowering's `match` wrote block arms the way the reasoning (and
Rust) writes them: `NdIf { … } => { … }` on its own lines, then the next arm, no comma.  The
parser refused it ("Expect token ,"), and so it refuses the documented example in
`LOFT_CONTROL.md` (`Holder { items } => { items += "y"; }` then `_ => { }`), which no gate
compiles.  No rule writes the separator down, so the standing default decides
(`formal/README.md`: rustc's answer where no rule is written): a comma separates arms, and
after an arm whose body is a block it may be left out.

**Fix.**
- The one home of the arm body, `Parser::parse_match_arm_body` (`src/parser/control.rs`),
  records whether the body opened with `{`.
- A separator helper, `match_arm_separator`, applies the rule at the two sites that demand
  the comma today (`control.rs:~7232`, the enum match, and `:~11558`, the scalar and struct
  match): optional before `}` or after a block body, required otherwise.
- `(M-Sep)` in `formal/matching.md`.

**Matrix:** a block arm then a `_` arm; then a variant arm; then an expression arm; each with
and without the comma; an expression arm without a comma is still refused, on both match
paths (enum, scalar).  Tests in `tests/parse_errors.rs` or beside the match cells, and the
`LOFT_CONTROL.md` example compiled by a test.

**Built — what changed from the design.**  `parse_match_arm_body` already computed whether
the body opened with `{` (a local, `block_arm`); it now publishes that LAST, after the body,
so a `match` nested inside the body cannot leave its own arm's answer behind.  That case is
a cell: an outer expression arm whose inner `match` ends in a block still needs its comma.
Tests: `parse_errors::a_block_bodied_match_arm_needs_no_comma` (enum path, scalar path,
nested), `an_expression_bodied_match_arm_still_needs_its_comma` (plain and nested).
Falsified: with the flag ignored the first fails.  `(M-Sep)` is in `formal/matching.md`.

A side-observation, not this step's: `enum E { … }` compiles from the command line but is
refused under the `code!` test harness ("conflicts with a constant of the same name", the
stdlib's `E`), so the two drivers disagree on one program.  To check before a fix is chosen.

## The language for this job

### What loft already offers (measured or established in @PLN202 / OCAML_BAR.md)

| construct | use in the interpreter | status |
|---|---|---|
| `match` with nested patterns, guards, or-patterns, as-bindings | dispatch over `Node`, `TypeT`, `DbParts` | PASS (C1–C5, H7) |
| exhaustive `match` over enums | adding a `Node` variant breaks every pass that forgot it | PASS (`c9`); nested holes are named by the outer variant only (C6) |
| record update `S { f: v, ..base }`, also for a variant narrowed by `is` | rewriting IR nodes during lowering | PASS; an unnarrowed enum base is refused naming the cure (measured) |
| head/tail slice arms `[]` / `[x, ..rest]` | walking argument and op lists | PASS (C8) |
| PEG patterns over a token vector, cursor match, sub-rules, `#lexeme` | 1a's parser; peephole matching over op lists | shipped (@PLN35, `tests/scripts/35*`) |
| plain fn-refs in a `vector` or `hash` | the operator dispatch table | PASS |
| phantom type parameters `Idx<NodeTag>` | typed indices: a definition number cannot be passed as a variable number | PASS: *"expected Idx<NodeTag>, got Idx<TypeTag>"* (measured; see X5, X6) |
| `T?`, `x?`, `??`, `h = f(x)?;` | absent values, early exit on null | PASS (H8) |
| generics, interfaces, abstract types (`pub` boundary) | the store's API kept apart from its internals | PASS (A1–A7, H3) |
| tail self-call as a jump `(R-TailSelf)` | loop-shaped helpers over scalars | PASS (H6) |
| mutually recursive types | `Dir`/`Entry`-like IR tables | PASS (H9) |
| `v.contains(x)` | keyword and op-class sets | PASS (measured) |

### Friction register (measured 2026-10-27)

| id | what | kind | proposal |
|---|---|---|---|
| **X1** | `store_load` refuses the program bundle: *"it has no layout sidecar (`.dschema`) beside it"*. The bundle carries its own schema (`Bundle.types`), but `ir_store::save_bundle` writes no sidecar, so loft cannot read the compiler's own output | **bug — fixed in S0.1** | `save_bundle` writes the sidecar ([1b-s0.md](1b-s0.md) § S0.1) |
| **X2** | `ir.loft` used as a library: every type and every field is private to its FILE (@C140; no package scope), so `use ir; ir::Bundle` is refused | **language fix — S0.3a** | measured: 167 `pub` (25 types, 142 fields) before another file of the same package reads one field.  The owner's cure: `#package` shows a file's internals to its own directory (S0.3a), and the file-local default stays |
| **X3** | a generic type alias does not parse: `type Env<T> = hash<B<T>[name]>;` → *"Expect token ;"* | friction | generic aliases, OCaml's `type 'a env = …` |
| **X4** | `hash<(integer, text)>` → *"tuple element types only allowed on vector / iterator"* | friction | tuple elements, and tuple keys, in keyed collections (OCaml `(int * string, v) Hashtbl.t`); memo tables keyed by (definition, type) want it (bar B2) |
| **X5** | a phantom tag needs a dummy field: `struct NodeTag { }` → *"Expect attribute"* | friction | an empty struct, or a declared abstract type with no body (OCaml `type node_tag`) |
| **X6** | a generic literal in a typed field does not take `T` from the field: `Holder { a: Idx { n: 1 } }` with `a: Idx<NodeTag>` → *"cannot tell what T is"* | friction, possibly a bug: the field's type names T | infer `T` from the destination field, as a typed local already does |
| **X7** | a by-value recursive variant is refused (*"Enum 'E' contains itself … use reference<E>"*), so `ir.loft` holds every child as a box-of-one `vector<Node>` and every read is `n.cond[0]` | refused by design (bar A5) — costed | a recursive field written by value (`cond: Node`) and LAID OUT as today's box-of-one vector: the stored IR stays byte-identical, and the reads drop `[0]`. Owner decision: it revisits A5's refusal |
| **X8** | no conversion between `float` and its bits (none in `default/*.loft`) | friction (to confirm in S1) | `to_bits`/`from_bits` for `float` and `single`; needed by an arena that stores floats in integer words |
| **X9** | no `in` operator and no set of scalars (@PLN179 015); `{4 in ks}` fails as *"Formatter error"* | friction; the message is a wrong-error finding of its own | `contains` serves; the message should say there is no `in` |
| **X10** | capturing closures cannot sit in a collection (@C116); no local recursive `fn` (@C63) | by design — costed | not expected to bite: the dispatch table holds plain fn-refs |
| **X11** | a read-only cache directory crashed every cold run, and every warm load from it, with an internal compiler error (`Store::open` panics) | **bug — fixed in S0.2** | `Store::try_open`; a read surface that cannot be opened is a miss ([1b-s0.md](1b-s0.md) § S0.2) |
| **X12** | inside a package, a file named like one of its modules (`tests/load.loft`) gets ITSELF for `use load;`, so every name reads "Unknown function" (on `main`) | **bug — fixed in S0.3b** | `own_module_path` never answers the current file, and asks the package's `src/` ([1b-s0.md](1b-s0.md) § S0.3b) |
| **X13** | `assert(x != null, …)` does not narrow `x`; the other null tests (`if … return`, `if != null`, `?? return`, `?? panic`) do, and `types.md` writes none of them as a rule | rule gap — fixed in S0.3d | owner: `(N-Guard)`, the guard narrowing written as a rule with `assert` as one of its guards ([1b-s0.md](1b-s0.md) § S0.3d) |
| **X14** | a block-bodied match arm demands a comma (`NdIf { … } => { … }` then the next arm), while Rust and the `LOFT_CONTROL.md` example leave it out | **bug — fixed in S1.1a** | the comma is optional after a block body (§ Steps, S1.1a) |

### New constructs taken from OCaml's type expressions

OCaml's type language is the model, within two fixed bounds.  **@C89** keeps enum payloads
NAMED (no `Num(integer)`), so nothing here is positional.  And a construct lands with its
formal rule written first, in the chapter that owns it.  Candidates, in the order the
register supports them:

1. **Recursive fields by value** (X7). OCaml `type expr = Add of expr * expr`, with named
   fields: `enum Node { NdIf { cond: Node, t: Node, f: Node }, … }`, stored as today's
   box-of-one.  This is the single biggest readability gain for an IR, and it keeps the
   bundle layout, so "identical IR" holds.  Rule home: `layout.md` + `types.md`.
2. **Generic type aliases** (X3). OCaml `type 'a env = (string, 'a) Hashtbl.t`: a
   compile-time substitution with parameters, as aliases already are without them (C55).
3. **Abstract tag types** (X5). OCaml `type node_tag`: a declared type with no
   representation, usable only as a type argument.  It makes typed indices free (`Idx<Def>`,
   `Idx<Var>`, `Idx<Ty>`), which the IR's dozens of raw integer numbers need.
4. **Tuples in keyed collections** (X4). OCaml's tuple types `int * string` as keys and
   elements.
5. **Exhaustiveness named through nesting** (bar C6). The diagnostic names `X { i: B }`
   rather than `X`, so a lowering pass over nested IR shapes learns which inner case it
   missed.

X1 is a bug and is fixed in S0, not proposed.  X6 is inference, not syntax.  Each of
these is built only when its register row says the interpreter keeps paying for its
absence.

## Open questions for the owner

1. **The directory's name**: `selfhost/` as proposed, or another.
2. **Moving `ir.loft`** from `tools/ir_schema/` into `selfhost/src/` as its one home (the
   schema generator then reads it there).  Recommended: yes; a copy would drift.
3. **X7 against the A5 refusal**: by-value recursive fields laid out as a box are the
   proposal; A5's refusal was about the by-value LAYOUT, which this keeps out.
4. **The P-perf bar**: ~1.5× of the Rust interpreter as @PLN91 states, measured on the
   loft interpreter compiled `--native-release` (the shipped tier).
5. **A label for the findings**: `hit-by:loft` with `Found-via: @PLN91 1b`, or a label of
   its own so the dogfood yield can be counted.
6. **`--mode selfhost` in loft-suite** is a change to that repository (a suite PR beside
   the loft PR that adds the interpreter).

## Documentation this plan owes

- [ ] `91-self-host/README.md`: strand table and phase ordering (1b first, S0–S7), link here.
- [ ] `tools/ir_schema/README.md`: the new home of `ir.loft`.
- [ ] `scripts/rule_tags.py`: citations from `selfhost/**/*.loft`; `formal/README.md` § Rule tags.
- [ ] `doc/claude/bars/OCAML_BAR.md`: the register's rows that are bar entries (X3, X4, X5, X7).
- [ ] TESTING.md: the selfhost suite mode and the corpus diff.
