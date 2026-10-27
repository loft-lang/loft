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

### S0.1 — the bundle carries its layout sidecar (fixes X1) — BUILT 2026-10-27

**What.**  `save_bundle` writes a `.dschema` sidecar beside the bundle, as every other
store writer does, so `store_load` can check the file against the type it is read through
(`@FR-L-Sound`) instead of refusing it as a bare image.

**Code points.**
- `src/ir_store.rs:809` `save_bundle`: after `materialize_bundle`, compute the root's
  layout identity and stage the sidecar beside `tmp`; rename the sidecar together with the
  bundle (the store first, the sidecar after, as the program cache's manifest is written
  last, `src/startup_cache.rs:463`).
- `src/schema_sidecar.rs:103` `LayoutIdentity::of_store(stores, slot, tp)` and `:336`
  `stage_beside`: existing, reused unchanged.
- `src/data_store.rs:400` (`IR_ROOT_REC`), `:487–489` (`BUNDLE_DATA`, `BUNDLE_TYPES`): the
  root the identity is computed for, the `Bundle` type of the IR schema.  To find while
  building: the type number of `Bundle` in the schema `data_store` registers.
- The refusal it removes: `src/database/allocation.rs:7105` (`BARE_IMAGE`), reached from
  `load_path` `:5350` through `schema_gate_ok`.

**Failure paths to cover.**
1. The sidecar is written but the identity is computed from the wrong root type: then
   `store_load` refuses with a layout mismatch instead of "bare". The test reads through
   `Bundle`.
2. The Rust IR schema and `ir.loft` compiled by loft lay `Bundle` out differently: the
   identities differ, and the load is refused naming the type. **This is the falsification
   the step exists for**: "the same types" is then a claim that does not hold, and the
   drift is fixed in the generator, not worked around.
3. A crash between the two renames leaves a bundle without a sidecar: it is refused as
   bare (the state before this step), never read unchecked.
4. The startup cache's warm load (`ir_read::open_bundle`) must ignore the sidecar: it is
   unchanged and must stay byte-identical. Covered by `ir_schema_roundtrip`.

**Tests.**  A Rust test beside `ir_read`'s bundle round-trip (`src/ir_read.rs:2134`):
`save_bundle` to a temp path, assert the sidecar exists and its verdict against a fresh
`Stores` holding the IR schema is raw-safe.  `cargo test --release --test
ir_schema_roundtrip` stays green.  The loft-side guard waits for S0.2, which gives a test
a stable bundle path.

**Not in this step:** a CLI flag to write a bundle at a chosen path (S0.2, designed after
this lands); the loft loader `selfhost/src/interp/load.loft` (S0.3).

**Built — what changed from the design.**  This is a step in the Rust tree, so it measures
the design rather than loft.  The test is
`ir_read::tests::a_saved_bundle_carries_the_layout_ir_loft_declares`.
- **Failure path 2 was wrong as written.**  A changed layout is not refused: `load_path`
  CONVERTS it (`load_upgraded`, `src/database/allocation.rs:~5361`).  So a drift between
  `ir.loft` and the Rust schema would have been converted silently.  The test therefore
  requires the verdict `Match` for both sides, not "the load succeeds".
- **The identity comes from a `Stores` of its own** (`register_ir_schema`,
  `src/ir_schema_gen.rs:238`, whose `IrSchemaIds::bundle` is the root).  So the image's
  bytes cannot change (failure path 4).  The write follows `@FR-S-Pair` exactly: stage the
  sidecar, rename the image, then rename the sidecar.
- **The sidecar staging name is fixed** (`<path>.dschema.tmp`, `src/schema_sidecar.rs:
  stage_to`), not per-process.  The C runtime and the guard `tests/scripts/2173-…` depend
  on it, so it stays.  For the bundle it is harmless: two processes saving one program
  write identical text, and any failure leaves the bundle refused, never read unchecked.
- **`Parser::parse` answers "no diagnostic at all"** (`src/parser/passes.rs:158`
  `self.diagnostics.is_empty()`), so an advice made it return false.  The test's first
  falsification failed at its "does not compile" assertion and never reached the layout
  comparison.  The test now asserts no error-level diagnostic.
