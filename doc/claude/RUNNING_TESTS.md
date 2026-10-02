<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Running the tests and reading what failed

How to run loft's own test suite without wasting a cycle, which part of it a command runs, and
how to read a failure once it is in front of you — the dump, the log presets, the per-kind fixes.
Where a test goes and how to write it: [TESTING.md](TESTING.md).  What stops a run that goes
wrong: [RUN_BOUNDS.md](RUN_BOUNDS.md).

| Question | Section |
|---|---|
| Is my tree ready for a suite run? | [Before the suite](#before-the-suite-make-check-rlib-one-second) |
| How do I start a run and wait for it? | [Agent](#-if-you-are-an-agent-run-the-blocking-form-and-let-the-harness-background-it) · [human](#preferred-shape-human-at-a-terminal--background--peek--wait) · [`make ci`](#waiting-for-make-ci) |
| Which tests does a command run? | [What a test run selects](#what-a-test-run-selects-find_problemssh) |
| One test, fast | [Running the Tests](#running-the-tests) |
| Why did this test fail? | [Additional Output Files](#additional-output-files) · [LogConfig](#logconfig--debug-logging-framework) · [`tests/scripts/` failures](#debugging-failures-in-testsscripts) |
| Did a test get slower? | [Test speed](#test-speed--a-report-never-a-gate-make-speed) |

## Reach for this first — efficient failure triage

After a change expected to touch many tests, do NOT iterate "run → see one failure → fix →
re-run → see the next failure": each cycle pays the compile and test-startup cost and finds one
failure.  Run one `--no-fail-fast` pass and read all the failures once —
[One-pass-find-all-problems workflow](#one-pass-find-all-problems-workflow) is how.

### Before the suite: `make check-rlib` (one second)

The tests link a built `libloft.rlib`, and **nothing in an ordinary edit loop rebuilds one**.
`cargo build --bin loft` refreshes the binary and leaves every rlib behind, so a session that
iterates on the compiler with `--bin loft` drifts, and the drift is invisible until a gate runs.

There are **three** of them, one per link target, and they drift independently — refreshing
one does nothing for the other two:

| rlib | linked by | cure |
|---|---|---|
| `target/release/libloft.rlib` | `--native`, the cdylib tests | `cargo build --release --lib` |
| `target/loft/html/wasm32-unknown-unknown/release/libloft.rlib` | `--html` | `loft cache warm`, or `cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random --target-dir target/loft/html` |
| `target/wasm32-wasip2/release/libloft.rlib` | the wasm library suite | `cargo build --release --target wasm32-wasip2 --lib --no-default-features --features random` |

A stale rlib does not fail like a compile error.  It surfaces minutes into a run as a handful of
tests failing for what look like unrelated reasons — `libloft.rlib not found for this build`, a
cdylib mtime that did not advance, a `--html` build panicking — each naming a file that is
present when you go and look.

`make ci` and `find_problems.sh` build what the selected tests link, so they need no
pre-flight.  **A bare `cargo test --release` builds none of them**, so check before one — BEFORE
the suite, because afterwards the check can only tell you the cycle was wasted:

```bash
make check-rlib          # all three, each with its own cure; skips a target that isn't installed
```

The general rule, for this and every instrument: ask what it did NOT refresh.  A partial
refresh that reports its successes misleads more than one that refreshes nothing.

### Waiting for `make ci`

Start and wait on a gate through `scripts/ci-run.sh`, never on the files it writes:

```bash
scripts/ci-run.sh start     # refuses when a gate is already running here
scripts/ci-run.sh status    # NOT-STARTED / QUEUED / RUNNING / DIED / the verdict, with its age
scripts/ci-run.sh wait      # exits once, when a verdict exists — or the run DIED
```

It records the run's own PID in `.ci-verdict` and checks the process, because the files lie
in both directions: a wait armed as the gate launches reads the PREVIOUS run's `result.txt`
before the new run has truncated it, and a run that is killed leaves `.ci-running` behind and
the last run's `CI-RESULT` line in place.  `result.txt` opens with a
`== make ci | <rustc> | <UTC timestamp> ==` header, so a verdict read by hand can be dated.

A gate killed at the same second as an unrelated waiter was killed from outside — usually a
sibling checkout's `pkill -f`, which is why nothing here kills by name.  And `grep -c` exits
**1** when the count is zero, so `grep -c "^ *FAIL" result.txt` as the last command of a status
check reports a clean gate as a failed command; add `|| true`.

### ⚠ If you are an AGENT: run the BLOCKING form and let the harness background it

There are **two** backgrounding mechanisms and using both cancels the notification.  The
script's `--bg` detaches a subshell and returns in seconds; a harness that watches the
*command* then sees a fast exit and has nothing left to report, so the agent is left polling.
The harness's own background mode watches the PID it started and fires a completion event.

So an agent wants the **blocking** command, backgrounded by the harness:

```
Bash(command="./scripts/find_problems.sh", run_in_background=true)   # curated, ~70 s
Bash(command="scripts/ci-run.sh start && scripts/ci-run.sh wait", run_in_background=true)   # full gate
```

One process, one notification, and the summary still lands in `/tmp/loft_problems.<id>.txt`.

⚠ **Never wait with a `pgrep` loop.**  `until ! pgrep -f "cargo nextest"; do sleep 5; done`
never terminates: the loop runs inside a shell whose own command line CONTAINS that string, so
`pgrep` matches itself — and a command chained after it never starts, while every status check
says it is running.  Poll with the script's `--peek` (reads state, not process tables) or
`--wait` (watches the recorded PID).

### Preferred shape (human, at a terminal) — background + peek + wait

```bash
./scripts/find_problems.sh --bg        # kick it off, returns immediately
./scripts/find_problems.sh --peek      # snapshot of any failures so far
./scripts/find_problems.sh --wait      # block until finished, then summarise
./scripts/find_problems.sh --stop      # stop THIS checkout's --bg run
```

`--bg` starts the run in a detached subshell and writes the raw log to
`/tmp/loft_test.<id>.log`, where `<id>` is a per-checkout tag derived from the repo root — so
sibling working trees can run the script at once without sharing pid, log or summary files.
The summary lands at `/tmp/loft_problems.<id>.txt` and is copied to the stable
`/tmp/loft_problems.txt`; every mode prints the exact paths.  `--peek` tails the live log and
pulls out `FAILED` markers, inline panics and SIGSEGV context.  `--wait` blocks on the
recorded PID, then summarises.

A full run is never blocked on in the foreground of a terminal: a human uses `--bg`, an agent
the harness's background mode.  `cargo clippy` and single-file runs
(`cargo test --release --test issues <prefix>`) stay foreground.

**Runner.**  The script uses `cargo nextest run --release --no-fail-fast --status-level fail`
when nextest is on `PATH` (it parallelises per test rather than per binary), and falls back to
`cargo test --release --no-fail-fast`.  The choice is logged at launch, so a change in suite
speed can be tied to it.

**Rebuild step.**  Before the tests it rebuilds, in parallel, the artefacts they link: the
sibling and fixture cdylibs under `lib/*/native/` and `tests/lib/*/native/`, the dev
`libloft.rlib` beside the test-profile `loft`, and — only when a selected binary needs them —
the release `libloft.rlib` with `target/release/loft` and the two wasm rlibs.  It decides by
READING the selected test sources, not by the subject's name; the curated and full runs need
everything.  A stale cdylib otherwise shows up as `cannot find function X in crate
loft_*_native` and a dozen unrelated failures.  `make test`, `make quick`, `make ci` and
`make run-tests` run the same step through the `rebuild-native-cdylibs` target.

Each step's time prints live and collects in `/tmp/loft_timings.<id>.txt`; `--wait` and the
foreground run end with a `=== Wall-clock timing summary ===` block that names what it skipped
and why, so a regressing step is named rather than "the suite is slow":

```
  cdylib lib/graphics/native                        1.910s
  cdylib lib/imaging/native                         0.885s
  ...
  wasm32 rlib                                       0.588s
  (rebuild_native_cdylibs total wall-clock)         1.949s
  cargo nextest run --release --no-fail-fast …    313.479s
```

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

## A cancelled run measures its FIRST failure and nothing else

A gate that stops at the first red is a measurement of one defect.  That is fine when a change
has one, and misleading when a change WIDENS something — a type, a predicate, a rule's reach —
because a widening's casualties are not co-located and each one hides the ones behind it.  One
such widening once cost three gates, one casualty each, in `parse_errors`, `rpc` and `wrap`:
no `--subject` covers those together.

So for a change that widens, run the whole suite `--no-fail-fast` —
`./scripts/find_problems.sh --bg` already does.  The cost is one full suite; the alternative is
learning the casualty count one gate at a time.

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
| `ir:<name>` | `LogConfig::ir_only(name)` | IR only, for functions whose name contains `<name>` (`n_<name>` to disambiguate) |
| `ref_debug` | `LogConfig::ref_debug()` | Full + stack snapshots on Ref/CreateStack ops |
| `bridging` | `LogConfig::bridging()` | Execution + bridging-invariant check |
| `crash_tail` or `crash_tail:N` | `LogConfig::crash_tail(N)` | Last N execution lines (default 50); flushed on panic |
| `fn:<name>` | `LogConfig::function(name)` | Only the named function |
| `variables` | `LogConfig::variables()` | IR + bytecode + variable table per function (no execution) |
| `all_fns` | `LogConfig::all_fns()` | Bytecode of **all** functions including `default/` built-ins; large but essential for diagnosing crashes whose opcode address falls inside a built-in |
| `scope_debug` | `LogConfig::scope_debug()` | Scope analysis to stderr: which reference variables are freed or skipped, and variables whose scope is not in the cleanup chain |
| `alloc_free` | full + `trace_alloc_free` | `[alloc #N at pc=…]` / `[free #N …]` lines, so a `Database N not correctly freed` panic traces back to its allocation by grepping `alloc #N` |
| `poison_free` | full + `poison_free` | Freed text/ref slots are poisoned, so a later read panics with "read from poisoned slot" |
| `locks` | full + `trace_locks` | Every store lock/unlock to stderr with its caller — the first tool for a "Write to read-only store" panic |
| `type_timeline:<var>` | full + `trace_type_timeline` | Every type change of variable `<var>`, naming the site (`LOFT_TIMELINE_BT=1` adds the stack) |

An unrecognised value falls back to `full`.  `scope_debug`, `locks` and `type_timeline` are read
by the module that emits them, so they also work in runs that build no `LogConfig`.

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

The fields and their documentation are in `src/log_config.rs` (`LogConfig`, `LogPhase`); each
preset above is a constructor there, and `LogConfig::from_env()` is the one place `LOFT_LOG` is
parsed into a config.

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
| `src/state/mod.rs`, `src/state/debug.rs` | `execute_log` — execution trace with all filters; `dump_code` — per-function bytecode dump |
| `tests/testing.rs` | Creates config via `LogConfig::from_env()`, passes to `show_code` + `execute_log` |
| `tests/wrap.rs` | Same: `LogConfig::from_env()` for docs/scripts file tests |
| `tests/log_config.rs` | Unit tests covering all filters, presets, and pipeline integration |

### Notes for Claude

- The bridging check (`check_bridging: true`) always reports a violation on the FIRST
  instruction of the root test function: `execute_log` places the sentinel return address at
  runtime position 4–7 while compile-time tracking starts at 0.  That one is harmless.
- `crash_tail` mode wraps the execution loop in `catch_unwind(AssertUnwindSafe(...))`;
  if a panic occurs the tail buffer is flushed to the log file before re-raising.

---

## What a test run selects (`find_problems.sh`)

`./scripts/find_problems.sh` defaults to the **curated** set: everything except a short, named
list of slow-and-few binaries — about 97 % of the tests in about a fifth of the time.  The
default is the cheap one on purpose: a full run is something you skip, and a check you skip is
not a check, so the expensive option costs a flag to ask for rather than a flag to avoid.

| | |
|---|---|
| `find_problems.sh` | curated — ~70s |
| `find_problems.sh --full` | every test — ~370s |
| `find_problems.sh --subject <name>` | one area — seconds |
| `find_problems.sh --changed [ref]` | the subjects the DIFF touches (uncommitted edits, or against `ref`) — seconds; falls back to curated, saying why, when the diff touches something every binary depends on |
| `find_problems.sh --list-subjects` | the subjects, and what the default excludes |

Selection flags combine with `--bg` / `--peek` / `--wait` / `--stop`, and the rebuild step
builds only what the selection links (§ [Preferred shape](#preferred-shape-human-at-a-terminal--background--peek--wait)).

### Why it curates by EXCLUSION

Because inclusion does not work.  An additive map ("you touched the parser, run these four
suites") has to predict which suite will catch a bug, which nobody can do in advance: an
over-broad change to the parser's `null()` was caught by `binary_io_matrix`, which no parser
map would have selected.  Excluding instead makes the miss set small, named and auditable, and
the suite's cost is concentrated enough to make that nearly free — a handful of binaries, each
slow AND few (`deliver_wasm` is minutes for a few tests), hold over half the test-seconds.
`--list-subjects` names them.

Nothing that skips can reach `main`: CI's `Test (ubuntu-latest)` runs the suite
**unsharded** and is a required check.  The local default is the fast loop; CI is the complete
gate.

### Subjects

`parser scopes codegen runtime store wasm packages lsp sql docs host`, defined in
`scripts/test_subjects.sh` as binary-name **patterns**, not lists: a list is incomplete the day
it is written, while a pattern picks up new binaries whose names match.  Patterns are expanded
against the real binary list before nextest sees them, because nextest treats a pattern
matching nothing as a filterset parse error, and because an expanded selection can be read back.

Subjects are a convenience for tight loops, **not** the safety mechanism.  The default being
subtractive is what makes it safe to leave them approximate: a gap in a subject costs seconds,
never coverage.

**Every binary matches at least one subject** — `doc_hygiene::every_test_binary_matches_a_subject`
asks the map's own `unmatched_binaries` and fails on a name it reports.  A new `tests/<name>.rs`
either carries a name an existing pattern picks up or extends the map in the same commit; the
failure message names the binary.

`--changed` is the same map read from the other side: `subject_paths` maps a source PATH to a
subject, so the diff picks the subjects, an edited `tests/<name>.rs` picks its own binary, and
an edited corpus file picks the three corpus runners.  That map is deliberately **partial**, and
its fallback makes partial safe: a `src/` path no row claims widens the whole run to the curated
set rather than letting the diff's other paths narrow it.  The rows cover the hot core (parser,
scopes, codegen, runtime, store), so an unmapped file costs breadth, never a false green.  Two
guards hold it up, and they fail on different things:
`doc_hygiene::every_subject_claims_the_paths_it_names` catches a row whose pattern has drifted
off a moved file, and `changed_selects_subjects_by_path` runs `changed_filter` on a synthetic
diff — only the second catches a map that nothing reads.

`make ci` uses the same mapping for ORDER only — `scripts/nextest_priority.sh` hands nextest a
`priority` override so the diff's binaries run first and a red gate says so in its first minute
(the gate is fail-fast); what runs is unchanged.

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

`make test` runs the `clippy` target first (`cargo fmt -- --check`, `cargo clippy --tests --
-D warnings`, `cargo check --no-default-features`), rebuilds the native cdylibs, deletes
`tests/generated/*` and `tests/dumps/*.txt`, then runs
`cargo test --release -- --nocapture --test-threads=1`, appending to `result.txt`.

### Fast-iteration workflow — don't spam the full suite

When iterating on one test family, give cargo a **name filter** so it builds and runs only the
tests you care about.  The filter is a case-sensitive substring match on the test name:

```bash
cargo test --release --test issues q3_to_json                 # the q3_to_json* tests
cargo test --release --test issues q3_to_json_of_jbool_true   # exactly one test
make iter TEST=q3_to_json TFILE=issues                        # the same, on the dev profile
```

`make iter` builds the dev profile, whose incremental rebuild after a one-file edit is about
ten times faster than release's; pass `PROFILE=release` for timing-sensitive tests.  It clears
`tests/dumps/` and `tests/generated/` first, as `make test` does.  Running a whole test binary
(`cargo test --release --test issues`, hundreds of tests) on every edit is the slow shape.

A test that runs the `loft` binary reuses the program's cached parse on a rerun, a dev build
included; a rebuild invalidates it.  A test that must see a cold parse — it times, counts or
traces the parse — sets **`LOFT_NO_CACHE=1`** on its command
([STARTUP_CACHE.md](STARTUP_CACHE.md#default-on-behaviour-and-the-off-switch)).

### Don't stack duplicate cargo invocations

Two `cargo test` runs from one checkout queue on the `target/` build lock, and each pays its own
startup.  The symptoms: output is slow to appear, several copies of one test binary run at high
CPU, and the harness reports "has been running for over 60 seconds" on a test that takes
milliseconds.  **Let a run finish before launching the next.**

If a run hangs, stop YOUR process by its PID — `find_problems.sh --stop` for its own run, or
`kill <pid>` of the test binary you started — never `pkill -f <name>`, which also reaches a
sibling checkout's run.  Then re-run with a narrower filter to find the looping test.  **Do
not** add `--test-threads=1` to serialise the mess; that hides which test loops.

### Diagnosing a hang vs a failure

- **Hang** — the test binary stays live at high CPU past its expected runtime.  Likely causes:
  a loop reading garbage memory (a text whose length field was written as a huge value), a
  format specifier that does not terminate, or recursion with no base case.  Narrow to one test
  (`cargo test --release --test <file> <exact_name>`), then [RUN_BOUNDS.md § Hang guard](RUN_BOUNDS.md#hang-guard-loft_max_ops)
  names the loop.  A hang in a test built with `code!()` and string escapes: move the repro to a
  standalone `.loft` file first, to tell the Rust-escaping plumbing from loft itself.
- **Failure** — the binary completes but the output does not match.  The dump is
  `tests/dumps/<file>_<test>.txt` (debug builds, or when `LOFT_LOG` is set); the `.result(…)`
  check runs after execution, so a failed test has its whole trace.

### One-pass-find-all-problems workflow

`cargo test` stops after the first test BINARY that exits non-zero, so a change that breaks
tests in several binaries shows them one run at a time.  `--no-fail-fast` keeps going across
binaries; `scripts/find_problems.sh` runs that, tees the log, and writes one summary to read
once (§ [Preferred shape](#preferred-shape-human-at-a-terminal--background--peek--wait) for
the modes and paths).  The summary looks like:

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

Read it once, plan all the fixes, apply, re-run.

- The summary says WHICH tests failed; each `tests/dumps/<file>_<test>.txt` says WHY.
- After the fixes, diff the new summary against the old, to confirm the change only closed
  problems.
- A test that is known to fail for a standing reason is not tracked here: it is `#[ignore]`d
  with a reason in `tests/ignored_tests.baseline`
  ([TESTING.md § Every skip says why](TESTING.md#every-skip-says-why-how-it-runs-instead-and-when-it-ends)).

**When NOT to use it:** while working on ONE test family, use the name filter above — faster
feedback, no summary to read.

---

## Debugging failures in `tests/scripts/` {#debugging-failures-in-testsscripts}

### Strategy overview

When a `tests/scripts/` file fails (under `wrap`, or `loft --tests <file>`), work from the
outside in:

1. **Run the failing file directly** — the error names the exact assert.
2. **Stop at it** — `loft debug <file>:<line>` reads and edits the live frame at the failing
   line ([DEBUG.md](DEBUG.md)); prefer it to adding `print` lines.
3. **Narrow to the failing assert** — cut the file down to the smallest program that still fails.
4. **Run via the Rust test framework** — the minimal case as `expr!(...)` in a `tests/*.rs`
   file gets `LOFT_LOG` dumps without changing the source.
5. **Use the debug binary** — `cargo build --bin loft` has extra runtime checks, so a segfault
   often becomes a Rust panic.

### Failure types and fixes

#### Assert fires with wrong value

```
Error: assertion failed: my assert message
  --> tests/scripts/<file>.loft:<line>:<col>
```

The message is the second argument to `assert()`.  Common causes:
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
- A value of the wrong layout where the runtime expects another (e.g. a struct-enum variant
  used as a plain enum).
- A construct the compiler accepts but the runtime does not implement, falling through to an
  unreachable branch.
- Remove the suspect line; if the segfault disappears, the line triggers the bug.

#### Parse error — "Dual definition of"

```
Dual definition of <name> at file.loft:line:col
```

A name is defined twice where one definition is expected.  loft overloads functions by
parameter type, so two functions with IDENTICAL parameter types are this error.  (A second
struct of the same name reports *"struct 'P' conflicts with a struct of the same name"*; two
structs sharing a FIELD name are fine — field lookups are scoped by type.)

#### Parse error — "Undefined type"

```
error: Undefined type string — did you mean 'text'?
```

The name is not a type in this program: a misspelling, a type from another language
(`string`, `int`, `long`), or a type from a library the file does not `use`.  Order does not
matter — a type may be used above its definition.

#### Wrong result from index range query

A range on a multi-key collection is ONE interval over the composite key, in key order
(`@FR-Slice-KeyedIter`).  `db.map[83..92, "Two"]` is the interval from key `(83)` to key
`(92, "Two")`, so every element whose first key lies inside it is returned, whatever its second
key — the trailing key bounds the END of the interval, it does not filter.  Write the expected
sequence out by hand in key order before writing the assert; to select on the second key, test
it inside the loop.

#### Compile error — "Cannot add elements to '...' while it is being iterated"

```
Error: Cannot add elements to 'v' while it is being iterated — use a separate collection or add after the loop
Error: Cannot add elements to a collection while it is being iterated — use a separate collection or add after the loop
```

This is a deliberate compile-time guard. Appending to a collection during iteration is
unsafe: vectors re-read their length on every step (so new elements are visited, risking
an infinite loop), and sorted/index insertions corrupt stored iterator positions.  It covers
both direct mutations (`v += x`) and field-access mutations (`db.items += x`).

**Fix options:**
- Collect additions in a separate variable and append after the loop: `extra = []; ... for e in v { ... extra += [x]; } v += extra;`
- Remove elements during iteration with `e#remove` in a filtered loop — this is the one safe in-loop mutation.

#### Wrong iteration order in sorted/index

Verify the sort direction: `-field` means **descending**, `field` means **ascending**.
A mismatch between the declared direction and the expected order is the most common mistake.
Trace the expected element sequence manually before writing the assert.
