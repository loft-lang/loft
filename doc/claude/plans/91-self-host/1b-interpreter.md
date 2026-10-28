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

**The order of the goals (owner, 2026-10-28).**  First full coverage: every test and program
the repository defines (the corpus, `tests/scripts`, the language suite's cells) gives
`--interpret`'s result.  Speed is measured from S1 on and reported (S1.4's 3× line), never a
gate.  **Start-up on par with the Rust interpreter is a goal for later, not a blocker**: the
analysis of how the Rust interpreter makes its start-up fast (the compiled stdlib, the
program cache) is recorded when it is done, and a start-up step is planned only after
coverage.

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

## Reusing the stdlib (owner, 2026-10-28)

**The stdlib is reused as much as possible**, because every function the interpreter runs from
its IR is one it cannot disagree with the compiler about.  The program image already holds
every definition of `default/*.loft`, compiled to the same IR as the program, so:

- **A stdlib function written in loft runs from the image**, as any user function does, and
  is never written again in the interpreter.  The lowering decides a call as the Rust
  interpreter's bytecode generator does, in its order (`src/state/codegen.rs`, a call of a
  definition): an operator (`Op` and a capital) is its op; a definition with a native symbol
  is a native; one with a loft body is called, even when it also carries a `#rust` template,
  which is for native code generation only; anything else is a native by name.  A method
  (`t_<len><Type>_<name>`) is no exception.
- **Only what has no loft body is written here**: the operators, as loft's own operator in
  `run.loft`, and the natives, each calling loft's own function of that name (`natives.loft`,
  S6).  Those two tables are the whole of the interpreter's own semantics; they grow only by
  a definition the stdlib cannot give in loft.
