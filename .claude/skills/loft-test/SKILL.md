---
name: loft-test
description: Reference for writing tests against the loft interpreter, --native backend, and WASM build. Apply whenever adding, editing, or reviewing tests/*.rs or tests/scripts/*.loft / tests/docs/*.loft. Covers test-binary layout, the `code!` and `cross_mode!` macros, the @EXPECT_ERROR / @EXPECT_FAIL / @ARGS / @NAME / @TITLE annotations on `.loft` files, ignore conventions, P-id rules, the `@falsified-at:` gate, and which check to run for a change.
user-invocable: false
---

# Loft Testing Reference

Always consult this before adding or modifying tests under `tests/`.
The loft project has 260+ integration test binaries (`ls tests/*.rs`
is the census) plus a custom testing framework; picking the wrong
binary or the wrong macro means either a slow CI cycle or a test that
doesn't actually validate the change.

For runtime debugging conventions (LOFT_LOG presets, dump files), see
the parent project's [TESTING.md](../../../doc/claude/TESTING.md).  This
skill covers the *authoring* side: where a test belongs, which macro
to use, and how to keep the suite fast.

---

## Test-binary layout

Each `tests/*.rs` file becomes one integration-test binary.  Pick the
binary that matches the kind of behaviour you're verifying.

| Binary | Purpose | Typical macro / harness |
|---|---|---|
| `tests/wrap.rs` | Loft script suites in `tests/scripts/*.loft` and `tests/docs/*.loft` driven through the **interpreter** (`dir` / `loft_suite`), plus the `wasm_dir` leg (`--native-wasm`, run under `wasmtime` when available). | Reads scripts from disk; no Rust-level macro. |
| `tests/native.rs` | Same scripts driven through `--native` (`native_dir` / `native_features` / `native_scripts`).  Catches codegen vs interp divergence. | Disk-driven; `find_loft_rlib` + `compile_native_job`. |
| `tests/issues.rs` | The regression register (800+ tests) — small named pins for fixes.  Prefer a `tests/scripts/*.loft` file where possible: it locks all three backends. | `code!(...)` |
| `tests/expressions.rs` | Language feature tests grouped by topic. | `code!(...)` and `expr!(...)` |
| `tests/parse_errors.rs` | Negative tests — the loft source must produce a specific diagnostic. | `code!(...).error(...).warning(...)` |
| the `*_matrix.rs` binaries | Cross-mode matrices (`tuple_matrix`, `closure_matrix`, `coroutine_matrix`, `mut_closure_matrix`, `binary_io_matrix`, `template_matrix`, `par_nested`) — every cell runs interp + `--native` and asserts byte-identical stdout. | `cross_mode!(name, body)` |

That is the load-bearing subset.  The full census is `ls tests/*.rs` — most binaries
are single-topic and self-describing (`exit_codes.rs`, `error_messages.rs`,
`leak.rs`, `codegen_emitter.rs`, `slots.rs`, golden harnesses like
`crystal_editor_gold.rs` / `layout_golden.rs`, …); TESTING.md carries the framework
reference.  Do not extend a table here — it drifts (this one carried two deleted
binaries for months).

**When unsure where a test belongs**, look for an existing test that
shares the *kind* of failure you'd reproduce (parse error → parse_errors,
runtime mismatch → issues, native-vs-interp → tuple_matrix, codegen
template → codegen_emitter).  Match the shape, don't invent a new file.

**Common module:** `tests/common/mod.rs` is `mod common;` from each
binary.  It exposes `cached_default()` (cached default-stdlib parse —
avoids a per-test stdlib re-parse) and `cross_mode::run_cross_mode()` (the cross-mode
harness).  Every helper there is `#[allow(dead_code)]` because not
every binary uses every helper.

---

## The `code!` macro — primary unit-test API

Lives in `tests/testing.rs`.  Used by issues / expressions / parse_errors /
slots / doc_hygiene and others (`grep -l 'code!' tests/*.rs` is the census).

```rust
mod testing;
extern crate loft;
use loft::data::Value;

#[test]
fn my_test() {
    code!("fn test() { x = 3 + 4; }")        // loft source
        .expr("x")                            // expression to evaluate
        .result(Value::Int(7));               // expected value
}
```

Chain methods:

| Method | Effect |
|---|---|
| `.expr(s)` | After running the loft source, evaluate this expression and capture its result for `.result()` |
| `.result(v)` | Assert the most recent `.expr()` produced this `Value` (`Value::Int`, `Value::Text`, `Value::Boolean`, `Value::Null`, …) |
| `.tp(t)` | Assert the result type matches |
| `.advice(msg)` | Expect an `advice:` diagnostic (the non-gating tier) |
| `.slots(…)` / `.invariants_pass()` | Slot-layout / invariant assertions (see `tests/testing.rs`) |
| `.error(msg)` | Expect this exact diagnostic (substring or full).  Format: `"<text> at <test_name>:<line>:<col>"`.  Multiple `.error(...)` calls assert multiple diagnostics. |
| `.warning(msg)` | Same as `.error` but for warnings.  Warnings don't suppress execution. |
| `.fatal(msg)` | Expect a fatal panic with this message. |

The loft source must declare `fn test() { ... }` — the framework
runs `test` after parsing.  Helper fns at file scope are fine.  **No
nested fns.**

`expr!` is shorthand for "wrap a single expression in `fn test() { … }`":

```rust
expr!("3 + 4").result(Value::Int(7));
```

Use `code!` when you need a multi-statement test or fixture
declarations; `expr!` for one-liners.

---

## The `cross_mode!` macro — interp ↔ native equivalence

Lives in `tests/common/cross_mode.rs` (its header doc is the contract — the
run-by-default decision is @PLN114).  Used by the seven matrix binaries
(`tuple_matrix`, `closure_matrix`, `coroutine_matrix`, `mut_closure_matrix`,
`binary_io_matrix`, `template_matrix`, `par_nested`); the
`run_cross_mode_expect` / `_rejected` / `_leak_free` variants cover
expected-output, must-refuse and leak-free cells:

```rust
mod common;

cross_mode!(my_cell, r#"
    fn test() {
        t = (3, 7);
        print("{t.0},{t.1}\n");
        assert(t.0 == 3 && t.1 == 7, "my_cell");
    }
"#);
```

Mechanics:
1. Writes the body to `/tmp/loft_xmode_<test_name>.loft`.
2. Runs `loft --interpret` and `loft --native` as subprocesses
   (uses `env!("CARGO_BIN_EXE_loft")`).
3. Captures both stdouts, normalises CRLF→LF and trailing
   whitespace, asserts:
   - both modes succeed (exit 0),
   - **both stdouts are byte-identical**.

**Body contract:**
- The body must declare `fn test() { … }` — that is the entry point.
- Helper fns must live alongside `fn test`, not nested inside it.
- The harness appends `fn main() { test(); }`, so don't include your own.

**The cells RUN BY DEFAULT** (@PLN114): the macro applies no `#[ignore]`, and the
whole tuple matrix takes on the order of seconds on a warm target/ — the cost was
never the issue; the silence was.  Run it like any binary:

```bash
cargo test --release --test tuple_matrix              # all cells
cargo test --release --test tuple_matrix e1_d1_int_int_local   # one cell
```

(Do NOT pass `-- --ignored` — with no ignored cells it runs ZERO tests and reads
as green.)

---

## Pure loft tests — `tests/scripts/*.loft` and `tests/docs/*.loft`

A `.loft` test file is **three tests for the price of one**.  The same
file is picked up by:

- `tests/wrap.rs::dir` (and `::loft_suite`) — runs it through the
  **interpreter** end-to-end.
- `tests/native.rs::native_dir` / `::native_scripts` — generates Rust source, compiles with
  `rustc`, runs the **native** binary.
- `tests/wrap.rs::wasm_dir` — compiles via `--native-wasm`, optionally
  runs with `wasmtime` (skipped silently if `wasm32-wasip2` or
  `wasmtime` is unavailable).

### What a `.loft` test asserts with

**`assert(condition, message)` is the whole vocabulary.** There is no `assert_eq`, no
`assert_true`, and no `Test::` namespace — those are habits from other languages and cost a
compile error (`Unknown function assert_eq_int`) before they cost anything else.

```loft
assert(seen == 3, "the generator arrives three times: {seen}");
```

Interpolate the value you GOT into the message. The condition carries what you wanted, so a
failure that prints only "assertion failed" tells the next reader nothing they cannot already
see in the source, while `got 2` tells them how far off it was. (A general `assert_eq` that
reports both sides is loft-lang/loft#1147.)

The message is a normal interpolated text, so a struct field, a length or a whole expression
can go in it.

**That's why pure loft tests have a bigger testing scope than Rust
integration tests.**  A `code!(...)` test in `tests/issues.rs`
exercises one execution path (the in-process interpreter via
`State::execute`).  A `.loft` script automatically exercises all three
backends — interpreter, `--native`, and WASM — and any divergence
between them surfaces as a backend-specific failure.  When you fix a
codegen bug, dropping a single `.loft` reproducer into `tests/scripts/`
locks all three backends in one stroke; the same coverage in Rust
would be three separate tests.

This is also why **the cross-mode Rust harness exists at all**: a matrix
needs precise per-cell control of which loft snippet runs in which
mode and a stdout-equivalence assertion.  For broader regression
coverage where you trust the assertions in the script body itself, a
`.loft` file under `tests/scripts/` is the lighter-weight option.

### Where to put a new `.loft` test

| Location | Purpose | Doc fields required? |
|---|---|---|
| `tests/scripts/<name>.loft` | Regression script for a fix or a feature corner case.  Live naming: the issue number as prefix (`1004-…`) or a descriptive slug (`a-format-spec-is-honoured-not-dropped.loft`). | No — but `@falsified-at:` is gated (below) |
| `tests/docs/<NN>-<topic>.loft` | Topic-level documentation script.  Output is part of the public language reference (`gendoc` writes HTML from these). | **Yes** — `@NAME` + `@TITLE` |
| `tests/lib/<name>.loft` | Library fixture for tests using `// @ARGS: --lib tests/lib`. | No |

### Test-driver annotations

All annotations are line comments of the form `// @KEY[: VALUE]`, placed
either in the file header (above the first declaration) or just above a
specific `fn`.

| Annotation | Scope | Effect |
|---|---|---|
| `// @NAME: <short>` | Header (docs only) | Short name for the HTML doc index.  Required in `tests/docs/*.loft`. |
| `// @TITLE: <text>` | Header (docs only) | Full title for the rendered doc page.  Required in `tests/docs/*.loft`. |
| `// @ARGS: --lib <dir>` | Header | Extra CLI args passed to both wrap.rs and native.rs runners.  Only `--lib <dir>` is recognised at the test layer; other flags are ignored.  Use to point a script at a fixture directory (e.g. `// @ARGS: --lib tests/lib`). |
| `// @EXPECT_ERROR: <substring>` | Anywhere | The script is expected to fail parse / scope-check / runtime with a diagnostic containing this substring.  Multiple `@EXPECT_ERROR:` lines accumulate.  Native runs **skip** files with `@EXPECT_ERROR` (the negative test only runs against the interpreter). |
| `// @EXPECT_WARNING: <substring>` | Anywhere | Like `@EXPECT_ERROR` but for warnings — execution proceeds. |
| `// @EXPECT_FAIL` | File-level (header) **or** fn-level (the comment block immediately above a `fn`) | Tolerate a panic.  File-level: parse / scope-check / runtime failures are accepted anywhere.  Fn-level: only the named fn's panic is tolerated; sibling fns still must pass.  Add a colon-trailing reason when known: `// @EXPECT_FAIL: native function not loaded`.  Native runs skip the file only for a FILE-LEVEL `@EXPECT_FAIL`; a fn-level one skips just that fn and the siblings still run natively (loft#1311). |
| `// #warn <text>` | Anywhere | Older-style expected warning.  Still supported; `@EXPECT_WARNING:` is preferred for new tests. |
| `// @falsified-at: <ref>` | Header — **GATED** for every `tests/scripts/*.loft` | The falsification receipt: `make falsify GUARD=<file> REF=<commit>` proves the guard FAILS on the build it was written to catch, and this records the answer (`@falsified-at: none — <reason>` when no such build exists).  `tests/doc_hygiene.rs` fails any scripts file without it (pre-existing files ride `tests/falsified.baseline`, a shrink-only ratchet).  TESTING.md § falsification. |

**Diagnostic-format rule:** the `<substring>` for `@EXPECT_ERROR` and
`@EXPECT_WARNING` is a substring match against the rendered diagnostic.
Don't include the trailing ` at file:line:col` location — that's
appended by the renderer and would break when line numbers shift.  Just
the message text.

### File-level skip arrays

When a script can't run in one backend (rare; usually a known feature
gap), name it in the relevant skip array instead of marking the script
itself with `@EXPECT_FAIL`:

| Constant | File | When to add |
|---|---|---|
| `SUITE_SKIP` (in `wrap.rs`) | Used for "interpreter can't run this" — extremely rare since the interpreter is the reference backend. |
| `WASM_SKIP` (in `wrap.rs`) | Add when WASM build can't accept the script (e.g. threading model differences).  An entry carries the open issue and the condition that removes it (TESTING.md § Every skip). |
| `NATIVE_SKIP` (in `native.rs`) | For `tests/docs/`: native-specific feature gap. |
| `SCRIPTS_NATIVE_SKIP` (in `native.rs`) | Same as `NATIVE_SKIP` but for `tests/scripts/`. |

Prefer `@EXPECT_FAIL` for "this is broken right now" (single source of
truth in the script) and the skip arrays for "this backend
fundamentally can't run this kind of script" (orthogonal to the bug
status).

### When a `.loft` test is preferred over a Rust unit test

- The bug reproduces in a sequence of statements that already mirrors
  loft idioms (file I/O, struct construction, vector ops).
- You want all three backends to assert the same behaviour without
  writing the assertion three times.
- The fix is in codegen / runtime, where backend divergence is the
  actual hazard.

### When a Rust unit test is preferred

- The behaviour is a pure parse-error (no runtime to verify) — use
  `code!(...).error(...)` in `tests/parse_errors.rs`, faster than
  spinning up wrap+native+wasm.
- You need precise control over a `Value` comparison or type — use
  `code!(...).expr(...).result(Value::Int(...))` in `tests/issues.rs`.
- The bug is in the testing harness itself.
- Cross-mode equivalence with byte-identical stdout matters (use
  `cross_mode!`).

---

## `#[ignore = "<reason>"]` conventions

Every `#[ignore]` attribute MUST carry a reason.  Bare `#[ignore]` is
opaque and breaks the audit trail.  **The reason MUST name the
trigger to resume** so a future `cargo test -- --ignored` run +
`grep` audit can locate every parked test and tell what should
re-activate it.

Reason categories:

| Reason format | Meaning | Example |
|---|---|---|
| `P###` / `#NNN` | Open bug — now a **GitHub Issue** (legacy `P###` ids survive as references; PROBLEMS.md is the closed archive).  Un-ignore in the commit that closes it (`Fixes #NNN`). | `#[ignore = "P207 — native char-tuple-elem eq codegen bug"]` |
| `<feature-tag> — <plan-ref>` | Waiting on a feature or plan phase that hasn't shipped.  Un-ignore in a one-line follow-up commit when the feature lands. | `#[ignore = "T1.8a — plan-06 phase 9a"]` |
| `<plan>-<phase>` | Pending implementation in a multi-phase plan.  Same un-ignore rules as a bug, but tracked via the plan rather than an Issue. | `#[ignore = "plan-14 phase 03"]` |
| `<plan> (<sub>) — un-ignore when <trigger>` | User-facing lock-in test: the test demonstrates a today-broken behaviour, marked ignored so CI stays green; auto-flips to PASS when the trigger fires. | `#[ignore = "plan-17 (A) caveat — implicit generic-tuple type inference; un-ignore when the parser propagates substituted return types to receiving variables (DEFERRED.md / USER_FACING.md)"]` |

**The trigger phrase is mandatory.** Acceptable forms include
`un-ignore when <X>`, `triggers when <X>`, or `<plan>-phase <N>`.
The convention is greppable: `cargo test -- --ignored` + reading
the reason should tell a future contributor what to do.

When un-ignoring, the commit message names the reason being retired and
the new test status (`P207 closes; cell flips from #[ignore] to PASS`).

### Lock-in tests for user-facing deferred items

When deferring an item that affects user code (anything that
belongs in `doc/claude/USER_FACING.md`), **write the lock-in test
in the same commit as the deferral**.  The test exercises the
today-broken shape, asserts the post-fix behaviour, and is
`#[ignore]`d with a trigger.  When the fix lands, the test goes
green automatically — preventing accidental release without the
fix.

Example (plan-17 phase 01 follow-up):

```rust
#[test]
#[ignore = "plan-17 (A) caveat — implicit generic-tuple type inference; un-ignore when the parser propagates substituted return types to receiving variables (DEFERRED.md / USER_FACING.md)"]
fn plan17_a_implicit_generic_tuple_type_inference() {
    code!(
        "fn min_max<T: Ordered>(a: T, b: T) -> (T, T) {
    if a < b { (a, b) } else { (b, a) }
}
fn run() -> integer {
    t = min_max(7, 3);                         // <- no annotation
    t.0 * 10 + t.1
}"
    )
    .expr("run()")
    .result(Value::Int(37));
}
```

The two-file index — `doc/claude/plans/DEFERRED.md` (every parked
item) and `doc/claude/USER_FACING.md` (user-visible subset) — is
the single source of truth.  A lock-in test references the
relevant file in its ignore reason so a future contributor can
trace the audit trail in one grep.

---

## Filing a bug you find while testing

Authoritative policy: `CLAUDE.md` § Bug-filing policy + `doc/claude/ISSUE_TRACKING.md`.
**Open bugs live as GitHub Issues now, not PROBLEMS.md rows** (PROBLEMS.md is the
closed/historical archive; the legacy `P###` ids survive only as references).

1. **Default is to FIX, not file.** A bug you surface while testing is the cheapest
   you'll ever fix — code paths warm, repro at hand: fix it + pin a regression.  File
   only when you are NOT fixing it now (it blocks the task, or it's M+/needs design).
2. **When you file → `gh issue create`** (the `bug_report` template): a minimal
   reproducer (expected vs observed per backend), `sev:` + `area:` labels, and a
   **verified** `wa:*` workaround.  Not a PROBLEMS.md row.
3. **Do NOT file** for: clippy / formatter complaints (fix in-branch, note the commit
   — memory `feedback_fix_old_clippy.md`); or a bug you fix in the SAME change (the
   fix + regression test ARE the record — close it with `Fixes #NNN`, don't also file).
4. **Pin every fix with a regression test.** Name it `p<NNN>_<short>` for a legacy
   P-id or `tests/scripts/NN-<slug>.loft` for a GitHub issue; reference it in the
   commit (`Fixes #NNN`).

---

## Targeted regression — which suites to run

**Just run `./scripts/find_problems.sh`.** Its default is the CURATED set —
everything except a short named list of slow-and-few binaries.  The current
numbers (test counts, timings, the excluded list) live in
`scripts/find_problems.sh` / `scripts/test_subjects.sh`, which also carry the
re-measure recipe — don't trust a copy of them here.  You no longer have to
guess which suites your change could break, and guessing is what the old table
here asked for: it named a fifth of the binaries, two of which had not existed
for some time.

Guessing does not work, and there is a worked example. An over-broad change to
the parser's `null()` was caught by `binary_io_matrix` — a binary no
parser-touches-these-suites map would ever have picked. Curating by INCLUSION has
to predict which suite will catch a bug, which nobody can do in advance; curating
by EXCLUSION leaves a miss set that is small, named and reviewable.

| You want | Run |
|---|---|
| the normal check before a commit | `./scripts/find_problems.sh` |
| a tight loop on one area | `./scripts/find_problems.sh --subject <name>` (seconds) |
| everything, incl. the slow-and-few | `./scripts/find_problems.sh --full` |
| to see subjects + what is excluded | `./scripts/find_problems.sh --list-subjects` |

Subjects are `parser scopes codegen runtime store wasm packages lsp sql docs
host`, defined as binary-name PATTERNS in `scripts/test_subjects.sh` so a new
binary joins the subject its name already matches. They are a convenience for
tight loops, not the safety mechanism — the default is subtractive, so a gap in
a subject costs seconds, never coverage.

What the default skips is the `HEAVY_BINARIES` list in `scripts/test_subjects.sh` —
binaries that are slow AND have very few tests. CI's
`Test (ubuntu-latest)` job runs the suite unsharded as a required check, so none
of them can be skipped on the way to main.

**Always run after the targeted set:**
- `cargo fmt --all -- --check`
- `cargo clippy --release --all-targets -- -D warnings`
- `cargo clippy --release --all-targets --no-default-features -- -D warnings`

The two clippy variants together are the local CI gate; the
`--no-default-features` variant catches lint debt in conditionally
compiled paths and is the one most often skipped.

---

## When the targeted set isn't enough

For multi-subsystem changes (compiler refactors, ABI changes, big
plans crossing parser+codegen+runtime), use the background full-run:

```bash
./scripts/find_problems.sh --bg     # detached cargo test --release --no-fail-fast
./scripts/find_problems.sh --peek   # mid-run stats
./scripts/find_problems.sh --wait   # block until done
```

`/tmp/loft_problems.txt` gets a structured summary (FAILED list,
stdout blocks, SIGSEGV context, wrap-suite `--nocapture` re-run if
a crash masks a `.loft` filename).  See
[TESTING.md § Preferred shape](../../../doc/claude/TESTING.md) for the
full rationale.

---

## Naming conventions

- `p<NNN>_<short_describe>` — regression test for P<NNN>.  Lives in
  the binary that exercises the relevant code path (most often
  `issues.rs` or `parse_errors.rs`).
- `<feature>_<aspect>_<expected>` — feature tests.  E.g.
  `tuple_match_binding`, `tuple_compound_assign_rejected`.
- `e<elem>_d<dest>_<sub>` — tuple-matrix cells.  Don't reuse this
  prefix outside `tests/tuple_matrix.rs`.

The `should_panic` attribute is rare — most negative behaviour goes
through `.error()` / `.warning()` on `code!`.  Reserve `should_panic`
for runtime panics that have no diagnostic-printing path.

---

## Pre-flight checklist for a new test

- [ ] Picked the right binary (matches the failing-shape).
- [ ] Used `code!` for unit tests, `cross_mode!` for runtime + cross-backend, `expr!` for one-liner expression results.
- [ ] If `#[ignore]`, the reason follows the conventions table above.
- [ ] If pinning a P-id fix, the test name starts with `p<NNN>_`.
- [ ] If un-ignoring, the commit message names the reason being retired.
- [ ] Ran the targeted-suite list, not the full suite, unless the change is multi-subsystem.
- [ ] `cargo fmt --all -- --check` and both clippy variants are green.
- [ ] No nested `fn` definitions in any loft body string.
- [ ] No `->` arm separators in any `match` (use `=>`).
- [ ] No `cross_mode!` body shorter than `fn test() { … }` (the harness appends `fn main`, nothing else).
- [ ] If introducing a new `tests/*.rs` binary, its name matches a subject pattern in `scripts/test_subjects.sh` (or extend one).
- [ ] If adding a `.loft` test, picked the right location: `tests/scripts/` (regression) vs `tests/docs/` (also drives HTML).
- [ ] `tests/docs/*.loft` files have both `@NAME:` and `@TITLE:` header comments.
- [ ] Every new `tests/scripts/*.loft` records `@falsified-at:` (run `make falsify` — the doc_hygiene gate fails without it).
- [ ] `@EXPECT_ERROR:` / `@EXPECT_WARNING:` substrings do NOT include the `at file:line:col` tail.
- [ ] `@EXPECT_FAIL` placement is correct: file-level only when the comment is in the header above the first declaration; fn-level only when the comment is the line(s) immediately above the target `fn`.

---

## Parser tracing (`LOFT_TRACE`)

Compile-time debugging vantages with near-zero overhead when
disabled.  Distinct from `LOFT_LOG` (runtime/bytecode) and
log-config (loft-program-level).  See `src/trace.rs` for the
implementation; this section documents *when to use it*.

### Enabling

```bash
# Single category:
LOFT_TRACE=call cargo run --release --bin loft -- --interpret /tmp/foo.loft

# Multiple categories, comma-separated:
LOFT_TRACE=call,field,generic cargo run ...

# Everything:
LOFT_TRACE=all cargo run ...

# During tests (use --nocapture to see stderr):
LOFT_TRACE=generic cargo test --release --test issues plan17 -- --nocapture
```

When `LOFT_TRACE` is unset (default), trace calls compile to one
bool load + one branch; the branch predictor learns "always-false"
fast and the overhead is below measurement.  Format-string args
are evaluated only when the branch fires.

### Currently registered categories

| Category | Site | Use when debugging |
|---|---|---|
| `call` | `Parser::call` after `find_fn` | "Which def did the parser resolve, and was it skipped as Generic?"  Used heavily in plan-17 (A). |
| `field` | `Parser::field` after attr lookup | "Did method dispatch find the attribute, what's the receiver type?"  Used in plan-17 (B). |
| `generic` | `predict_generic_return_type` + `try_generic_instantiation` | "What did first-pass predict?  What did second-pass instantiate?"  Used in plan-17 (A). |
| `match` | `expect_match_arm_arrow` | "What arrow did the parser see at the arm boundary?"  Used in P206 + plan-18. |

### Adding a new category — selective rule

Trace points are for **recurring** diagnostic vantages, not one-off probes: a
bug-specific `eprintln!` stays local and is removed before commit; a vantage you
expect to revisit becomes a `loft_trace!(category, …)` call.  The mechanics of
adding a category (field, initialiser, macro arm) are documented in
`src/trace.rs` itself — follow it, then extend the table above.

### When NOT to use `LOFT_TRACE`

- Runtime/bytecode debugging — use `LOFT_LOG` (TESTING.md).
- Loft-program-level logging — use `log_info` / `log_warn` /
  `log_error` from inside loft programs (STDLIB.md § Logging).
- One-time diagnostic probes — temporary `eprintln!` is fine
  for a single session; remove before commit.

---

## Cross-references

- [TESTING.md](../../../doc/claude/TESTING.md) — runtime debugging knobs:
  `LogConfig`, `LOFT_LOG`, dump file format, `LOFT_DUMP_DEPTH`.
- [ISSUE_TRACKING.md](../../../doc/claude/ISSUE_TRACKING.md) — where bugs
  live (open → GitHub Issues, closed → PROBLEMS.md archive) + the item
  lifecycle.  Run `gh issue list` before filing a new one.
- [DEVELOPMENT.md](../../../doc/claude/DEVELOPMENT.md) — branch policy,
  commit ordering, push gate.
- [loft-write skill](../loft-write/SKILL.md) — for the loft-source
  side of test bodies (types, syntax, error→fix table).
- `tests/testing.rs` — `code!` + `expr!` macro source.
- `tests/common/cross_mode.rs` — `cross_mode!` harness source.
- `tests/native.rs` — donor of `find_loft_rlib`,
  `compile_native_job`, `run_native_job` helpers.
