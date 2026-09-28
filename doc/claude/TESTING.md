<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Testing loft

Where a test of the loft interpreter goes, and how to write it: the Rust harness, the `.loft`
suites, the generated and matrix tests, and the gates every test passes through.

| Question | Doc |
|---|---|
| How do I run the suite, and read what failed? | [RUNNING_TESTS.md](RUNNING_TESTS.md) |
| Does my guard actually catch the defect it names? (`make falsify`) | [GUARDS.md](GUARDS.md) |
| What stops a run that loops, grows or reads out of bounds? | [RUN_BOUNDS.md](RUN_BOUNDS.md) |
| How do I run the tests that need a database server, valgrind, a display or a browser? | [TEST_ENVIRONMENTS.md](TEST_ENVIRONMENTS.md) |
| How does a loft user test their own program (`loft test`)? | [LOFT_TEST.md](LOFT_TEST.md) |
| Which diagnostics may a test's `--deny-warnings` fail on? | [DIAGNOSTICS.md § Diagnostic tiers](DIAGNOSTICS.md#diagnostic-tiers--what---deny-warnings-may-fail-on) |
| What was measured and decided before (the record) | [TESTING-history.md](TESTING-history.md) |

## Overview

The suite has three homes for a test, in the order to reach for them:

1. **`tests/scripts/*.loft`** — the language corpus: a self-contained loft program per file,
   run on the interpreter by `tests/wrap.rs` and on `--native` by `tests/native.rs`.  The
   default home for a new test ([§ `tests/scripts/`](#testsscripts--standalone-loft-test-suite)).
2. **`tests/docs/*.loft`** — the user documentation: each file is a page and a runnable
   program.
3. **`tests/*.rs`** — Rust integration tests, for what a script cannot express: an exact
   diagnostic set, a Rust API, a compiler internal, a process-level fixture.

A Rust `code!` test also writes `tests/generated/<file>_<name>.rs`, the native Rust the code
generator emits for that snippet.  It is an inspection artefact that codegen tests read, not
a test that anything compiles ([§ Generated Test Files](#generated-test-files-testsgenerated)).

---

## Entry Points

### `tests/*.rs` — interpreter test files

Each file directly in `tests/` is its own Cargo integration-test binary (more than 350 of
them); `./scripts/find_problems.sh --list-subjects` groups them by subject.  A new Rust test
joins an existing binary of its subject ([§ Design intent and growth policy](#design-intent-and-growth-policy)).
The files every other test builds on:

| File | Contents |
|---|---|
| `testing.rs` | The `code!` / `expr!` framework; a module, not a runnable target |
| `wrap.rs` | Runs `tests/docs/`, `tests/scripts/`, `tests/comparisons/` and the library suites on the interpreter |
| `native.rs` | The same corpora under `--native` |
| `common/cross_mode.rs` | The `cross_mode!` harness: one cell, both backends, identical stdout |
| `parse_errors.rs` | Diagnostic fixtures that assert an exact error set |
| `issues.rs` | Minimal reproducers for issues (see [PROBLEMS.md](PROBLEMS.md)) |

A file that uses the framework includes `mod testing;`.

### Leak gate (`run_test` in `wrap.rs`)

After running a `.loft` file's functions, `run_test` calls
`state.collect_store_leaks()` and **hard-fails** if any heap store is unfreed at
program exit — making the `tests/scripts/` + `tests/docs/` corpus a leak
regression net (a new scope-free leak in any covered file breaks CI).  The allow
list, `SCRIPTS_LEAK_ALLOW`, is empty: a file that legitimately leaks at program end goes there
with a one-line rationale, and any other leak is a missing free to fix.  The complementary **native** leak gate runs generated
binaries with `LOFT_NATIVE_LEAK_CHECK=1` (`tests/leak_cases.rs`,
`tests/common/cross_mode.rs`); the dedicated shape corpus lives in
`tests/leak.rs`.

⚠ **`loft --tests <file>` does NOT run this gate** — only `wrap.rs` does.  Running a
leak regression test the quick way therefore reports `ok` while the file leaks, which
reads exactly like a fix that works.  Check a new leak guard with
`cargo test --test wrap`, or run the file the plain way (`loft --interpret <file>` with
a `main`, which does print the by-type warning).

---

## The Testing Framework (`tests/testing.rs`)

### Macros

```rust
code!("loft source code")   // parse and run a block of loft code
expr!("loft expression")    // shorthand: wraps the expression in a test() fn
```

Both macros call into `testing_code` / `testing_expr`, which construct a `Test` struct and capture the Rust function name via `stdext::function_name!()`. The function name is parsed to extract:

- **`self.name`** — the short function name (e.g. `define_enum`)
- **`self.file`** — the containing module name (e.g. `enums`)

These two strings name the generated file and the dump.

⚠ **A `code!` snippet is parsed as STDLIB SOURCE, and some behaviour is gated on that.**  The
macro parses against the cached stdlib `Data`, so the snippet's source IS `STD_SOURCE`, and any
feature whose gate reads `source != STD_SOURCE` is OFF inside it (`e2_rewrite_enabled` is one:
a nullable element such as `vector<Color?>` is refused in `code!` and compiles as a user file).
Which half a question falls in is not visible from the snippet, so a cell whose subject is a
nullable ELEMENT belongs in `tests/scripts/`, and **a `code!` cell about nullability is run
against the pre-fix build before it is trusted**.

### The `Test` struct

```rust
pub struct Test {
    name: String,         // short test name
    file: String,         // module / file name
    expr: String,         // loft expression to evaluate
    code: String,         // loft code block (may be empty)
    warnings: Vec<String>,
    advice: Vec<String>,
    errors: Vec<String>,
    fatal: Vec<String>,
    sizes: HashMap<String, u32>,
    result: Value,        // expected interpreter result
    tp: Type,             // expected type (when needed)
    expected_slots: Option<String>,  // the stack-slot layout `.slots()` pins
}
```

### Builder methods

Tests are configured with a fluent builder API before the `Test` is dropped:

| Method | Purpose |
|---|---|
| `.result(Value::...)` | Assert the `test()` function returns this value |
| `.tp(Type::...)` | Override the inferred result type (needed for booleans, enums) |
| `.expr("...")` | Set the loft expression (shorthand for a `test()` routine) |
| `.error("...")` | Expect a specific parse/type error (repeatable) |
| `.fatal("...")` | Expect a fatal parse error |
| `.warning("...")` | Expect a specific warning (repeatable) |
| `.advice("...")` | Expect a specific advice (repeatable); asserts the tier, not only the text |
| `.slots("...")` | Pin the stack-slot layout of `n_test`; `.slots("")` panics with the computed layout to paste back |

The diagnostic set must match EXACTLY: a warning the snippet emits and the test does not
name fails it.

### Execution model — `Drop`

**All test logic runs inside `impl Drop for Test`.** There is no explicit `.run()` call; the test executes automatically when the `Test` value goes out of scope at the end of the `#[test]` function.

The `drop` implementation:

1. Constructs a `Parser` from the cached default library (`default/`).
2. Appends a synthesised `test()` function (see below) when `.expr()` or `.result()` was set.
3. Parses the combined loft source via `p.parse_str(...)`.
4. Validates struct sizes against any `.sizes` entries.
5. Calls `assert_diagnostics` — panics if the actual warnings/advice/errors do not exactly
   match the expected set — and stops if parsing failed.
6. Runs `scopes::check` (scope/type analysis).
7. Calls `generate_code` (writes `tests/generated/`), in every build profile.
8. Runs `byte_code`, checks `.slots()` when set, and executes `test`.
9. **Debug builds only:** logs bytecode and execution trace to `tests/dumps/<file>_<name>.txt`.

With `--features emit-repro`, the assembled source is also written to
`/tmp/loft-repro/<test>.loft` before parsing, so a failing test replays standalone.

### Synthesised `test()` function

When `.expr("...")` and `.result(...)` are both set, the framework generates a loft snippet:

```loft
pub fn test() {
    test_value = { <expr> };
    assert(
        test_value == <result>,
        "Test failed {test_value} != <result>"
    );
}
```

When `.result()` is `Value::Null` with a non-unknown type (i.e. testing that the expression returns null), it generates:

```loft
pub fn test() {
    <expr>;
}
```

---

## Validation matrices (`tests/{tuple,template,…}_matrix.rs`)

A **validation matrix** is a test binary that systematically covers
a 2-axis grid of language-feature interactions, with every cell
running under both backends (interp + `--native`) via the
`cross_mode!` harness in `tests/common/cross_mode.rs`.

The family is `ls tests/*_matrix.rs`; two of them, as examples of the shape:

| Binary | Plan | Axes | Cells |
|---|---|---|---|
| `tests/tuple_matrix.rs` | @PLAN14 | element type × destructure shape | tuple bug surface |
| `tests/template_matrix.rs` | @PLAN17 | T-parameter usage × bound shape | bounded-generic / interface surface |

### Pattern

- **Every cell runs by default** (@PLN114).  A whole matrix takes seconds; an `#[ignore]`d
  cell is a cell nobody runs, and the defects the matrices exist to find sat behind that
  attribute until the set was run by hand.
- **Cell name encodes the matrix coordinate** so the test name
  identifies the cell at-a-glance (e.g. `u3_b1a_addable_inline_pair_with_sum`
  = T-usage U3 × bound B1 Addable × specific shape).  Naming
  conventions are per-plan (@PLAN14 `e<E>_d<D>`, @PLAN17
  `u<U>_b<B>_…`) to avoid cross-binary collision.
- **PASS / FIX / CLOSED** — every cell is one of three states.
  PASS = cell is covered by a passing test; FIX = cell needs
  implementation work, tracked as a `#[ignore]`d test that's
  expected to start passing once the fix lands; CLOSED = design
  decision (no cell test; reason recorded in `DESIGN_DECISIONS.md`).
- **Bug yield is the headline metric.**  Each filed issue blocks plan acceptance; each PASS
  cell becomes a regression net.

Run a whole matrix:
```bash
cargo test --release --test template_matrix
```

A single cell:
```bash
cargo test --release --test template_matrix u3_b1a_addable_inline_pair_with_sum
```

The `cross_mode!` macro (heavy-by-default) is documented in detail
in [`.claude/skills/loft-test/SKILL.md`](../../.claude/skills/loft-test/SKILL.md)
§ "The `cross_mode!` macro" — read that before authoring matrix
cells.

The historical per-plan matrix definitions and bug-discovery records are in the closed plans
[`plans/finished/14-tuple-validation/`](plans/finished/14-tuple-validation) and
[`plans/finished/17-template-validation/`](plans/finished/17-template-validation).

---

## Generated Test Files (`tests/generated/`)

`Test::generate_code`, called from `Drop::drop`, writes these in every build profile, so a
codegen-inspection test can read them under `cargo test --release`.  They are git-ignored,
cleared by `make test`, and compiled by nothing: a `tests/` subdirectory is not a Cargo
test target.

### `tests/generated/default.rs`

Rewritten on every test execution: the native Rust of the default library alone, everything
up to `start` (the definition count before the test's own code was parsed).  A reference
snapshot of the default-library schema, with no `#[test]`.

### `tests/generated/<file>_<name>.rs`

Written only when a test has a non-null `.result` or a non-unknown `.tp`, named after the Rust
module and the test function: `define_enum` in `tests/enums.rs` writes
`tests/generated/enums_define_enum.rs`.  It holds the crate-level `#![allow]`s, the `use loft::…`
imports, an `init` that rebuilds the whole type schema (default library plus the test's own
types), the functions reachable from `n_test` (`output_native_reachable`), and a
`#[test] fn code_<name>()` that calls `init` and `n_test`.

---

## The drop gate (`tests/ownership_drop_gate.rs`)

**What it checks.** `formal/heap.md (H-Drop)`: a droppable resource is released exactly once,
at the death of the record that owns it; and `(H-Drop-Not)`: the places that rule names release
nothing.  The free-side instruments — `LOFT_POISON`, the leak check, `LOFT_OWN_ORACLE=check` —
cannot see a drop: each is clean on a program that runs a hook twice or never.  So this gate
scores the release itself.

**How a cell reports.**  Each cell is a small generated program.  Its resource type prints
`M<id>` when a resource is minted, `D<id>` when the hook runs and `R<id>` when the program
reads it; `X<id>` marks a resource `(H-Drop-Not)` leaves to the author.  A copy keeps the id,
so one id names one resource across all of its copies.  `score` turns the trace into findings:

| kind | meaning |
|---|---|
| `DOUBLE` | released more than once |
| `LOST` | never released |
| `EARLY` | released, then read again |
| `LATE` | released after its cell returned |
| `UNMINTED` | released an id that was never minted — the hook read freed memory |
| `RELEASED_NOT` | released a resource marked `X` |
| `REFUSED` / `CRASHED` | did not compile, or stopped inside the cell — never scored as clean |
| `EMPTY` / `REMINT` | a fault in the harness: the cell minted nothing, or two resources share an id |

The generator never states an expectation — the trace does — so a new cell needs no expected
value.  A new axis is a row in `SOURCES`, `DESTS` or a `COAL_*` table; a hand-written shape goes
in `pilot_cells`.

**Every cell runs in its own process.**  A doubled release corrupts what later code in the same
process sees: a cell that is clean on its own loses its release when it runs after a doubling
one.  So a batch result is not a measurement of any one cell.  On Linux an interpreter cell runs
under a 2 GiB address-space limit, because `LOFT_MEMORY_LIMIT` ([RUN_BOUNDS.md § Store-memory ceiling](RUN_BOUNDS.md#store-memory-ceiling-loft_memory_limit)) is armed only under
`loft test`, and a corrupt length can allocate without bound.

**The baselines.**  `tests/ownership_drop_gate.baseline` (interpreter) and
`tests/ownership_drop_gate.native.baseline` list every cell that is not clean, as
`cell KIND,KIND`.  A line is an open defect registered in `formal/heap.md` (`D-heap-1`,
`D-heap-7`) — not accepted behaviour.  The test fails on a NEW line (a cell that now releases
wrongly, or differently) and on a GONE line (a fix: retire the line in the same commit).  A
missing baseline fails; it is never written on first sight.  Each run also prints every cell
where the two backends disagree.

```bash
cargo test --release --test ownership_drop_gate                            # both legs, ~40 s warm
LOFT_BLESS_DROP_GATE=1 cargo test --release --test ownership_drop_gate     # only after reading the diff
```

The native leg is four tests (`every_cell_releases_each_resource_once_on_native_0..3`, every 4th
cell from each offset) so no test outgrows the per-test duration gate.  Each checks, and under
`LOFT_BLESS_DROP_GATE=1` rewrites, only its own cells' lines of the one native baseline — a merge
under a lock file, so the bless command is unchanged and parallel chunks cannot clobber each other.

⚠ **A second instrument records the same fact.**  `tests/double_move.rs` also runs its cells with
the refusal off and hard-codes each one's release and `double-move` warning counts, so a change
that moves a release decider moves BOTH.  Re-blessing the drop gate while leaving it unmeasured
ships its reds to whoever joins next.  Run `--test double_move` beside the gate, and re-measure its counts from the
program's own `DROP:` lines, not from the assertion that failed first.

**The lease verdicts.**  `formal/heap.md`'s copy-lease rules give every cell a verdict
(`lease_verdict`): `Once` (every copy is a move, a view or a fresh value) or `Refused` (a copy
whose value is still used), or `Open` naming the plan's
question that decides it.  `every_cell_disagreeing_with_the_lease_rules_names_its_open_deviation`
reads both baselines and requires each `Once` cell that releases wrongly to be listed under
exactly one OPEN deviation (`LEASE_DEVIATIONS`), and each listed cell to still fail — so a fix
retires its cell from the list in the same commit, and a deviation closed in the register while a
cell still measures it fails.  `the_census_names_the_copy_each_cell_makes` runs
`LOFT_DROP_COPY_CENSUS` over every cell: a copied structure is reported from its root, a view
reports nothing, a program with no hook reports `0 sites`, and each of the census's two verdict
columns is held to its own hand-derived oracle.  `lease=` — the rule read off the line — refuses in
exactly the cells `lease_verdict` refuses (201 of 276) and in no `Once` cell; `liveness=` — whether
the copied value is used afterwards, kept for `(H-Elide)` — refuses in exactly the cells
`liveness_verdict` refuses and never reports `unreached`.  An `item` site (a droppable placed in a
tuple literal) is a placement the rule judges, not an emitted copy, so it is not held to a root.
`every_emitted_copy_of_a_droppable_has_a_lease_verdict` compiles every cell with the census and
`LOFT_COPY_MANIFEST` on, through the interpreter and the native generator, and requires every copy
of a droppable a generator EMITS to carry a census verdict — the copies the IR never shows are
where a refusal built on the IR alone would leak.

**It can fail.**  `the_scorer_names_each_kind_of_wrong_release` feeds a hand-written trace for
each kind, and `every_cell_is_distinct_and_mints` rejects a cell that could only ever be clean.
Against the compiler, a change to a release hand-off moves exactly the cells that ride it,
on each backend — a move in a cell nobody expected is the finding.

## `tests/wrap.rs` — shared runner for docs and scripts tests

`run_test(path, debug)` is the core of every test in `tests/wrap.rs`:

1. Creates a `Parser`, loads the default library, parses the given `.loft` file.
2. Checks diagnostics against `// #warn`, `@EXPECT_ERROR`, and `@EXPECT_WARNING`
   annotations.  An unexpected error or an unclaimed warning fails the test.
3. If the file has `@EXPECT_ERROR` annotations, the Rust suite attributes each error to its
   enclosing function, blanks that cell and re-parses (loft#1242), so a refusing cell and a
   running cell share a file and both are checked.

   ⚠ **The CLI does not do that peel.** `loft --tests <file>` on a mixed file runs the
   refusal cells and SKIPS every running one, silently — a deliberately broken assertion
   in such a file still reports `ok`.  So verify a mixed guard through
   `cargo nextest --test wrap loft_suite`, or **keep the two kinds in separate files**,
   which is what the two `the-reference-par-…` guards do and why they are a pair.
4. Runs `scopes::check` and `byte_code` inside `catch_unwind`.  If the compiler
   panics and the file has `@EXPECT_FAIL` annotations, the panic is tolerated.
5. Discovers every zero-parameter user function and calls each inside `catch_unwind`
   (`main` included), so a failing assert in one does not stop the rest.  Functions
   annotated `@EXPECT_FAIL` tolerate panics.

   **The CLI runner's rule is not this one** (loft#1010).  `loft --tests` / `loft test`
   run only the `test_*` functions of a file that names any; only a file that names none
   has every zero-parameter function run and counted, `main` included
   ([LOFT_TEST.md § Writing tests](LOFT_TEST.md#writing-tests)).  The two rules are not
   interchangeable, which is why loft#1010 is a decision rather than a patch.
6. In debug builds, writes a bytecode dump to `tests/dumps/<filename>.txt` first.
   If `debug = true`, also writes an execution trace using `execute_log`.

### Annotations supported by `wrap.rs`

| Annotation | Scope | Effect |
|---|---|---|
| `// #warn <text>` | File | Warning must appear; missing → fail |
| `// @EXPECT_ERROR: <text>` | Per-function or file header | Parse error containing `<text>` must appear; missing → fail |
| `// @EXPECT_WARNING: <text>` | Per-function or file header | Warning containing `<text>` must appear; missing → fail |
| `// @EXPECT_FAIL: <text>` | Per-function (before `fn`) or file header | Runtime panic is tolerated |

**Every expectation must match.**  An `@EXPECT_ERROR` or `@EXPECT_WARNING` whose diagnostic
does not appear fails the file (loft#929), and the check runs even when the file produced NO
diagnostics at all.  **One diagnostic answers one expectation**, in both harnesses: two cells
expecting the same text need two reports, and under `loft test` a cell's expectation first
takes a report located in its own function, so a failure names the cell that went quiet
(loft#1727; `tests/expectation_credit.rs`).

**An expectation claims only what it names.**  Under `loft test` / `--tests` — the path a
library's CI takes, with `LOFT_DENY_WARNINGS=1` — an `@EXPECT_WARNING` quiets the warnings
containing its text and nothing else: every OTHER warning in the file is printed and fails
`--deny-warnings` exactly as in a file with no expectation (C123;
`tests/post_scope_lints_under_tests.rs`,
`an_expected_warning_does_not_exempt_the_other_warnings_in_its_file`).

The rule is per ANNOTATION, not per file, and that distinction is the whole of it.  An
annotation written above a `fn` binds to that function; only one written ahead of every
`fn`/`struct`/`enum` is file-level.  Both kinds are scored by the same predicate — the
declared substring must appear in some error the file produced.  Asking instead whether the
file produced *any* error would credit every annotation in a file that errs anywhere
(loft#1261).

Two directions have to hold, and only together do they mean anything:

* **every ERROR is claimed** by some annotation — this is what catches a diagnostic that
  was reworded, since the new text matches nothing and is reported as unexpected;
* **every ANNOTATION is matched** by some error — this is what catches a refusal that
  stopped being emitted at all.  Nothing else can: the annotation goes unmatched while
  every error present is still claimed, so the file looks exactly like one that passed.

The second is the one worth the machinery.  A guarantee lapses, the annotation asserting it
survives, and a suite checking only the first direction goes on reporting that the
guarantee holds.  `tests/expectation_credit.rs` pins both, and its control row pins that a
file whose expectations are genuine is still green — without which a harness that refused
every annotated file would satisfy the rest.

### An error fixture asserts ONE pass, never both

`Parser::parse` runs pass 2 only when pass 1 finished without an error:

```rust
let lvl = self.lexer.diagnostics().level();
if lvl != Level::Error && lvl != Level::Fatal { /* pass 2 */ }
```

A large share of loft's diagnostics are emitted by `!first_pass` code — `Unknown variable`,
the const/`&` checks, match exhaustiveness, the @PLN25 N-Store family, the type-mismatch
messages.  **One pass-1 error therefore silences every pass-2 diagnostic in the same
file**, and an `@EXPECT_ERROR` for one of those can never match, however correct its
wording.

So a fixture holds pass-1 errors OR pass-2 errors.  The split is visible in the naming:

| Pass 2 (needs a clean pass 1) | Pass 1 (aborts before pass 2) |
|---|---|
| `102-expected-errors.loft` | `102b-pass1-expected-errors.loft` |
| `36-parse-errors.loft` | `36b-pass1-parse-errors.loft` |
| `35b-format-errors-unknown-var.loft` | `35-format-errors.loft` |

Pass 1 emits the lexer's own errors (`Misplaced '_' in number literal`), the definition
and type checks (name conflicts, camel-case, `Undefined type`), and everything
`typedef::fill_all` reports (type cycles, the reserved-`key` hash guard).  When a new
`@EXPECT_ERROR` does not fire, check which half it landed in before rewording it.

### Whole-program lints run here too

`warn_dead_stores`, `warn_double_move` and `warn_lost_temp_writes` run in `run_test` in the
same window `src/main.rs` uses — after `Parser::parse`, before `scopes::check`.  So this
suite confirms their warnings and catches their false positives, as the CLI does (loft#929).

**Annotation placement rules** (same as `test_runner.rs`):
- An annotation directly before a `fn` line (no blank lines between) binds to that function.
- An annotation in the file header (before any `fn`/`struct`/`enum`) is file-level.
- A blank line between the annotation and the `fn` clears the pending annotation.

`LOFT_LOG` is respected: `LogConfig::from_env()` is called in `run_test` exactly as in `testing.rs`.

Named test entrypoints in `tests/wrap.rs`:

| Test name | What it runs | Notes |
|---|---|---|
| `dir` | All `tests/docs/*.loft` files + HTML doc regeneration | Skips files listed in `SUITE_SKIP` |
| `comparisons` | All `tests/comparisons/*.loft` files | The comparison pages' claims |
| `loft_suite_00` … `loft_suite_07` | `tests/scripts/*.loft`, split in eight chunks | Skips files in `ignored_scripts()`; `loft_suite_chunks_cover_the_corpus` pins the count |
| `loft_suite_whole_corpus` | The whole corpus in ONE process, in order | `#[ignore]`d — the nightly release-gate job; `LOFT_SCRIPT_FIRST` / `LOFT_SCRIPT_LAST` bisect over it |
| `library_suite` | The library packages' own tests | Skips per `LIB_PKGS_SKIP` / `LIB_TESTS_SKIP` |
| one per topic file | `integers` → `01-integers.loft`, `files` → `19-files.loft`, … | The `script_test!` list in `tests/wrap.rs` |
| `last` | `tests/docs/16-parser.loft` | — |
| `threading` / `logging` | `tests/docs/19-threading.loft` / `20-logging.loft` | — |
| `file_debug` | `tests/docs/13-file.loft` with execution trace | — |
| `parser_debug` | `tests/docs/16-parser.loft` with execution trace | `#[ignore]` — run with `cargo test -- parser_debug --ignored` |

Run one topic file with `cargo test --test wrap <name>`, e.g. `cargo test --test wrap files`.

### WRAP_LOCK — serialisation guard

All `#[test]` functions in `wrap.rs` acquire a process-wide `static Mutex<()>` (`WRAP_LOCK`)
before calling `run_test`. This prevents two tests from executing the same script concurrently
when Cargo runs the test binary with multiple threads (the default). Without this guard,
for example, `loft_suite` and `files` would both execute `19-files.loft` at the same time,
causing filesystem races.

The lock is poisoning-tolerant (`unwrap_or_else(|e| e.into_inner())`): a panicking test
releases the lock and the next test can proceed.

### Every skip says why, how it runs instead, and when it ends

An ignore or a skip is a ROUTING, never a resting place: the test still runs somewhere, or a
named condition removes the entry. Each class has one home and a guard.

- **`#[ignore = "…"]` and `#[cfg_attr(<cfg>, ignore = "…")]`** — every one in `tests/*.rs`
  and `src/**/*.rs` is listed in `tests/ignored_tests.baseline`
  (`doc_hygiene::ignored_tests_baseline_is_current` fails on any drift; regenerate with
  `python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline`), and every reason
  names the run it rides — `doc_hygiene::every_ignore_reason_says_how_it_runs` accepts
  `--ignored` (by hand, with the command), a nightly job (`miri.yml`'s `release-gate-sweeps`,
  `ci.yml`'s `test` job step "Differential oracle"), `on demand` / `manually`, or a platform
  (`Windows`: the `cfg_attr` ignores whose guard is the resource cap the platform lacks).
  `make release-checklist`'s `A-ignores` reads the same file. A reason that only says WHY
  (`heavy`, `a measurement`) fails the guard until it also says where the test runs.
- **Suite skip lists** — `wrap.rs::{SUITE_SKIP, WASM_SKIP, LIB_PKGS_SKIP, LIB_TESTS_SKIP}`,
  `native.rs::{NATIVE_SKIP, SCRIPTS_NATIVE_SKIP, LIB_PKGS_NATIVE_SKIP, LIB_TESTS_NATIVE_SKIP}`,
  `html_wasm.rs::{LIB_PKGS_WASM_SKIP, LIB_TESTS_WASM_SKIP, LIB_PKGS_NODE_SKIP,
  LIB_PKGS_WASMTIME_SKIP}`. An entry carries the open issue that explains it and the
  condition that removes it. Today every list is empty except the `html_wasm` platform
  limits: `server` (a listener; a browser/WASI guest has no accept — by
  construction), `hex_world` on node (no filesystem; ends with a JS-host VirtFS bridge),
  `imaging` on wasmtime (no canvas codec; ends with a pure-wasm PNG decoder), `input` on
  wasmtime (the graphics crate is absent from the wasip2 sysroot; ends when it is packaged).
  **Check an entry's blocker STATE before trusting it**: an entry outlives its blocker
  silently, and an inert entry reads exactly like an open gap.
- **`tests-network/`** — the `web` fixture keeps its live-network tests
  (`tests/fixtures/libs/web/tests-network/http.loft` and the `ws_*.loft` echo regressions) in
  a directory no suite walks, because they need a reachable host. Run them by hand against
  the echo server in `tools/zt-c-web-staging/README.md` § Verification, from a copy OUTSIDE
  the lib tree (inside it, `--lib` auto-discovery double-resolves). The class ends when CI
  gains a network leg.

### LOFT_DUMP — controlling debug output in docs/scripts tests

In debug builds, `run_test` (called by `dir`, `loft_suite`, `threading`, etc.) normally
writes a bytecode dump to `tests/dumps/<filename>.txt`. Set `LOFT_DUMP=1` in the environment
to enable this write for non-debug (`debug=false`) test runs:

```bash
LOFT_DUMP=1 cargo test --test wrap dir   # writes bytecode dumps for every docs file
```

Without `LOFT_DUMP=1`, the dump is suppressed for the normal `dir`/`loft_suite` tests
(only written when `debug=true`, i.e. for `file_debug` and `parser_debug`). This avoids
writing ~20 large files during a routine `cargo test` run.


---

## `tests/docs/` — end-to-end loft files

**Purpose: user documentation.** Each file produces one HTML page via `@NAME`/`@TITLE` headers and `//`-comment prose. They are also valid runnable loft programs, so `dir` both regenerates HTML docs and validates the language features shown in each page.

Not connected to the `Test` builder API. The `last` test runs only the final file for fast iteration.

Docs files, `00`–`38`.  The table below covers `00`–`22`; the rest are
listed by `ls tests/docs`, which is the only count that cannot go stale.  A library's
getting-started page is NOT here: it lives in the library, under its own `docs/`, and is
run by that package's CI (@PLN149 step 9).

| File | Topic |
|---|---|
| `00-general.loft` | General language features |
| `01-keywords.loft` | Keyword coverage |
| `02-text.loft` | Text operations |
| `03-integer.loft` | Integer arithmetic |
| `04-boolean.loft` | Boolean logic |
| `05-float.loft` | Floating-point |
| `06-function.loft` | Functions, defaults, recursion |
| `07-vector.loft` | Vectors |
| `08-struct.loft` | Structs |
| `09-enum.loft` | Enums |
| `10-sorted.loft` | Sorted collections |
| `11-index.loft` | B-tree index |
| `12-hash.loft` | Hash collections |
| `13-file.loft` | File I/O |
| `15-lexer.loft` | Lexer/parser library use |
| `16-parser.loft` | Parser library use |
| `17-libraries.loft` | Library imports and extension methods |
| `18-locks.loft` | Store locking and `const` parameters |
| `19-threading.loft` | Parallel execution (`par(b=worker, threads)` for-loop clause) |
| `20-logging.loft` | Runtime logging (`log_info`, `log_warn`, `log_error`, `log_fatal`) |
| `22-time.loft` | Time functions (`now`, `ticks`) |

---

## File Layout Summary

```
tests/
  *.rs                    # one Cargo test binary each (more than 350)
  testing.rs              # the code!/expr! framework (a module, not a target)
  wrap.rs  native.rs      # the corpus runners: interpreter / --native
  *_matrix.rs             # validation matrices (cross_mode!, both backends)
  common/cross_mode.rs    # the both-backend harness
  scripts/                # the language corpus, one loft program per file; wordlist.txt
  docs/                   # the user documentation, runnable (00-…loft through 38-…loft)
  comparisons/            # the comparison pages' claims as programs
  fixtures/  lib/         # fixture packages and libraries the suites load
  generated/              # codegen output per code! test (git-ignored, compiled by nothing)
  dumps/                  # bytecode + trace dumps (debug builds, git-ignored)
```

---

## Key Constraints

- **Test order within a binary is not deterministic** — Cargo runs tests in parallel.
  `make test` passes `--test-threads=1` to run them in order and capture their output into
  `result.txt`.
- **`tests/generated/` is output, not tests.**  Nothing compiles it; the `generated/`
  review workspace the Makefile's `generate` and `gtest` targets name does not exist in the
  tree.

---

## `tests/scripts/` — standalone loft test suite

**Purpose: the primary, long-term comprehensive test suite for the loft language.**
Every language feature and standard-library function should eventually have coverage here.
Each file is a self-contained loft program: its `fn main()` or its `test_*` functions assert
correct behaviour.  No HTML generation, no `@NAME`/`@TITLE` headers.  It runs through the
`loft` binary or through `tests/wrap.rs` (the `loft_suite_*` chunks), and under `--native`
through `tests/native.rs`.

### Design intent and growth policy

`tests/scripts/` is the canonical place for new tests. When adding a feature, fixing a bug, or
covering an untested language behaviour, the default choice is to extend an existing script or
add a new one — not to add a Rust `.rs` test.

**Add to `tests/scripts/` when:**
- Testing language semantics: operators, control flow, type coercion, collections, formatting, etc.
- Testing standard-library functions.
- Covering an edge case in correct (non-error) code.
- Writing a regression test for a runtime bug fix.

**Add to `tests/*.rs` only when the scenario cannot be expressed as a loft script:**
- The test asserts the EXACT diagnostic set, or a diagnostic's tier (`code!` with `.error` /
  `.warning` / `.advice`).  A script pins that one diagnostic appears with `@EXPECT_ERROR` /
  `@EXPECT_WARNING`.
- The test calls Rust APIs directly (`threading.rs` low-level `run_parallel_int`/`run_parallel_raw`
  tests, `data_structures.rs`, `log_config.rs`).
- The test exercises compiler internals that only surface via the Rust test framework
  (`.slots()` layouts, `.sizes`).

**Prefer `tests/scripts/` over `code!()` in `.rs` files.**  If a test can be written as
plain loft code with `assert()`, put it in the appropriate script file — do not wrap it in
`code!(r#"..."#)` inside a `.rs` file.  The `code!()` macro exists for cases that need Rust
assertions on compiler output, not as a convenience wrapper for loft code.  Script tests are
also validated by the native test runner (`cargo test --test native`), giving automatic
dual-mode coverage.

**When a `.rs` test and a script test cover the same behaviour**, the `.rs` test should be removed
— the script is the authoritative version.

**What grows the gate is binaries and compile-spawning tests, not tests** (@PLN159).  A corpus
file costs ~5 ms to run and nothing to build; a new `tests/*.rs` binary is a compile plus a link in
every build (dev/test, release, clippy `--all-targets`); and a test that spawns `rustc`, a cargo
build or a wasm build is 10–60 s of CPU that does not parallelise and starves every test beside
it.  So a new Rust test joins an existing binary of its subject unless it needs its own
process-level fixture, and a test that needs a compiled artifact shares ONE fixture per binary
(build once behind a `OnceLock`, or through the content-keyed cache the corpus runner uses —
`native_cache_key`) rather than compiling per test.

**Naming a bug regression: use the GitHub issue number.**  A regression for a fixed bug is
`tests/scripts/<issue>-<slug>.loft` — e.g. `366-native-abib-scalar-literal-arg.loft` guards #366,
`368-nullable-struct-return-heap-param.loft` guards #368.  This makes the test greppable from the
issue and back: the issue's `fixed-pending-merge` comment names the file, the file's header cites
`#<issue>`.  Don't reuse a feature/era number for a bug (a fix for #366 filed under `304-…` is a
mis-file — rename it).  The original topic suites (`01-integers`, `05-enums`, …) keep their
sequential feature numbers; those predate the issue-number convention and are not renamed.

The topic files keep their sequential feature numbers (`01-integers.loft`, `05-enums.loft`,
…); everything since is named by issue number.  `ls tests/scripts` is the list (about 1800
files); `wordlist.txt` holds the edge-case keys `37-stress.loft` reads.

Run with:

```bash
cargo test --release --test wrap loft_suite   # every chunk, interpreter
./target/release/loft --tests tests/scripts/06-structs.loft   # one file
```

The `cargo test` path uses `run_test` from `tests/wrap.rs` (§ [`tests/wrap.rs`](#testswraprs--shared-runner-for-docs-and-scripts-tests)):
an unexpected error or an unclaimed warning fails the file, and a failing `assert` panics
with its message, naming the function.

### Known language quirks affecting test authoring

The following behaviours differ from what one might naively expect:

| Behaviour | Correct approach |
|---|---|
| `empty = []` is a compile error (`Variable 'empty' has unknown type`) | Give it a type: `empty: vector<integer> = [];` |
| `#index` in `for i in 10..14` is the loop variable (10–13), not a 0-based count | Use `#count` for 0-based counting |
| An omitted integer struct field is `0`, not null | Assert `== 0`; `== null` is a `redundant-null-check` warning |

---

## See also
- [PROBLEMS.md](PROBLEMS.md) — Known bugs, limitations, workarounds, and fix plans
- [CLAUDE.md](../../CLAUDE.md) — Project orientation: execution path, key data structures, branch policy, documentation index
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy (LOFT_LOG presets, scope bugs, slot conflicts), working with Claude

## Open work

| Item | Section | Status |
|---|---|---|
| **Fenced examples in API doc comments are not executed** — an example in a `pub` item's doc comment is not run or asserted, so a doc can disagree with its code.  The mechanism is designed (@PLN121: extract → run both backends → gate → ship only what ran), but the **domain is empty**: measured 2026-08-25, **6 fenced examples in 1962 `pub` items** across the stdlib and all 8 library repos, none in a published package.  Building an extractor, runner, gate and registry field for six examples would be five mechanisms serving one file. | — | 🔕 Deliberately not built.  **Trigger to revisit: re-run the count** — if a package starts writing fenced examples, @PLN121's steps 3–7 are still the plan.  The assert-less half shipped (`tests/doc_hygiene.rs::every_doc_page_asserts_something`). |

---

