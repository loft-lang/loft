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

**No dependency on the rustc side (owner, 2026-10-28).**  loft is to lose its rustc
dependency eventually, so the interpreter may depend on the C runtime's implementation — the
operators and natives as the C backend compiles them — but never on loft's Rust code: no
`#rust` native added for its sake, no routine that exists only in the Rust interpreter.  Its
reference semantics are then the C runtime's; where they differ from `--interpret`, the C
backend's own sweep (`scripts/c_sweep.py`) is what reports it.

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
picked from a fixed list, and always with a short code example of the cases it decides, each
line commented with what happens today and under the proposal.  `#package` (S0.3a) is the example: the question about a package
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
- **A routine the C runtime already defines is reached through `selfhost/crt`**, the
  interpreter's own library (§ S2.2), never through a stdlib declaration added for it.

**Built (2026-10-28):** `lower.loft`'s `call` follows that order (`is_operator`, the native
symbol, `has_body`).  `selfhost/kernels/stdlib.loft`: `clamp` (which calls `min` and `max`)
and `floor_mod` run from their stdlib IR — 5, 27 and 12 for `-5`, `12` and `4`, both backends —
and `abs`, a loft body over `OpAbsInt`, is refused by name at the operator.  Writing `has_body`
found loft#2213 (a never-returning `??` default read a struct-enum as absent), fixed on main's
behalf first.

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

Phase S1 (scalars, control flow, calls, the speed check: S1, S1.1–S1.4, S1.1a, X16–X18) is
built; its steps are in [1b-s1.md](1b-s1.md).  S1.1b's open steps follow below.

### S1 closed; S2.1 — the coverage census, and a `main` that returns nothing — BUILT 2026-10-28

**S1 is closed**: its gate holds (the kernels answer as `--interpret` does) and its speed is
measured and reported (S1.4; the owner: not a gate).  S1.1b T3 and T4 stay queued as the
language fixes S1 found.  **S2** carries text, floats and format strings; its gate is the
suite chapters `operational`, `calls` and `formatting` (123 cells).

**S2.1 makes "more tests" a number.**  Every later step is chosen from its output, by yield.
- **`selfhost/cover/run_program.loft`**: loads an image and runs its `n_main` with no
  arguments; a refusal goes to stderr as `refused: <reason>` with exit 3, an interpreter
  fault (a `panic`) keeps loft's own exit.  Compiled once (`--native-c --ship --out`) and run
  per program.
- **`selfhost/cover/census.py`**: over `tests/scripts/*.loft` and the language suite's cells
  (`LOFT_SUITE_DIR`), skipping what is not a plain run — `@EXPECT_ERROR`, `@EXPECT_FAIL`,
  `@ARGS`, a file with no `main` — for each: `--emit-image`, then `loft --interpret` for the
  expected stdout and exit, then the runner.  Each program is `pass` (stdout and exit equal),
  `refused` (with its reason), `wrong` (ran, answered differently) or `crash`.  It prints the
  counts and the refusal reasons ranked by how many programs each blocks, and keeps the rows
  in a TSV under `target/` (local, not committed; the plan records the counts per step).
  `--chapter` narrows to suite chapters, `--jobs` runs in parallel.
- **A function that returns nothing** (a `void` result): its body is lowered as statements,
  then `PushNull` and `Ret`, so every function leaves exactly one value; a call to one in
  statement position is followed by `Pop`, as any dropped value is.

**Code points:** `selfhost/src/lower.loft` (the result type decides value or statements),
`selfhost/cover/` (new).

**Cases:** the census runs over the whole corpus and classifies every program; a program the
interpreter carries end to end (a void `main` over integers, no output) passes; one printing
is refused naming `n_println` or the format op — the first rows of the S2 worklist.

**Design → code.**
- As designed.  The gate chapters (`operational`, `calls`, `formatting`): **48 of their 123
  cells are plain runs, 0 pass, 48 refused** — 41 first by a text literal (`NdText`), 2 by
  `OpNullRefSentinel`, then one each by a float null, a boolean literal and three more texts.
- **The census misses most of the suite**: a cell is mostly `fn test_…` functions, which
  `loft --tests` runs one by one, not a `main`.  The runner should run them the same way
  (every zero-argument function the file declares, an assert's failure being that cell's
  failure), or two thirds of the gate stays invisible.  Folded into S2.2.
- The whole-corpus baseline follows in the next commit's note.

### S2.2 — texts and floats: a stack per kind; the format operators reachable — BUILT 2026-10-28

**Found by S2.1:** 900 of the 1 025 plain programs stop first at a text, nearly all at the
empty work text a formatted string is built in.