- **Falsified:** a field added to `Bundle` in `ir.loft` fails the test at the layout
  assertion (`Changed(LayoutDiff { changed: ["Bundle"] })`); the restored file passes.
- **End to end:** a loft program declaring `ir.loft`'s types `store_load`s a bundle the
  rebuilt binary wrote (`fn twice` + `fn main`) and reads its 845 definitions, `n_twice`
  and `n_main` among them.  So the root is where `store_load` places it.
- **A consequence the design missed: eviction.**  Every program bundle and stdlib image in
  the startup cache now has a sidecar, and neither eviction removed it
  (`src/cache.rs: prune_dir`'s `remove_pair`, `prune_stdlib_dir`).  Both now remove it,
  naming it through `schema_sidecar::dschema_path`, and their tests
  (`prune_dir_evicts_oldest_over_budget`, `stdlib_images_keep_the_newest_and_nothing_else`)
  assert that an evicted bundle's sidecar goes with it and a kept one's stays.
- **Learned for later steps:** the cached image is taken after `scopes::check` AND after
  `compile::close_program` (`src/main.rs:~10236`).  So "the same IR" is the IR after the
  program-level rewrites too.  This matters for ownership.md's identical-IR boundary.
- **Not run:** `ir_schema_roundtrip`, which the design named.  It never calls
  `save_bundle`, so it could not move.

### S0.2 — `loft --emit-image <out.store> prog.loft` — BUILT 2026-10-27

**What.**  A command that writes a program's IMAGE (the bundle exactly as the startup cache
saves it, with its S0.1 sidecar) to a path the caller names, and runs nothing.  It is how a
test, the loft-suite `selfhost` mode and the loft interpreter get the IR of any program.
@PLN205 names the same artefact as the input of a backend that is a separate part, so this
is its first spelling.  The name follows `--emit-c <out.c>`.

**Code points.**
- `src/main.rs:~7621` (beside `emit_c`): `let mut emit_image: Option<String>`; `:~7793`
  (beside the `--emit-c` arm): parse `--emit-image <path>`, `native_mode = false`, as
  `--emit-c` does.
- `src/main.rs:~10213–10236`, the program cache's save point: under `--emit-image` the run
  reaches the same state the cache saves (cold parse, `scopes::check`, `close_program`
  under the same openness), then calls `ir_store::save_bundle(&p.data,
  &p.database.types, out)` and exits 0.  The program cache is neither read nor written in
  that run, so the image is always a fresh parse and the user's cache is untouched.
- `src/main.rs: print_help` (`:~230`, beside `--native-emit`): one line.
- `src/ir_store.rs: save_bundle`: reused as S0.1 left it.
- Under a build without the `mmap` feature (the wasm build), `save_bundle` does not exist:
  the flag is refused naming why.

**Failure paths.**
1. **The image is not the cache's image** (taken before `close_program`, or under another
   openness): the "same IR" claim then means two different things.  The test compares the
   emitted image byte for byte with the bundle the cache writes for the same program, in a
   cache directory of its own (`XDG_CACHE_HOME`, `src/cache.rs: cache_base_dir`).
2. **A program with errors**: the diagnostics print as for a run, the exit is non-zero,
   and no file is written.
3. **A path that cannot be written**: exit 1 naming the path, and nothing partial is left
   (`save_bundle` writes through a temp file and renames).
4. **The program runs anyway**, for example because a later mode switch overrides the
   flag: the test's program prints, and the test asserts that stdout is empty.
5. **`--emit-image` without a path**: refused, as `--emit-c` without one is.

**Tests.**  A Rust test beside `tests/c_emit.rs` (`tests/emit_image.rs`) that runs the built
binary:
- path 1: the emitted image is byte-identical to the cache's, its sidecar exists, exit 0;
- path 2: an erroring program exits non-zero and writes no file;
- path 3: an unwritable path exits 1 and leaves nothing;
- path 4: stdout is empty.

Then the loft-side guard can exist: `tests/scripts/` cannot run a second process, so it
belongs to S0.3, whose loader reads an image the test harness emits.

**Not in this step:** the loader in loft (S0.3).

