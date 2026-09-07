
# Testing Framework

## Reach for this first — efficient failure triage

After a refactor expected to touch many tests, do NOT iterate
"run → see one failure → fix → re-run → see the next failure":
each cycle pays the compile + test-startup cost and discovers
only one failure at a time.  Run a single `--no-fail-fast`
pass and read the captured failures once.

### Before the suite: `make check-rlib` (one second)

The tests link a built `libloft.rlib`, and **nothing in an ordinary edit loop
rebuilds one**.  `cargo build --bin loft` refreshes the binary and leaves every rlib
behind.  So a session that iterates on the compiler with `--bin loft` drifts, and the
drift is invisible until a gate runs.

There are **three** of them, one per link target, and they drift independently:

| rlib | linked by | cure |
|---|---|---|
| `target/release/libloft.rlib` | `--native`, the cdylib tests | `cargo build --release --lib` |
| `target/wasm32-unknown-unknown/release/libloft.rlib` | `--html` | `cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random` |
| `target/wasm32-wasip2/release/libloft.rlib` | the wasm library suite | `cargo build --release --target wasm32-wasip2 --lib --no-default-features --features random` |

Refreshing the native one does nothing for the other two — which is how a re-run that
fixed three native failures still went red on `moros_editor_html_smoke` and
`wasm_library_suite` alone, a second full cycle later.

It does not fail like a compile error.  It surfaces roughly nine minutes in, as a
handful of tests failing for what look like unrelated reasons — `libloft.rlib not
found for this build`, a cdylib mtime that did not advance, a `--html` build
panicking — each naming a file that is present when you go and look.  The cost is a
whole cycle, every time.

`make ci` builds all three itself, beside the wasm builds it already ran, so it
needs no pre-flight — a gate that refused on something it could build would be
friction after every edit, and that is how a check gets switched off.  **A bare
`cargo test --release` builds none of them**, so run the check yourself before one:

```bash
make check-rlib          # all three, each with its own cure; skips a target that isn't installed
```

**And run it BEFORE the suite, not after the verdict** — afterwards it cannot save the
run, only tell you the cycle was wasted.  `find_problems.sh` was the trap: it rebuilds
the six cdylibs and BOTH wasm rlibs and prints them as timing rows, so its own output
reads as *the rlibs are handled* — while the one it skipped was
`target/release/libloft.rlib`, the only one an ordinary `cargo build --bin loft` loop
makes stale — two of the three rlibs, with the missing one hidden behind the two rows
naming the ones it did build;
a full `4594 passed` was read off a run whose native half linked a library four commits
behind the fix it was gating (2026-09-06).  It now schedules that build too, so
`--bg` needs no pre-flight either.  The general form: ask what an instrument did NOT
refresh, because a partial refresh that reports its successes misleads more than one
that refreshes nothing.

**Waiting for a backgrounded `make ci` — do not race the file it writes.** The recipe
truncates `result.txt` and writes `.ci-running` only after `make` has started, so a wait
loop armed in the same breath as the launch sees NEITHER yet, exits immediately, and
reads the PREVIOUS run's `result.txt`. That reports the last run's verdict as this one's
— once here as a `CI-RESULT: FAILED` for a run that had not begun. Give it a moment and
confirm the marker exists before waiting on its absence:

```bash
nohup make ci > ci.log 2>&1 &
sleep 25 && test -f .ci-running || echo "the gate never started — read ci.log"
until [ ! -f .ci-running ]; do sleep 30; done; tail -3 result.txt
```

`result.txt` opens with a `== make ci | <rustc> | <UTC timestamp> ==` header for exactly
this reason: it dates the verdict, so a stale one can be told from a fresh one by
reading rather than by remembering.

**The other end of the same race: a run that is KILLED mid-flight.** It leaves both
halves of the state misleading, in opposite directions:

* `.ci-running` is still there, so `until [ ! -f .ci-running ]` waits forever on a run
  that no longer exists.  Before clearing it by hand, confirm nothing is actually
  building — `for p in $(pgrep -x "cargo|rustc"); do readlink /proc/$p/cwd; done`, which
  also tells you whether the build belongs to THIS checkout.  A bare
  `pgrep -f "make ci"` is not that check: it matches its own command line.
* `result.txt` is truncated but still holds the PREVIOUS run's
  `CI-RESULT: ALL GATES PASSED`, so grepping for the verdict after a kill can report a
  pass that never happened.  **Gate on the lock's absence first, then read the verdict**
  — and read the timestamp header, which is what dates it.

This is not hypothetical: two gates here were killed as collateral when a SIBLING
checkout's run was killed (2026-08-25), the case `scripts/find_problems.sh` warns about
in its own comments — *"`pkill -f nextest` reaches into other checkouts and starts a
run-killing battle."*  The tell is that the `make ci` shell and its independent waiter
died in the same second; a per-process kill takes one process, not two unrelated ones.

⚠ And do not end a status command with `grep -c "^ *FAIL" result.txt`: `grep -c` exits
**1** when the count is ZERO, so a clean gate is reported as a failed command.  Three
passing runs were misread that way in one session.  Use `|| true`, or put the count first.

### Preferred shape — background + peek + wait

```bash
./scripts/find_problems.sh --bg        # kick it off, returns immediately
# keep working; while the ~60-90 s run proceeds:
./scripts/find_problems.sh --peek      # snapshot of any failures so far
# when done:
./scripts/find_problems.sh --wait      # block until finished, then summarise
# result: /tmp/loft_problems.txt
```

`--bg` starts the test runner in a detached subshell and writes
the raw log to `/tmp/loft_test.<id>.log`, where `<id>` is a
per-checkout tag derived from the repo root — so sibling working
trees (e.g. two agents) can run the script concurrently without
sharing pid/log files.  The summary lands at the per-checkout
`/tmp/loft_problems.<id>.txt` AND is copied to the stable
`/tmp/loft_problems.txt`; every mode prints the exact paths.  `--peek` tails the live log
and pulls out any `FAILED` markers, inline panics, and SIGSEGV
context (last ~15 lines before each crash).  `--wait` blocks on
the background pid and then produces the final summary.

**Test runner choice.**  The script prefers `cargo nextest run
--release --no-fail-fast --status-level fail` when nextest is on
`PATH` (typical loft suite is 2-3× faster wall-clock at test
execution because nextest parallelises at the test level rather
than the binary level).  Falls back to `cargo test --release
--no-fail-fast` when nextest is not installed.  The runner choice
is logged on launch so a regression in test execution speed can
be tied to a runner / profile change.

Before the test run starts, the script rebuilds every sibling
cdylib under `lib/*/native/`, plus `tests/lib/*/native/` and the
`wasm32-unknown-unknown` rlib (when its `target/` directory
exists).  The suite dlopens these libraries through
`extensions::load_all` or links them via `--native`; when the
`.rlib` / `.so` is older than its source, rustc surfaces a
confusing `cannot find function X in crate loft_*_native` and
cascades a dozen unrelated test failures.  Cargo is incremental,
so a clean tree is ~free; a stale tree costs one recompile but
stops a whole class of misleading reports.  The same freshness
step is wired into `make test`, `make quick`, `make ci`, and
`make run-tests` via the `rebuild-native-cdylibs` target.

**Parallel rebuild + per-step timings.**  All cdylib + wasm32
rebuilds run in parallel under `rebuild_native_cdylibs`; total
wall-clock is the slowest single step rather than the sum.  Each
step's timing prints to stderr live and accumulates in
`/tmp/loft_timings.<id>.txt`.  At the end of `--wait` (and the
foreground path) the script prints a `=== Wall-clock timing
summary ===` block so a regressing step is named, not just "the
suite is slow."  Format:

```
  cdylib lib/graphics/native                        1.910s
  cdylib lib/imaging/native                         0.885s
  ...
  wasm32 rlib                                       0.588s
  (rebuild_native_cdylibs total wall-clock)         1.949s
  cargo nextest run --release --no-fail-fast …    313.479s
```

Running in the background is the default for a reason: the
suite takes long enough that blocking on it wastes cycles you
could spend reading the failure pattern that's already showing
up in the log.  **Never run the full test suite in the
foreground** — always go through `--bg`.  `cargo clippy` and
single-file tests (`cargo test --release --test issues
<prefix>`) stay foreground.

### Foreground shape (small contexts)

When you just want one run and are happy to wait:

```bash
./scripts/find_problems.sh                  # streams to stdout + log
./scripts/find_problems.sh /tmp/log /tmp/problems  # custom paths
```

The summary has one `test NAME ... FAILED` line per failure,
the stdout block for each, a SIGSEGV-context block when a
binary crashed, and (if a wrap-suite crash was detected) a
re-run of `loft_suite` under `--nocapture --test-threads=1`
that recovers the crashing `.loft` file's name.