**A stack per kind.**  The IR is typed (every op is one type's: `OpAddInt`, `OpAddFloat`,
`OpAppendText`), so the run loop keeps one stack per KIND, each holding loft values of that
kind, null included: `I` (`vector<integer?>`: integer, long, boolean, character, a plain enum —
the S1 stack), `F` (`vector<float?>`: float, single) and `T` (`vector<text?>`).  Everything
else (references, collections, tuples) stays refused until S3.
- **The lowering gives every node its kind**: a literal by its variant, a variable by its
  `type_def`, a call by the definition's `returned`, a block by its `result`, an `if` by its
  branches.  Each variable gets a slot in its kind's frame; a frame's size is three counts,
  and a function's parameters fill the first slots of their kinds, in order.
- **Ops carry the kind**: `Load`/`Store`/`Pop`/`Ret` per kind; a float or text constant is
  an index into the `Lowering`'s own constant tables (an op's operands are integers); a call
  takes its arguments from the kinds' stacks and leaves its value on its result's.
- **The float and text operators run as loft's own**: `x + y` on floats, `t1 + t2` on texts,
  the comparisons, the conversions by `as`.  A text-WRITING operator names its destination
  variable: `OpAppendText`/`OpAppendCharacter` append to that slot, `OpClearText`/
  `OpFreeText` empty it.

**The format operators run through the interpreter's own library of C runtime routines
(owner, 2026-10-28).**  `OpFormatInt`/`Float`/`Single`/`Text` have no loft spelling: a format
spec is static (F-Spec), and their radix, fill, precision, sign and direction arrive as
runtime values.  The C runtime already renders them (`loft_format_long`, `loft_format_float`,
`loft_format_text`), so the interpreter calls those, through `selfhost/crt`:
- **`crt` is a library of the interpreter's, not the stdlib's.**  Its `[c] shim`
  (`shim/crt.c`) calls the C runtime's routines, bound by `#c`, which works on `--interpret`
  (the shim is loaded), `--native` and `--native-c` (it is linked).  `src/c/loft_crt.h` makes
  the runtime's headers includable by a shim, which must then be compiled at `-O1` or above.
- **Only a routine the C runtime already defines goes into `crt`**; everything else the
  interpreter needs is loft code.  The stdlib gains no declarations for the interpreter's
  sake, and the compiler no changes: an operator's `const` parameters stay immediates.
- `crt::format_int`, `format_float` and `format_text` take the format operator's operands in
  the operator's own order and own the output buffer.  `selfhost/crt/tests/crt.loft` checks
  each against loft's own rendering of the same spec.
- **The lowering** pushes the value and then the operator's operands as ordinary values (a
  boolean as 1 or 0), and emits one `Format` op whose kind is the destination's and whose `b`
  is the value's kind: 0 integer (6 operands), 1 float and 2 single (5), 3 text (3).  **The run
  loop** pops them (an integer value lies under its operands on the integer stack, a float or
  text value on its own stack) and appends `crt`'s rendering.

**Cases** (both backends, hand-computed): a text literal, `+` on texts, `+=` of a text and a
character, a float sum and comparison, a call returning a text and one taking a float, a
null text and a null float propagating; the census before and after; the S1 tests unchanged.

**Design → code (first part, 2026-10-28).**
- Built: the stacks per kind; float, single and text operators as loft's own; the text
  writers on a slot or through a reference; and **`&text`** (`KTextRef`), which the design had
  not foreseen: a function that answers a text gets a hidden `&text` return buffer from the
  scope pass, and its caller hands it a stack reference (`OpCreateStack(__work_c1)`).  A
  `&text` variable holds the absolute index of the text it refers to, on the integer stack;
  reading it reads that text, storing and the stack forms of the writers write through it.
  Hidden parameters count as parameters, since the caller's arguments hold them.
- **A format with the plain `"{v}"` spec runs now**: the lowering checks its operands are the
  ones the parser gives a plain hole (`OpFormatInt(w, v, 10, 0, 32, false, false, 2)`,
  `OpFormatFloat`/`Single(w, v, 0, -1, 32, false, 2)`, `OpFormatText(w, v, 0, 2, 32)`) and the
  run loop appends `"{v}"`, which compiles to that operator with those operands.  Any other
  spec waited for `crt` (third part).
- A refusal now names its function, and an uncarried variable its type.
- The probe program (a text-returning call, `+=` of a text and a character, float and single
  arithmetic, a null text, a formatted boolean) prints what `--interpret` prints.  The gate
  chapters, first blockers: records (`TyReference`) 27, `n_assert` 7, `TyOptional` 3,
  `TyEnum` 2, `TyVector` 2, format specs 3 — records are S3's, `n_assert` is the first
  native.