**Built — what changed from the design.**  The tests are in `tests/arc_e_program_cache.rs`
(`emit_image_writes_the_cache_image_and_runs_nothing`,
`emit_image_refuses_without_leaving_a_file`), next to the program cache they compare with.
- **Failure path 3 met a bug on `main`: X11.**  `Store::open` panics on a file it cannot
  create (`src/store.rs: open_mapped`).  The startup cache calls `save_bundle` on every cold
  run, so a read-only cache directory crashed every such run with an internal compiler
  error, and a warm load from one did too.  Fixed first, under the dogfood rule:
  - `Store::try_open` (a file that cannot be created, opened or sized is an `io::Error`);
  - `open_read_surface` returns `io::Result`;
  - `adopt_read_surface` (`src/ir_read.rs`), the one home of the three loaders, makes it
    their cache miss;
  - `save_bundle` and `save_data` propagate it.

  `open` keeps panicking for callers that treat it as a broken invariant.  The matrix:
  - C1 read-only cache dir: was exit 101, now runs.
  - C2 read-only cache base: was exit 101, now runs.
  - C3 `--native`: runs.
  - C4 warm load from a read-only cache: was exit 101, now a miss and a cold parse.
  - C4b writable warm: still a hit.
  - C6 user store builtins: never reach `Store::open`.

  Guard: `a_read_only_cache_is_a_miss_never_a_crash` (C1, C2, C4; skipped under root, where
  permissions do not bind).  Left as it is: a read-only cache is never USED warm, because
  the read surface maps the file read-write.  That is slower, not wrong.
- **"The program cache is neither read nor written" holds; the standard library's image is
  another cache.**  The first run of the test asserted the whole cache directory was
  untouched, and failed: the stdlib image (one per build, shared by every program) was
  written.  The `--interpret` run the image is compared with uses it too, so the assertion
  was narrowed to `program-*` files.
- **Falsified:** without `close_program` the emitted image differs from the cache's image,
  so the byte comparison is not vacuous.
- `CHANGELOG.md`: the flag under New, the read-only cache under Bug fixes.

### S0.3a — `#package`: a file shows its internals to its directory and the one above — BUILT 2026-10-27 (catalogue entry at the merge)

**Why here.**  S0.3 met X2: under C140 every type and field is private to its FILE, so a loft
program split over several files pays `pub` on everything it shares between them (167 items
for `ir.loft` alone).  The owner's ruling (2026-10-27): the file-local default stays, because
an author should think about what escapes a file; a file that is part of something bigger
says so, and position decides the group.  No `loft.toml` is involved.  A language fix found
by the dogfood work, built before the work continues.

Privacy here is not about hiding information.  It is about ENFORCING THE RIGHT ABSTRACTIONS
(owner): code above a layer must go through that layer's surface.  So the question is never
"who may know this?" but "which code composes this?".

**The rule.**  One sentence, from which every case follows:

> A file whose directives include `#package` reads, to every other file in its own
> directory and in the directory directly above it, as if every item it declares and every
> field of those items were `pub`.  To any other file it reads as C140 says.

```
#package
struct Frame { pc: integer, base: integer }   // run.loft, lower.loft, … beside it see pc
fn step(f: Frame) -> integer { f.pc + 1 }     // and may call step, build a Frame, match it
```

Consequences, each a case of that sentence rather than a rule of its own:
- **One-sided consent.**  The file that shows its internals says so; a file that reads them
  says nothing.  A directory may mix: `run.loft` and `lower.loft` say `#package`, while
  `store.loft` leaves it off and keeps its internals behind its `pub` functions.
- **Its directory and one up (owner, 2026-10-27).**  Deeper directories hold the more
  detailed algorithms and data structures, and the level above composes them, so a detail
  shows its internals to its own directory and to its direct composer: `interp/store/arena.loft`
  to `interp/store/` and to `interp/run.loft`.  Not further up, so each layer is open to the
  one that builds on it and walled from everything beyond; and never down, because a detail
  needing its composer's internals would be a cycle in the design.  A published library is
  `<pkg>/src/*.loft`, whose parent `<pkg>/` holds no code, so the extra level grants nothing
  there.  A flat file vendored into a project's `lib/` shows its internals to the project root,
  which is acceptable: the reason is the abstraction, not secrecy, and vendored code already
  sits in its consumer's tree.