See § [One-pass-find-all-problems workflow](#one-pass-find-all-problems-workflow)
below for the full rationale and when NOT to use this shape.

## Contents
- [Overview](#overview)
- [Entry Points](#entry-points)
- [The Testing Framework (`tests/testing.rs`)](#the-testing-framework-teststestingrs)
- [Generated Test Files (`tests/generated/`)](#generated-test-files-testsgenerated)
- [Additional Output Files](#additional-output-files)
- [LogConfig — Debug Logging Framework](#logconfig--debug-logging-framework)
- [`tests/wrap.rs` — shared runner for docs and scripts tests](#testswraprs--shared-runner-for-docs-and-scripts-tests)
- [`tests/docs/` — end-to-end loft files (user documentation)](#testsdocs--end-to-end-loft-files)
- [File Layout Summary](#file-layout-summary)
- [Running the Tests](#running-the-tests)
- [Validating Generated Code — the `generated/` Workspace](#validating-generated-code--the-generated-workspace)
- [Key Constraints](#key-constraints)
- [`tests/scripts/` — standalone loft test suite](#testsscripts--standalone-loft-test-suite)
- [Debugging failures in `tests/scripts/`](#debugging-failures-in-testsscripts)
- [What a run did NOT check — scope, admission, coverage](#what-a-run-did-not-check--scope-admission-coverage)
- [How a guard reads green while the defect stands](#how-a-guard-reads-green-while-the-defect-stands)

---

## Overview

The loft test suite has two distinct layers:

1. **Interpreter tests** (`tests/*.rs`) — Rust integration tests that parse and run loft code through the full compiler pipeline, validating results, errors, and warnings at the interpreter level.
2. **Generated Rust tests** (`tests/generated/*.rs`) — self-contained Rust files emitted by the interpreter tests (debug builds only) that replay the same logic through the compiled code generator, validating the generated Rust output.

Both layers share a common structure: the interpreter tests drive everything, and the generated tests are a by-product of running them.

---

## Entry Points

### `tests/*.rs` — interpreter test files

Each file is a Cargo integration test (auto-discovered because it lives directly in `tests/`). The test files are:

| File | Contents |
|---|---|
| `expressions.rs` | Type-check tests, labeled loops, mutual recursion, null returns, character appends (simple arithmetic/loop tests live in `tests/scripts/`) |
| `enums.rs` | Complex enum definitions, polymorphism via parent enum, JSON formatting, nested types |
| `strings.rs` | Complex string operations: UTF-8 indexing, reference params, rfind, parsing loops |
| `objects.rs` | Struct creation, `:#` pretty-print format, field references, text independence, mutable reference params |
| `vectors.rs` | Complex vector/sorted/index/hash operations; remove-by-key; for-comprehension; large growth |
| `sizes.rs` | `sizeof` expressions and struct layout (complex struct/collection byte sizes) |
| `data_structures.rs` | Combined data structure behaviour |
| `parse_errors.rs` | Tests that expect specific parse/type errors (all diagnostic — must stay in `.rs`) |
| `immutability.rs` | Immutability diagnostics (`ref never modified`, `const mutated`) |
| `slot_assign.rs` | Stack-slot assignment correctness (no overlapping slots) |
| `log_config.rs` | Unit tests for the `LogConfig` debug-logging framework |
| `threading.rs` | Low-level Rust parallel API tests only (`run_parallel_int`, `run_parallel_raw`, `run_parallel_text`); end-to-end parallel tests live in `tests/scripts/22-threading.loft` |
| `issues.rs` | Minimal reproducers for known open/fixed issues (see [PROBLEMS.md](PROBLEMS.md)) |
| `expressions_auto_convert.rs` | Auto-conversion edge cases (hand-written) |
| `wrap.rs` | Runs `.loft` files from `tests/docs/`; generates HTML docs |
| `testing.rs` | The framework itself; not a runnable test target |

Each file includes `mod testing;` which pulls in `tests/testing.rs` as a module.

### Leak gate (`run_test` in `wrap.rs`)

After running a `.loft` file's functions, `run_test` calls
`state.collect_store_leaks()` and **hard-fails** if any heap store is unfreed at
program exit — making the `tests/scripts/` + `tests/docs/` corpus a leak
regression net (a new scope-free leak in any covered file breaks CI).  Files
with known, pre-existing program-end leaks (top-level `main` locals that aren't
scope-freed at the very end — see [@P322](PROBLEMS.md)) are grandfathered in
`SCRIPTS_LEAK_ALLOW`.  When a new file legitimately leaks (an intentional
program-end allocation), add its name there with a one-line rationale; otherwise
fix the missing free.  The complementary **native** leak gate runs generated
binaries with `LOFT_NATIVE_LEAK_CHECK=1` (`tests/leak_cases.rs`,
`tests/common/cross_mode.rs`); the dedicated shape corpus lives in
`tests/leak.rs`.

⚠ **`loft --tests <file>` does NOT run this gate** — only `wrap.rs` does.  Running a
leak regression test the quick way therefore reports `ok` while the file leaks, which
reads exactly like a fix that works.  Check a new leak guard with
`cargo test --test wrap`, or run the file the plain way (`loft --interpret <file>` with
a `main`, which does print the by-type warning).  Measured: a loft#1019 guard passed
under `--tests` on the very binary it was written to fail on, and `wrap.rs` then
hard-failed it with ten leaked records.

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

These two strings determine where the generated test file is written.

### The `Test` struct

```rust
pub struct Test {
    name: String,         // short test name
    file: String,         // module / file name
    expr: String,         // loft expression to evaluate
    code: String,         // loft code block (may be empty)
    warnings: Vec<String>,
    errors: Vec<String>,
    fatal: Vec<String>,
    sizes: HashMap<String, u32>,
    result: Value,        // expected interpreter result
    tp: Type,             // expected type (when needed)
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

### Execution model — `Drop`

**All test logic runs inside `impl Drop for Test`.** There is no explicit `.run()` call; the test executes automatically when the `Test` value goes out of scope at the end of the `#[test]` function.

The `drop` implementation:

1. Constructs a `Parser` and loads the default library from `default/`.
2. Appends a synthesised `test()` function (see below) when `.expr()` or `.result()` was set.
3. Parses the combined loft source via `p.parse_str(...)`.
4. Validates struct sizes against any `.sizes` entries.
5. Runs `scopes::check` (scope/type analysis).
6. **Debug builds only:** calls `generate_code` (writes `tests/generated/`).
7. Calls `assert_diagnostics` — panics if the actual warnings/errors do not exactly match the expected set.
8. If parsing succeeded: runs `byte_code` + `state.execute("test", ...)`.
9. **Debug builds only:** logs bytecode and execution trace to `tests/dumps/<file>_<name>.txt`.

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

Family today:

| Binary | Plan | Axes | Cells |
|---|---|---|---|
| `tests/tuple_matrix.rs` | @PLAN14 | element type × destructure shape | tuple bug surface |
| `tests/template_matrix.rs` | @PLAN17 | T-parameter usage × bound shape | bounded-generic / interface surface |

Future matrices follow the same shape (coroutine validation
is active under `plans/finished/16-coroutine-validation/`; match
validation pending in `plans/29-match-validation/`).  Closure validation
shipped as `plans/finished/15-closure-validation/` 2026-05-12;
22 cells in `tests/closure_matrix.rs` plus 5 leak guards in
`tests/leak.rs::p15_phase0[345]_*_no_leak`.

### Pattern

- **Every cell is `#[ignore]` by default.**  Each cell shells out to
  `loft --interpret` and `loft --native` (the latter invokes `rustc`)
  — too heavy for the default `cargo test` path.
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
- **Bug yield is the headline metric.**  Plan-14 found 2 P-issues
  in 15 cells (13%); @PLAN17 found 6 P-issues in 6 phases (close
  to the predicted 5-10).  Each filed P-issue blocks plan
  acceptance; each PASS cell becomes a regression net.

Run a whole matrix:
```bash
cargo test --release --test template_matrix -- --ignored
```

A single cell:
```bash
cargo test --release --test template_matrix -- --ignored u3_b1a_addable_inline_pair_with_sum
```

The `cross_mode!` macro (heavy-by-default) is documented in detail
in [`.claude/skills/loft-test/SKILL.md`](../../.claude/skills/loft-test/SKILL.md)
§ "The `cross_mode!` macro" — read that before authoring matrix
cells.

For the per-plan matrix definitions and bug-discovery records, see:

- [`plans/finished/14-tuple-validation/`](plans/finished/14-tuple-validation) (closed 2026-05-11)
- [`plans/finished/17-template-validation/`](plans/finished/17-template-validation) (closed 2026-05-09)

---

## Testing race-prone and backend-divergent mechanics

Two methodology rules govern the hard cases — concurrency and
interpret-vs-native divergence.  Both say the same thing: **the answer lives at
small scale; large scale only verifies that the small-scale answer was sound.**

### Race conditions: reason small, scale only verifies

**Real race-condition testing starts small.**  A race is a property of the
*mechanic* — which memory is shared, accessed how, synchronised how — and that is
established by **reasoning about the mechanic at n=1, deterministically and
readably**.  A large-scale stress run does **not *find*** the race; it **verifies
that the small-scale soundness claim held**.  Relying on stress to *discover*
races is epistemically weak in both directions: a clean 10 000-iteration run
proves nothing about absence (the window may be narrow), and a dirty one is only a
louder hint you should have reasoned it out.  Stress is a backstop against your
reasoning being *incomplete* — never the source of the answer.  (It is the same
"trust the statistics" gloss the project distrusts elsewhere — cf. the store
refcount in [GOALS.md § Goal E](GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth):
opaque machinery papering over a deterministic truth you could reason to.)

So the ladder is: **reason the mechanic small → state the soundness claim →
*only then*, if the claim is "race-free *because* lock X holds under contention",
reach for stress (or the [plan-53 sanitizer](plans/finished/53-sanitizer-ci-lever/README.md)
engine) to confirm lock X actually holds.**  A mechanic whose small-scale reasoning
concludes "no shared mutable state" has *nothing left for stress to verify*.

*Worked example* (plan-57 `probes/bugs/`): the `parallel {}` capture probing
settled entirely at n=1.  Reasoning about the worker model — each arm gets a
**read-only heap clone** (`clone_locked_for_worker`) + a **private stack** —
shows it is *deterministic isolation*: no shared mutable state, so no data race is
*possible*.  Every failure is correspondingly deterministic (heap mutation →
crash on the read-only clone; scalar write → silent loss to the private stack;
native → arm bodies don't run at all).  Run a million times, identical every time.
Stress would have added zero; the small-scale probes were the whole answer.

### A fixed delay is not synchronisation — use a barrier

The corollary for **multi-process** tests, learned twice on the same test.
`multiplayer_v2::v2_two_clients_with_spectator_routing` needs two clients
*concurrently connected* so each observes the other's spectator frames. @P229a made
that happen with a **fixed post-handshake pause** in each client
(`LOFT_TICTACTOE_CLIENT_DELAY_MS=200`) plus a 50 ms spawn stagger.

That is a **bet that a partner's startup is faster than the pause**, and it lost under
full-suite load (2026-07-27): the failure log showed client A completing all three
moves *and its GameOver* before B's MAP frame ever reached the server, so neither side
saw the other. The decisive detail is *where* the variance lives — in each client's
**process startup** (spawn + parse + connect), which no wall-clock number bounds. A
bigger pause only lengthens the bet.

**The fix is to make the concurrency a fact, not a probability:** the driver spawns
both clients, waits until **both** have printed their "handlers learned" line, and only
then creates a **rendezvous file** both are spinning on
(`LOFT_TICTACTOE_GO_FILE`). Neither can move before both are registered.

Three properties worth copying:

- **The barrier releases unconditionally**, even if a client never reported, so a
  missing participant fails on a *real* assertion instead of hanging on its spin.
- **The waiter's spin is bounded** (30 s), so a driver that never writes the file
  degrades to the old behaviour rather than wedging the run.
- **Remove a stale barrier file before spawning.** A run killed between create and
  cleanup would otherwise let the next run's clients sail straight through, silently
  restoring the flake — the guard is one `remove_file`.

The payoff is coverage, not just quietness: with overlap guaranteed the assertion
tightened from *"at least one side observed the other"* to **both**, which the
timing-based version could not afford. An OR over two directions is satisfied by a
half-working router.

*Reading a flake honestly:* two synthetic load models (8-way CPU spin, concurrent loft
startups) failed to reproduce the original failure, so the fix rests on the
**mechanism** — read off the failure log — plus the barrier making that mechanism
impossible by construction, and on two clean full-suite runs. It is not a
red→green reproduction, and saying so is part of the result.

⚠ **Synthetic load is the wrong abstraction, and a second finding says why.**
`registry_index::tests::a_pair_torn_by_a_refresh_settles_into_the_new_generation`
failed under `make ci` and then PASSED the next `make ci` **on the same binary**;
four reproduction rigs found nothing — isolation (0/30), 48 busy-loops on 24 cores
(0/20), the CI `TMPDIR` with 39 562 files in it (0/25), and the whole lib suite in
parallel (0/3). Busy loops are preemptible and the scheduler still honours a timer;
what blew the test's budget was being scheduled beside the **rustc subprocesses**
of the `--native` binaries. nextest schedules dynamically, so it landed at 774/4364
in the passing run and 955/4364 in the failing one. **Ask what the failing
environment RAN, not how busy it was** — and reach for one re-run first, because two
runs of one binary disagreeing is the cheapest proof of a flake there is.

The cure was to **delete the race, not widen the margin**: the test slept 15 ms
against a 40 ms settle budget, and `thread::sleep` guarantees a minimum, so a bigger
sleep only moves the failure rate. Finishing the refresh from inside the `accept`
closure — which runs once per attempt, after both files are read — places the second
rename in exactly the window the field report describes, on every machine. Dropping
the thread costs no coverage because the concurrency half is
`a_replaced_file_is_never_read_half_written`'s next door, and that one is robust for
the reason this one was not: it exits on a **condition** under a deadline instead of
racing a fixed sleep. ⚠ A deterministic sequencing test needs its sequencing
asserted (`attempts == 2`) or it passes for a reader that never re-read at all.

### `assert`'s message runs too

`assert(cond, msg)` is an ordinary call, so **both** arguments are evaluated — the
message string is built whether or not the assertion fails ([formal/calls.md](formal/calls.md)
`F-Args`). A side-effecting call written once in the condition and again in the message
therefore runs **twice**:

```loft
c: vector<integer> = [0];
assert(bump(c) == 5, "got {bump(c)}");   // c[0] is now 2, not 1
assert(c[0] == 1, "…ran once");          // fails, and the fix is not here
```

Bind first (`got = bump(c); assert(got == 5, "got {got}")`). It costs a line and it is
the difference between a cell that measures the call and one that measures two of them —
which is exactly the kind of failure that reads as a bug in the code under test.

### Backend divergence: the differential check is the instrument

Interpret-vs-native disagreement is also an n=1, deterministic property — and the
`cross_mode!` validation matrix above **is** the instrument for it: every cell
runs on both backends and the disagreement (if any) shows up in a single
deterministic run.  The catch the `parallel {}` probing exposed: a per-backend
assertion that *passes on both backends* does **not** prove parity.  test-80 /
test-81 both "pass" on native precisely **because** native silently no-ops the
arms — `assert(true)` and asserts that hold *when the arms do nothing*.  A real
parity check asserts **identical observable output across backends**, not "each
backend satisfied its own assertion".  Parity (GOALS.md Goal D) has no standing
detector the way Goal A has the sanitizer and Goal E has `LOFT_STORE_GUARD`;
`cross_mode!` is the closest we have — point it at a mechanic and it catches
divergence the day it lands, provided the cell's oracle is the *cross-backend*
output, not a self-satisfying assert.

### A round-trip test is closed under its own encoder

The same self-satisfaction has a second, quieter shape: a **write it, read it
back, compare** test only ever exercises the inputs its own writer produces.
Every value the writer cannot emit is untested *and unmentioned* — coverage reads
100 %, because the decoder function really was entered.

`imaging` shipped this for months (@PLN141).  Its suite saved a `Canvas` as PNG
and reloaded it, pixel for pixel, across small / non-square / extreme / solid
shapes — all green.  Its encoder always writes 8-bit RGB, so RGBA, greyscale,
grey+alpha, palette and 16-bit files were never read by any test; the decoder
handed the raw buffer through and cut it into three-byte pixels, so an RGBA image
came back with **5** pixels for a 2×2 file, channels shifted one byte, against a
header still claiming 2×2.

The instrument is an input the code under test **did not produce**: a fixture
authored outside the system, with hand-computed expected values.  A good prompt
for finding where you owe one is asking what a *worked example* for the function
would have to assert — that question names the outside values by construction,
which is how this one was found.

### A guard that never failed is not a guard — `make falsify`

**`make falsify GUARD=tests/scripts/<file>.loft REF=<commit-before-the-fix>`**
(`scripts/falsify.sh`). It builds `REF` in a cached worktree, runs the guard THERE and
HERE, and compares four channels apart — **exit code, assertion failures, leaked stores,
panic** — so the verdict names *which one moved*:

```
backend      tree       exit|asserts|leak|panic       verdict
interpret    control    0|0|kt=78 Sk×156|none
interpret    here       0|0|none|none                 falsified
…
// @falsified-at: 3ca5ec79 — interpret leaked kt=78 Sk×156 -> clean, native leaked … -> clean
```

Paste that line into the guard.  `doc_hygiene::every_new_guard_records_its_control`
requires one on every file added under `tests/scripts/`, against the ratchet in
`tests/falsified.baseline`; `// @falsified-at: none — <reason>` is the honest opt-out for a
file that genuinely cannot fail on any earlier build.

**A defect only an instrument can see is scored with the instrument armed.**  `LOFT_POISON=1
LOFT_STRICT_STORES=1 make falsify GUARD=… REF=…` passes both through to the control and to
this tree.  Measured need: a stale work-ref reclaiming, in place, a store number another
record had since taken (QUALITY.md B7u) read as `INERT` in plain mode on every shape tried —
the allocator hands the stale buffer its own freed number back — and as six assertion
failures under the arena poison.  A guard falsified that way says so beside its line, and
names the CI leg that runs with the instrument (the nightly poison sweep), because on the
plain suites it passes on every build.

⚠ **Three shapes it cannot score, and all report INERT — which reads as "your guard measures
the wrong thing" when the truth is "this tool cannot see this kind of fix".**

*A guard with no `main`.*  `falsify` runs the file as a PROGRAM, so a file whose cells are
each their own entry point (the `tests/scripts/` convention) runs NOTHING there and reports
`0|0` on both sides.  The tell is **zero assertion failures on the CONTROL** — a real control
almost always has some.  Give such a guard a `main()` that calls its cells; the suite runner
is happy either way.

*A guard whose subject is a WARNING.*  `expect_channel` counts only `@EXPECT_ERROR` and
`@EXPECT_FAIL`, and `entry_modes` routes only those two through `--tests`; a guard declaring
`@EXPECT_WARNING` that also has a `main` — which a leak or value guard wants — gets a direct
run, where nothing matches warnings at all.  Both trees read `expect -`, and the guard is
INERT however loudly it fires.  Measured on
`a-payload-binding-warns-when-its-subject-is-given-another-variant.loft` (loft#1397): INERT
from the tool, `0 -> 2` reports by hand.  Score such a guard BY HAND until this grows a
warning channel — run it on both builds and count the reports — and say so in the
`@falsified-at` line, so the next reader does not re-run the tool expecting a verdict.
Note the interaction with the shape above: giving a leak guard a `main` is the CURE there and
is what triggers this one.

*A fix in a `lib/*.loft` library.*  `falsify` swaps the BINARY and passes `--path <worktree>/`
for the stdlib, but `use <name>` resolves to the repo-relative `lib/<name>.loft` — strace shows
the process opening that literal path — so the control run loads the CURRENT, fixed library and
both sides agree.  `--lib` does not redirect it either: a deliberately corrupted copy behind
`--lib` changed nothing.  Verify by hand instead — restore the library from the ref in place,
run the guard, restore it byte-identically — and record THAT in `@falsified-at` rather than the
line the tool prints.  `the-lexer-decodes-an-escape-once.loft` is the worked example.

⚠ **A third shape scores, and its green still covers only ONE defect: a control that ABORTS
before the program runs.**  An internal compiler error, a parse refusal, a panic in codegen —
any of these stops the control at the exit channel, so `falsify` reports a clean `falsified`
while `asserts` reads `0|0` because no assertion in the file ever executed.  That verdict is
true of the abort and says nothing about any cell the abort was standing in front of.

The tell is the same as the no-`main` shape — **zero assertion failures on the CONTROL** —
but here the cause is the opposite: not that nothing ran, but that something stopped
everything.  Read the control's EXIT column to tell them apart.

It matters because a crash is the loudest defect in a file and usually not the only one.
loft#1310's control aborted with an internal compiler error; fixing that alone left four
cells compiling, running, and yielding NOTHING at exit 0 — two further silent-wrong defects
the abort had been masking, neither of which `falsify` could see before or after.  So after
fixing a compile-time abort, re-run the whole matrix on VALUES rather than trusting the
verdict, and write into `@falsified-at:` which channel the tool actually scored and which
defects it is blind to.

**Why a record and not just a habit.** Four distinct channels reported success while
measuring nothing in a single afternoon (QUALITY.md § B6m), and two defects passed a full
green gate the same day:

| how a check reported success while measuring nothing |
|---|
| the wrong ENTRY POINT — `run_test` runs `main` when the file HAS one and every zero-parameter function otherwise, so `--interpret` on a `main`-less guard runs no assertion, and `--tests` on a `main`-ful one runs the HELPERS |
| a success marker the ERROR REPORT echoes — loft prints the offending source line, so grepping stdout for the literal the program prints on success scores every failure as a pass |
| a MONOTONE gate — the leak channel cannot score an over-free, because freeing more than you should always reads as an improvement |
| a cell that never reaches its subject — a non-null return never reaches the join bind the cell was written for, so it passed on a control |

The tool derives the entry point from the FILE rather than taking it as an argument,
because picking it wrongly is the first of those and the easiest to repeat.

⚠ **An on/off comparison inherits the blindness of the entry point both sides share.**
Comparing a compiler arm's effect under `--tests` on a `main`-ful guard answered
"4 passed / 4 passed" — which reads as *this changes nothing* and means *neither side ran
the thing that changes*.

**The control builds are cached, and the cache prunes itself.**  Each ref costs about 2 GB
under `~/.cache/tmp/loft-falsify/<ref>` and `<ref>-target` (the native leg links
`libloft.rlib` and its dependency rlibs, so the binary alone is not enough), beside a
`head-target` and a `shared-target`.  Kept forever, they held 364 GB on 2026-09-05 and filled
the root filesystem, and `make ci` failed in the NATIVE corpus with `FAIL unknown-mode` after
four `loft: low space in /var/tmp/loft-test-scratch-… — reclaimed … MB` lines — a disk-full
symptom that reads like a code fault.  The script now keeps the `LOFT_FALSIFY_KEEP` (default 4)
most recently used controls and removes the rest with their worktrees.  A gate that fails on an
unrelated suite right after "low space" lines is the disk: `df -h /`, then § Scratch hygiene.

### Scratch hygiene — what loft writes to a temp directory, and what removes it

Every native compile, html export and test probe lands in the temp directory (`TMPDIR`; under
`make ci` the per-checkout `/var/tmp/loft-test-scratch-<id>`), and each family has one rule
that removes it.  Measured 2026-09-05 before the rules existed: 434 GB under one `~/.cache/tmp`.

| what | who writes it | what removes it |
|---|---|---|
| `loft_native_bin_<pid>`, `loft_native_<pid>.rs` | a `--native` run's compile | the run itself when it ends normally; a run killed from OUTSIDE (a `timeout` wrapper, a harness kill, Ctrl-C) cannot, so **every native compile first sweeps the artefacts of dead processes** (`platform::reclaim_dead_native_scratch`, silent).  Sixteen thousand of them, 151 GB, had accumulated with nothing looking |
| `loft_test_native_<stem>_bin` / `.key` / `.rs` | `--tests --native`, a per-file binary cache keyed by stem | the low-space reclaim (aged entries) and `sweep_scratch.sh --days` |
| `<dir>/.loft/cache/<entry>` | the program cache a test writes beside its probe — every probe has a fresh name, so the cache only grows (13 GB in one test's dir) | `sweep_scratch.sh` (entries older than a day) |
| `loft_html_*`, `loft_p*`, `loft_rebuild_*`, `loft-*` | the html, probe, rebuild and serve suites | `sweep_scratch.sh` (older than a day) |
| `~/.cache/tmp/loft-falsify/<ref>{,-target}` | `make falsify` control builds | the script itself, LRU to `LOFT_FALSIFY_KEEP` |
| `~/.cache/tmp/claude-<uid>/<project>/<session>` | the agent harness's per-session scratch (170 GB, 284 sessions) | `make sweep-scratch` (older than two weeks) |
| `target/debug/deps` | cargo: every test binary of every dependency hash ever built (76–110 GB per checkout) | `make sweep-target` (`cargo sweep --time 14`) |

`make ci` runs `scripts/sweep_scratch.sh` on its own scratch at the start of every gate (it
used to keep seven days); `make sweep-scratch` runs it on the checkout's scratch and on
`TMPDIR` with the session prune, and prints `df` after.  Both touch only loft's own names,
only dead pids or aged entries, and never a sibling checkout's gate scratch.  A run of `df -h /`
before a gate is cheaper than reading a `FAIL unknown-mode` as a code fault.

### The set a suite RUNS is not the set it CONTAINS (`LOFT_TRACE_ASSERTS`)

The third shape of self-satisfaction, and the quietest: an `assert` that is written,
compiled, and **never executed**.  Nothing reports it.  A skipped file prints a `skip`
line and passes; a function nobody calls costs a compile and no more; a branch never
taken looks exactly like one that held.  Measured in 2026-08: **81 of `tests/scripts`'
9 803 assert sites had never run**, across three mechanisms nobody had reason to suspect
(QUALITY.md § *81 assertions the corpus contained and never ran*).

`LOFT_TRACE_ASSERTS=<path>` appends `file:line` for every `assert` that EXECUTES.  The
hook is in `n_assert`, which is both the interpreter's implementation and the one a
`--native` binary links, so one setting covers both backends and every process a suite
spawns (it appends, never truncates).  Diff the trace against the `assert(` sites in the
source and the silent ones are named:

```sh
LOFT_TRACE_ASSERTS=/tmp/ran.txt cargo test --release --test wrap loft_suite
# then: every `assert(` line in tests/scripts that has no `file:line` in /tmp/ran.txt
```

It reads the position the COMPILER injected into the call, so it doubles as a check on
that: a whole file tracing at a constant offset from its own source means the injected
line is wrong, which is how loft#625's mechanism was found at a second site (a failing
assert printed **another** assert's source under its caret).

Two things make an assertion unreachable often enough to be gated, both in `tests/wrap.rs`
and both proven able to fire:

* **`a_refusal_file_carries_no_runtime_assertions`** — a firing `@EXPECT_ERROR:` stops the
  whole file: `run_test` returns at *"ok (errors consumed)"* and `native_scripts` skips
  it.  So a file asserts a refusal **or** it runs; the corpus convention for both is a
  companion file (`102`/`102b`, `36`/`36b`, `1067`/`1067b`).  An `assert` inside an
  `@EXPECT_ERROR` function is fine — it is the use that makes the refused expression
  matter.
* **`every_assertion_is_reachable_from_the_entry_point`** — when a file has a `main`, both
  runners execute ONLY `main`; every other zero-parameter function is dropped.  Call it
  from `main` or delete it.

Both are UNDER-approximations on purpose: neither can see a branch never taken, and both
allow the deliberate dual guard (`432b`, `751`), where an `@EXPECT_ERROR`-annotated `main`
carries assertions that run only if the refusal ever regresses.  `LOFT_TRACE_ASSERTS` is
how the rest gets re-measured — a report, not a gate, because a gate over 9 800 sites
would need a line-numbered allow-list and would rot.

⚠ **A skip is a decision that can take a second thing with it.**  `native_scripts` decided
to skip on `src.contains("@EXPECT_ERROR")` over the whole file, so five scripts — 79
assertions, `93-vector-advanced.loft`'s 49 among them — left the native suite because a
comment in each *mentioned* the tag while recording that the file had STOPPED being a
refusal case.  Both runners now read one `common::expect_tag`.  When two harnesses ask
the same question, they get one implementation of it.

---

## Database backends: sqlite gates CI, all four are the local bar

loft binds four SQL backends through `#c` — **sqlite, PostgreSQL, MariaDB and
duckdb** — behind one `SqlDb` interface, and the property worth testing is that a
generic routine gives the *same* answer on all four.  Only one of them can be a
gate.

**The rule:**

- **Every routine is CI-checked against sqlite.**  It needs no server and no
  install, so a skip there is never environmental — it means the library went
  missing or the availability question broke.  `tests/native.rs` asserts sqlite
  ran on Linux for exactly that reason.
- **All four are runnable LOCALLY, and that is the real bar.**  PostgreSQL and
  MariaDB need a live server, duckdb a 70 MB library no distribution ships;
  none of that belongs in CI.  Before landing anything that touches the SQL
  layer, run all four locally.

**CI cannot cover the other three, and the docs must not imply it does.**  A
green CI run means "sqlite agreed", never "the four agree".  The cross-backend
claim is a local measurement, and when it matters it should be re-run and the
result written into the plan or the commit message rather than assumed to have
held since last time.

**How a fixture selects a backend:** `LOFT_SQLDB_MODE=sqlite|postgres|maria|duckdb`
for `uniform.loft` and `round_trip.loft` (@PLN133's gate).  duckdb additionally
needs `LD_LIBRARY_PATH=$HOME/.local/lib` unless the library is on the loader
path.  A backend that is not reachable prints `SKIP` and is never counted as a
pass — which is the reason the mode is a variable rather than a compiled-in list.

**The general rule, for every probe that needs something outside the process** (a
database, a browser, a server): *reaching the subject is a PRECONDITION, not a
measurement.* A probe that could not reach it rendered nothing and asserted nothing, so a
FAILURE from it is a claim it never made — it must SKIP. Everything after the connection
keeps its failure codes: once the subject answers, a wrong result is a real result and
stays red.

`tools/html_render_check.mjs` had this one case too narrow and it cost two weeks of red:
a MISSING chrome exited 2 (skip), but a chrome that was installed and never answered its
debugging port fell into the generic catch and exited 3, which reads as "the page was
wrong". `tests/gl_text_bridge.rs` was red on `main` from 2026-08-06 on a probe that never
loaded a page.

⚠ **Widening a skip is a reduction in coverage, so prove BOTH directions before believing
it** — that the unreachable case now skips, AND that the reachable case still runs and
still asserts. A skip that swallows a genuine failure is worse than the red it replaced
(the sibling hazard is § "a gate that skips looks like a gate that passes").

### Running the other three

Both servers run as ordinary system services on a development box; the fixtures
find them through environment variables with working defaults:

| backend | how it is reached | default |
|---|---|---|
| sqlite | `libsqlite3.so.0`, no server | `:memory:` |
| PostgreSQL | `LOFT_PG_CONN` — set up with `scripts/setup-test-databases.sh --pg` | `dbname=loft_test_pg` |
| MariaDB | `LOFT_MY_CONN` — set up with `scripts/setup-test-databases.sh --maria` | `host=127.0.0.1 user=loft pass=loft db=loft_test_uni` |
| duckdb | `libduckdb.so` on `LD_LIBRARY_PATH` — install with `scripts/fetch-duckdb.sh` | declared `[c] optional-libs`, so absence is not an error |

### PostgreSQL and MariaDB: the setup, and where to look when it breaks

**`scripts/setup-test-databases.sh` creates both.**  It is idempotent, never
drops anything, and **checks before it escalates** — on a box that is already set
up it needs no `sudo` at all and acts as a verifier.  Run `--pg` or `--maria` for
one half.

| | PostgreSQL | MariaDB |
|---|---|---|
| measured against | 16.14 | 10.11.14 |
| service | ordinary system service, port 5432 | ordinary system service, port 3306 |
| database | `loft_test_pg` | `loft_test_uni` |
| identity | **the OS user**, via unix-socket peer auth | `loft@localhost` / `loft@127.0.0.1`, password `loft` |
| override | `LOFT_PG_CONN` | `LOFT_MY_CONN` |

**The two servers authenticate differently, and that is not an accident of this
box.**  The fixture's PostgreSQL default is `dbname=loft_test_pg` — no user, no
host — so it is a peer connection as whoever runs the tests, and the role to
create is *that person*, not a shared `loft` role.  MariaDB is reached over TCP
with an explicit user and password, which is why one has a credential in the
fixture and the other does not.

**The MariaDB user is SCOPED on purpose**: `GRANT ALL ON \`loft\_test%\`.*` and
nothing else, so anything outside `loft_test*` answers `ERROR 1044`.  A suite that
can drop a developer's other schemas is one bad `DROP` away from a very bad day.
The setup script **verifies** this by trying to create a database outside the
pattern and expecting refusal — the GRANT text saying the right thing is not the
same as the server enforcing it.

The password is `loft`, in the clear, in `uniform.loft`.  That is deliberate and
safe **only** because of the scoping above: it is a local test credential for a
user that can reach nothing but `loft_test*`.  Do not reuse the pattern for a
user with wider rights.

**Symptom → where to look:**

| what you see | where the fault is |
|---|---|
| `SKIP postgres …` / `SKIP maria …` | the server is down, or the database/role is missing.  Run the setup script — it will tell you which. |
| `@PLN23 backends exercised:` missing one | same thing from the driver.  **Read this line**; green with three backends absent looks exactly like green with four passing. |
| `ERROR 1044` from MariaDB | the scope working as designed.  The test wanted a schema outside `loft_test*` — fix the test, do not widen the grant. |
| `peer authentication failed` on PostgreSQL | the OS user has no role.  `sudo -u postgres createuser --createdb $(id -un)`. |
| PostgreSQL passes but floats differ | check `extra_float_digits` — it must be ≥ 1, and it defaulted to 0 before PG12 (@PLN133 P3). |
| both servers fine, results differ between them | a real finding: the `SqlDb` contract is what makes the four interchangeable.  Compare the whole line. |

**CI has neither server** and is not expected to.  Nothing here is reproduced by
a green CI run.

### duckdb: where it comes from, and where to look when it breaks

**`scripts/fetch-duckdb.sh` installs it into `~/.local/lib`.**  It downloads a
PINNED upstream release, verifies a recorded `sha256` of the extracted
`libduckdb.so`, and refuses to install anything else.  Nothing else fetches it —
not CI, not `loft install`, not the test suite.

It is **not** vendored, for two reasons worth keeping straight: duckdb is MIT
licensed so redistribution would be legal, but the library is ~70 MB and git
history is permanent, and upstream already publishes exactly this artifact.  It
does not live in `~/.loft/lib` either — that holds loft *packages*, not native
shared libraries.

**The dependency chain, in the order a failure travels it:**

```
scripts/fetch-duckdb.sh   pins VERSION + EXPECT_SHA, writes ~/.local/lib/libduckdb.so
        ↓
LD_LIBRARY_PATH           the only thing that makes it findable — no rpath, no ldconfig
        ↓
[c] optional-libs         tests/fixtures/sqldb/duckdb/loft.toml declares "libduckdb.so"
        ↓
c_call::resolve           dlopens it on the first miss (@PLN24 arc G)
        ↓
src/shim.c                loft compiles this with `cc` at parse time — it names NO
                          duckdb symbol, so it builds even where the library is absent
        ↓
LOFT_SQLDB_MODE=duckdb    selects the backend in uniform.loft
```

**Symptom → where to look:**

| what you see | where the fault is |
|---|---|
| `SKIP duckdb …`, everything else green | the library was not found — `LD_LIBRARY_PATH` unset, or `~/.local/lib/libduckdb.so` gone.  Re-run `scripts/fetch-duckdb.sh`. |
| `@PLN23 backends exercised: [...]` without `duckdb` | the same thing, seen from the driver.  **Read this line** — a green run with duckdb absent looks identical to one with duckdb passing. |
| `sha256 mismatch … NOT installing` | upstream re-cut the release, or the pin is stale.  Decide which, then edit `EXPECT_SHA` **on purpose** — never to make the message go away. |
| `the archive did not contain libduckdb.so` | upstream changed the zip layout.  The script prints the listing; fix the extraction, do not work around it by hand. |
| a `cc` failure mentioning `shim.c` | not a duckdb problem at all — the shim is deliberately free of duckdb symbols so it compiles without the library.  Look at the C toolchain. |
| duckdb answers but DIFFERS from the other three | a real finding: the `SqlDb` contract is what makes the four interchangeable.  Compare the whole line, not one field. |

**One diagnosed caveat, so it is not rediscovered** (@PLN133 P3): a float written
through this fixture round-trips exactly on PostgreSQL and MariaDB and fails 19
times in 500 on duckdb.  Fifteen of those are a **duckdb parser bug** —
a decimal literal whose digit run is **275–294 characters** is read as a value
exactly **10^256 too small**, silently, while the same value in *exponent
notation* is correct.  The other four are ordinary 1–2 ULP parser rounding.

loft walks into it because **`"{v}"` renders a float as a full decimal expansion
with no exponent**, so any float above ~1e274 becomes a 275+ character literal.
**Write floats to SQL quoted (`CAST('{v}' AS DOUBLE)`), in exponent notation, or
bound — never as a bare `"{v}"`.**  Quoting is measured at 0/500 on duckdb and
needs no change to loft's rendering; it does NOT help sqlite, whose own
text→REAL converter loses the same 1 in 2000 either way.

**A copy in a scratchpad or a build directory is not durable.**  When it
evaporates the duckdb cell silently drops back to `SKIP` and the local
four-backend bar quietly becomes a three-backend one — the same class of
invisible coverage loss as a self-skipping test.

`LOFT_SQLDB_MODE` picks the backend for `tests/fixtures/sqldb/uniform.loft`.  A
backend that cannot be reached prints `SKIP` on stdout and the driver in
`tests/native.rs` **recognises a skip as a skip** — it is never counted as a
pass, and the set that actually ran is printed (`@PLN23 backends exercised: […]`).
Read that line: it is the only thing distinguishing "four agreed" from "sqlite
agreed and three were absent".

**The test databases are scoped on purpose.**  The MariaDB `loft` user reaches
only `loft_test*`; anything else answers `ERROR 1044`.  A test suite that can
drop a developer's other schemas is a bug waiting for a bad `DROP`.

**A measured caveat worth carrying into any float work here** (@PLN133 P3): the
four engines do not render a `double` to text the same way, and the obvious
spelling loses precision on two of them — `CAST(v AS TEXT)` on sqlite is inexact
for 94% of random doubles.  Do not assume a value survives a text round trip;
measure it per backend, with a sweep rather than a handful of hand-picked values.

---

## Generated Test Files (`tests/generated/`)

Generated files are written only in **debug builds** (`#[cfg(debug_assertions)]`). They are produced inside `Test::generate_code`, called from `Drop::drop`.

### `tests/generated/default.rs`

Written on every test execution (overwritten each time). Contains the compiled Rust representation of the default library only — everything up to `start` (the definition count before the test's own loft code was parsed). This file has no `#[test]` function; it serves as a reference snapshot of the default-library schema.

### `tests/generated/<file>_<name>.rs`

Written only when a test has a non-null `.result` or a non-unknown `.tp` (i.e., tests that validate output). The file name is `<file>_<name>.rs` where `<file>` is the Rust module name and `<name>` is the test function name.

For example, the test:
```rust
// in tests/enums.rs
#[test]
fn define_enum() {
    code!("enum Code { ... }")
        .expr("...")
        .result(Value::str("..."));
}
```
produces `tests/generated/enums_define_enum.rs`.

### Structure of a generated file

```rust
#![allow(unused_imports)]
#![allow(unused_parens)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unused_mut)]
#![allow(clippy::unnecessary_to_owned)]
#![allow(clippy::double_parens)]

extern crate loft;
use loft::database::Stores;
use loft::keys::{DbRef, Str, Key, Content};
use loft::external;
use loft::external::*;
use loft::vector;

fn init(db: &mut Stores) {
    // Registers all types from the default library + the test's own types.
    // Enumerations via db.enumerate / db.value.
    // Structs via db.structure / db.field.
    // Ends with db.finish().
    ...
}

fn n_test(stores: &mut Stores) { ... }  // generated Rust translation of the test's loft code

// Additional generated functions for each loft function defined in the test.

#[test]
fn code_<name>() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
```

The `init` function reconstructs the full type schema — both default-library types and any types added by the test — so each generated file is a fully self-contained Rust integration test.

---

## Additional Output Files

### `tests/dumps/<file>_<name>.txt` (debug builds only)

Written by `Test::output_code`. The content is controlled by a `LogConfig` value
selected at test time (see [LogConfig — Debug Logging Framework](#logconfig--debug-logging-framework) below).

Default content (preset `full`):

- The raw loft source code for the test.
- All type definitions introduced by the test (types beyond those in the default library).
- IR (intermediate representation) for each non-default function.
- Bytecode disassembly with slot annotations (`var=name[slot]:type`).
- The execution trace with variable-name annotations on stack-access steps.
- **Inline struct/vector dumps** on every opcode that produces or consumes a `DbRef`.

Set the `LOFT_LOG` environment variable before running tests to select a different preset.

#### Inline struct/vector dump format

Every `DbRef` result in the execution trace is shown as a compact single-line dump:

```
  44:[44] VarRef(var[20]=__ref_1) -> #2.1 { x: 1.5 }[44]
 109:[56] VarRef(var[32]=l) -> #3.1 { name: "diagonal", start: #2.1 { x: 1.5, y: 2.5 }, end_p: #3.1 { } }[56]
 161:[44] VarRef(var[32]=l) -> #3.1 { name: "diagonal", start: #3.1 { x: 1.5, y: 2.5 }, end_p: #3.1 { x: 10, y: 20 } }[44]
```

- `#store.record` prefix identifies which allocation each record lives in
- Null fields are suppressed; freshly-allocated structs show only set fields
- Nested structs expand to depth 2 by default (`{...}` beyond that)
- Vectors show up to 8 elements by default (`...N more` beyond that)

Adjust with environment variables (no recompile needed):
```bash
LOFT_DUMP_DEPTH=3 LOFT_DUMP_ELEMENTS=4 cargo test -- my_test
```

These files are useful for debugging compiler output and are not committed.

---

## LogConfig — Debug Logging Framework

`src/log_config.rs` provides structured control over what appears in the
`tests/dumps/*.txt` files and in the interpreter's execution trace.

### Selecting a preset at test time

Set the `LOFT_LOG` environment variable before `cargo test`:

```bash
LOFT_LOG=minimal   cargo test --test expressions expr_add   # execution only
LOFT_LOG=static    cargo test --test objects                 # IR + bytecode, no execution
LOFT_LOG=ref_debug cargo test --test objects reference       # snapshots on Ref ops
LOFT_LOG=bridging  cargo test --test expressions             # bridging invariant warnings
LOFT_LOG=crash_tail:20 cargo test --test vectors             # last 20 execution lines
LOFT_LOG=fn:helper cargo test --test expressions             # one function only
LOFT_LOG=variables cargo test --test slot_assign             # variable table per function
```

| `LOFT_LOG` value | Preset | Description |
|---|---|---|
| `full` *(default)* | `LogConfig::full()` | IR + bytecode + execution, slot annotations |
| `static` | `LogConfig::static_only()` | IR + bytecode; no execution trace |
| `minimal` | `LogConfig::minimal()` | Execution for `test` only; no IR/bytecode |
| `ref_debug` | `LogConfig::ref_debug()` | Full + stack snapshots on Ref/CreateStack ops |
| `bridging` | `LogConfig::bridging()` | Execution + bridging-invariant check |
| `crash_tail` or `crash_tail:N` | `LogConfig::crash_tail(N)` | Last N execution lines; flushed on panic |
| `fn:<name>` | `LogConfig::function(name)` | Only the named function |
| `variables` | `LogConfig::variables()` | IR + bytecode + variable table per function (no execution) |
| `all_fns` | `LogConfig::all_fns()` | Bytecode of **all** functions including `default/` built-ins; large but essential for diagnosing crashes whose opcode address falls inside a built-in |

The `variables` preset appends a table after each function's bytecode showing every variable's
name, short type, scope number, stack-slot range `[start, end)`, and live interval `[first_def, last_use]`.
Arguments are marked with `arg`.  Variables that have no slot yet (`stack_pos == u16::MAX`) or
that were never defined still appear so the full picture is visible.  Example:

```
variables for myfile:fn n_find_max(nodes:vector<ref(Node)>) -> integer
  #    arg  name                 type           scope  slot         live
  ----------------------------------------------------------------------
  0    arg  nodes                vec<ref(382)>  0      [0, 12)      -
  1         best                 int            1      [16, 20)     [6, 32]
  2         _vector_1            vec<ref(382)>  2      [20, 32)     [8, 15]
  3         n#index              int            2      [32, 36)     [10, 17]
  4         n                    ref(382)       3      [36, 48)     [19, 28]
```

### `LogConfig` struct

```rust
pub struct LogConfig {
    /// Which phases to include in the output.
    pub phases: LogPhase,           // { ir: bool, bytecode: bool, execution: bool }

    /// Only log IR/bytecode/execution for functions whose name contains one
    /// of these strings.  None = all functions.
    pub show_functions: Option<Vec<String>>,

    /// Only include execution steps whose opcode name (without Op prefix)
    /// contains one of these strings.  None = all opcodes.
    pub trace_opcodes: Option<Vec<String>>,

    /// Keep only the last N lines of the execution trace.  On panic the
    /// buffer is flushed before re-raising.  None = unlimited.
    pub trace_tail: Option<usize>,

    /// Append var=name[slot]:type to bytecode and =varname to execution steps.
    pub annotate_slots: bool,

    /// Capture a stack snapshot after every opcode whose name contains one
    /// of these strings.  None = never snapshot.
    pub snapshot_opcodes: Option<Vec<String>>,

    /// Number of bytes to print per snapshot.
    pub snapshot_window: usize,

    /// Warn when runtime stack_pos deviates from compile-time expected value.
    pub check_bridging: bool,

    /// Print the variable table (name, type, scope, slot, live interval) after
    /// each function's bytecode.  Enabled by the `variables` preset.
    pub show_variables: bool,

    /// Include functions from the `default/` built-in library in the bytecode
    /// dump.  Enabled by `LOFT_LOG=all_fns`; essential for diagnosing crashes
    /// whose opcode address falls inside a built-in.
    pub show_all_functions: bool,

    /// Dump live variables after every traced opcode.  Replaces the
    /// `LOFT_DUMP_VARS` env-var check (which was unsafe in parallel tests).
    pub dump_vars: bool,
}
```

### Building a custom config

```rust
use loft::log_config::{LogConfig, LogPhase};

let config = LogConfig {
    phases: LogPhase::execution_only(),
    trace_opcodes: Some(vec!["Call".to_string(), "Return".to_string()]),
    annotate_slots: true,
    ..LogConfig::full()
};
```

### Key implementation files

| File | Role |
|---|---|
| `src/log_config.rs` | `LogConfig`, `LogPhase`, `TailBuffer` definitions and presets |
| `src/compile.rs` | `show_code(writer, state, data, config)` — static IR + bytecode output |
| `src/state/debug.rs` | `execute_log(log, name, config, data)` — execution trace with all filters |
| `src/state/debug.rs` | `dump_code(f, d_nr, data, annotate_slots)` — per-function bytecode dump |
| `tests/testing.rs` | Creates config via `LogConfig::from_env()`, passes to `show_code` + `execute_log` |
| `tests/wrap.rs` | Same: `LogConfig::from_env()` for docs/scripts file tests |
| `tests/log_config.rs` | Unit tests covering all filters, presets, and pipeline integration |

### Notes for Claude

- `src/main.rs` re-declares `mod log_config;` because it re-includes all source modules
  directly rather than importing from the library crate.
- The bridging check (`check_bridging: true`) will always report a violation on the
  FIRST instruction of the root test function because `execute_log` places the sentinel
  return address at runtime position 4–7 while compile-time tracking starts at 0.
  This is a known harmless offset, not a real bug.
- `crash_tail` mode wraps the execution loop in `catch_unwind(AssertUnwindSafe(...))`;
  if a panic occurs the tail buffer is flushed to the log file before re-raising.

---

## What a test run selects (`find_problems.sh`)

`./scripts/find_problems.sh` defaults to the **curated** set: everything except a
short, named list of slow-and-few binaries. **3733 of 3833 tests in ~70s** rather
than ~370s.

The default is the cheap one on purpose. At six minutes a full run is something
you skip, and a check you skip is not a check — so the expensive option costs a
flag to ask for rather than a flag to avoid.

| | |
|---|---|
| `find_problems.sh` | curated — ~70s, 97.4% of the tests |
| `find_problems.sh --full` | every test — ~370s |
| `find_problems.sh --subject <name>` | one area — seconds |
| `find_problems.sh --list-subjects` | the subjects, and what the default excludes |

Selection flags combine with `--bg` / `--peek` / `--wait` / `--stop`.

### Why it curates by EXCLUSION

Because inclusion does not work, and there is a measurement rather than an
opinion behind that. An additive map ("you touched the parser, run these four
suites") keeps 4 binaries of 177 and drops 173 — and it demonstrably misses real
regressions: an over-broad change to the parser's `null()` was caught by
`binary_io_matrix`, which no parser map would have selected. Curating by
inclusion has to predict which suite will catch a bug, which is exactly the thing
nobody can do in advance.

Excluding instead makes the miss set small, named and auditable — and the cost
profile makes it nearly free, because the suite's cost is extremely concentrated:

```
top 5  binaries =  49% of test-seconds
top 10 binaries =  64%
top 20 binaries =  81%
```

**Eight binaries of 177 (4.5%) hold 57% of the work**, each slow AND few:
`deliver_wasm` alone is 1011s for 17 tests. Dropping exactly those eight is the
whole difference between 370s and 70s.

Nothing that skips can reach `main`: CI's `Test (ubuntu-latest)` runs the suite
**unsharded** and is a required check. The local default is the fast loop; CI is
the complete gate.

### Subjects

`parser scopes codegen runtime store wasm packages lsp sql docs host`, defined in
`scripts/test_subjects.sh` as binary-name **patterns** rather than lists — a list
is incomplete the day it is written (the first draft left 91 of 177 binaries
unreachable), while a pattern picks up new binaries whose names already match.
Patterns are expanded against the real binary list before being handed to
nextest, because nextest treats a pattern matching nothing as a filterset parse
error, and because an expanded selection can be read back and checked.

Subjects are a convenience for tight loops, **not** the safety mechanism. The
default being subtractive is what makes it safe to leave them approximate: a gap
in a subject costs seconds, never coverage.

## Test speed — a report, never a gate (`make speed`)

```bash
make speed             # what drifted, on the tests that carry a number
make speed-discover    # which tests are slow enough to deserve one
make speed-bless       # write the measured numbers back into the tests
```

Every slow test carries its expected cost **in itself**, above its `#[test]`:

```rust
// @speed 12.4
#[test]
fn a_slow_test() { … }
```

`scripts/test_speed.py` measures, compares, and **prints**. It exits 0 whatever
it finds. A time assertion fails for reasons the test is not about — a busy
machine, a different CPU, a change somewhere else in the suite — and a build
that goes red on those teaches everyone to widen the band until it means
nothing, so the one real regression arrives inside a band nobody trusts.
Correctness is what fails a build; speed is what you read.

Timeouts keep their job: they bound what we do **not** control — a socket, a
process spawn, `rustc`. They are a liveness bound, not a speed measurement, and
a test that takes its whole timeout tells you nothing about how fast it is.

### The unit, and why it is not seconds

`units = seconds × (CAL_REFERENCE_MS / this machine's calibration)`, where the
calibration is a fixed integer loop with nothing to do with loft. One unit is
about one second on the machine the constant was pinned on. The two obvious
alternatives are both wrong here:

* **Raw seconds** move with the machine and the load.
* **A share of the suite's total** moves when any OTHER test changes — make the
  hash faster and every unrelated test's share rises, so the report would accuse
  a dozen innocent tests of regressing every time something got faster. That is
  the exact failure this exists to avoid.

The reference constant only sets the scale; it cancels in a comparison, so a
slow box changes the absolute numbers and not the drift.

### Three things measured, each of which broke a naive version

Each is why the tool works the way it does, with the number that settled it:

1. **One run measures cache warmth.** Blessed from a single run and re-run
   immediately, **113 of 139 tests moved past ±25%, every one of them faster** —
   `multiplayer_v2::server_detects_and_retries_a_stolen_port` by 39x. Nothing had
   changed but the build cache and the page cache. Hence best-of-`--repeat`
   (default 2): cold caches and load only ever make a run *slower*, so the
   smallest observation is the least contaminated.
2. **Parallel wall-clock is mostly contention.** Warm, freshly blessed, and
   re-run, **48 of 134 still moved**, in both directions. nextest runs 24 tests at
   once and no serial calibration models that. Hence the measuring pass is
   `--test-threads=1` over the **annotated tests only** — affordable exactly
   because the report is about slow tests, a few dozen of them. `discover` is the
   separate wide parallel pass; it may be noisy, because it only answers "is this
   over a second", never "did it change".
3. **A machine that changes mid-run invalidates the scale.** One calibration is
   applied to the whole run, so the tool calibrates at both ends and says so when
   they disagree by more than 20%.

Residual noise is load, and the report names it rather than hiding it. Read a
single report as a hint; read the annotation's own history — `git log -p` on that
line — as the trend. A steady drift is a series of small diffs and a real
regression is one large one, both reviewable at the moment they land.

### What it does not measure

It calibrates CPU, so a test dominated by `rustc`, disk or the network normalises
poorly. And it is wall-clock: where a **deterministic counter** exists — claimed
records, allocations, bytes — prefer that and assert on it. A counter is
identical on every machine at any load, which is why
`data_structures::hash_growth_frees_the_table_it_replaces` can pin "2000 entries
claim 9 records" as an exact expectation while no timing could.

## Store-memory ceiling (`LOFT_MEMORY_LIMIT`)

The sibling of the execution timeout, for the failure it cannot catch. A corrupted
length does not always end in a bad dereference — often it ends in an **allocation**,
and a time bound is no help there: chasing loft#796 a single run reached 59.6 GiB in
seconds. The kernel's OOM killer is worse than useless as a diagnostic, because it
reports only that a process died and is free to kill a bystander instead of the
culprit (it took out two unrelated agent sessions).

So the ceiling lives inside loft, where the thing being allocated still has a name.
When a store growth would cross it the run stops **at that growth** — the store is
left exactly as it was — and prints what filled the heap:

```
loft: store memory limit reached — 1.7 GiB in use, limit 2.0 GiB

  the growth that crossed it
    a store of type `main_vector<Cell>` (kt=78) growing 1.7 GiB → 4.0 GiB
    the store was allocated at pc=8001

  where the memory is
    main_vector<Cell>        kt=78        1.7 GiB  in 1 store
    ?                        kt=65535     9.6 KiB  in 2 stores

  One type holding nearly all of it in ONE store is a runaway length;
  the same total spread over very many stores is a leak.
```

That last line is the whole point of the breakdown: **one store vs very many** is what
separates a runaway length from a leak, and it is the first thing you would otherwise
have to go and measure.

| Mechanism | Default | How |
|---|---|---|
| Ordinary run | **off** | loft is unbounded by default; a real program may want the machine |
| Under `--tests` / `loft test` | **on, 2 GiB** | a test wanting tens of GiB is a bug either way |
| Env var | — | `LOFT_MEMORY_LIMIT=<size>` — `2G`, `512M`, `64M`, or `0` to remove it |

A limit that cannot be parsed is reported and the default is **kept** — a typo must
never silently remove the ceiling.

Implementation is `src/store_budget.rs` (accounting, ceiling, report) with the
accounting asserted at every site in `src/store.rs` that allocates, reallocates or
frees a store buffer: `new`, `open`, `resize_store`, `shrink_to`, `clone_locked`,
`snapshot_copy` and `Drop`. Type names reach the report through
`Stores::publish_type_names`, called once per run and inert when no ceiling is set.
Guarded by `tests/store_memory_limit.rs`.

**No `file:line` in the report, deliberately.** The interpreter's span table records
CALL SITES only, so resolving an arbitrary allocation pc through it returns the
nearest span *below* — routinely in an unrelated function. A diagnostic that sends
the reader to the wrong file costs more than one that stays quiet, so the report
prints the pc and stops there; `LOFT_STORES=timeline` resolves the same pc against the
denser per-run table.

## Hang guard (`LOFT_MAX_OPS`)

> **Reach for `perf` FIRST when the hang is in a build you already have.**  This guard needs a
> rebuild with debug-assertions flipped on (below), which is minutes; a running process can be
> sampled in seconds and needs nothing:
>
> ```bash
> <the hanging command> & BGPID=$!
> sleep 6 && perf record -F 199 -g -p $BGPID -o /tmp/hang.data -- sleep 6
> kill -9 $BGPID; perf report -i /tmp/hang.data --stdio --no-children | head -15
> ```
>
> `gdb -p` is the reflex and it gave NOTHING here (ptrace is restricted on this box), so `perf`
> is the one to try first.  What it buys over a timeout is the same thing `LOFT_MAX_OPS` buys —
> the loop, not just the fact — and it works on a release build.
>
> ⚠ **And read the answer as a LOCATION, not a cause.**  A hang whose samples are all in
> `State::execute_argv` is an interpreter loop that is not terminating; it does not follow that
> the DEFECT is in the interpreter.  Measured (D-bind-26): a compile-time analysis decided to
> materialise the temp a `for` loop iterates, so the loop walked a copy while its body emptied
> the original — the whole fault was in `scopes.rs`, with no compile-time symptom at all, and
> the corpus reported only two 300s timeouts.  When the samples name a runtime loop, ask what
> COMPILED that loop differently, and bisect the change set by building and timing.

The third sibling, and the only one that is **debug-assertions only**. The
interpreter counts executed operations and, on reaching the ceiling, panics with the
last sixteen ops — each resolved to `function+offset: OpName`. That trail is the
whole value of it: a timeout tells you a run did not finish, this tells you which
loop it did not finish in.

⚠ **"Debug-assertions only" does NOT mean "any debug build" — and by default that is
every build you have.**  `Cargo.toml` carries

```toml
[profile.dev.package.loft]
debug-assertions = false        # ~270x on the hot-path store guards
```

so the loft LIBRARY is compiled without them, and the guard is absent from the binary
`cargo build --bin loft` produces AND from the test binaries.  Measured 2026-08-25 three
ways: the panic message string is missing from `target/debug/loft` and from
`target/debug/deps/wrap-*`; and an infinite loop under `LOFT_MAX_OPS=100000` runs until
`LOFT_TIMEOUT` hard-kills it, where the same program on a build with the flag flipped
panics with *"ran 100000 operations without finishing"*.

To actually use it, flip that one line to `true` and rebuild — ideally into a separate
`--target-dir`, so the main target tree is not invalidated:

```bash
sed -i 's/^debug-assertions = false/debug-assertions = true/' Cargo.toml   # revert after
cargo build --bin loft --target-dir /tmp/loft-dbg
LOFT_MAX_OPS=100000 /tmp/loft-dbg/debug/loft --interpret --path . prog.loft
```

(`--path .` because the stdlib is found relative to the binary.)  The same applies to
every other `#[cfg(debug_assertions)]` item in `src/` — **93 of them**, including
`check_arg_ref_allocs` and `check_ref_leaks`.  The store LEAK check is unaffected because
it is not gated at all.

A count cannot tell a long run from a hung one, which is the trap loft#919 walked
into. The ceiling was 100M ops, two tests of the library suite legitimately execute
more than that, and the only signal the guard had for "this is long" was the wording
it uses for "this is hung" — so the debug-assertions gate read as *known red* for a
reason that was never about those tests, and a gate read that way stops being run.

| Mechanism | Default | How |
|---|---|---|
| Any interpreter run, debug assertions ON | **on, 4e9 ops** | roughly a minute of debug-assertion interpretation |
| Debug assertions OFF (every release build) | **off** | the counter does not exist |
| Env var | — | `LOFT_MAX_OPS=<count>`, or `0` to remove it |

The default clears the project's own suite with room to spare — set it *low*
(`LOFT_MAX_OPS=50000000`) when you are hunting a hang and want the trail sooner. An
unparseable value is reported and the default is **kept**, the same rule
`LOFT_MEMORY_LIMIT` follows.

Implementation: `crate::keys::max_ops` (one cached env read) read once outside the
dispatch loop in `src/state/mod.rs`; the trail is the `trail_pos` / `trail_op` ring
beside it.

## Execution timeout (`LOFT_TIMEOUT` / `--timeout`)

Guards against hangs that would wedge `cargo test` or `find_problems.sh`.  The
deadline mechanism is layered:

- **Cooperative diagnostic check** fires at `T` (the requested deadline) — runs at
  every loft "checkpoint" (loft fn-entry on both backends, lexer recovery loop).
  Raises a typed `Timeout`, dumps the rich call stack (interpreter: `crash_tail` +
  `StackFrame`; native: `CALL_STACK` thread-local), exits cleanly with `124`.
- **Watchdog hard-kill** fires at `T + grace` — runs on a background thread and
  calls `std::process::abort()`, guaranteeing termination even when execution is
  stuck in arbitrary Rust / native / blocking syscall.  Prints a shared breadcrumb
  so the kill is still informative:

  ```
  [timeout] hard-kill after 300s+2s grace: phase=run-interpret fn=helper952 \
            file=tests/14-workflow.loft:31 entry=test_the_stuck_one
  ```

  `fn`/`file` name the loft function the run was in; `entry` names the entry point it
  was reached from, which under `--tests` is the TEST — the field that says which of
  the swept-in files is responsible.  **loft#952:** the interpreter used to checkpoint
  ONCE, with the literal `"<entry>"`, so every interpreted hang reported
  `fn=<entry> file=?:0` and the culprit had to be recovered by grepping raw output.
  It now carries the breadcrumb per call, as `--native` always did.

Configuration:

| Mechanism | Default | How |
|---|---|---|
| Explicit CLI flag | (off) | `loft --timeout <secs> <program.loft>` |
| Env var | (off) | `LOFT_TIMEOUT=<secs> loft <program.loft>` |
| Auto-arming under `--tests` / `loft test` | **on, 300s** | `loft --tests <file>` arms 300s unless explicitly overridden |

⚠ **`--tests` takes its path as the next NON-FLAG argument, and it only steps over
`--native`, `--no-warnings` and `--deny-warnings` on the way.** `--interpret` is not
in that list, so `loft --tests --interpret <file>` does not run `<file>` — the path
stays at its default `.` and the WHOLE TREE runs, `target/` included, for many
minutes. Put the backend flag first (`loft --interpret --tests <file>`) or leave it
off. The tell is a single-file run that does not finish; the parse site is the
`--tests` arm in `src/main.rs`.

Implementation lives in `src/timeout.rs` (watchdog thread, deadline atomics,
breadcrumb store) + checkpoint calls scattered through `src/state/mod.rs`
(interpreter dispatch), `src/codegen_runtime.rs` (`cr_check_deadline()` injected
at native fn-entry / loop back-edges), and `src/lexer.rs` (parse-time recovery
loop).

A hung `--interpret` program exits **124/SIGABRT** at `T+grace` reporting
`phase=run-interpret`; a hung *compile* reports `phase=parse`.  Subprocess
isolation for the in-process cargo harness (`tests/wrap.rs::script_suite`) is
NOT yet wired — a hung script there still wedges the whole `cargo test` run.
T4 harness subprocess isolation shipped via `.config/nextest.toml`'s
`slow-timeout` (300s default / 600s ci, `terminate-after = 1`) —
nextest escalates SIGTERM → SIGKILL on a hung test process,
localizing the hang to ONE test binary; other test binaries
continue.  Full closure record at
[`plans/finished/49-execution-timeout/`](plans/finished/49-execution-timeout).

---

## `tests/wrap.rs` — shared runner for docs and scripts tests

`run_test(path, debug)` is the core of every test in `tests/wrap.rs`:

1. Creates a `Parser`, loads the default library, parses the given `.loft` file.
2. Checks diagnostics against `// #warn`, `@EXPECT_ERROR`, and `@EXPECT_WARNING`
   annotations.  Unexpected errors fail the test; unexpected warnings are logged
   but tolerated.
3. If the file has `@EXPECT_ERROR` annotations, execution is skipped (the compiler
   can't produce valid bytecode for a file with intentional parse errors).  Since
   loft#1242 the RUST suite does better than skipping: it attributes each error to its
   enclosing function, blanks that cell and re-parses, so a refusing cell and a running
   cell share a file and both are checked.

   ⚠ **The CLI does not do that peel.** `loft --tests <file>` on a mixed file runs the
   refusal cells and SKIPS every running one, silently — a deliberately broken assertion
   in such a file still reports `ok`.  So a guard verified with the documented CLI command
   has had only half of itself checked.  Either verify a mixed guard through
   `cargo nextest --test wrap loft_suite`, or **keep the two kinds in separate files**,
   which is what the two `the-reference-par-…` guards do and why they are a pair.
4. Runs `scopes::check` and `byte_code` inside `catch_unwind`.  If the compiler
   panics and the file has `@EXPECT_FAIL` annotations, the panic is tolerated.
5. Discovers all zero-parameter user functions as entry points.  If `main` exists,
   only `main` is called.  Otherwise all `fn test_*()` functions run individually
   with `catch_unwind`.  Functions annotated `@EXPECT_FAIL` tolerate panics.

   **The CLI runner's rule is not this one** (loft#1010).  `loft --tests` / `loft test`
   run EVERY zero-parameter function — `main` included, alongside the tests, and a
   helper whose name has no `test_` prefix as well — and count each in the total.  A
   function that takes a parameter is the only thing skipped, so a parameter (even an
   unused one) is today's way to say "not a test".  Measured on both backends: a file
   with `fn setup()` and `fn test_one()` reports `(2 fns: setup, test_one)` and runs
   `setup`'s body.  57 files in this corpus have both a `test_*` and another
   zero-parameter function, 19 of them a `main`, so the two rules are not
   interchangeable and narrowing the CLI to `test_*` would silently drop what those
   `main`s assert — which is why loft#1010 is a decision rather than a patch.
6. In debug builds, writes a bytecode dump to `tests/dumps/<filename>.txt` first.
   If `debug = true`, also writes an execution trace using `execute_log`.

### Annotations supported by `wrap.rs`

| Annotation | Scope | Effect |
|---|---|---|
| `// #warn <text>` | File | Warning must appear; missing → fail |
| `// @EXPECT_ERROR: <text>` | Per-function or file header | Parse error containing `<text>` must appear; missing → fail |
| `// @EXPECT_WARNING: <text>` | Per-function or file header | Warning containing `<text>` must appear; missing → fail |
| `// @EXPECT_FAIL: <text>` | Per-function (before `fn`) or file header | Runtime panic is tolerated |

⚠ **Check `loft_suite` the way the GATE checks it, not by hand.** `cargo test --release
--test wrap loft_suite` reports failures that `make ci` does not — measured 2026-08-22:
`25-narrow-nullable-field-sentinel-collision` as *"was @EXPECT_FAIL, now compiles"*, and a
panic on `75-native-stub`'s expected-fail naming stale cdylibs. Neither is real. `make ci`
runs nextest under the **test profile**, and the same scripts pass there. It was confirmed
not to be a code range (identical output from binaries either side of it) and not stale
artefacts (`make check-rlib` and `make rebuild-native-cdylibs` changed nothing) — so
`@EXPECT_FAIL` behaves differently per cargo profile, and the gate only ever exercises one
of them. That is a real coverage hole and is not yet filed; what matters day to day is
that a by-hand `cargo test` on this suite can cost twenty minutes chasing two failures
that do not exist. Reach for `make ci`, or `./scripts/find_problems.sh`.

**Every expectation must match.**  `@EXPECT_ERROR` and `@EXPECT_WARNING` used to be
collected and then dropped, so an annotation whose diagnostic had been reworded, narrowed
or removed kept passing.  When that was measured, **56 of the 167 `@EXPECT_ERROR`
annotations in the tree were inert** (loft#929).  Both are now fatal, and the check runs
even when the file produced NO diagnostics at all — the other way an expectation went
unlooked-at.

The rule is per ANNOTATION, not per file, and that distinction is the whole of it.  An
annotation written above a `fn` binds to that function; only one written ahead of every
`fn`/`struct`/`enum` is file-level.  Both kinds are scored by the same predicate — the
declared substring must appear in some error the file produced — because a per-function
site that asked instead whether the file produced *any* error credited every annotation in
a file that produced one anywhere (loft#1261).

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
wording.  That, not message drift, was the cause of most of the 56.

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
same window `src/main.rs` uses — after `Parser::parse`, before `scopes::check`.  Until
loft#929 they ran only in the CLI, so this suite could neither confirm one of their
warnings nor catch a false positive from one: `894-lost-write-through-returned-struct.loft`
carried an `@EXPECT_WARNING` for a diagnostic the harness had no way to produce.

**Annotation placement rules** (same as `test_runner.rs`):
- An annotation directly before a `fn` line (no blank lines between) binds to that function.
- An annotation in the file header (before any `fn`/`struct`/`enum`) is file-level.
- A blank line between the annotation and the `fn` clears the pending annotation.

`LOFT_LOG` is respected: `LogConfig::from_env()` is called in `run_test` exactly as in `testing.rs`.

Named test entrypoints in `tests/wrap.rs`:

| Test name | What it runs | Notes |
|---|---|---|
| `dir` | All `tests/docs/*.loft` files + HTML doc regeneration | Skips files listed in `SUITE_SKIP` |
| `loft_suite` | All `tests/scripts/*.loft` files | Runs all entry points; skips files in `ignored_scripts()` |
| `integers` … `stress` | One `tests/scripts/` file each (16 tests) | See `script_test!` table below |
| `last` | `tests/docs/16-parser.loft` | — |
| `threading` | `tests/docs/19-threading.loft` | — |
| `logging` | `tests/docs/20-logging.loft` | — |
| `file_debug` | `tests/docs/13-file.loft` with execution trace | — |
| `parser_debug` | `tests/docs/16-parser.loft` with execution trace | `#[ignore]` — run with `cargo test -- parser_debug --ignored` |

Individual script tests (generated by `script_test!` macro):

| Test name | Script file |
|---|---|
| `integers` | `01-integers.loft` |
| `floats` | `02-floats.loft` |
| `text` | `03-text.loft` |
| `booleans` | `04-booleans.loft` |
| `control_flow` | `05-control-flow.loft` |
| `functions` | `06-functions.loft` |
| `enums` | `05-enums.loft` |
| `structs` | `06-structs.loft` |
| `control_flow` | `07-control-flow.loft` |
| `functions` | `08-functions.loft` |
| `lambdas` | `09-lambdas.loft` |
| `vectors` | `11-vectors.loft` |
| `collections` | `12-collections.loft` |
| `map_filter_reduce` | `13-map-filter-reduce.loft` |
| `formatting` | `14-formatting.loft` |
| `min_max_clamp` | `17-min-max-clamp.loft` |
| `math_functions` | `18-math-functions.loft` |
| `files` | `19-files.loft` |
| `binary` | `20-binary.loft` |
| `binary_ops` | `21-binary-ops.loft` |
| `script_threading` | `22-threading.loft` (named `script_threading` to avoid clash with `threading`) |
| `stress` | `37-stress.loft` |
| `single_type` | `52-single.loft` |
| `logging_script` | `53-logging.loft` |

Run any single script with `cargo test --test wrap <name>`, e.g.:
```bash
cargo test --test wrap files
cargo test --test wrap collections
```

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
  **Check an entry's blocker STATE before trusting it**: `input` sat in
  `LIB_PKGS_NATIVE_SKIP` for three months after both its blockers closed, `191-source-dir`
  named `#268` a quarter after it closed, `19-threading` was "the WASM threading model
  differs" while it ran green under wasmtime, and both `web/http.loft` entries named a file
  no suite walks — four inert entries that read as four open gaps.
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

Docs files, `00`–`38` — **36** of them.  The table below covers `00`–`22`; the rest are
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
  testing.rs              # Framework: Test struct, macros, Drop impl, generate_code
  expressions.rs          # Interpreter tests: type-check, labeled loops, null returns
  enums.rs                # Interpreter tests: complex enums, polymorphism, JSON
  strings.rs              # Interpreter tests: complex string ops, reference params
  objects.rs              # Interpreter tests: structs, :#format, mutable references
  vectors.rs              # Interpreter tests: complex vector / sorted / hash
  sizes.rs                # Interpreter tests: struct sizes / sizeof (complex layout)
  data_structures.rs      # Interpreter tests: combined data structures
  parse_errors.rs         # Interpreter tests: expected parser errors (diagnostic)
  immutability.rs         # Interpreter tests: immutability diagnostics
  threading.rs            # Interpreter tests: Rust-level parallel API
  tuple_matrix.rs         # Plan-14 validation matrix (cross-mode; ignored by default)
  template_matrix.rs      # Plan-17 validation matrix (cross-mode; ignored by default)
  expressions_auto_convert.rs  # Hand-written generated-style test (pre-generator)
  issues.rs               # Regression tests for known issues (see [PROBLEMS.md](PROBLEMS.md))
  wrap.rs                 # Runner for docs/ and scripts/; also generates HTML docs
  common/
    cross_mode.rs         # Harness used by *_matrix.rs binaries (interp ↔ native equivalence)
  docs/
    00-general.loft ... 38-call-it-yourself.loft  # User documentation loft programs (36 files)
    wordlist.txt                           # Edge-case string keys for 21-stress.loft
  generated/
    default.rs            # Default-library schema snapshot (no #[test])
    <file>_<name>.rs      # One file per result-bearing interpreter test
  dumps/
    <file>_<name>.txt     # Bytecode + trace dumps (debug, not committed)
  scripts/
    01-integers.loft ...  # Feature test loft programs (no HTML generation)
    wordlist.txt          # Edge-case string keys for 37-stress.loft
```

---

## Running the Tests

```bash
# Run all interpreter tests (generates tests/generated/ as a side effect):
cargo test

# Run a specific interpreter test file:
cargo test --test enums

# Run a specific test function:
cargo test --test enums define_enum

# Run only docs/scripts tests (wrap.rs):
cargo test --test wrap

# Full test cycle including generated tests (see Makefile):
make test
```

`make test` runs the `clippy` target first (which runs `cargo clippy`, `rustfmt`, and `cargo run --bin gendoc` to regenerate HTML docs), then:

1. Deletes all files in `tests/generated/` and `tests/result/`.
2. Runs `cargo test -- --nocapture --test-threads=1`, appending output to `result.txt`.

### Fast-iteration workflow — don't spam the full suite

When iterating on a specific test family (e.g. `p54_*`, `q3_*`,
`b7_*`) during development, use a **tight name prefix filter**
so cargo only builds + runs the tests you care about.
The full `cargo test --release --test issues` suite compiles to
~244 tests and takes ~30 s; a prefix-filtered run is ~1–2 s
plus the one-time compile.

```bash
# Good — 1-2 s, runs ~5 tests:
cargo test --release --test issues q3_to_json

# Good — runs exactly one test:
cargo test --release --test issues q3_to_json_of_jbool_true

# Bad — runs the full 244-test suite every time (~30 s),
# even though you only needed the 5 q3 tests:
cargo test --release --test issues
```

The filter is a **case-sensitive substring match** on the test
name, no separator required.  Don't add `-- q3_` (with the
`--` flag separator) — that works too but has more parsing
overhead.  A bare prefix is the Rust-standard pattern.

### Don't stack duplicate cargo invocations

If you invoke `cargo test` while a previous `cargo test` from
the same terminal is still running (or still live in the shell's
background), both invocations queue on the `target/` build lock.
Each cargo invocation also pays the 1-2 s startup cost.
Symptoms of stacked runs: test output is slow to appear; a
`ps aux | grep issues-` shows several copies of the test
binary running at >60 % CPU; the harness reports "has been
running for over 60 seconds" on a test that should finish
in milliseconds.

**Rule:** always let a `cargo test` run complete before
launching the next.  If a run hangs (suspicious of an infinite
loop in a new test), kill the *specific* test binary and
inspect:

```bash
# See what's running:
ps aux | grep -E "issues-|cargo test" | grep -v grep

# Force-kill all test binaries + cargo driver:
pkill -9 -f "issues-"; pkill -9 -f "cargo test"
```

Then re-run with a narrower filter to identify which test
is looping.  **Do not** add `--test-threads=1` to "serialise
the mess"; that masks the bug and makes finding the real
looper harder.

### Diagnosing a hang vs a failure

- **Hang** — test binary stays live at high CPU for more than
  its expected runtime.  Likely root cause: an infinite
  loop reading garbage memory (e.g. a String whose `len`
  field got written as a huge value), a format-specifier
  that doesn't terminate, or a recursive call with no base.
  Narrow to one test via `cargo test --release --test <file>
  <exact_name>` and run under `LOFT_LOG=full` to get the
  bytecode trace up to the hang point.
- **Failure** — test binary completes but output doesn't
  match expected.  Diagnostic output lives in
  `tests/dumps/<file>_<test>.txt` (under debug builds or
  when `LOFT_LOG` is set).  Check the end of the dump file
  for `FAILED` markers.  The test harness's `.result(…)`
  check runs after execution, so a failed test has the full
  bytecode trace available.

Hangs caused by escape-sequence parsing in `code!()` have
appeared twice now (e.g. `q3_to_json_of_jstring_with_escapes`
— the loft parser's handling of `\\` inside
Rust-double-escaped string literals fed through `code!()`).
When a test involving string escapes hangs, move the repro
to a standalone `.loft` file first (`/tmp/foo.loft`) to
isolate whether the bug is in the test plumbing or in loft
itself.

### One-pass-find-all-problems workflow

When a refactor touches code that many tests cover, the default
"run → see one panic → fix → re-run → see next panic" loop pays
the build + test-startup cost on every iteration.  A single
~60-second test pass already runs every test if the early
failures don't abort the whole binary — Rust test harnesses
default to "continue on failure" within a single test binary,
but `cargo test` itself stops after the FIRST test binary that
produces a non-zero exit status.

Use `--no-fail-fast` to keep going across all test binaries,
redirect to a file, and read the file once.  The checked-in
helper `scripts/find_problems.sh` wraps this:

```bash
./scripts/find_problems.sh
# → /tmp/loft_test.<id>.log   raw test output (per-checkout tag)
# → /tmp/loft_problems.txt    compact failure summary (stable copy)
```

**Peek while a run is in flight.**  `tee` writes the log live,
so you can inspect failures before the whole suite finishes:

```bash
./scripts/find_problems.sh --peek
# reads the per-checkout live log, prints any FAILED markers found so far
# plus their stdout blocks; shows the current tail if none yet
```

Optional: `./scripts/find_problems.sh <log-path> <problems-path>`
to write to other locations.  Equivalent inline:

```bash
cargo test --release --no-fail-fast 2>&1 | tee /tmp/loft_test.log
{
  grep -E "^test .* FAILED$" /tmp/loft_test.log
  echo
  grep -B1 -A6 "^---- " /tmp/loft_test.log
} > /tmp/loft_problems.txt
```

`/tmp/loft_problems.txt` has the structure:

```
test errors_accessor_path_on_failure ... FAILED
test q3_to_json_pretty_three_level_nesting ... FAILED
... (all failure headers)

---- errors_accessor_path_on_failure stdout ----
thread 'errors_accessor_path_on_failure' (3741234) panicked at src/native.rs:172:5:
expected #errors entries for bad input (errors_accessor_path_on_failure:5)
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- q3_to_json_pretty_three_level_nesting stdout ----
thread 'q3_to_json_pretty_three_level_nesting' (3741198) panicked at tests/testing.rs:437:13:
Test failed {"a":{"b":[1]}} != "{ ... " (q3_to_json_pretty_three_level_nesting:9)
... (one block per failed test)
```

Read this once, plan all the fixes, apply, re-run.  No
"two-pass-per-failure" cycle.

**Why this isn't the default:**

- `cargo test`'s default fail-fast is useful for fast feedback
  when iterating on ONE area — you see the first problem
  immediately without waiting for the rest.
- `--no-fail-fast` is what you want for a refactor sweep where
  you EXPECT multiple failures and want to plan the fix order.

**Patterns to apply:**

- For per-test diagnosis, the `tests/dumps/<file>_<test>.txt`
  file (debug builds or `LOFT_LOG=...` set) carries the full
  bytecode trace.  The problems file points you at WHICH
  tests failed; the dump file tells you WHY each one failed.
- After applying fixes, re-run the same command — diff the new
  problems file against the old to confirm your changes only
  closed problems (didn't introduce new ones).
- Stable problems are easier to track via the
  `ignored_tests.baseline` mechanism (see § Test Coverage Gaps)
  — that's the long-term home for "known-failing" tests; the
  problems file is for in-flight refactor work.

**Checked-in helper:** `scripts/find_problems.sh` (executable,
takes optional `$1` log path and `$2` problems-summary path).
Reach for the script instead of typing the inline pipeline —
future sessions will find it by `ls scripts/` or by greping for
"find_problems" in this doc.

**When NOT to use this workflow:**

- During focused development on ONE test family — use the
  prefix-filter form (`cargo test --release --test issues
  q3_to_json`).  Faster feedback, no problems-file overhead.
- When you've JUST landed a substantial change and want to
  confirm zero regressions — `cargo test --release` (default
  fail-fast) tells you faster if SOMETHING broke; only flip
  to `--no-fail-fast` once you know there are multiple failures
  to triage.

---

## Validating Generated Code — the `generated/` Workspace

Two directories:
- `tests/generated/` — ephemeral output from interpreter tests (158+ files, cleared by `make test`)
- `generated/tests/` — committed reviewed subset; standalone Cargo workspace with `loft = { path = ".." }`

| Target | Purpose |
|---|---|
| `make generate` | `meld tests/generated/ generated/tests/` — review and copy approved files into the committed corpus |
| `make gtest` | `cargo clippy --tests`, `rustfmt`, `cargo test` inside `generated/` — lint, format-check, and run all promoted tests |
| `make meld` | Compare `tests/generated/text.rs` and `fill.rs` against `src/` counterparts; open meld if they differ |

```
cargo test (debug)
  └─► tests/generated/*.rs   (158+ files, ephemeral)
        │
        ▼  make generate  (meld review)
        │
        ▼  generated/tests/*.rs  (committed, reviewed subset)
              │
              ▼  make gtest
                   clippy → rustfmt → cargo test  (inside generated/ workspace)
```

---

## Key Constraints

- **Generated tests are debug-only.** `generate_code` and `output_code` are guarded by `#[cfg(debug_assertions)]`. Release builds (`cargo test --release`) skip file generation entirely.
- **`default.rs` has no `#[test]` function** and is excluded from the second-pass Cargo registration.
- **`expressions_auto_convert.rs`** exists as a hand-written `tests/` file from before the generator existed; the corresponding generated file is skipped to avoid a Cargo name collision.
- **Test execution order within a file** is non-deterministic (Cargo runs tests in parallel by default). `make test` passes `--test-threads=1` to force sequential execution and capture output deterministically into `result.txt`.

---

## `tests/scripts/` — standalone loft test suite

**Purpose: the primary, long-term comprehensive test suite for the loft language.**
Every language feature and standard-library function should eventually have coverage here.
Each file is a self-contained loft program with a `fn main()` that asserts correct behaviour.
No HTML generation, no `@NAME`/`@TITLE` headers. Can be run directly through the `loft` binary
or via `cargo test --test wrap loft_suite`.

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
- The test expects a compile-time error or warning (all `parse_errors.rs`, `immutability.rs`,
  `format_strings.rs` diagnostics).
- The test calls Rust APIs directly (`threading.rs` low-level `run_parallel_int`/`run_parallel_raw`
  tests, `data_structures.rs`, `log_config.rs`, `expressions_auto_convert.rs`).
- The test exercises compiler internals that only surface via the Rust test framework
  (`slot_assign.rs`).

**Prefer `tests/scripts/` over `code!()` in `.rs` files.**  If a test can be written as
plain loft code with `assert()`, put it in the appropriate script file — do not wrap it in
`code!(r#"..."#)` inside a `.rs` file.  The `code!()` macro exists for cases that need Rust
assertions on compiler output, not as a convenience wrapper for loft code.  Script tests are
also validated by the native test runner (`cargo test --test native`), giving automatic
dual-mode coverage.

**When a `.rs` test and a script test cover the same behaviour**, the `.rs` test should be removed
— the script is the authoritative version.

**Naming a bug regression: use the GitHub issue number.**  A regression for a fixed bug is
`tests/scripts/<issue>-<slug>.loft` — e.g. `366-native-abib-scalar-literal-arg.loft` guards #366,
`368-nullable-struct-return-heap-param.loft` guards #368.  This makes the test greppable from the
issue and back: the issue's `fixed-pending-merge` comment names the file, the file's header cites
`#<issue>`.  Don't reuse a feature/era number for a bug (a fix for #366 filed under `304-…` is a
mis-file — rename it).  The original topic suites (`01-integers`, `05-enums`, …) keep their
sequential feature numbers; those predate the issue-number convention and are not renamed.

In `cargo test` mode, `run_test` writes a bytecode dump to `tests/dumps/` in debug builds.
No generated Rust code is produced.

```
tests/scripts/
  01-integers.loft         arithmetic, bitwise, null, type conversions
  02-floats.loft           float/single arithmetic, math functions, null (NaN)
  03-text.loft             concatenation, len, indexing, slicing, UTF-8, search, join
  04-booleans.loft         logical ops, short-circuit, null truthiness
  05-enums.loft            plain enums, struct-enum variants, polymorphic dispatch
  06-structs.loft          constructors, methods, virtual fields, JSON/format
  07-control-flow.loft     if/else, for loops, ranges, break, named break, loop metadata
  08-functions.loft        default args, reference params, early return, recursion
  09-lambdas.loft          lambda syntax, short |x| form, fn(x:T) form, type hints
  10-match.loft            match expressions, pattern binding
  11-vectors.loft          literals, append, slice, iteration, removal, #index/#first/#count
  12-collections.loft      sorted, index, hash — lookup, ordered iteration, range queries
  13-map-filter-reduce.loft  map, filter, reduce higher-order functions
  14-formatting.loft       format specifiers: integers, floats, booleans, text, single
  15-random.loft           rand, rand_seed, rand_indices — range, reproducibility
  16-time.loft             time-related operations
  17-min-max-clamp.loft    min, max, clamp for integer, single, float; null
  18-math-functions.loft   exp, ln, log2, log10 for single and float; null
  19-files.loft            text file I/O: lines(), move/delete, path safety
  20-binary.loft           binary file I/O: typed reads/writes, endianness
  21-binary-ops.loft       binary operations: seek, set_size, incomplete read
  22-threading.loft        parallel_for: all return types, context args, methods, text
  23-field-overlap-structs.loft  field-offset overlap across structs
  24-field-overlap-enum-struct.loft  field-offset overlap enum/struct
  25-sorted-enum-variant-range.loft  sorted collection with enum keys
  26-dead-assignment.loft  dead assignment detection
  27-format-specifiers.loft  extended format specifiers
  28-references.loft       reference parameter semantics
  29-strings.loft          complex string operations
  30-expressions.loft      expression edge cases
  31-text-param.loft       text parameter handling
  32-collections-regressions.loft  collection regression tests
  33-lambdas-fn-refs.loft  bare function references, fn-ref dispatch
  89-sizeof.loft           sizeof expressions and struct layout
  90-immutability.loft     immutability constraints
  91-null-coalescing.loft  null coalescing operator
  92-vector-loop-push.loft loop-variable push into vector
  93-vector-advanced.loft  vector regressions and advanced cases
  94-block-copy-opt.loft   block-copy optimisation
  95-alias-copy.loft       alias/copy semantics
  96-slot-assign.loft      slot assignment correctness
  97-native-vectors.loft   native-mode vector behaviour
  98-struct-order-in-use.loft  struct declaration order across `use`
  35-format-errors.loft    format string error handling
  36-parse-errors.loft     parse error recovery
  37-stress.loft           build-and-free cycles; reads wordlist.txt
  38-parse-warnings.loft   parse warning validation
  39-diagnostics-passing.loft  diagnostic edge cases that should pass
  wordlist.txt             edge-case string keys for stress tests
```

Run with:

```bash
cargo test --test wrap loft_suite   # run all tests/scripts/ files via the test framework
make loft-test                      # build loft (release) then run every file
./target/release/loft tests/scripts/06-structs.loft   # run one file
```

The `cargo test` path uses `run_test` from `tests/wrap.rs`, which:
- Fails on any compiler diagnostic (including warnings such as "Variable never read")
- Writes a bytecode dump to `tests/dumps/<filename>.txt` in debug builds
- Respects `LOFT_LOG` for the bytecode dump

Each file has a `fn main()` that calls `assert(condition, message)` for every case.
A failing assert panics and prints the message, naming the failed test.

### Known language quirks affecting test authoring

The following behaviours differ from what one might naively expect:

| Behaviour | Correct approach |
|---|---|
| `for _ in text_var` → "Variable never read" warning → test fails | Use a named variable, or restructure to avoid iterating text just for a count |
| ~~`for _ in enum_vector` → infinite loop~~ **FIXED** | `for x in v` now terminates correctly for `vector<PlainEnum>` |
| `empty = []` → "Indexing a non vector" compile error | Use a typed one-element vector then remove it: `t = [99]; for v in t { v#remove; }` |
| `"Purple" as Direction` returns `0`, not null sentinel `255` | Check format string: `"{bad}" == "null"` rather than `!bad` |
| `#index` in `for i in 10..14` returns the loop variable value (10–13), not 0-based count | Use `#count` for 0-based counting; `#index == loop_var` for integer ranges |
| Default struct integer fields are `0`, not null | Assert `== 0`, not `== null` |
| Same variable name in multiple sequential `{ }` blocks: `validate_slots` exempts same-name+same-slot pairs (Issue 28, fixed) | Both same-name and different-name sequential blocks now work |
| Two *differently-named* reference/vector/text variables in a long function that share a slot and have overlapping `first_def`/`last_use` intervals trigger a false `validate_slots` panic (Issue 29, unfixed) | Order the code so the second variable is introduced after the last use of the first; see `19-files.loft` (`lines()` test placed last) |
| `to_uppercase` / `to_lowercase` / `replace` return `Str` (16 bytes), not `String` (24 bytes) | Use `stores.scratch` pattern (see [INTERNALS.md](INTERNALS.md)) |
| ~~`for r in sorted if cond { r#remove; }` with large N gives silently wrong results~~ **FIXED 2026-03-14** (PROBLEMS #33 — no actual bug; test confirmed passing) | — |
| ~~`for r in index_var { r#remove; }` with large N panics "Unknown record"~~ **FIXED 2026-03-14** (PROBLEMS #35 — `fill_iter` loop_db_tp and `state::remove` both fixed) | — |
| ~~`for i in 0..N { idx[i, name] = null; }` leaves 1 record behind (large N)~~ **FIXED 2026-03-14** (PROBLEMS #34 — `tree::remove` now always updates root pointer even when last element removed) | — |

---

## Debugging failures in `tests/scripts/` {#debugging-failures-in-testsscripts}

### Strategy overview

When `make loft-test` reports a failure, work from the outside in:

1. **Run the failing file directly** — the panic message names the exact assert.
2. **Narrow to the failing assert** — comment out asserts below the first failure to isolate it.
3. **Print intermediate values** — add `print("{var}")` before the assert to see the actual value.
4. **Run via the Rust test framework** — convert the minimal case to `expr!(...)` in a `tests/*.rs`
   file; this enables `LOFT_LOG` debug output without modifying the source.
5. **Use the debug binary** — `cargo build --bin loft` produces a binary with extra runtime
   checks; segfaults often produce clearer output or trigger a Rust panic instead.

### Failure types and fixes

#### Assert fires with wrong value

```
panicked at src/fill.rs:1772:5: my assert message
```

The message is whatever string was passed as the second argument to `assert()`.
Add `print("{actual}")` directly before the failing assert to see the actual value.
Common causes:
- Off-by-one in an expected range or loop count — trace manually.
- Floating-point rounding — use `round()` before comparing or widen the tolerance.
- Format output differs from expected — print both sides and compare byte-by-byte.

#### Segfault (no output)

```
Segmentation fault (core dumped)
```

The interpreter hit an unguarded memory access.  Run the debug binary for a Rust panic
instead of a silent crash:

```bash
cargo build --bin loft          # debug build, slower but safer
./target/debug/loft tests/scripts/05-enums.loft
RUST_BACKTRACE=1 ./target/debug/loft tests/scripts/05-enums.loft
```

Common causes:
- Calling a feature that is not yet implemented (e.g. `enum_value as integer`,
  unimplemented stdlib method) — the interpreter falls through to an unreachable branch.
- Passing a wrong type where the runtime expects a specific layout (e.g. a struct-enum
  variant used as a plain enum).
- Remove the suspect line; if the segfault disappears, the line triggers the bug.

#### Parse error — "Dual definition of"

```
Dual definition of <name> at file.loft:line:col
```

A name is defined twice in the same scope.  Common triggers:

- **Nested format string with escaped quotes**: `"outer {\"inner\"}"` — previously the
  lexer treated `\"` as ending the outer string. This was fixed in 2026-03-14 via the
  `in_format_expr` flag in `src/lexer.rs`; `\"` inside `{...}` now works correctly.
- **Two struct definitions with the same field name**: this is now safe — field lookups
  are type-scoped. Verified by `tests/scripts/23-field-overlap-structs.loft` and
  `24-field-overlap-enum-struct.loft`.
- **Re-declaring a function with identical parameter types**: loft allows overloading by
  type; identical signatures are an error.

#### Parse error — "Undefined type"

A type name appears before its `struct`/`enum` definition.  Move the definition above its
first use, or above any function that references it.

#### Wrong result from index range query

If a range query like `db.map[83..92, "Two"]` returns unexpected elements, the most likely
cause is a **field-offset conflict**: two structs defined in the same file share a field
name at different positions.  For example:

```loft
struct A { key: text }           // key is field 0
struct B { nr: integer, key: text }  // key is field 1
```

When both `sorted<A[-key]>` and `index<B[nr,-key]>` exist in the same file, the compiler
may resolve `key` to the wrong field number for one of the lookups.

Fix: use distinct field names, or place conflicting struct definitions in separate test files.

#### Compile error — "Cannot add elements to '...' while it is being iterated"

```
Error: Cannot add elements to 'v' while it is being iterated — use a separate collection or add after the loop
Error: Cannot add elements to a collection while it is being iterated — use a separate collection or add after the loop
```

This is a deliberate compile-time guard. Appending to a collection during iteration is
unsafe: vectors re-read their length on every step (so new elements are visited, risking
an infinite loop), and sorted/index insertions corrupt stored iterator positions.

**Fix options:**
- Collect additions in a separate variable and append after the loop: `extra = []; ... for e in v { ... extra += [x]; } v += extra;`
- Remove elements during iteration with `e#remove` in a filtered loop — this is the one safe in-loop mutation.

**Scope:** The guard covers both direct variable mutations (`v += x`) and field-access
mutations (`db.items += x`) as of 2026-03-14.

#### Wrong iteration order in sorted/index

Verify the sort direction: `-field` means **descending**, `field` means **ascending**.
A mismatch between the declared direction and the expected order is the most common mistake.
Trace the expected element sequence manually before writing the assert.

---

## Loft Test Runner (`--tests`)

The `--tests` CLI flag provides a built-in test runner for loft programs.  It
discovers and executes test functions in `.loft` files without requiring Rust
or `cargo test`.

### Writing tests

Any zero-parameter function whose name starts with `test_` is a test function —
the underscore is part of the rule, so `testDouble` and a function called exactly
`test` are not tests and nothing says so:

```loft
fn test_addition() {
    assert(1 + 2 == 3, "basic addition");
    assert(10 + 20 == 30, "larger addition");
}

fn test_string_length() {
    assert("hello".len() == 5, "text length");
}
```

Test functions use `assert(condition, message)` to validate behaviour.  A
failing assertion marks the test as failed; the runner continues with the
remaining tests in the file.

Helper functions, structs, and other definitions can coexist in the same file —
once the file names at least one `test_*`, those are the whole set and everything
beside them is a helper.

**A file that names NO `test_*` is the other case, and it is the one that lets
`--tests` be pointed at a plain program at all**: there, every zero-parameter
function is run and counted, `main` included, and a parameter — even an unused
one — is the only way to say "not an entry point" (loft#1010). That is how the
reference chapters are checked. The rule is one `if` in
`src/test_runner.rs`: a file that declares a `test_*` has said which functions
are tests.

### Running tests

```bash
loft --tests                  # run tests in current directory (recursive)
loft --tests tests/           # run tests in a specific directory
loft --tests file.loft        # run all tests in a single file
loft --tests file.loft::name  # run a single test function
loft --tests 'file.loft::{a,b}'  # run specific test functions
loft --tests --no-warnings    # suppress warning output
```

Inside a package, `loft test [target]` runs `tests/` and takes the file **however you
spell it** — `loft test draw`, `loft test draw.loft` and `loft test tests/draw.loft`
are one target, so a path pasted out of the runner's own output works (loft#913).  A
`::selector` combines with any of them (`loft test tests/draw.loft::test_foo`), and a
selector that names no test function is an ERROR, not a `0 passed` success.

The runner:
1. Recursively discovers `.loft` files under the given directory (default: `.`).
   When given a single `.loft` file, runs only that file.
2. Parses each file and finds all callable functions (zero-parameter, or
   single `vector<text>` parameter when `@ARGS` provides argv).
3. Applies the optional `::name` or `::{a,b}` filter to select specific functions.
4. Runs each test function independently.  A failed `assert` marks the test as
   failed but does not abort the run.
5. Reports per-file and per-directory summaries.
6. Exits with code 0 if all tests pass, 1 if any fail.

### Native mode (`--tests --native`)

```bash
loft --tests --native tests/scripts     # compile and run all scripts natively
loft --tests --native file.loft         # single file
loft --tests --native file.loft::name   # single function
```

When `--native` is combined with `--tests`, each file is compiled to a native
Rust binary via `output_native_reachable` + `rustc`, then executed:

1. Generate Rust source with all selected test functions called from a
   generated `main()`.  Files with `fn main()` use the loft main directly.
2. Compile with `rustc` (links against `libloft.rlib`).
3. Run the binary and check exit status.

**Binary cache:** Generated `.rs` files and compiled binaries are kept in
`/tmp/loft_test_native_*`.  An FNV-1a content hash (`.key` sidecar) prevents
recompilation when the source hasn't changed.  Typical speedup: 8–10x on
repeated runs.

**Stale rlib detection:** Before native compilation, the runner compares
`libloft.rlib` mtime against `src/` and `default/` source mtimes.  If any
source is newer, `cargo build --lib` runs automatically.

**Limitations:**
- `@EXPECT_FAIL` tests are skipped (native can't catch panics for matching).
- `@EXPECT_ERROR` files are skipped (can't compile intentionally broken code).

### Output format

```
  ok    tests/math.loft  (2 tests)
  FAIL  tests/text.loft::test_empty_concat
  FAIL  tests/text.loft  (1 failed, 3 passed)

  tests/: 1 failed, 5 passed

test result: FAILED. 1 failed; 5 passed; 6 total; 2 files  [ran on the interpreter only — native not exercised: loft test --native]
```

**The result line names the backend it came from.**  `loft test` and `loft test
--native` each exercise exactly ONE backend, so a bare `ok` was identical
whether the other backend was clean or had never been compiled once — silence
read as coverage.  (A consumer found a quarter of their packages had never been
native-compiled while `loft test` stayed green throughout; they could only
discover it by running the native sweep by hand.)  The scope rides on the
DEFAULT path, because that is the path that was lying.

Under `--native` the note also reports `N skipped` — tests counted as passing
that never ran on that backend (`@EXPECT_FAIL` / `@IGNORE`, or a file with no
native-runnable function), so a green count cannot stand in for coverage it does
not have.

**The same line reports @PLN86 admission scope** (#631).  `loft test` applies the
`[sandbox]` policy from the nearest `loft.toml` at or above each test file (the
package root, since a test lives in `tests/` while the code it exercises lives in
`src/`), and an admission violation FAILS the file just as a compile error does —
a rejected script cannot run at all, so a suite that reported it green was
reporting on something the host would refuse to load.  Three states:

| Note | Meaning |
|---|---|
| `admission checked on N files` | a policy designated code, and it was admitted |
| `a [sandbox] policy is present but designated nothing here` | the selectors matched no function — admission covered NOTHING |
| `no [sandbox] policy — admission not exercised` | nothing to check |

The middle state is the one worth reading: admission used to engage only on the
run path and via `loft sandbox-check`, so a package could carry a deliberate
capability violation and stay green.  Passing quietly under a policy that matches
nothing looks identical to real coverage, which is why it gets its own note rather
than falling back to "no policy".

Files with no `fn test*()` functions are silently skipped.  Hidden directories
(starting with `.`) and `.loft/` artifact directories are excluded from the
recursive walk.

### Flags

| Flag | Effect |
|------|--------|
| `--tests [dir\|file]` | Discover and run test functions (default dir: `.`) |
| `--tests file::name` | Run a single test function in a file |
| `--tests file::{a,b}` | Run specific test functions in a file |
| `--native` | Compile to native Rust instead of interpreting (with `--tests`) |
| `--no-warnings` | Suppress warning diagnostics in test output |

### The shared library base (loft#925)

Each test file is its own program with its own parser — a shared one would let one
file's definitions leak into the next — so a `use`d library used to be loaded from
source once per file, and twice at that, since both parse passes re-run the use
region.  A suite therefore paid the PRODUCT of its file count and its library's
size.

Files are now grouped by their leading `use` region (a `#cwd` directive included,
verbatim), the region is parsed once per group, and every file after the first
starts from a copy of that parse.  dryopea's 81-file suite: 238 s → 209 s, output
byte-identical.  See [PERFORMANCE.md](PERFORMANCE.md) for the numbers and the
three decisions that carry the win.

| Env var | Effect |
|---|---|
| `LOFT_NO_TEST_BASE=1` | Parse every file's libraries for itself, as before.  The control half of an A/B on ONE binary, and what `tests/test_base_equivalence.rs` compares against — a run whose output differs from it is a bug in the sharing. |
| `LOFT_TEST_BASE_REPORT=1` | Name on stderr each `use` region that got a shared base, or was refused one.  Reach for it when a suite did not get faster; it is also what keeps the equivalence guard from silently comparing a run to itself. |

A group of ONE file never builds a base — the base is built when a second file asks
for the same region — so `loft test <one-file>` costs exactly what it did.  A base
is also refused outright under a `[sandbox]` policy (admission reads what the parse
recorded about designated functions) and whenever the region's own parse raises an
error (the error belongs to the file the reader is shown, so it is left to that
file's own parse to re-emit).

---

## Debug boundary checks (debug builds only)

Three `debug_assert!` checks fire automatically in debug builds (`cargo test`)
with no env-var needed.  They catch the most common runtime bug patterns at the
point of first access, before corruption propagates:

| Check | File | Catches |
|---|---|---|
| `store_nr < allocations.len()` | `src/keys.rs` `store()` / `mut_store()` | DbRef pointing to a non-existent store (e.g. light-worker borrow range too small) |
| `fld + size ≤ record_size` | `src/store.rs` `addr()` / `addr_mut()` | Field access past the end of a claimed record (e.g. wrong `pos` in a returned DbRef) |
| `stack.pos ≥ size_of::<T>()` | `src/database/mod.rs` `get<T>()` | Stack underflow from popping more bytes than were pushed (e.g. wrong native-function arg order) |

All three are zero-cost in release builds.

> **Note:** `[profile.dev.package.loft] debug-assertions = false` opts the
> loft package itself out of `debug_assertions` even in dev/test builds (for
> hot-path performance).  The boundary checks above — and EVERY other
> lib-side `debug_assert!` / `#[cfg(debug_assertions)]` check (the H5
> two-pass contract, `Store::valid`/`validate`, codegen sanity asserts, the
> `[set_var]` width warnings) — are therefore **silent on every platform**
> during ordinary `cargo test` runs, in both dev AND `--release` profiles.
> The only builds that check them are the cargo-fuzz target (which forces
> `-Cdebug-assertions`) and an explicit calibration run — see
> [DEBUG.md § The debug-assertions calibration run](DEBUG.md#the-debug-assertions-calibration-run-target-da).
> The first-ever such calibration (2026-07-03, @PLN85) found four
> long-latent H5 producers and a latent-assert inventory; believing "the
> suite is green" for a DA-gated invariant is a calibration failure — the
> instrument is not installed in that build.  Latent
> out-of-bounds writes inside `Store` are tolerated by Linux's allocator
> slack (16-byte chunk minimum) but caught by Windows as
> `STATUS_HEAP_CORRUPTION (0xc0000374)` at deallocation — see the valgrind
> section below for how to surface this on Linux without waiting for
> Windows CI.

---

## Occasional valgrind pass (Linux)

The loft codebase has a large `unsafe` surface in `src/store.rs`,
`src/database/`, and `src/parallel.rs` (raw `addr`/`addr_mut`, LLRB
free-tree rotations, claim/free splits, worker store adoption).  Linux
runs the test suite cleanly because the system allocator over-allocates
small chunks; latent OOB writes land in slack and don't corrupt anything
visibly.  The same code on Windows hit `STATUS_HEAP_CORRUPTION` once the
heap manager validated chunk metadata at deallocation.

Valgrind's memcheck tool catches this class of bug instantly: every
load/store is instrumented and OOB accesses fail loudly, regardless of
allocator behaviour.

### Recipe

```bash
scripts/valgrind-sweep.sh              # every script + document, interpreter AND native
scripts/valgrind-sweep.sh tests/docs   # one tree, or a list of files
```

One command, one verdict, per-file logs in `target/vg/`.  It runs `loft --interpret` on every
file (`--tests` for `tests/scripts`, whose files have no `main`) and, for `tests/docs`, builds
each document with `loft --native` and hands the cached binary in `<dir>/.loft/cache/` to
memcheck directly — the compiled program is where the native runtime's `unsafe` runs, and
`--trace-children` cannot reach it without also tracing rustc.  About fifteen minutes on
24 cores; `VG_JOBS` bounds the parallel memchecks (each takes ~200 MB).

Two decisions are built in, and both are measurements rather than taste:

- **Only an invalid access or a DEFINITELY lost block is red.**  Rust's hashbrown tables and
  boxed strings keep interior pointers, so every process-lifetime table — the parser's
  `Data`, the native emitter registry — reads as "possibly lost" at exit: 179 such records on
  a run with no defect in it.  `--errors-for-leak-kinds=definite` is that decision spelled
  where valgrind reads it; a possibly-lost record is still in the log for anyone who wants it.
  The one suppression, `scripts/valgrind.supp`, is the deliberate interning of a declared
  text field default — bounded, one block per field — and nothing else: the LSan file also
  hides the four text-construction frames on the premise that they leak only on a fault
  path, and this sweep measured that premise false (a text returned from a call on two arms,
  or read straight out of a vector element, loses one buffer PER CALL with no fault at all).
- **A leaked or over-freed STORE is not a valgrind error.**  The store arena is one valid
  allocation (DEBUG.md § Debugging store-ownership bugs), so that half of the release's
  memory gate is `M-leaks` under `LOFT_STRICT_STORES=1`, and this sweep does not pretend to
  cover it.

### When to run

- **Before a release** — once per release cycle.  Catches any latent
  UB introduced since the last pass.
- **After significant `unsafe` changes** in `Store`, `Stores`, the
  parallel runtime, or the LLRB free-tree (`fl_*` in `src/store.rs`).
- **When a Windows-only failure appears** with heap-corruption-style
  symptoms (`0xc0000374`, `LdrpAllocate*`, `RtlReportFatalFailure`).

Not a CI default: too slow for every PR.  Tracked as a release-blocker
gate in [RELEASE.md](RELEASE.md) — run on the tag candidate, not on
every push.

---

## Test Coverage Gaps

Last updated 2026-04-02.  Overall: **71.3% line / 74.9% function**.

### Files with 0% or critically low coverage

| File | Line % | Key gaps |
|---|---|---|
| `src/documentation.rs` | 0% | HTML doc gen — covered by `gendoc` binary only |
| `src/radix_tree.rs` | 0% | Planned feature, unused |
| `src/native_utils.rs` | 12.3% | WASM/installed-layout paths |
| `src/database/allocation.rs` | 38.6% | Store growth, boundary conditions |
| `src/logger.rs` | 39.3% | Production mode, rotation, rate limiting |
| `src/extensions.rs` | 45.5% | Plugin dedup, library load failures |
| `src/variables/validate.rs` | 45.6% | Scope cycle detection, sibling conflicts |
| `src/database/search.rs` | 46.5% | Multi-key range queries |

### Priority gap areas

1. **Vector reverse/sort** — `.loft` script test; closes `reverse_vector()` 0% gap
2. **Database store boundaries** — `limits.rs`; important for correctness
3. **Database range queries** — `.loft` scripts with multi-key sorted collections
4. **Parser stress / error recovery** — new `parser_stress.rs`; high robustness value
5. **Logger production mode + rotation** — extend `logger_severity.rs`
6. **DbRef edge cases** — add to `data_structures.rs`
7. **Slot validation paths** — synthetic IR tests in `tests/slots.rs`

### Features tested only in `tests/*.rs` (not scriptable)

| Feature | Rust test file |
|---|---|
| Parallel worker API | `threading.rs` |
| Data structures API (Stores/tree/hash) | `data_structures.rs` |
| Logger severity routing | `logger_severity.rs` |
| Code generation correctness | `issues.rs` |
| Code formatter roundtrips | `format.rs` |
| Native compilation pipeline | `native.rs` |
| WASM compilation | `wasm_entry.rs` |

---

## Headless OpenGL testing (Xvfb)

Loft GL examples create a real winit/GLX window, so they normally need an X
display. For CI / sandbox environments without `$DISPLAY`, we run them under
**Xvfb** (the X Virtual Framebuffer). Xvfb is a software X server that
keeps everything in memory — no GPU, no monitor, no compositor required.

### Required tools

```bash
sudo apt-get install -y xvfb x11-utils x11-apps xdotool imagemagick
```

- `xvfb-run` — wrapper that starts Xvfb on a free display, runs the inner
  command with `$DISPLAY` set, and tears Xvfb down on exit.
- `xdotool` — searches for a window by name and returns its X11 ID.
- `import` (from ImageMagick) — captures a window or the root drawable
  to a PNG file.

### Running a single GL example headlessly

```bash
xvfb-run -a -s "-screen 0 800x600x24" \
    target/release/loft --interpret \
        --path /home/ubuntu/loft/ \
        --lib /home/ubuntu/loft/lib/ \
        lib/graphics/examples/25-brick-buster.loft
```

`-a` picks an unused display number. `-s` passes args to Xvfb itself.
Mesa's software rasterizer (`swrast`/`llvmpipe`) handles the actual GL
draw calls — the binary doesn't know it's running headless.

### Capturing a screenshot

You can't take a screenshot *after* `xvfb-run` returns because that's when
Xvfb dies. The capture has to happen *while* loft is running. The pattern:

1. Background loft inside a wrapper script.
2. Poll for loft's window via `xdotool search --name "."`.
3. Wait for some animation/render time.
4. `import -window <id> out.png`.
5. Kill loft, exit. `xvfb-run` then tears down Xvfb.

A working wrapper script lives at `/tmp/snap_inner.sh` during dev sessions:

```bash
#!/bin/bash
SCRIPT="$1"
OUTPUT="$2"
POST_WAIT="${3:-4}"

target/release/loft --interpret \
    --path /home/ubuntu/loft/ --lib /home/ubuntu/loft/lib/ \
    "$SCRIPT" >/tmp/loft.log 2>&1 &
LOFT_PID=$!

# Poll up to 10s for loft's window to appear
WIN_ID=""
for i in $(seq 1 20); do
    sleep 0.5
    WIN_ID=$(xdotool search --name "." 2>/dev/null | tail -1)
    [ -n "$WIN_ID" ] && break
    ps -p $LOFT_PID >/dev/null 2>&1 || break
done

sleep "$POST_WAIT"   # let the render loop produce interesting frames

if ! ps -p $LOFT_PID >/dev/null 2>&1; then
    echo "FAIL: loft exited before capture (script too short)"
    exit 1
fi

import -window "$WIN_ID" "$OUTPUT"
kill $LOFT_PID 2>/dev/null; wait $LOFT_PID 2>/dev/null
```

Then run it inside `xvfb-run`:

```bash
xvfb-run -a -s "-screen 0 800x600x24" \
    /tmp/snap_inner.sh \
    lib/graphics/examples/25-brick-buster.loft \
    /tmp/brick-buster.png 6
```

`POST_WAIT` matters: short scripts (`for _ in 0..300`) finish before the
capture; long-running ones (e.g. `for _ in 0..1000000` like brick-buster) stay
alive indefinitely. For animated examples, increase `POST_WAIT` to capture
a different frame in the animation cycle.

### Gotchas

- **The loft window is a child of the X root, not the root itself.**
  `import -window root` captures an empty Xvfb root if no window manager
  is parenting/compositing children. Always grab loft's window by ID.
- **`LIBGL_ALWAYS_SOFTWARE=1` makes things WORSE under Xvfb.** Without it,
  Mesa picks `swrast_dri.so` automatically; with it, the GL context fails
  to initialise and `gl_create_window` returns false.
- ~~**Some loft examples panic under Xvfb with `Delete on locked store`.**~~
  **Fixed** — the underlying P120 use-after-free is closed
  (see `tests/issues.rs::p120_*` and CHANGELOG).  Retest on HEAD if a
  similar panic reappears; it's a new bug, not P120.
- ~~**RGB↔BGR channel swap in GL captures.**~~  **Fixed (P133)** —
  Xvfb + Mesa-swrast + ImageMagick `import` reads the framebuffer with
  R and B swapped.  On-screen rendering is correct; only captured PNGs
  were wrong.  `tests/scripts/snap_smoke.sh` now applies
  `convert -separate -swap 0,2 -combine` post-`import`, and the golden
  PNG was regenerated.
- **Polling for `xdotool search --name "."`** matches *any* named window.
  If the test environment has other X clients running, narrow it down by
  passing the window title used in `gl_create_window`.

### Using Xvfb to run the cargo test suite

```bash
# Run all GL-touching tests under Xvfb in one shot
xvfb-run -a cargo test --release
```

The test process inherits `$DISPLAY` from `xvfb-run`. Tests that don't
touch GL ignore it; tests that *do* touch GL get a working framebuffer.

### Headless valgrind on a GL example

For leak/UB checking on a GL example, combine Xvfb with valgrind:

```bash
xvfb-run -a -s "-screen 0 800x600x24" \
    valgrind --tool=memcheck --leak-check=full \
             --show-leak-kinds=all --log-file=/tmp/v.log \
        target/debug/loft --interpret \
            --path /home/ubuntu/loft/ --lib /home/ubuntu/loft/lib/ \
            lib/graphics/examples/25-brick-buster.loft

grep -E "definitely lost|indirectly lost|possibly lost|ERROR SUMMARY" /tmp/v.log
```

Debug-build loft + valgrind + Mesa swrast is **very slow** — expect
10-100x slowdown. Use a short loop count for ad-hoc checks.

---

## Debugging `loft --html` WASM traps

WASM compiled in release mode strips Rust panic strings — a
runtime fault surfaces as a bare `RuntimeError: unreachable executed`
with no message.  This can send diagnosis deep into the wrong place
(we spent several sessions chasing "panic in bytecode dispatch" for
P137 when the real cause was `Instant::now()` in init).

### The technique

1. **Write a minimal reproducer.** If `fn main() { }` or
   `fn main() { println("hi"); }` traps, the bug is in WASM init
   (Stores::new / stdlib load / host-import wiring), **not** in any
   user-code path.  Don't bisect user code yet.

2. **Rebuild the same generated Rust as a debug wasm** to preserve
   panic symbols in the stack trace.  The `loft_html.rs` file is
   written to `/tmp/loft_html.rs` by `loft --html` immediately before
   the rustc invocation; grab a copy before the wrapper deletes it:

   ```bash
   ./target/release/loft --html /tmp/app.html --path $(pwd)/ app.loft &
   sleep 0.3 && cp /tmp/loft_html.rs /tmp/loft_html_saved.rs
   wait
   ```

3. **Compile the saved Rust without `-O`** so Rust's panic machinery
   still emits symbols:

   ```bash
   rustc --edition=2024 --target wasm32-unknown-unknown \
     --crate-type cdylib \
     --extern loft=target/wasm32-unknown-unknown/release/libloft.rlib \
     -L dependency=target/wasm32-unknown-unknown/release/deps \
     -L dependency=target/release/deps \
     /tmp/loft_html_saved.rs -o /tmp/debug.wasm
   ```

4. **Run in Node with `tools/wasm_repro.mjs`** and print the stack:

   ```bash
   node tools/wasm_repro.mjs /tmp/debug.wasm
   ```

   The debug build's stack shows `_ZN...` mangled Rust symbols.  The
   first non-panic-machinery symbol is the function that panicked —
   often `std::...::now`, `Vec::index`, `Option::unwrap`,
   `core::panicking::panic_fmt`.

### Repro harness — `tools/wasm_repro.mjs`

Loads a WASM file with loose stub imports (loft_io and loft_gl via a
Proxy that answers any method with a no-op) and runs `loft_start`.

```bash
node tools/wasm_repro.mjs <path/to/wasm> [--trace]
```

Exit code 0 = clean run; 1 = trap.  `--trace` records every host
import call into a buffer printed on trap — revealing which loft
function last reached the host boundary before the fault.

Used by the **`tests/html_wasm.rs::p137_html_hello_world_does_not_trap`**
regression test, which builds a hello-world `.loft` program through
`--html`, extracts the WASM, and runs the harness.  Skipped in
environments without node or the wasm32-unknown-unknown rustup target.

### Why the release trap had no message

Rust compiled for `wasm32-unknown-unknown` with `-O` uses
`panic = "abort"` by default: any panic becomes a bare
`(unreachable)` instruction with no format string, no location, no
call to a panic handler.  On native + debug this is the opposite —
panics carry a source location and a formatted message.  When the
browser / Node engine hits the unreachable, it produces a generic
`RuntimeError: unreachable executed` with only the WASM function
index in the stack trace.

The fix path is always: rebuild without `-O`, get a symbol-preserving
stack, and the panic site appears.

---

## See also
- [PROBLEMS.md](PROBLEMS.md) — Known bugs, limitations, workarounds, and fix plans
- [CLAUDE.md](../../CLAUDE.md) — Project orientation: execution path, key data structures, branch policy, documentation index
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy (LOFT_LOG presets, scope bugs, slot conflicts), working with Claude

## Open work

| Item | Section | Status |
|---|---|---|
| **Fenced examples in API doc comments are not executed** — an example in a `pub` item's doc comment is not run or asserted, so a doc can disagree with its code.  The mechanism is designed (@PLN121: extract → run both backends → gate → ship only what ran), but the **domain is empty**: measured 2026-08-25, **6 fenced examples in 1962 `pub` items** across the stdlib and all 8 library repos, none in a published package.  Building an extractor, runner, gate and registry field for six examples would be five mechanisms serving one file. | § Doc tests | 🔕 Deliberately not built.  **Trigger to revisit: re-run the count** — if a package starts writing fenced examples, @PLN121's steps 3–7 are still the plan.  The assert-less half shipped (`tests/doc_hygiene.rs::every_doc_page_asserts_something`). |
| ~~**@P229b — Windows `pick_free_port` rebind race**~~ — **CLOSED 2026-05-29** via the v2 probe (PR #228).  Un-ignored `v2_single_client_completes_game` on `windows-latest`; CI showed it PASSING.  All 10 `multiplayer_v{2,3,5}.rs` `#[cfg_attr(target_os = "windows", ignore = "P229b…")]` ignores dropped in the follow-up.  The 2026-05-21 leading hypothesis (bind-then-drop race) was incorrect; @P229b was incidentally resolved in some recent Rust toolchain or transitive dep update.  Bug record: [PROBLEMS.md @P229](PROBLEMS.md). | — | ✅ Closed; row kept for the lesson — "don't apply unverified-from-Windows-output hypotheses blindly" stands. |

---

## What a run did NOT check — scope, admission, coverage

`loft test` ends with a line stating what the run **left out**, not only what it did.
The reason is one repeated defect: a bare `ok` looked identical whether the other half
had been checked or had never run once, and people read that silence as coverage. A
consumer found a quarter of their packages had never been native-compiled while
`loft test` stayed green; another injected a deliberate capability violation and the
suite passed. Three things are reported for that reason:

- **Backend scope** — `[ran on the interpreter only — native not exercised: loft test
  --native]`. Each invocation exercises exactly one backend.
- **Admission** — whether a `[sandbox]` policy was present and how many files it
  actually covered. A policy that designates nothing says so.
- **Function coverage** — the functions the suite never entered.

### Function coverage

Every function defined in the package under test that no test entered is listed with
its file, line, and name:

```
coverage: 4 of 8 functions were never entered by these tests
  src/regex.loft:100  find
  src/regex.loft:105  split
  src/regex.loft:110  text.regex_find
  src/regex.loft:115  text.regex_split
```

A fully-covered package says so explicitly (`coverage: all 36 functions were entered`),
so "no coverage line" can never be misread as "everything is covered" — which would
reproduce the very defect the report exists to remove. Ten entries print by default;
`LOFT_COVERAGE=list` prints them all.

**It is a list, never a percentage, and never a gate.** A percentage becomes a target,
and a coverage target produces tests written to reach a line rather than tests that
check a behaviour — the metric goes green over code nobody validated. And a gate would
fail exactly the case the package system exists to support: a library is written
*before* its consumers, so it legitimately starts with little coverage. Each line here
is instead an individual, checkable fact — this code did not run — and the only way to
remove one is to actually run the function.

What is deliberately **not** counted, because counting it would make the number lie:

| Excluded | Why |
|---|---|
| `#native` declarations | No loft body to enter — they dispatch to Rust, so a native-backed package would read 100% uncovered however well tested. |
| Dependencies, the stdlib | A package is not answerable for code it did not write; charging it would make its number depend on how much of a dep it happens to touch. |
| The test file's own functions | They are the drivers, and the runner already reports on them. |
| Generated lambdas | Not written by the author. |

Generators count when **iterated**, not when created: a generator's body runs on resume,
so `it = gen();` with no loop over it has run none of it and stays on the list.

Coverage is recorded on the interpreter (`State::fn_call` and the coroutine resume), so
`loft test --native` prints no coverage line — the interpreter leg carries it. Test
adequacy is a property of the tests, not of the backend.

Guarded by `tests/function_coverage.rs`, which asserts the quiet directions as hard as
the loud one.

---

## How a guard reads green while the defect stands

A guard that cannot fail is worse than no guard: it is a standing claim that the behaviour
is checked. `make falsify` catches the commonest case — a guard that never failed on the
build it was written to catch — but it only answers for the commit you name. These are the
shapes that survive it, each one measured here rather than imagined.

**A probe whose TEST is looser than its question, which is a different failure from the one
below and fails the other way.**  Setup contamination puts the answer into the channel; this
one accepts the wrong answer out of it — and it almost always fails toward "nothing found",
which reads as clean.  SUBSTRING where EXACT was meant is the commonest form.  Measured
2026-09-07 counting formal rules with no code citation: the predicate included
`"0 citation" in <summary>`, which is `True` for **"10 citation(s)"** and "20 citation(s)", so
every rule with exactly ten or twenty citations counted as having none.  Three chapters read
one too high, and nothing in the output could show it — a plausible number, no error, no empty
cell to notice.  It survived being used to CORRECT someone else's table.
**Assert on the structured thing** (here: does a `src/` line come back at all), never on a
substring of a human-readable summary; and when a number disagrees with someone else's, check
whether the DIRECTION of the difference is possible before explaining it — the two trees here
were in a superset relation, so the sign was already impossible and no story about them could
have been true.

**A probe whose SETUP contains the thing it tests for measures the setup.**  This is the one
that survives "measure first", because the probe runs first and still answers the wrong
question — a probe written from a hypothesis inherits the hypothesis.  Measured 2026-09-07,
while checking whether a bracketed `pgrep -f "[x]…"` waiter self-matches: both the bracketed
and the unbracketed form went into ONE shell invocation, so the wrapper's `argv` carried the
plain string, the bracketed regex matched it, and the run reported *"the bracket self-matches"*
— the exact failure the probe existed to detect, committed by the probe. Split apart, one
per invocation, the bracketed form exits on the first poll and the plain one loops forever:
the opposite conclusion. The tell is that the probe and its subject share a channel — the same
command line, the same directory, the same cache, the same process. **Ask what the setup itself
puts into the channel being read, and run one cell per invocation when the answer is "the thing
I am looking for".**  A positive AND a negative control in one run is the commonest way in.

**A reproduction that hits a WARM CACHE measures nothing — and the tell is the clock.**  A
red `make ci` named a native cell that took **2.2 s** in the gate; every attempt to reproduce
it took **51 ms**.  On that basis it was reported "not reproducible" three ways — 20 serial
runs, 48 at 16-way concurrency, the whole test binary under nextest — and all of it was
vacuous: the runs hit a warm artifact cache and never reached the publish path that breaks.
A pass 40× faster than the failure is not a pass, it is a different experiment.  Two matching
traps rode along: the cache lives in `<script dir>/.loft/cache/`, **not** under `TMPDIR`, so
"reproduced with the gate's own `TMPDIR`" was another empty cell; and a harness notice about
low memory (it kills background tasks on its own budget) was read as a statement about the
BOX while `free` showed 55–59 GB available at that very moment.  **State the failure's cost —
wall-clock, or a counter like *did it compile?* — and check the repro matches it BEFORE
reporting a negative.**

**Racing a race is usually not a falsifiable guard; assert the PROPERTY the race violates.**
The obvious guard for the cache-publish race — N concurrent cold-cache runs of one source,
assert all succeed — **passed on the pre-fix build** (18 runs): the window is too narrow to
hit on demand, so it measured nothing while looking like a regression test.  The deterministic
reading is the **inode**.  `fs::copy` opens the destination with truncate and streams into the
SAME inode, which is exactly what lets a reader's already-accepted file empty out underneath
it; `rename` swaps a different inode in and a process still exec'ing the old one keeps a
complete file.  So `publish(a); i1 = ino(dest); publish(b); i2 = ino(dest); assert_ne!(i1, i2)`
— no timing, and it fails on the pre-fix publish with the same inode on both sides.  Keep the
old implementation in the test file as a POSITIVE CONTROL (`assert_eq!` on the copy form) so
the probe proves it discriminates without an inverse edit of the product.  If the racing test
is kept for the end-to-end shape it still covers, its header must say it does NOT reproduce
the defect and name the guard that does — an inert experiment recorded is reusable, an inert
experiment mislabelled is a false green.

**A "did this copy too much?" control is not the same cell for every element type, and the
wrong one argues for reverting a correct fix.**  The natural control for an over-wide copy is
*an undisturbed view must still ALIAS — a write through it reaches the container*.  That is
right for a RECORD tail and INVERTED for a COLLECTION one: `(B-Copy)` makes a whole-value
vector bind a copy, so a write through it reaching nothing is the documented answer, in the
branch spelling exactly as in the plain one.  Read the record way, loft#1399's fix looked like
the over-wide version it had been warned against — a shipped release answered 3 where the
branch answered 2 — and the next step would have been to revert something correct.

What settles it is the PLAIN spelling of the SAME tail on the SAME build, not the same shape on
an older build.  The release turned out to be the inconsistent side: it aliases a collection
projection in the branch spelling while copying it in the plain one.  Two habits fall out of
that.  When the shipped binary is the thing under suspicion, isolate by inverse-editing your
own change out and rebuilding rather than trusting it as the only oracle — that is what showed
the difference predated the edit entirely.  And pin BOTH boundaries as cells, so the next
reader does not have to re-derive which kind aliases.

**A one-of-a-kind fixture makes a positional read look right.**  Reading a tuple member's
backing work-ref off the element type's dep list, `deps.first()` passed every cell of a matrix
whose tuples had ONE droppable member — and the lists are UNIONED across a tuple's heap
members, so every element carries the same list and `first()` names the FIRST member whatever
you ask about.  The two-member cell is the only shape that can tell a correct pairing from a
coincidence: it released one resource twice and the other once.  Whenever an index, an offset
or a position is being read, one cell has to carry TWO of the thing being indexed.

**Undoing a lowering means undoing its TYPE, and a value-only matrix cannot see the half you
left.**  A monomorph pass that removed a tuple-member copy unwrapped the VALUE and left the
element's dep on the backing whose copy was gone; every value cell still answered correctly and
the store was freed by nobody.  It surfaced only because the probe printed the run's leak line
beside the values.  A guard over a lowering that can be undone asserts value AND leak, or it is
measuring one half of the change.

**A CONSTANT index is trusted by contract, so a nullability cell built on it never
produces the `τ?` it means to test.**  `(N-Index)` types `v[i]` as `τ?`, but `v[9]`, `v[k]`
under `for k in …`, `v[i]` behind `if i < len(v)` and arithmetic over those are trusted
(@PLN102 D1) and read NON-NULL — the overrun faults to null at run time, in band.  The
phase-1 `(N-Join)` hold cell wrote `j = 2; j = dv[9]; assert(j == null)` and passed on every
build: `dv[9]` was an `integer`, nothing was widened, and the assert read the sentinel the
runtime left in the slot.  With a plain variable index the same program was REFUSED (the
widen did not exist).  A cell that needs a `τ?` from an index reads it through a variable
the compiler cannot prove in range — and the receipt is the cell compiling at all.

**The HARNESS does not run the program the way a user does, so a `tests/scripts/*.loft`
guard can be vacuous for a whole CLASS of defect.** `loft --tests` discovers and calls the
`test_*` functions; it is not `loft prog.loft`, and two things a plain run does are simply
absent from it. Measured 2026-09-01, both against the released binary, both giving a guard
that passed on the very build it was written to catch:

* **the script classifier never runs.** loft#1271's defect was `split_top_level` ending a
  top-level item early, which hid the following `fn main` and got an ordinary program
  desugared as a beginner script. The corpus guard file passes `--tests` on the pre-fix
  binary and FAILS as a plain `loft --interpret` run of the same file.
* **a store leak is not reported.** loft#1273 retained one record per inline call; the
  `tests/scripts` guard passes `--tests` on the pre-fix binary while the same shapes as a
  plain program print `Warning: 1 stores not freed at program exit`.

**A `--lib <dir>` override is DROPPED when the working directory has its own `lib/`, so a
probe against a modified copy of a library measures the original.** Resolution is
first-wins and the cwd-relative `lib/` is probed before `--lib`
(`parser/mod.rs::lib_path`), so the flag cannot reach any name `lib/` also provides —
silently, since a dropped flag raises nothing. Measured 2026-09-04 on loft#1339: three
runs of a patched `lib/parser.loft` copy, launched from the repo root with `--lib <copy>`,
all scored the UNMODIFIED tree; a copy with a line of non-loft appended to it ran clean
through the same command. It surfaced only because an instrumented probe printed nothing.
The control that settles it is a copy that CANNOT run — corrupt the file you are pointing
at, and if the run stays clean the flag was never honoured. Then either work from a
directory with no `lib/` of its own, or point the entry script somewhere else; loft#1352
tracks the precedence, with loft#930 and loft#963 as the two other ways the same flag goes
missing.

**A bare `--native` run does not leak-check, so "native is clean" is a control that never
fired — and reporting it as a BACKEND DIVERGENCE puts a false claim in the tracker.** The
native leak check is opt-in (`LOFT_NATIVE_LEAK_CHECK`, emitted by
`generation/mod.rs::NATIVE_LEAK_CHECK_TAIL`); with it unset the generated binary never
looks, so it prints nothing whether or not a store leaked. DEBUG.md documents how to arm
it; the trap is one step further on, at the moment the unarmed silence is written down as
a measurement. Measured 2026-09-04 on loft#1344: a hand matrix over six consumers scored
`interpret leak / native clean` for four of them, and the issue was filed claiming the two
backends disagree. `make falsify` — which arms the variable — reported the native leak on
the same guard, and re-running the matrix with `LOFT_NATIVE_LEAK_CHECK=1` gave leaks at
exactly the same four consumers on BOTH backends. The divergence never existed. Two things
follow: arm the variable before any native leak cell, and treat a disagreement between your
own comparison and `make falsify` as the tool being right until you have shown otherwise —
it builds and runs the control the way the suite does, which is the whole reason it exists.

**A probe that FAILS TO PARSE reads as a silent pass, and the "did it run?" check can be
fooled by the error itself.** Measured 2026-09-02 while crossing the six defended-fault-site
spellings of `D-op-5`: four cells carried a literal `\n` into the loft source, failed to
parse, produced no log record — and were scored "ran, and silent", which is the shape of a
PASS. The guard against that is a RAN column, but the first one did not work either: it
grepped the output for the program's own marker, and the parse error ECHOES the offending
source line, so `print("g6")` appeared in the failure text and the cell reported that it had
run. Score RAN on something the program cannot forge — a `^error` line, or an exit code —
never on its own output.

**The before/after oracle has to PREDATE the defect, and the released binary often does not.**
The installed release is the usual before-half (the installed release as the before/after oracle),
and it answers nothing for a bug introduced after it shipped. Measured 2026-09-02 on
`D-clo-14`: 2026.8.0 showed no leak and the CURRENT tree showed no leak, which reads as
"closed" and is really "the oracle predates the bug". What settled it was a second control —
a DIFFERENT, still-open defect run through the same channel (`D-own-16`'s shape reports
`kt=81 SN×4`), proving the channel fires. When the before-half and the after-half agree,
check that the before-half could ever have disagreed.

**And the channel itself can be the wrong one.** `D-clo-14`'s leak is freed at FRAME exit, so
`Warning: N stores not freed at program exit` says nothing about it; the defect is unbounded
PEAK growth inside the frame, and `LOFT_ALLOC_SITES=1` shows 389 live stores at N=400 where
the exit check shows zero. A guard on the exit channel would have been green for the life of
the defect. Read the entry's own numbers — this one says "peak 4 -> 403 at N=400" — and
reproduce THAT measurement, not the one your instrument happens to offer.

`make falsify` says INERT for both, and **INERT is the correct answer there** — the harness
cannot see the channel, so neither tree can differ. Do not widen the guard until you have
asked which run reports the thing you are guarding. The homes that DO see them:

| what you are guarding | where it can fail |
|---|---|
| a leak | a `.loft` file in `tests/leak_cases/clean/` — `tests/leak_cases.rs` runs it as a plain program on BOTH backends |
| script classification | `src/script.rs`'s unit tests (`is_script` / `split_top_level` directly), plus a CLI test in `tests/script_mode.rs` |
| anything else the CLI decides before parsing | a Rust test that spawns the binary, as `tests/panic_halts_both_backends.rs` does |

A `tests/scripts` file can still be worth keeping beside those — loft#1271's is, because
`script::tests::no_corpus_file_classifies_as_script` sweeps `tests/` and names it with the
pre-fix scanner restored. But then the file's falsifying power is that it SITS THERE, not
that its rows run, and its header has to say so.

**An ad-hoc `--native` run LINKS `libloft.rlib`, and `cargo build --bin loft` does not
rebuild it.** So the loop *edit → `cargo build --bin loft` → `loft --native probe.loft`*
compiles the probe with the NEW compiler and links the OLD library, and the answer reads as a
measurement. Measured 2026-09-01: a whole native verification of a store-lifetime fix was
taken this way, looked green, and the regression it hid — `1114`'s named twin reading `7` from
a recycled slot after a capture was freed — surfaced only from `make ci`.

The tell is what makes it convincing rather than suspicious: **`--interpret` runs inside the
binary and is always current, so the two backends "agree" precisely because one of them is not
being tested.** That defeats the repo's own *"verify on BOTH backends"* rule by making the
cheap way to obey it dishonest — an agreement oracle is only as good as the guarantee that
both sides are live, and a stale rlib silently turns a differential test into a single-backend
one that reports as a double.

`make check-rlib` is a one-second pre-flight that says so, and it is worth running before
treating ANY ad-hoc `--native` result as evidence, not only before a bare `cargo test`. The
iteration loop for codegen or store-lifetime work is `cargo build --release --lib --bin loft`.
Note this is NOT the native artifact cache, which keys on `BUILD_ID` and flips correctly on a
source change — it is the rlib alone.

**A CONTROL cell scored in the same file as the thing it controls can blank every channel
`falsify.sh` reads.** A control usually fails on the pre-fix build too — that is what makes it a
control — and if it fails LOUDLY it fixes the file's exit code at the same value on both trees.
`falsify` then reports INERT and the guard measures nothing, while looking like a guard with a
control in it. Measured twice in one day: loft#1211's refusal file scored a dense `const` control
beside the cell, so both trees read `1|0|none|none` and the movement disappeared; loft#1212's five
cells mixed three SILENT wrong answers with two that already reported (an ICE and an arithmetic
message), and the two loud ones pinned the exit at 1 and hid the other three entirely.

Split by the CHANNEL that moves, not by the story: the cells whose answer is silent go in one
file, where the exit moves 0 → 1 (or the panic clears); the cells that already reported go in
another, whose `@falsified-at` records the diagnostic identity instead and says plainly that
`falsify` reads INERT for it. Removing a control is not always a loss either — a build where the
mechanism dies outright still fails an `@EXPECT_ERROR` file on its own unmatched annotation, so
the control's job is often already done by the harness.

**`make falsify` scores an annotation-gated file through its EXPECTATIONS, not its exit.** A
passing `@EXPECT_ERROR` guard exits 1, so the exit channel reads *"THIS TREE IS NOT CLEAN"* on
both trees and can never move — which is why the tool used to print NOT FALSIFIED for a guard
that was working. loft#1224 added the `refusals` and `expect` columns for exactly this, so the
verdict can now come off a channel that moves: `interpret expectations matched 4/6 -> 6/6` is a
falsification, and the unmatched cells name themselves.

**And the whole file is scored, because the run goes through the SUITE (loft#1253).** An
annotation-scored guard used to be run as a plain program, on the reasoning that only a direct
run PRINTS the diagnostic being compared. True, and it does not follow: `Parser::parse` runs
pass 2 only when pass 1 finished clean, so ONE pass-1 refusal silences every pass-2 diagnostic
in the file, and a guard mixing the two scored `expect 1/5` with all five cells matching — a
number not merely incomplete but readable as its own opposite. `tests/wrap.rs` has peeled that
since loft#1242 (attribute each error to its function, blank that cell, re-parse, check the
UNION), so `falsify.sh` asks the suite instead of re-deriving it. Under `--tests` the EXIT
channel moves as well: a file whose declared errors all occur exits 0, one with an unmatched
declaration exits 1. The column now reads `FAIL/6 -> 6/6` — the suite's verdict and a declared
count, never a guessed fraction, because a guessed fraction is what did the damage.

**The GATE has the same failure mode as a guard, and its verdict line is the channel.**
`make ci` printed `CI-RESULT: ALL GATES PASSED` beside `error: could not compile` — its clippy
phase had failed, the run continued into the tests, and the success line was emitted anyway.
Reading that line, or the `4537/4537` count beside it, measures the test phase alone. Check
`grep -c "^error" result.txt` as well; CI_BUDGET.md § `CI-RESULT` carries the detail and the
`-D warnings` half (a bare `cargo clippy` does not show you what the gate denies). The general
form is the one this section is about, one level up: **a channel that reports success while
measuring nothing is not less likely because the channel is the gate.**

**A doc-comment lands on the NEXT item, so an insert between a doc and its function silently
re-parents it.** Three times in one day: `lhs_base_var`'s and `declared_range`'s docs had both
drifted onto `range_default`, and fixing that I twice created the same thing — a new helper
inserted above `default_native_value` and above `construct_move_rewrite` took their docs with
it. The reader then lands on a function described by someone else's paragraph, and nothing
fails. `clippy::doc_lazy_continuation` catches only the sub-case where the stolen doc ends in a
list item. **When inserting a function, put it after the previous item's `}` and before the next
item's doc — not before the doc you happen to be reading.**

⚠ **`FAIL/1` on a guard the suite runs green was the READER of that column, not the guard.** The
suite pluralises its own noun — "1 expected error", "6 expected errors" — and the pattern that
scraped the count matched only the plural, so every guard declaring exactly ONE expectation
scored `FAIL/1` on both trees while passing. Twenty-four of the corpus's guards declare exactly
one. Fixed 2026-09-01; the lesson generalises past the regex, because the column a reviewer
consults to find out whether a guard is live is the last place an unread failure can hide.

**A boolean prints `false` for every byte that is not `1`.** So a garbage `u8` in a boolean
slot renders exactly like the right answer, `!b` reads `true` for it, and only `b == false`
separates them — the two spellings a reader reaches for first are the two that hide it.
Measured on loft#1254's empty-body stub, where the value printed `false` on both backends
while `b == false` was FALSE on the broken one. **Assert the COMPARISON, not the rendering**,
whenever the subject is a boolean; a guard grepping stdout for `false` passes on the defect.
The same caution reads across to `character` (codepoint 0 renders as nothing) and to any type
whose formatter maps several bit-patterns onto one glyph.

**Two `@EXPECT_ERROR` cells declaring the IDENTICAL substring make each other vacuous.** The
suite matches declarations against the UNION of every parse round (loft#1242), not cell by
cell, so either annotation consumes either diagnostic: corrupt one and the other still
satisfies the file. Measured — a guard's two `const` locals were both named `u`, so
`Cannot modify const variable 'u'` was declared twice, and the cell added for loft#1252 passed
while proving nothing. **Give each cell a distinguishable subject** (rename the variable, the
function, the field) so its declared substring can only be satisfied by its own diagnostic.
The hand check below is what finds this; reading the file does not, because two cells that
differ in the shape being tested can still be identical in the sentence the compiler prints.

**A guard of expected failures still needs its non-vacuity proved by hand**, and falsify cannot
give you that in either direction: replace each expected substring in turn with a word the
compiler never prints, check the suite fails, restore it. Five for five is the proof; the
`@falsified-at` note is the place to record that you ran it.

**The fallback has the same shape as the wrong answer.** `b.c ?? []` on a nullable
collection field took the wrong branch and answered the empty field: length 0, which is
exactly what the correct `[]` default answers too. Five guards covered that field shape
(`909`, `917`, `920`, `922`, `936`) and every one of them writes `?? []`, so all five passed
through the whole life of loft#1120. **Choose a default the wrong answer cannot imitate** —
a default of length 2 separates an empty arm (0), a present arm (1) and the default (2) in
one read. The natural default to reach for is the type's zero, and the zero is very often
also what the bug returns.

**The fixture is smaller than the quantum being measured.** @PLN146 F2 gates "a range read
fetches only the pages the keys touch", and read green because the 19 720-byte pack is a
third of one 64 KiB page — a paged read of it *is* a whole-file read, correctly, so the gate
could not have failed whatever the code did. Every resource measurement has a quantum (a
page, a block, a frame, a tick); a fixture below it reports the fixture's size, not the
code's behaviour. Padding to ~4 MB took the same gate to 9 % of the file, 2 pages per key.

**The control does not fire.** Removing `pick_in`'s viewport guard left every test green:
the probe point sat where the camera missed the target anyway, so it answered `-1` with or
without the guard. The cell asserted the right conclusion from a condition that was never
load-bearing. **Assert the precondition first** — that the mark IS under the point — so the
only thing left saying no is the guard under test.

**A control reads the same on both trees for OPPOSITE reasons.** The "does not fire" trap has a
harder sibling: a control cell whose reading is identical before and after, arrived at by two
different mechanisms, so it can never say which tree it is on. A cell written for loft#1241 —
*"with the append inside a loop the fold is declined, so the local must keep its slot"* — reads
"slotted" on the fixed build because the fold is declined, and reads "slotted" on the broken one
because the fold happened and left a stale dep behind. Passing on both, it is a control in name
only. Before writing a control, name what it reads on the CONTROL tree and check that against the
build, exactly as for the guard itself; where the two readings coincide, say so and put the
control where it can differ (that one moved to the `.loft` guard, which scores the ANSWER and
falsifies with six assertion failures on the control build).

**Every cell reaches the same SEAM.** A guard can sweep values, types and directions
thoroughly and still ask one question, because all of its cells enter the machinery at the
same place. `tests/scripts/25-nullable-narrow-implicit-checked.loft` pins the implicit
checked narrowing into a nullable narrow target across four functions, an `if` in each,
in-range and out-of-range arms and two source types — and every one of them is a RETURN.
A return, an argument and a struct-literal field all reach that rule through `convert`; an
annotated local assignment does not reach `convert` at all. So `d: u8? = p + 10` kept a
value outside its own declared range for seven weeks under a green guard whose author had
plainly enumerated (loft#1246). **Ask a guard which SEAM each cell enters, not only which
value it carries** — the seams a store can enter are countable (local, field, element,
struct literal, argument, return, compound), and a guard that names them is checkable
against that list, while one that varies values inside a single seam is not.

**The predicate has no reachable negative.** A predicate that cannot be false in your
program cannot be wrong in a way anything notices. `document.fonts.check()` answers *true*
for a family nothing declares (no unloaded face ⟹ vacuously satisfied), so the browser text
bridge's "does this page have the font?" was inverted in exactly the case it existed for,
for two years, silently.

**A count is asserted where identity is meant.** Two entries and two entries naming ONE
record have the same length. `901-linked-group-fill.loft` scores a key sum and a name-byte sum for that
reason (`k == 3` for 1 + 2, which two entries naming one record cannot reach), and `1159-…`
asserts length, key sum and a real lookup together — the defect it guards left `index` reading `len` 1 and iterating 1 over a
structure in which no key was findable.

**The oracle is a hand number where a working sibling exists.** Where one spelling of an
operation is already correct, score the new one against IT, not against a number you wrote
down: `1159-…` and `1160-…` compare the bulk and binding write routes to the element-wise
and direct-field spellings cell for cell. A hand number freezes your model of the answer;
the sibling spelling freezes the language's.

**The guard's own harness never ran it.** A `tests/scripts/` file with no `main` runs
nothing under `--interpret` and still exits 0 — read the assertion COUNT, not the exit code.
The leak channel is scored only by the `wrap.rs` harness, so a leaking guard reads green
under a bare `loft --tests`.

**A hand-built matrix is not the adversarial gate.** A matrix tests the shapes you thought
of. Relaxing `inline_struct_return`'s `dep.is_empty()` guard passed every hand-built cell —
7 shapes, both backends, values and leak channel, a 1000-iteration use-after-free probe
under `LOFT_POISON` + `LOFT_STRICT_STORES` — and `tests/ownership_fuzz_gate` failed it
immediately, then falsified three successive narrowings of it (loft#1118). Run the
project's generative gates before believing a lifetime change.

**A lifetime matrix scored without `LOFT_STRICT_STORES=1` is not scored.** Three cells that
read "ok" in loft#1143's matrix were use-after-frees reading freed bytes. Re-score every
leak or lifetime matrix under it before believing a green.

**Fixing a write can move the silence rather than remove it.** When a fix REDIRECTS a write,
assert the read through the OLD name as well as the new one. loft#1160's first fix sent the
record to the right field and every subject-side cell went green, while reading the binding
back inside the block that wrote through it still answered 0.

**A passing cell can pass for no reason, and then the axis it establishes is fiction.**
loft#1201's matrix had `xs.map(pair)` clean beside a faulting `xs.map(|x| …)`, which reads as
a lambda-vs-named-function boundary. The named cell was clean because its yield slot carried
a dep whose index was a callee ATTRIBUTE number resolved against the CALLER's variable table —
adding two unrelated locals to the caller moved that dep onto a `text` local. Non-empty was
all that suppressed the free. The real axis was the RETURN FORMER (struct clean, vector
broken), one the matrix never varied. **Test a control's cleanliness the way you test a
failure**: perturb something the fact should not depend on — an unrelated local, a reordering,
a rename — and see whether the cell still passes for the reason you think it does.

**And the mirror: an assertion can read RED while nothing is wrong.** A checker that
over-approximates is described as "stricter, never blinder", which is the right default and is
not free — the cost is a false abort, and on a gate that stops at the first failure a false
abort hides every real finding behind it. `check_text_return_path` counted a free on a loop's
BREAK arm as reaching the code the break jumps over, so
`for v in it { if done { free(v); break; } return v; }` — the shape of every early-returning
loop over a text source — read as freeing the value it returns. It hard-failed the nightly
debug-assertions gate on a program with nothing wrong with it, and the programs behind it went
unchecked for as long as it stood. **Before believing an assertion, check that the path it
names is a path that actually reaches the site**: here the fix was to separate the two ways an
arm can decline to fall through — a `Return` hands its frees nowhere, a `Break` hands them to
what follows the LOOP — because collapsing them either way is wrong in one direction or the
other.

**A green boundary matrix says nothing about a fix's BLAST RADIUS.** The matrix varies the
axes of the DEFECT; a fix's risk is everything else that reaches the same site. loft#1201's
first repair widened two ownership pairings at once: all 30 cells stayed green on both
backends and sixteen test binaries leaked (`placement_parity`, `n2_cdylib`, `leak`,
`leak_cases`, `nullable_ret_buffer`, `ownership_oracle`, `alias_link_baseline` and the script
corpus). Only the suite could find that, so a lifetime change is not verified by its matrix —
the matrix says the defect is closed, the suite says nothing else opened.

**A reverted change is a MEASUREMENT — record what it measured, not just that it failed.** The
instinct when a change does not work is to drop it and move on, and the cost of that is paid by
whoever tries the same thing next. Twice in one day across two checkouts a written-up revert
saved the other author a cycle: a leak note stopped a text route from shipping half-checked, and
a note about which emit path a gate bypasses redirected a free-discipline reading. What is worth
writing down is the CHANNEL that failed it and the cure that looked right and was not — a fix
that is correct on every value cell and wrong on ownership tells the next person exactly where
to aim, while "did not work" tells them nothing.

**A leak fix that silences ONE backend is not a fix.** The interpreter and the native sweep
disagree about an unowned store, so a mark that quiets one can leave the other leaking — and
the value cells, both backends' answers and a thousand-test suite can all be green while it
does. Check `LOFT_STRICT_STORES=1` on `--interpret` AND `LOFT_NATIVE_LEAK_CHECK=1` on
`--native`; one of them alone reads clean for the wrong reason. Measured on loft#1225, where
marking a place-seeded accumulator `skip_free` made native clean and left interpret leaking
three stores, because the real question was who owns the store at all.

**A fix that makes a dropped statement start EXECUTING is invisible on the diagnostic
channel.** Sweeping the corpus for a change's new DIAGNOSTICS can only find programs the change
newly refuses — never programs whose VALUES it changed, which is the whole population when the
fix turns a silent no-op into a real effect. Measured on loft#1221: a ~2000-file sweep for two
new diagnostics reported a clear blast radius, and `make ci` then found a corpus test whose
assertion had been passing BECAUSE of the defect — a linked group filled through both members
doubled its vector once the second append stopped being dropped. The right instrument is a
differential: run the same program on both binaries and compare STDOUT. Reach for it whenever a
fix makes something start happening rather than start reporting.

Its visibility is the second half of the lesson: the keyed member of that group dedups by key
and read the same either way, so only the VECTOR member's length moved. A consumer asserting on
the keyed member sees nothing at all.

The differential has two traps of its own, both paid for. Compare like-for-like PROFILES — a
release-against-debug diff accused that change of a SIGSEGV which was the known release-only
loft#1216. And it is blind to every `test_`-only file: those have no `main`, so `--interpret`
runs nothing in them and they diff clean whatever changed. `tests/wrap.rs` is what covers those,
and it is what caught the case the corrected differential still missed.

**The defect's own EMISSION PATH decides which syntactic position exercises it.** A guard is
written in whatever position reads naturally — usually a lookup straight inside an `assert` —
and that position may never reach the code the defect lives in. loft#1217's broken branch is in
the `Set` emission path, so a keyed lookup written as an argument compiled correctly on the
pre-fix build and `make falsify` read INERT on both backends; the same lookup BOUND TO A LOCAL
first reached the branch and failed. This is the general form of the argument-position trap: do
not ask *"which position is safe?"* but *"which position does the emitter this defect lives in
actually see?"*, and answer it by falsifying rather than by reading. It cost two guards in one
day, both caught only by the tool.

**`make falsify`'s verdict is an AND across backends, so a one-backend defect reads NOT
FALSIFIED.** A native-only fault leaves the interpreter correctly inert, and an
`@EXPECT_ERROR`-scored guard exits 1 when it PASSES — both make the summary line say the guard
did not move while the per-backend rows show that it did. Read the rows, not the verdict, and
record which channel is the real one in the guard's own `@falsified-at` header. Seen three
times: loft#1211 (refusal-scored), loft#1216 (in-process vs binary) and loft#1217 (native-only).

**Split a refusal guard by the PASS its check runs in, not by topic.** A pass-1 error stops the
file before anything gated on `!first_pass` runs, so two refusal cells that look like siblings
cannot share a file when their checks fire in different passes: the second cell's
`@EXPECT_ERROR` goes unmatched for a reason that has nothing to do with the fix under test, and
reads as a broken guard. Measured on loft#1215/#1221, where @PLAN52's ambiguity check is not
pass-gated and fired in pass 1 while the `(N-Store)` cell beside it needed pass 2. Topic is the
tempting axis and the pass is the load-bearing one.

**A parser-global's lifetime can be shorter than the construct it describes.** The shape is
`self.flag = false; parse(); read self.flag`, and it breaks when `parse()` re-enters the same
function for a sub-expression — the nested entry runs the clear again and erases what the outer
one recorded. It is invisible to a guard written with the simplest spelling, because the
simplest spelling has nothing after the construct: `b.d? += […]` kept its discharge flag only
because nothing is parsed after the `?`, while `h?[k] = v` lost it to the index parse
(loft#1214). So a guard for anything flag-driven needs a cell with something PARSED AFTER the
feature, not only the minimal one.

---

**A control that reddens a DIFFERENT test has found a dead gate, not a working control.**
@PLN144 P4's control — accumulate elapsed time as float seconds instead of integer
microseconds — was built to break the 30/60 Hz agreement cell; the suite went red on the
ping-pong cell instead, and chasing that gap proved the 30/60 Hz cell cannot see float
accumulation at all (at 12 fps the two schedules agree at every sample out to 60 000 ticks).
The pass/fail bit is the wrong reading; the NAME is the reading. Write down which test must go
red BEFORE running a control, assert on that name, and when a different one fires, treat the
named test as unable to fail on its own subject. The cure there was a second cell that TICKS
(eight `advance(100000)`) rather than jumps (one `advance(800000)`), because only the repeated
addition can see `0.7999999999999999` land on frame 7 where 8 is right.

**A temp probe named after a HASH of its source shares a truncation window.**
`tests/use_analysis.rs` named each probe file by a hash of its program text, with a comment
saying that is what keeps parallel tests apart — and three tests sharing one `const …_SRC`
resolved to the SAME path. Identical final bytes do not make the WINDOW safe: `fs::write`
truncates to zero first, and under nextest each test is its own process, so one test's spawned
`loft` can open the file mid-rewrite and analyse an empty program — a borrow base derived from
nothing, on whatever schedule the runner picks. Name a probe after the TEST, not its content.

**A stale test binary masks a diagnostic-message change.** After changing a compiler-emitted
message, a local `cargo test --test X` or `find_problems.sh` run can report green while CI's
clean build fails: the test binary was not relinked after the `src/` edit, so inline
`.error("…old wording…")` assertions still matched the old output. It cost two CI round-trips
on one PR. After changing any compiler output, run the affected test binary from a build you
watched relink, or `cargo clean -p loft` first.

**A leak the DRIVER reports beats one inferred from memory growth.** To prove a cursor's
scope-end hook finalises its `sqlite3_stmt *`, ask the library: `sqlite3_close` answers
`SQLITE_BUSY` while any statement on the connection is unfinalised, so the fixture abandons
forty cursors mid-walk, closes, and asserts the connection reported nothing — a return code
computed by the library on every run, and proved to FAIL with `OpDrop` renamed away. It also
caught a second thing the test was not looking for: a round-trip fixture closing a connection
with its cursor live, which refused the close and left the database locked. Before writing a
growth-threshold leak harness, look for the API that already refuses to proceed while the
resource is held (`sqlite3_close`/`SQLITE_BUSY`, `PQfinish`, `close(2)` on a busy fd) and
store the refusal where the caller already looks.

**A per-item marker on stdout cannot attribute a fault printed on stderr.** `tests/wrap.rs`
announces each script with `println!` and every runtime complaint — `BUG (#306)`, warnings,
crash reports — goes to stderr; stdout is block-buffered when it is not a tty, so the
interleaving you read is a buffering artefact that names the wrong script. loft#920 was
attributed to `296-file-error-paths.loft` that way; `LOFT_TRACE_SCRIPT=1` puts the marker on
stderr, and the real answer was `75-native-stub.loft`, whose DELIBERATE panic was the cause.
Put the marker on the stream the fault uses.

**`raise()` is not a fault, so it cannot test a fatal-signal handler end to end.** loft's crash
reporter arms SIGSEGV/SIGABRT/SIGBUS with `SA_RESETHAND`: the handler reports, the disposition
resets, and the default action takes the process — but only a real faulting INSTRUCTION
re-executes after the handler and re-raises. `libc::raise(SIGSEGV)` runs the handler and then
RETURNS, and the child exits normally. Fault for real in a forked child
(`std::ptr::write_volatile(std::ptr::null_mut::<u8>(), 1)`, volatile so it cannot be optimised
into something that never faults) and assert `WIFSIGNALED`.

## Diagnostic tiers — what `--deny-warnings` may fail on

Two tiers, and the difference is contractual rather than cosmetic:

| tier | renders | gates `--deny-warnings` | LSP severity |
|---|---|---|---|
| `Level::Warning` | `warning:` | **yes** | Warning (2) |
| `Level::Advice` | `advice:` | **never** | Hint (4) |

**The rule for choosing: a diagnostic gates if and only if ignoring it can produce a
wrong result.** A lost write, `len(text)` indexed as bytes, a nullable reaching a
non-null slot — those gate. A deprecation, a perf note, a preferred spelling — those
advise.

The split is not a convenience. With one tier the compatibility doctrine contradicted
itself: `revalidate-libs.yml` states that a new deprecation must not fail an
already-shipped library, while that library's own CI runs `LOFT_DENY_WARNINGS=1` and
fails on any warning. `not null` — a deliberate no-op kept parseable so unrepublished
libraries keep loading — therefore made those libraries unable to pass their own CI
without editing code they never touched.

**There is deliberately no `LOFT_DENY_ADVICE`.** The moment advice can gate, cosmetics
block a release and the split has bought nothing.

Writing tests against a tier:

- `Test::warning("…")` / `Test::advice("…")` in `tests/testing.rs` assert the tier, not
  just the text — that is what keeps the split from silently eroding.
- `@EXPECT_WARNING` in a `.loft` script matches **either** tier: it asks whether a
  diagnostic fired, not which tier it landed in.
- `loft test` prints both; only the Warning bucket reaches the deny gate.

### The complexity advice, and why it is counted at parse time

`LOFT_NO_COMPLEXITY` opts out of a nudge when a function's **cognitive** complexity
reaches `keys::COMPLEXITY_ADVICE_AT` (40).

Cognitive, not cyclomatic: a construct costs `1 + nesting`, so depth is what is expensive.
Eight sequential `if`s cost 8; three nested cost 6; a flat `match` costs 1 however many arms
it has. "Many branches" and "hard to follow" are different properties, and a lint that
confuses them fires on every wide dispatch and gets switched off.

**It is counted as the source is parsed, and that is not an implementation detail.** loft has
no AST between the parser and the Value IR, so any whole-program pass sees post-desugar code.
Measured on the IR: five `??` discharges with no author-written branch score 10, and one plain
`for x in v` scores 5 — a reading that charges people for using the null model and the loop
forms idiomatically. An IR version was built and measured before being discarded; the numbers
are in the commit that added `Parser::complexity`.

Two calibration facts worth keeping:

- The boundary is set from the corpus, not chosen: over 5,972 functions of real loft the
  distribution runs p50 1, p90 15, p95 27, p98 47. 40 speaks for ~3%.
- The score is charged on **pass 2 only** — the parser runs twice, and charging both doubles
  every score (eight flat `if`s read 16). The nesting counter still tracks on both passes, or
  pass-1 bodies are charged at a stale depth.

Also discarded on evidence, so it is not re-derived: a live-interval "cut point" signal (no
variable crosses a boundary ⇒ two independent halves). It fires on 45% of long functions and
flags a one-line vector add, because the absence of a spanning variable is a property of
sequential evaluation, not of separable logic.

### The interface nudges — parameters and default values

Two more advices sit beside the complexity one, deliberately SEPARATE from it and from each
other, because they measure different burdens with different fixes.

**`LOFT_NO_PARAM_COUNT`** — 8 or more REQUIRED parameters (`keys::PARAM_ADVICE_AT`).
Parameters with a default do not count (they cost a caller nothing) and neither do
compiler-injected hidden ones (`__retbuf`, work buffers). Folding this into the complexity
score was measured and rejected: `th_subdiv` takes 12 required parameters with a complexity
of **2** — trivial to read, hard to call — so at +1 per parameter it scores 14 and stays
silent, missing the very case that motivates the check. It would also make the complexity
message untrue, since most of such a score would not be control flow. 86% of real loft takes
4 or fewer; `>=8` is 2.1%.

**`LOFT_NO_DEFAULT_HINT`** — 2 or more TRAILING booleans, none defaulted
(`keys::BOOL_FLAG_ADVICE_AT`). This one advertises a feature rather than reporting a fault,
and the trigger is deliberately conservative: one trailing flag is idiomatic (1.0% of real
loft), 96.9% of functions have none at all, and `>=2` covers 2.1%. A nudge that fired on the
common shape would be suppressed, taking the feature it was advertising with it.

It goes quiet the moment it is taken — a function whose trailing booleans already have
defaults does not fire. That property is what separates a nudge from nagging, and it is
asserted rather than assumed.

The message states that adoption is free under the compatibility promise, because that is the
part people do not know: giving an existing parameter a default is **additive**, so every call
that passes it today keeps working unchanged and new calls may omit it.