- **Moving a native into loft is the preferred cure** when the interpreter would otherwise
  need a hand-written body for it and the loft version is as fast — the language gains the
  function, and the interpreter loses a place to differ.

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
- **The base.**  `origin/main`.  The other agent's C-backend performance branch
  (`origin/laptop-c-perf`) was joined here at `45bb2708b` for S1 and the join DROPPED at the
  rebase onto `main` a5703bf91 (2026-10-28): that squash (#2205) carries this branch's S0 and
  the C-perf commits through `f8343fa6c`, and S1.1–S1.3 need nothing of the eight after it.
  S1.4, the measurement, joins C-perf again, or waits for its next merge; until then this
  branch merges independently of it.
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
  `45bb2708b`.  Its four design→code findings are X15–X18; X15 is S1.1b.  **BUILT
  2026-10-28:** with X15 (S1.1b T1, T2) and X16–X18 fixed, `fib` lowers to the hand-written
  list on both backends (`tests/selfhost.rs`, and `loft test --native` in `selfhost/`).  The
  two `NdSpan` arms (`inner[0]?`) still draw `warning[tail-call-not-jumped]` until S1.1b T4
  makes a fresh argument jump; the IR's spans are shallow, so the call is correct meanwhile.
- **S1.2 — the run loop**: `fib(25)` answers what `--interpret` answers.
- **S1.3 — loops**: `NdLoop`, `NdBreak`, `NdSet` and the iterator form a `for` lowers to,
  for `mix` and `collatz_len`.
- **S1.4 — the measurement**: best of five wall times per kernel, both sides, numbers in
  this section, and the gate read against them.

### S1.4 — the measurement: the loft interpreter against the Rust one — DESIGNED 2026-10-28

**The owner's ruling (2026-10-28):** measure first, before T3 and T4.  Parity is not a gate:
the strand goes on whatever the number.  But where the loft interpreter, compiled through the
C backend, is more than **3×** slower than the Rust interpreter on a kernel, the performance
stream (the C-backend agent) is told, with the row.  This settles open question 4 for S1.

**How.**  `python3 selfhost/bench/measure.py` (best of five per side, on a quiet box):
- **Rust side:** `loft --interpret kernels/<k>.loft`; the kernel's `main` times its own call
  with `ticks()` and prints the answer and the microseconds, so start-up is left out.
- **Loft side:** the same kernel's program image (`--emit-image`) run by
  `selfhost/bench/run_kernel.loft`, compiled `--native-c --ship` (the C backend as it ships);
  it times the run apart from the image load and prints both.
- The two answers must agree (`fib(30)` 832 040, `mix(5000000, 1)` 6 495 169,
  `longest(1, 100000)` 77 031 351) or the row is not a measurement; a row over 3× says so.
- First on `main`'s C backend; the C-perf join, when it comes, is measured again beside it.

**Code points:** `selfhost/kernels/*.loft` (`main` times its call), `selfhost/bench/run_kernel.loft`
and `selfhost/bench/measure.py` (new).

### S1.3 — loops: `mix` and `collatz_len` lower and run — BUILT 2026-10-28

**What the IR is.**  A `for i in a..b` reaches the bundle as a block holding a `loop`: the
index starts one below `a`, and each turn an iterator BLOCK steps it, breaks past the end
(`if end <= i break(0) else null`) and answers it, before the body.  So the new nodes are
`NdSet` (an assignment), `NdLoop`, `NdBreak { n }` / `NdContinue { n }` (`n` loops out),
`NdNull` (the empty `else`), and a block in statement position.  No iterator object: the
range form is plain integer code, and `NdIter` stays refused.

**The lowering** (`selfhost/src/lower.loft`):
- `NdSet { var, inner }`: the value, then `Store var`.  `NdNull` as a statement: nothing.
  A block as a statement: every operator a statement.  An `if` as a statement: `cond`,
  `JumpIfFalse`, the then-side, and where the else-side is not `NdNull`, a `Jump` over it.
- `NdLoop`: its start is noted, its block lowered as statements, and a `Jump` back to the
  start ends it.  `NdBreak { n }` emits a `Jump` to be patched to the end of the loop `n`
  levels out; `NdContinue { n }` jumps to that loop's start.  The open loops are a stack in
  the `Lowering` (starts, and the break sites with the loop each belongs to).
- **The stack depth is tracked statically**: each emitted op moves it by its known effect, an
  `if`'s else-side restarts from the depth its then-side started at.  A `break` or `continue`
  taken while values are pending (a loop inside an operand) would leave them on the stack,
  so it is refused by name, never lowered wrong.  The kernels never do it.
- A call whose value is dropped (an expression statement) is followed by `Pop`.

**The run loop needs nothing new**: `Store`, `Pop`, `Jump` and `JumpIfFalse` are carried.

**Cases** (computed outside this code — Python, and `loft --interpret` for the two large
ones): `mix(10, 1)` 41, `mix(5000000, 1)` 6 495 169; `collatz_len(9)` 20, `collatz_len(27)`
112; `longest(1, 10)` 9 020, `longest(1, 100000)` 77 031 351.  A `break` with a value pending
is refused, by a probe written to have one.

**Code points:** `selfhost/src/lower.loft`, `selfhost/kernels/{mix,collatz}.loft` (new),
`selfhost/tests/run.loft`.

**Design → code.**
- As designed; all six values and the refusal pass on both backends
  (`selfhost/tests/run.loft`; the probe is `selfhost/kernels/pending.loft`, which
  `--interpret` runs to 6).
- **A range with variable bounds guards them against null** before the loop
  (`if !OpConvBoolFromInt(start) { start = 1; end = 0 }`), and starts the index at
  `OpConvIntFromNull()`, which the constant range of `mix` does not.  So three more codes:
  `NonNull` (`OpConvBoolFromInt`, the stdlib's `v != i64::MIN`), `Not` (`OpNot`, its `v != 1`,
  a boolean being 1 or 0 on the stack) and `PushNull`, each with its own stack effect.
- Not yet run: the `tests/selfhost.rs` harness over the whole package, held for another
  session's timing window; it runs before S1.4.

### S1.2 — the run loop: `fib(25)` answers what `--interpret` answers — BUILT 2026-10-28

**What.**  `selfhost/src/run.loft`, `pub fn run(b: ir::Bundle, entry: integer, args:
vector<integer>) -> Outcome`, with `struct Outcome { value: integer, refused: text }`: the entry's
result, or the first thing S1 does not carry, named (never a guessed value).

- **Lowering on demand.**  `lower::lower` gains the frame's size: `slots` (the definition's
  variable count, `len(d.variables.variables)`) and `params` (its attribute count).  `run`
  lowers a function the first time it is called and keeps it in a `vector<lower::Lowering>`
  indexed by definition number, with a `vector<boolean>` saying which are lowered.