- **Membership is location, never route.**  Whether a file was reached by `use`, `--lib` or
  the registry does not matter; what decides is where it is.  A library's files share with
  each other, while its consumers, in other directories, see its `pub` surface as before.
- **Imports follow.**  To a directory mate, a `#package` file's items are `pub`, so a glob
  `use a;` binds them bare and a selective `use a::helper` binds one.  Two mates exporting
  the same name collide exactly as two libraries' `pub` names do (the existing rule).
- **It only relaxes.**  A program that compiles today compiles unchanged; only refusals
  between directory mates disappear.  So it is compatible (COMPATIBILITY.md).
- **`api_surface` is unchanged.**  `#package` grants nothing outside those two
  directories, so a library's published surface (`pub_visible`) is the same.  Below,
  "a mate" means a file in the directory or the one above.

**Code points.**
- `src/parser/mod.rs:~2320`, the directive loop beside `#cwd` and `#link`: `Some("package")`
  records the file; the unknown-directive message lists `#package` too.
- `src/data.rs`, beside `is_name_only` (`:~12757`): the ONE home of the question.
  - `package_files: HashSet<…>` holds the canonical paths of the files that said
    `#package`, filled in pass 1, read in pass 2.
  - `Data::shares_internals(owner_file, user_file) -> bool` answers: owner said `#package`,
    and the user's canonical directory is the owner's, or the owner's parent.  Cite
    `@FR-F-Package`.
  - `source_files` (`:5728`) gives a library source's file.
- The four refusal sites, each of which has its own copy of "same source or same file"
  today, gain `|| shares_internals(owner, here)`:
  - `src/parser/visibility.rs: check_visibility` (`:~80`): fields, literals, variants,
    patterns, keys;
  - `refuse_unpublished_method` (`:~171`): `.m()` and `lib::m`;
  - `refuse_building_a_name_only_type` (`:~214`);
  - `Data::qualified_refusal` (`src/data.rs:~12786`), which gets the user's file from its
    callers `parser/definitions.rs:4651` and `parser/objects.rs:649`.
- Imports: `Data::exported`, `exported_names` and `passes_on` (`src/data.rs:12724–12752`)
  judge per library and do not know the importer.  They get the importing source, so the
  bindings at `:12408`, `:12474`, `:12617`, `:12699` and `:12715` admit a mate's items.
- Docs, see sub-step 4.

**Failure paths (the matrix).**  Axes: directive (present / absent) × place (same directory,
the parent, the grandparent, a subdirectory, a sibling directory) × access (field read, field write,
literal, variant build, variant match, `lib::fn`, method, a name-only type built, glob
import bare, selective import) × direction (the declaring file has it / the reading file
has it).
1. **Only the declaring file's directive counts**: B reading A's internals, where only B
   says `#package`, is refused.
2. **The parent is admitted; the grandparent, a subdirectory and a sibling directory are
   refused** (`src/sub/x.loft` reading `src/a.loft` is downward; `src/b/y.loft` reading
   `src/a/x.loft` is a sibling).
3. **Path spelling**: `./a.loft`, an absolute path and a symlinked directory name the same
   directory (canonical parents); two directories with the same last name do not.
4. **A collision through glob imports** from two mates reports as two libraries' names do.
5. **The warm cache**: removing `#package` changes the file's hash, so the next run parses
   and refuses again, never a stale admission.
6. **The census** (`LOFT_TRACE_VISIBILITY=1`, `scripts/pub_census.sh`) stops listing a
   mate's use, because it reads the same check.
7. **The stdlib and registry libraries** carry no `#package`: unchanged.

**Sub-steps** (each small, tested and pushed before the next):
1. **Built** (`e9667b46d`).  The directive is parsed and recorded; `shares_internals` with
   unit tests (same directory, subdirectory, parent, unrelated, no directive, spelling
   variants).  No behaviour change yet; the unknown-directive message names `#package`.
   Revised with the parent level: the parent cell is admitted, and the grandparent and a
   sibling are refused.