**Design → code (second part).**
- **The first native, `n_assert`**, runs as loft's own `assert`, handed the program's file and
  line (explicit arguments are accepted), so a failure exits 1 after the program's stdout as
  under `--interpret`.  Natives are one table (`native_index`); `n_env_variable` is the second.
- Unary minus, boolean `==`/`!=`, a text's `len` and `size`, and a dropped value (`NdDrop`).
- **Two wrong answers found and fixed**, both the interpreter's: `'\0' as integer` must be null
  (loft's NUL convention, `OpConvIntFromCharacter`), which the identity had made 0 — now loft's
  own `as character?` then `as integer?`; and a hidden parameter (a generic's text return
  buffer) is created after the locals, so slots went to variables in index order and its
  argument landed in another slot — parameters now take their slots first.
- **The census: 1 025 plain programs, 16 pass, 0 wrong, 0 crash.**  The first blockers are now
  almost all values that live in the store: records 663, vectors 117, struct enums 52,
  hashes 33, tuples 17, function values 16, iterators 14, sorted 7 — S3 and S4 — then
  `OpRangeDefault` 10 (three constant operands), `OpConvIntFromEnum` 4, `OpDivIntNullable` 3.

**Design → code (third part).**
- `crt` is built as designed, green on all three backends.  Building it found loft#2217
  (`#c` read every scalar argument as 8 bytes on `--interpret`, so a boolean, enum or
  character reached C with stack garbage), fixed.
- The lowering and run loop call `crt` as designed; the plain-spec check is gone.
  `OpCastSingleFromFloat` (`x as single`) lowers to `Round`, as its conversion twin does.
- `selfhost/kernels/format.loft`: 14 specs over the three kinds (radix, `#`, fill, `+`,
  alignment, precision, a null integer, a single), each against a hand-written rendering.
  The package's tests pass on `--interpret`, `--native` and `--native-c`.
- The census's small operators: `abs` of each kind, the nullable `/` and `%` (equal to the
  plain ones for every pair), a plain enum as its number, `OpRangeDefault`, a text slice (as
  loft's own `t[a..b]`, which is that operator), and the casts by their `OpCast…` names.
  **The census: 1 018 programs, 27 pass, 0 wrong, 0 crash**; what is left is the store's.

### SU1 — `store_load` maps the image instead of copying it — DESIGNED 2026-10-28

**Why now (owner, 2026-10-28):** the store's file format IS its memory format, so a store
image can be used where it lies: mapping it costs no algorithm, and start-up then costs the
pages a run touches.  For loft code that is guaranteed: a loft program works on store records
directly, so a mapped image IS its data.  The Rust interpreter is the harder case — its
routines work on Rust structures, so its program cache decodes the mapped bundle into `Data`
(`ir_read.rs::read_data`), and leaving the bodies in the map answered wrong (loft#2012).  The
loft interpreter reads the program image with `store_load`, and so does every loft program
that ships a dataset.  Measured by S1.4's `load us` column, before and
after.

**What happens today** (`src/database/allocation.rs::load_path`, `src/store.rs::Store::load`):
the file is read into a buffer (`image_bytes`), copied again into the store's word-aligned
allocation, and its recorded-words header cleared.  Two copies of the whole image per load.

**Where it lands (owner, 2026-10-28: no rustc dependency).**  The interpreter ships through
the C backend, whose runtime reads the image with an `fread` loop into a buffer it then adopts
(`src/c/loft_image.h`, `store_load`).  So the mapping is built in the C runtime FIRST — POSIX
`mmap(MAP_PRIVATE)`, a copy-on-write view on Windows — and the Rust runtime's
`Store::load` follows only to keep `--interpret` level with it.

**The change** (the same in both runtimes; the Rust names below): where the build maps files
(the `mmap` feature: not wasm), `Store::load` maps the image **copy-on-write**
(`memmap::MmapOptions::map_copy`, private) instead:
- **The promise of `store_load` is kept as written**: "writes after it stay in memory" — a
  written page is the process's own copy, and the file never changes.  The header clear is
  such a write.
- **The first growth moves the store to the heap**: a private mapping cannot grow, so
  `resize_store` allocates, copies the mapped words, drops the mapping, and goes on as a heap
  store.  Drop releases the mapping, never a heap free of a mapped pointer.
- **The precondition**: the file is not truncated in place while mapped (a read past the end
  of a mapping is a SIGBUS).  loft's own writers replace a file by rename
  (`persist_copy`'s `.tmp` + `rename`), which leaves a live mapping on the old file; a
  foreign writer that truncates is outside the rule.
- **`(S-Mapped)` widens** (`formal/structures.md`): besides the runtime's own files, an IMAGE
  loaded by `store_load` may be mapped copy-on-write; what a program reads is unchanged
  (`(S-Paged-Same)`, the whole-image load's answers), and the file is never written.
- **A switch to bisect by**: `LOFT_NO_MAPPED_LOAD=1` keeps the copy (BOTH_BACKEND_SWITCHES.md).
- The memory ceiling counts the mapped size as today's copy does, so a limit run answers the
  same.

**Code points:** `src/c/loft_image.h` (`store_load`: map instead of `fread`, the first growth
copies), then `src/store.rs` (`Store::load`, a private-mapping backing beside `file`,
`resize_store`, `Drop`), `src/database/allocation.rs::load_path` (unchanged in shape),
`doc/claude/formal/structures.md` (S-Mapped), BOTH_BACKEND_SWITCHES.md.

**Cases** (both backends, `LOFT_STRICT_STORES=1`, `LOFT_POISON=1`): the whole store-persist
and paged-load corpus through it; a write after load answers in memory and leaves the file's
bytes unchanged; a growth after load keeps every record; a `store_persist_copy` over the
mapped file while it is loaded leaves the loaded values as they were; the switch restores the
copy; S1.4's load column before and after.

**After it, SU2**: the loft interpreter keeps its lowered ops in a store of its own beside the
image, mapped at start-up the same way — functions are named by their global definition
number inside one image, so nothing needs relocating.

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
| **X14** | a block-bodied match arm demands a comma (`NdIf { … } => { … }` then the next arm), while Rust and the `LOFT_CONTROL.md` example leave it out | **bug — fixed in S1.1a** | the comma is optional after a block body ([1b-s1.md](1b-s1.md) § S1.1a) |
| **X15** | a tail self-call on a part of an enum parameter is refused under @C155: `walk(n.inner[0])`, and `walk(inner[0])` through the match binding, though `(R-TailSelf)` admits a projection of a parameter (on `main`) | **bug — fixed in S1.1b T1** | the predicate sees the variant check and the binding as parts of the parameter; the owner's rules for the rest (§ Steps, S1.1b) |
| **X16** | inside a package, a test file binds a SIBLING test file named like a module (`tests/lower.loft`'s `use load;` gets `tests/load.loft`), on `main` (loft#2212) | **bug — fixed** | from the package's `tests/` subtree, `own_module_path` asks `src/<id>.loft` first ([1b-s1.md](1b-s1.md) § X16–X18) |
| **X17** | `x?` of a record from ANOTHER module falls back to `null`, with no diagnostic (on `main`, loft#2209); under `#package` the same shape is refused as a default of type `unknown`, and beside a same-named local record as `Item` not assignable to `Item` | **bug, silent-wrong — fixed ([1b-s1.md](1b-s1.md) § X16–X18)** | `build_default` pins the record's name to its definition for the sub-parse, and a sub-parse that builds anything else is `None`, reported |
| **X18** | a nullable plain enum is not narrowed by `if x != null` (on `main`, loft#2211): its test lowers to a double negation | **bug — fixed** | `narrowing_from_condition` reads any negation as its operand's proof on the other side ([1b-s1.md](1b-s1.md) § X16–X18) |
| **X19** | an element read is `τ?` even where the index is in range by construction, so an interpreter's frame restore and op fetch each take `?? panic(…)`; left out, the nullability spreads to every counter restored from them (37 warnings from three reads) | **by design — costed** (S1.2) | the discharge is honest (a missing frame is a bug); a read the bounds guard proves in range could be `τ`, as `(N-Domain)` already proves the index |
| **X20** | a bare variant name is refused as a branch of an `if` assigned to an untyped local (`h = if c { Red } else { Blue }`: *"bare variant 'Red' has no type here"*), while a typed local, a `return`, a comparison and a match arm resolve it; the lowering wrote `Kind.KNone` in such places | **language — fixed, @C157** | owner: a name is refused only when two definitions own it and nothing in its context picks one; a sole owner resolves, and an `if` or `match` arm takes the enum its sibling arms name |
| **X21** | the run loop's `prog = progs[def] ?? panic(…)` on a call and on a return copies the whole `Lowering` record each time (`(B-Copy)`: a record local is bound by value, and nothing proves the one write into `progs` misses the element `prog` names); loft-13 measured `loft_copy_claims` at 46 % of `run_kernel`'s time on fib under `--native-c --ship`, which is most of the 20× against the Rust interpreter | friction — measured; with the owner (loft-13's memory-model proposal: a read-only copy stays a view until its source is written) | the code stays as written (the readable-code ruling); a binding of `progs[def].ops` alone is held in place and would show the gap |

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
   backend, the performance stream is told ([1b-s1.md](1b-s1.md) § S1.4).
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