- **Two explicit stacks, no recursion** (the frame limit is the reason the lowering is flat):
  the value stack `vector<integer>` with a pointer `sp`, and the frames as three integer
  vectors (`f_def`, `f_pc`, `f_base`) with `fp`.  A push overwrites below the length and
  appends at it; a return resets `sp` to the frame's base.  Nothing is ever removed, so the
  stacks only grow to the deepest point the program reaches.
- **One loop.**  The current frame is held in locals (`def`, `pc`, `base`, `ops`) and saved
  into the frame vectors only at a call; each turn fetches `ops[pc]`, steps `pc`, and
  `match`es the code:
  - `PushInt a` pushes `a`; `Load a` pushes slot `base + a`; `Store a` and `Pop` (S1.3) write
    and drop.
  - an operator pops `y`, then `x`, and pushes loft's own `x OP y` — a comparison pushes 1 or
    0 through `if x OP y { 1 } else { 0 }`, so a null operand behaves as in the program;
  - `Jump a` sets `pc`; `JumpIfFalse a` pops and jumps on 0;
  - `Call d n`: the arguments are the top `n` values, so the callee's base is `sp - n`; its
    other slots are zeroed up to `slots`; the caller's `pc` is saved and the callee entered;
  - `Ret`: pops the value, resets `sp` to `base`, and either answers it (the entry's frame) or
    pushes it in the caller.
- **The interpreter's own number.**  The test compares with values computed outside this
  code: `fib(0..10)` is 0 1 1 2 3 5 8 13 21 34 55, `fib(25)` is 75 025 (what
  `loft --interpret` prints for `kernels/fib.loft` at 25).

**Code points:** `selfhost/src/run.loft` (new), `selfhost/src/lower.loft` (`slots`,
`params`), `selfhost/tests/run.loft` (new).

**Cases** (hand-computed, both backends): the eleven small values and 75 025; a call into a
function the lowering refuses answers `refused` naming it, not a value; an entry with the
wrong number of arguments is refused.

**Design → code.**
- As designed; `fib(0..10)`, `fib(25)` = 75 025 and both refusals pass on both backends
  (`selfhost/tests/run.loft`).
- **The value stack is `vector<integer?>`.**  A program's integer may be null, so its values
  are carried as `integer?` and loft's own operators propagate the null; a `?` there would
  have turned a null into 0, a wrong answer.  `Outcome.value` is `integer?` for the same
  reason.
- **The bookkeeping reads discharge with `?? panic(…)`** — the op at `pc`, a saved frame, a
  lowered callee: an element read is `τ?` even where the index is in range by construction,
  and a missing one is an interpreter bug.  Without it every counter restored from a frame
  turned `integer?`, and 37 warnings followed from three reads.  Registered as X19.

### S1.1b — a tail self-call jumps wherever it can, and warns where it cannot (fixes X15) — DESIGNED 2026-10-28

**Found by S1.1.**  `lower.loft` walks the IR tree by recursion, and its tail self-calls are
refused under @C155: `NdSpan { inner } => expr(l, b, inner[0]?)` (`lower.loft:90`, and
`statement` at `:141`).  C155 refuses a tail self-call whose frame the jump cannot reuse,
because left a call it might stop at 10 000 frames.  That refuses what MIGHT fail, while the
same walk written as `r = expr(…); r` compiles with the same bound.

**The owner's rules for this step:**
1. When the language can implement a construct correctly and efficiently, it does.  A
   compile error is only for a program that CERTAINLY fails: it never ends, or it certainly
   runs out of store memory.
2. The SEMANTIC layer copies data rather than keep many stores alive; the CODE layer removes
   the copies again where it can prove them unneeded (@C122: the contract is the semantics).
3. A slow construct that stays in the code — a real copy, a tail self-call left a call — is a
   `warning`, so that the libraries do not ship it.  The reason is circumstantial (library
   CI), so it is written as its own clause of the tier rule (DIAGNOSTICS.md § Diagnostic
   tiers), not folded into "ignoring it can produce a wrong result".

**Measured** (probes on `--interpret` and `--native`, `target/debug/loft` of this branch):

| argument for `n: N` | its IR | refused because | what it is |
|---|---|---|---|
| `n.inner[0]` | the field read sits under the variant check `if n is Wrap then n else OpNullRefSentinel()` | `borrows_from_params` does not look through that `if` | a part of `n` |
| `inner[0]` | `inner` is the match binding `_mv_inner_1["n"] = OpGetField(n, …)`, never released | every local reads as one the jump releases | a part of `n` |
| `inner[0]?` | on a missing element `?` builds a default `N` in `__ref_1`, which the frame frees at its exit | a fresh value | fresh on one arm, a part of `n` on the other |

Two facts the design rests on: today's call keeps a fresh argument's store until the CALLER's
function exits (`__ref_p2_1` lives to `main`'s end), so a recursion holds every one until it
unwinds; and a `reference<T>` field points across stores (`w.r` sees a later write to `a`, on
both backends), so a fresh value can point into an older store.  The IR types hold no
`reference` field (`selfhost/src/ir.loft`).

**The semantic rule.**  At a tail self-call each store-held parameter is rebound:
- to the argument itself, when it lives in a CALLER's store (the parameter, a sibling, a
  part of one — `n.inner[0]`, a match binding of one);
- otherwise (a fresh value, the `?` default, a part of a value the frame made) to a COPY of
  it in the parameter's own store, which the loop owns and frees at every exit.
Then every other store the frame owns is released, after all the arguments are read.

**The invariant:** after every jump each parameter points into a caller's store or into the
one store the loop owns for it.  The frame owns nothing else, so the memory a tail recursion
holds is bounded by its arguments, never by its depth.

**The code layer removes the copy** with one runtime rule, and only in a function whose tail
calls need it: of the stores the jump would release (its locals' and the old parameter
stores'), each one a new argument LIVES IN is kept — handed to that parameter's slot, once
when two parameters share it — and the rest are freed.  A fresh whole value, a whole local,
the `?` default and a part of the parameter's own store then cost no copy at all.  What is
left to copy is a value that may POINT into a store the jump releases: an argument whose type
holds a `reference`, and a body that stores a parameter's pointer through another parameter
(`a.r = n`).  Those copy (the `tail-call-copy` warning) or, until that clause is built, stay
a call (the `tail-call-not-jumped` warning).

**Steps** (each designed in full when the one before it lands):
- **T1 — the sharing fix (X15; Contract: settled).**  BUILT (below).
- **T2 — a frame the jump cannot reuse is an ordinary call, with a warning.**  BUILT.  @C155's
  refusal becomes `warning[tail-call-not-jumped]`, naming the reason and the cure; the tier
  rule gains its library clause; the error-message case 63 and the suite cell
  `R-TailSelf/a-tail-self-call-the-frame-cannot-jump-is-refused` move from the error to the
  warning (the cell is a loft-suite change beside the PR).  S1.1 compiles from here.
- **T3 — a function that certainly never ends is refused.**  `error[endless-recursion]`: no
  path through the body leaves without first calling itself.  The `i1058_*` runtime tests
  that overflow on purpose (`runaway1058`, `spin1058`) take a base case that never fires.
- **T4 — the owned jump.**  The runtime rule above, with exit frees at every `return`; the
  first clause takes scalar, void and text results.  The `?` default and fresh arguments
  jump; `walk(n.inner[0])` on an owned `n` keeps its one store.
- **T5 — a store-held result** delivered from an owned store: handed to the caller.
- **T6 — the copy with a sharing map** for arguments that hold a `reference`, with
  `warning[tail-call-copy]`; and the pointer-storing body.
- **T7 — store reuse** (two stores per parameter, cleared rather than claimed), only if the
  T4 measurement shows the claim/free per step.

**Cases, written before building** (hand-computed, value + leak, both backends, under
`LOFT_STRICT_STORES=1` and `LOFT_POISON=1`; the harness must fail with the step reverted):
1. `walk(make())` then `walk(n.inner[0])`: one store, freed at the exit (T4).
2. A fresh argument every step, `count(S { x: n.x - 1 })` to 10⁶: flat memory, exact value (T4).
3. `inner[0]?` with the element present on some steps and missing on others (T4).
4. Two parameters in one owned store, `walk(n.left, n.right)` on a fresh `n`: freed once (T4).
5. A swap of an owned and a borrowed parameter (T4).
6. Each shape still declined: an ordinary call, the right value, the warning (T2).
7. A body with no path that returns: the compile error (T3).
8. The three spellings of the table, and a part through two levels (`n.inner[0].inner[0]`)
   and through a nested match binding: jump, right value (T1).

#### T1 — the sharing fix (fixes X15, loft#2207; and loft#2208) — BUILT 2026-10-28

**What.**  `borrows_from_params` (`src/tail_frame.rs`), the one predicate the refusal and the
rewrite share, also accepts as a part of a parameter:
- `if c then P else OpNullRefSentinel()` with `P` a part of a parameter — the variant check
  a field read of an enum parameter lowers to (either arm may be the sentinel);
- a local whose every write is null or a part of a parameter and which no release names —
  the match payload binding (`_mv_<name>_N`).
Both shapes are what `(R-TailSelf)` already promises ("a projection rooted at one"), so no
rule text changes: this closes a deviation from it.  With them:
- the refusal in `definitions.rs` reads `tail_frame::off()`, as its doc and the generic
  path (`generics.rs`) already do, so `LOFT_NO_TAIL_SELF=1` keeps the calls;
- the refusal's cure names the match binding too.

**Code points:** `src/tail_frame.rs` (`borrows_from_params`, a helper for the binding),
`src/parser/definitions.rs` (the `off()` test).

**Verification:** case 8 as a Rust test on both backends, at a depth (20 000) past the frame
limit so a call cannot pass; `inner[0]?` still refused (T2 changes that); falsified with the
two new arms removed; `make rewrite-census` (admissions may only rise); the parser and
codegen subjects.

**Design → code.**
- As designed: `part_of_params` reads the three spellings of a part (a projection op, the
  variant check, a binding), following a binding to its writes at most four levels deep;
  `binds_a_part` asks `op_writes_variable`, split out of `writes_variable`, so a local any op
  claims, frees or re-points is not a part.  The refusal reads `off()`.
- **Found: loft#2208, on `main` too.**  Case 8's binding of a binding (`two_levels`) was
  admitted and then left a CALL: the scope pass wraps such an arm in an `Insert` (a block's
  second spelling, `_mv_deeper_1 = null` before the `if`), which both the finder
  (`Finder::value`) and the rewrite (`opt::tail_self::value`) read as no tail.  `main` shows
  it with a struct parameter, where the parser admits the call.  Fixed at both walks.  What
  stays unguarded: nothing compares the parser's admitted calls with the pass's jumps, so a
  third spelling would again be silent.
- Guards: `tests/scripts/2207-…` (the three spellings and `grow`, 20 001 levels, 20 006 each)
  and `tests/scripts/2208-…` (20 005), both backends, `LOFT_STRICT_STORES=1`, `LOFT_POISON=1`,
  no leaked store.

#### T2 — a frame the jump cannot reuse is an ordinary call, with a warning — BUILT 2026-10-28

**What.**  The two refusal sites keep their predicate and change their level: `definitions.rs`
(pass two) and `generics.rs` (`refuse_instance_tail`, per instance) emit
`warning[tail-call-not-jumped]` where they emitted an error.  The rewrite already declines such
a frame without a word (`opt::tail_self`, the census's `declined`), so the call stays a call:
nothing else changes in what is emitted.  The message keeps the reason and the cure and states
the cost:

```
warning[tail-call-not-jumped]: `build` calls itself in tail position, but this call stays an
ordinary call, which stops at 10 000 frames: parameter `acc` (a `text`) can only be passed
through as itself here.  Write the recursion as a loop (`while`), or keep that value outside
the frame
```

- **The tier.**  DIAGNOSTICS.md § Diagnostic tiers gains its second clause: a diagnostic may
  also gate when it reports a cost the compiler cannot remove, names a cure that takes it to
  zero, and no published library can hold the shape when it is introduced (here: every shape
  was a compile error until now).  CLAUDE.md's summary of the tiers says the same in a line.
- **The register.**  @C155 reads "stays an ordinary call, with the warning; refused only when
  it certainly fails" (DESIGN_DECISIONS_FAILURE.md, its index line in DESIGN_DECISIONS.md), the
  old text moving to DESIGN_DECISIONS-history.md; `(R-TailSelf)` and its "In words" say the
  same where they say REFUSED.
- **The code.**  A row in DIAGNOSTICS.md; the code and a trigger in `tests/e1_code_set.rs`.
  The cure stays inline: there is no mechanical edit to offer.

**Code points:** `src/parser/definitions.rs`, `src/parser/generics.rs` (the function becomes
`warn_instance_tail`), `tests/e1_code_set.rs`, `tests/error_messages/cases/63_…` (renamed
`…_warns`, its baseline re-captured with `UPDATE_GOLDEN=1`), DIAGNOSTICS.md, CLAUDE.md, the
register, `formal/rewrites.md`; in loft-suite, `R-TailSelf/a-tail-self-call-the-frame-cannot-
jump-is-refused` becomes `…-warns` (`@EXPECT_WARNING`) and the line in
`a-self-call-outside-tail-position-stays-a-call` that points at it.

**Cases** (hand-computed):
- The suite cell's six functions, now run: `swap(1, Rec 1, Rec 2).n` 2, `swap(1, "a", "b")`
  `b`, `fresh_arg(2, "")` 2, `literal_arg([1], 2)` 1, `local_arg([1], 2)` 1,
  `rebound(Rec 0, 3)` 3 — `2 b 2 1 1 3`, each with its warning.
- The cost, stated: `fresh_arg(20000, "")` halts at the frame limit on both backends.
- `walk(a, inner[0]?)` (the lowering's shape) compiles with the warning and answers right
  at a small depth; S1.1 compiles from here.
- Falsified: the control (the T1 commit) refuses each, so the `expect` channel moves.

**Design → code.**
- As designed, plus a conditional `--explain` fix the code-set gate asks of every code: *keep
  it an ordinary call on purpose: bind its value first, `r = f(…); r`*, needing *the recursion
  stays well under 10 000 frames for every input it is given* (`@F16`).
- Case 63 also lost the unrelated `println` type error it carried, so it renders the warning
  alone (exit 0, `3`).
- The suite change is the loft-suite branch `91-self-host` (03d1f13, from the pinned
  6fbddfa); `tests/loft-suite.pin` names it.  A local run that walks the suite needs
  `LOFT_SUITE_DIR` at that checkout: `../loft-suite` holds the old refusal cell.
- Measured: the cell `2 b 2 1 1 3` with its five warnings and all six `R-TailSelf` cells on
  both backends (`make suite`); `fresh_arg(20000)` halts at the frame limit; `inner[0]?`
  compiles with the warning and answers 7.  The control (main 6f31a726b) fails the cell as
  parse errors.

#### T3 — a function that certainly never ends is refused — DESIGNED 2026-10-28

**What.**  `error[endless-recursion]` at the function's name: every way through its body
calls itself before it can finish, so no input lets it end — the one case the owner's rule
keeps a compile error for.  The answer must be CERTAIN, so the predicate only says yes where
nothing before the self-call could leave:

- a self-call is certain; so is any operator or call one of whose ARGUMENTS is (they are all
  evaluated first);
- a block is certain when one of its operators is and none before it could leave;
- `if c { a } else { b }` is certain when `c` is, or both `a` and `b` are;
- an assignment or a `return` is certain when its value is;
- everything else is not: a loop (it may run no turn), and any operator that could LEAVE —
  a `return`, a `break` out, or a call of anything but a plain operator (a user function or
  `assert` may halt, and a halt ends the program).

So `fn f(n) { f(n) }`, `fn g(n) -> integer { 1 + g(n + 1) }` and an `if` whose both arms
recurse are refused; a base case, an `assert` before the call, a call of another function
before it, or the call inside a loop are not.  Generic functions are asked per instance,
where the warning of T2 is asked.

**What it changes.**  Programs that halt at the frame limit on purpose become refused:
the `runtime_errors.rs` guards of loft#1058 (`spin1058`, `runaway1058`-shaped
`1 + f(…)`) take a base case that never fires (`if n < -1 { return 0 }`), keeping what they
measure; the other overflow guards (`deep`, `rec1058`, oracle 32) have base cases already.

**Code points:** the predicate in `src/tail_frame.rs` beside `tail_calls` (it walks the same
shapes), the refusal in `src/parser/definitions.rs` and `src/parser/generics.rs` beside T2's
warning; a row in DIAGNOSTICS.md, the code in `tests/e1_code_set.rs` (no mechanical fix: a
`FIX_BLOCKED` row naming that a base case is the author's), an error-message case.

**Cases**: the three refused shapes; the five kept ones; a deeper certain one
(`{ x = f(n); x }`); a function reached only through another (mutual recursion: not asked —
the rule is about one function's own body).

### X16–X18 — a test's module, a record's default, an enum's narrowing (fix loft#2212, loft#2209, loft#2211) — BUILT 2026-10-28

**X16.**  `tests/lower.loft`'s `use load;` bound `tests/load.loft`, the test OF load: X12 only
stopped a file binding itself.  From inside the package's `tests/` subtree `own_module_path` now
asks `src/<id>.loft` first; a helper with no module of its name is still found beside the test.
Cases (both backends, `--tests` and `loft test`): a sibling, the test named after its module, a
helper, a test one level deeper, a module chain through `src/`, and a script outside any package —
all pass.  Guard: `module_name_clash::a_test_binds_the_packages_module_not_a_sibling_test_of_its_name`.


**Found by S1.1** (`b.data.definitions[def_nr]?` in `lower.loft`), measured wider than filed:
`x?` on a record declared in another module answers `null` on `main`, silently, where the
same code in one file answers the record's default.  The record arm of `build_default`
(`src/parser/operators.rs`) builds `S {}` by sub-parsing the text `Item {}` in the CALLING
file, on a lexer whose diagnostics are dropped: there the bare name names nothing (a `null`
default), this file's own `Item` (refused as `Item` not assignable to `Item`), or, under
`#package`, an `unknown`.

**What.**  The spelling is pinned to the definition for the sub-parse (`cur_type_vars`, as
the type-variable arm already pins a placeholder), and a sub-parse that builds anything but
that record answers `None`, which the caller reports — never a default typed as the record.

**Cases** (hand-computed, both backends, `LOFT_STRICT_STORES=1`, no leak): another module's
record with declared defaults and a record nested from a third module, `5 dflt 11`; this
file's own same-named `Item`, 99; a struct enum from another module, its first variant
`hit 3`; a record that is not `pub`, reached through the functions that publish it, 42.
Guard: `tests/scripts/2209-…` with `tests/lib/default2209*.loft`.

**X18.**  `if c != null` left a nullable plain enum `Code?` (the lowering's
`operator_code(name)`), so passing it on warned.  On a plain enum the test lowers to
`OpNot(OpNot(OpConvBoolFromEnum(c)))`, and `narrowing_from_condition` read only a single
negation (a heap null test, `!v`).  One arm now reads any negation: "the test is `p` proves
`v`" is the statement "the negation is `!p` proves `v`", so it is sound for every arm below
it, and it replaces the two single-negation arms.  Cases (both backends): `!=`, `==` with the
use in `else`, the guard clause in both spellings — `2 1 0`, no warning; the null side and a
compound condition still warn.  Guard: `tests/callarg_nstore.rs`
(`a_nullable_plain_enum_narrows_like_a_record_*`); on main only `!=` warned.

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
| **X15** | a tail self-call on a part of an enum parameter is refused under @C155: `walk(n.inner[0])`, and `walk(inner[0])` through the match binding, though `(R-TailSelf)` admits a projection of a parameter (on `main`) | **bug — fixed in S1.1b T1** | the predicate sees the variant check and the binding as parts of the parameter; the owner's rules for the rest (§ Steps, S1.1b) |
| **X16** | inside a package, a test file binds a SIBLING test file named like a module (`tests/lower.loft`'s `use load;` gets `tests/load.loft`), on `main` (loft#2212) | **bug — fixed** | from the package's `tests/` subtree, `own_module_path` asks `src/<id>.loft` first (§ Steps, X16–X18) |
| **X17** | `x?` of a record from ANOTHER module falls back to `null`, with no diagnostic (on `main`, loft#2209); under `#package` the same shape is refused as a default of type `unknown`, and beside a same-named local record as `Item` not assignable to `Item` | **bug, silent-wrong — fixed (§ Steps, X17)** | `build_default` pins the record's name to its definition for the sub-parse, and a sub-parse that builds anything else is `None`, reported |
| **X18** | a nullable plain enum is not narrowed by `if x != null` (on `main`, loft#2211): its test lowers to a double negation | **bug — fixed** | `narrowing_from_condition` reads any negation as its operand's proof on the other side (§ Steps, X17 and X18) |
| **X19** | an element read is `τ?` even where the index is in range by construction, so an interpreter's frame restore and op fetch each take `?? panic(…)`; left out, the nullability spreads to every counter restored from them (37 warnings from three reads) | friction — costed (S1.2) | the discharge is honest (a missing frame is a bug); a read the bounds guard proves in range could be `τ`, as `(N-Domain)` already proves the index |

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
4. **The P-perf bar**: answered for S1 (owner, 2026-10-28) — not a gate; over 3× on the C
   backend, the performance stream is told (§ Steps, S1.4).
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