2. **Built.**  The four refusal sites consult it.  Matrix cells 1–3 and 6 for every access
   kind, in `tests/pub_visibility.rs`, beside C140's own cells.  **Found while building:** a
   file a `use` switches into never passed the directive loop (the lexer switches mid-parse,
   `Parser::switch_to_dep`), so a used file's `#package` was a syntax error.  The loop is now
   `Parser::parse_file_directives(entry)`, also run after the switch.  In a used file only
   `#package` is admitted: `#cwd` and `#link` set how the whole program runs, and are refused
   there naming where they belong (they were "unexpected '#'" before; no corpus or registry
   file says either in a used file).
3. **Built.**  Imports.  One question for all four binding paths (`import_all`, `import_name` and their
   `_overwrite` twins, `src/data.rs`): `Data::importable(key, lib, into) -> Option<(def,
   passes_on)>`, which is `exported` widened by `shares_internals` against the importer's file
   (`source_files[into]`).  `importable_names` serves the glob form.  A shared item binds
   with `passes_on == false` even under `pub use`, so a mate cannot carry another file's
   internals out of the two directories.  The diagnostic helpers that ask `exported` to find
   a cure (which library publishes a name) keep asking the public surface.  Cells:
   - a glob `use units;` binds a private function and type bare;
   - a selective `use units::(secret)` binds one, and `use units::(twice)` binds a method;
   - two mates exporting one name collide as two libraries' names do (cell 4);
   - **no leak through `pub use`**: a file two levels up that imports a mate which wrote
     `pub use units::*;` reaches only `units`' `pub` items.

   **Built — what changed from the design.**
   - **The `passes_on == false` flag was a duplicate.**  Falsifying the no-leak cell by
     removing it changed nothing.  Whether a binding re-exports is `Data::passes_on`'s, and it
     is judged on the DEFINITION (`pub_visible`), so a private item never passes on through
     `pub use` whatever its binding says.  The flag was removed: `importable` answers only
     whether a name can be imported, and the one rule for re-export stays where it was.  The
     cell is falsified on the path that can leak instead: with `shared_with` ignoring the
     importer, the far program reaches `secret` and prints 42.
   - **A confounded negative.**  The "two levels up" import cell first read a private field
     too, so `check_visibility` refused it whatever the import did; under the same mutation
     it still passed.  It now calls a bare private function only, and fails under the
     mutation.
   - Tests: `package_items_import_bare_for_a_mate`, `package_names_collide_as_pub_names_do`
     (the same refusal as two `pub` names, compared line for line),
     `package_internals_do_not_pass_on_through_pub_use`.
4. **Built**, but for the catalogue entry.  The documentation.
   - `formal/calls.md`: `(F-Visible)` names `(F-Package)`, a new rule holding the sentence
     above.
   - `DESIGN_DECISIONS_SYNTAX.md` C140: the owner's ruling of 2026-10-27, recorded.
   - `LOFT_DECLARATIONS.md` § visibility, and `LOFT_DESIGN.md` beside `#cwd`/`#link`.
   - `CHANGELOG.md` New, and the `loft-write` skill.
   - The `@F47` catalogue entry (library imports / modules; C140 named `@F2`, which is `??`,
     and now names `@F47`): edit the features issue, then `make features-fetch && make
     features-gen`.  **Deferred to the merge**: the tracker is shared by every branch, so
     editing it now would hand `main`'s next `features-fetch` the documentation of a feature
     only this branch has.
   - A `tests/reference` snippet: not needed.  `LOFT_DECLARATIONS.md`'s example is short
     and inline, and the cells in `tests/pub_visibility.rs` carry the behaviour.

**Not in this step:** sharing downward, beyond one level up, or across sibling directories.
Each can be widened later if a real program asks for it; a widening is a relaxation.

### S0.3b — a file never `use`s itself (fixes X12) — BUILT 2026-10-27

**Found by S0.3.**  `selfhost/tests/load.loft` says `use load;` and every `load::…` read
*"Unknown function"*: the import bound the TEST FILE itself.  It is on `main` (2026.10.0), and
it breaks the layout PACKAGES.md documents (`tests/draw.loft` testing module `draw`).

**Cause.**  loft#976's rule says a bare `use <module>` inside a package binds the package's
own `src/<module>.loft`.  The code that finds that file, `Parser::own_module_path`
(`src/parser/imports.rs:822`), looks beside the CURRENT FILE (`<dir>/<id>.loft`,
`<dir>/lib/<id>.loft`): right for a file in `src/`, but from `tests/load.loft` the first
candidate is the file itself.  A file is never its own module, and the rule's home is
`src/`.

**Fix.**  `own_module_path` never answers the current file, and after the two
current-directory candidates it asks the package root's `src/<id>.loft`.  The root comes
from `resolution_scope::project_root`, the one home of "the package a file is in", which the
same `use` code already asks (`src/parser/mod.rs:~2465`).  The order keeps a file in `src/`
exactly as it is.

**Matrix (measured on `main` before the fix):**
- M1 `pkg/tests/load.loft` says `use load;`, with `src/load.loft` present: the bug.  Must bind
  `src/load.loft`.
- M2 `pkg/tests/loader.loft` says `use load;`: works today through the flat library route.
  Must keep working, now through the own-module route.
- M3 `pkg/src/other.loft` says `use load;` for a sibling: works.  Unchanged.
- M4 a script outside any package, named like the `--lib` library it uses: works (the
  advice names the clash, the library wins).  Untouched: no package, no own-module rule.
- M5 `pkg/tests/pkg.loft` says `use pkg;`, the package's own name: works through the
  special case beside the rule.  Unchanged.

**Tests.**  `tests/module_name_clash.rs`, beside loft#976's cells: M1 and M2 as `loft test`
runs of a package; M3 and M5 stay covered by the cells there.  The selfhost package's own
test then runs as S0.3 designed it.

**Built.**  As designed; one cell, `a_test_named_after_its_module_binds_the_module_not_itself`
(M1 and M2, parsed in process like loft#976's cells rather than through `loft test`, which
reaches the same `use` code).  Falsified: without the "never the current file" filter M1
fails again.  The module-clash suite (21) is green.

### S0.3c — a package's tests see what `#package` shares — BUILT 2026-10-27

**Owner's ruling (2026-10-27): tests are part of the package.**  Under S0.3a a package's
`tests/` sits beside `src/`, so it was neither the shared directory nor the one above, and a
test of an internal step (lowering, say) could only reach it by making it `pub` for every
consumer.

**The rule** (an addition to `(F-Package)`'s sentence, still purely positional):

> A file anywhere inside a directory named `tests` (that directory or below it) reads
> `#package` sharing as if it sat in the `src` directory beside that `tests`.

So `pkg/tests/lower.loft` and `pkg/tests/lowering/if.loft` both see what `pkg/src/lower.loft`
shares (`src/` is its directory) and what `pkg/src/interp/x.loft` shares (`src/` is the one
above).  The subtree is included because tests are not a layer (owner): nothing composes on
top of a test, so depth inside `tests/` walls nothing off.  Among the `src/` modules the
layering is unchanged.  Nothing depends on a
`loft.toml`: `src/` and `tests/` are the package layout PACKAGES.md already fixes, and a
directory that only happens to be called `tests` gains something only when a `src/` beside
it holds a `#package` file.

**Code point.**  `Data::shares_internals` (`src/data.rs`), the one home: before comparing
places, a user directory inside a `tests` directory (the nearest ancestor so named, itself
included) is replaced by that directory's sibling `src`.  The
imports (`shared_with`) and the four refusal sites follow, because they ask it.

**Matrix.**
- `pkg/tests/t.loft` reading `pkg/src/a.loft` (`#package`): admitted.
- `pkg/tests/t.loft` reading `pkg/src/interp/x.loft` (`#package`): admitted (one up).
- `pkg/tests/sub/t.loft` reading `pkg/src/a.loft`: admitted (the subtree of `tests`).
- `pkg/src/tests/t.loft` reading `pkg/src/a.loft`: refused (a `tests` INSIDE `src` is a
  subdirectory of `src`, and its sibling `src` does not exist).
- `other/tests/t.loft` reading `pkg/src/a.loft`: refused (another package's tests).
- `pkg/tests/t.loft` reading `pkg/src/a.loft` without `#package`: refused (C140).
- End to end: `tests/pub_visibility.rs` builds and reads a private item from `pkg/tests/`.

**Docs.**  `(F-Package)` gains the clause, plus C140's ruling, `LOFT_DECLARATIONS.md`, the
`loft-write` skill and the CHANGELOG entry.

**Built — what changed from the design.**  The mapping is applied IN ADDITION to the plain
comparison, never instead of it: a `#package` helper inside `tests/` must still share with
the tests beside it (cell `a test helper shares with tests`), and replacing the user's place
outright would have hidden it.  Tests: `data::package_tests::package_tests_read_as_their_src`
(the subtree, one up from `src/interp`, `src/tests` refused, another package's tests refused,
the helper) and `pub_visibility::package_tests_see_what_src_shares` (end to end from
`pkg/tests/lowering/`, a consumer refused).  Falsified: without the mapping the end-to-end cell
fails.

### S0.3 — the bundle opens in loft — DESIGNED, next

**What.**  The first loft code of the strand: a `selfhost` package whose module `load` opens
an image (S0.2) as the IR's own records and answers questions about it.  It is the first
step that measures loft.  The design below is written the way the problem reads; the
design→code note after the build records where loft made it read otherwise.

```loft
// selfhost/src/load.loft — as the reasoning goes
use ir;

// The image at `path`, or null when it is missing, written under another layout, or not
// an image at all (store_load refuses each, naming why).
pub fn load(path: text) -> ir::Bundle? {
  b = ir::Bundle {};
  if !store_load(b, path) { return null; }
  b
}

// The definition the compiler named `name` (`n_main` for `fn main`), or null.
pub fn definition(b: ir::Bundle, name: text) -> ir::Definition? {
  for d in b.data.definitions { if d.name == name { return d; } }
  null
}

// How many definitions the image holds: the stdlib's and the program's.
pub fn definitions(b: ir::Bundle) -> integer { len(b.data.definitions) }
```

```loft
// selfhost/tests/load.loft — the loader seen from outside, through its pub surface
use load;

fn test_an_image_opens_as_the_ir() {
  b = load::load(env_variable("LOFT_SELFHOST_IMAGE"));
  assert(b != null, "the image opens");
  assert(load::definitions(b) > 800, "the stdlib and the program: {load::definitions(b)}");
  assert(load::definition(b, "n_twice") != null, "the program's own function is there");
  assert(load::definition(b, "n_nowhere") == null, "a name the program lacks is not");
}
```

**Code points** (revised 2026-10-27, after S0.3a and two probes).
- `selfhost/loft.toml`, `selfhost/src/load.loft`, `selfhost/tests/load.loft`: new.
- `selfhost/src/ir.loft`: the IR vocabulary, moved from `tools/ir_schema/ir.loft` (decision 1),
  with `#package` as its first line (S0.3a), so `load.loft` beside it reads every type and
  field.  Its `fn main` goes: measured, the generator emits the committed schema byte for
  byte without it, and with `#package` too.  So no driver file is needed.  The `@FTR-078`
  worked-example tag above that `main` moves to the file's head (it describes the whole
  vocabulary), and `make examples-index` re-points `examples-index.tsv`.
- The generator follows the file: `scripts/ir_schema_check.sh` (`SRC`), the Makefile's
  `ir-schema-check`/`ir-schema-regen` help text, `tools/ir_schema/README.md`, the header
  `extract.py` writes into `src/ir_schema_gen.rs` (regenerated with `--fix`; only the
  header line may change), the doc comments that name the old path (`src/data.rs`,
  `src/ir_store.rs`, `tests/doc_hygiene.rs`), and the S0.1 test's path
  (`src/ir_read.rs: a_saved_bundle_carries_the_layout_ir_loft_declares`).
- **Tests go through `load`'s `pub` surface.**  `selfhost/tests/` is a sibling of `src/`, so
  it is not a mate of `ir.loft` (S0.3a).  That is the right abstraction for a test of the
  loader: `pub fn load(path) -> Bundle?`, `pub fn definition(b, name) -> Definition?` and
  `pub fn definitions(b) -> integer` are what it offers, and `Bundle` and `Definition`
  are name-only outside the package.
- `tests/selfhost.rs` (new binary; the selfhost suite will grow into it): emits an image of
  a two-function program with `loft --emit-image`, then runs `loft test` in `selfhost/` with
  the image's path in `LOFT_SELFHOST_IMAGE`, which the loft test reads with
  `env_variable`.

**Failure paths.**
1. `store_load` into a `Bundle {}` literal places the root elsewhere than S0.1's probe
   (which declared the types in the same file): definitions read empty.  The test asserts
   the count and two names.
2. An image of another layout (an older build): `load` answers null, and the refusal
   names the type.  Test: an image whose sidecar's `Bundle` row is edited.
3. Not an image (any other file): null.
4. `pub` changes a layout after all: the S0.1 test fails (it compares the sidecar with
   `ir.loft` compiled by loft).

**Owner decisions.**
1. **`ir.loft`'s home.**  Decided (owner, 2026-10-27): move it into `selfhost/src/` as its
   one home, and point the generator at it.  The alternative is a `path` dependency on
   `tools/ir_schema/`, which leaves the vocabulary in a tools directory while it becomes
   the core.
2. **X2, `pub` on 167 items** (C140: privacy is per FILE; there is no package scope).
   First decided "pay it"; then the owner designed the cure, `#package` (S0.3a), built
   first.

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
| **X1** | `store_load` refuses the program bundle: *"it has no layout sidecar (`.dschema`) beside it"*. The bundle carries its own schema (`Bundle.types`), but `ir_store::save_bundle` writes no sidecar, so loft cannot read the compiler's own output | **bug — fixed in S0.1** | `save_bundle` writes the sidecar (§ Steps, S0.1) |
| **X2** | `ir.loft` used as a library: every type and every field is private to its FILE (@C140; no package scope), so `use ir; ir::Bundle` is refused | **language fix — S0.3a** | measured: 167 `pub` (25 types, 142 fields) before another file of the same package reads one field.  The owner's cure: `#package` shows a file's internals to its own directory (S0.3a), and the file-local default stays |
| **X3** | a generic type alias does not parse: `type Env<T> = hash<B<T>[name]>;` → *"Expect token ;"* | friction | generic aliases, OCaml's `type 'a env = …` |
| **X4** | `hash<(integer, text)>` → *"tuple element types only allowed on vector / iterator"* | friction | tuple elements, and tuple keys, in keyed collections (OCaml `(int * string, v) Hashtbl.t`); memo tables keyed by (definition, type) want it (bar B2) |
| **X5** | a phantom tag needs a dummy field: `struct NodeTag { }` → *"Expect attribute"* | friction | an empty struct, or a declared abstract type with no body (OCaml `type node_tag`) |
| **X6** | a generic literal in a typed field does not take `T` from the field: `Holder { a: Idx { n: 1 } }` with `a: Idx<NodeTag>` → *"cannot tell what T is"* | friction, possibly a bug: the field's type names T | infer `T` from the destination field, as a typed local already does |
| **X7** | a by-value recursive variant is refused (*"Enum 'E' contains itself … use reference<E>"*), so `ir.loft` holds every child as a box-of-one `vector<Node>` and every read is `n.cond[0]` | refused by design (bar A5) — costed | a recursive field written by value (`cond: Node`) and LAID OUT as today's box-of-one vector: the stored IR stays byte-identical, and the reads drop `[0]`. Owner decision: it revisits A5's refusal |
| **X8** | no conversion between `float` and its bits (none in `default/*.loft`) | friction (to confirm in S1) | `to_bits`/`from_bits` for `float` and `single`; needed by an arena that stores floats in integer words |
| **X9** | no `in` operator and no set of scalars (@PLN179 015); `{4 in ks}` fails as *"Formatter error"* | friction; the message is a wrong-error finding of its own | `contains` serves; the message should say there is no `in` |
| **X10** | capturing closures cannot sit in a collection (@C116); no local recursive `fn` (@C63) | by design — costed | not expected to bite: the dispatch table holds plain fn-refs |
| **X11** | a read-only cache directory crashed every cold run, and every warm load from it, with an internal compiler error (`Store::open` panics) | **bug — fixed in S0.2** | `Store::try_open`; a read surface that cannot be opened is a miss (§ Steps, S0.2) |
| **X12** | inside a package, a file named like one of its modules (`tests/load.loft`) gets ITSELF for `use load;`, so every name reads "Unknown function" (on `main`) | **bug — fixed in S0.3b** | `own_module_path` never answers the current file, and asks the package's `src/` (§ Steps, S0.3b) |

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
