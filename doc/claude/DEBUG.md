
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debugging Strategy

The primary debug surface is the `LOFT_LOG` environment variable, which selects a
preset defined in `src/log_config.rs`. Set it before running a test:

```bash
LOFT_LOG=minimal cargo test -- my_test 2>&1 | head -200
LOFT_LOG=ref_debug cargo test -- my_test 2>&1 | head -500
LOFT_LOG=full cargo test -- my_test 2>&1
```

---

## Contents

- [Interactive debugging (`loft debug`)](#interactive-debugging-loft-debug)
- Moved to their own guides: [`LOFT_LOG` presets and traces](DEBUG_TRACE.md) · [probe matrices](DEBUG_PROBES.md) · [introspection CLI](DEBUG_INTROSPECT.md) · [store faults](DEBUG_STORES.md) · [index and viewer](DEBUG_VIEW.md)
- [Debugging a Parse Error or Wrong IR](#debugging-a-parse-error-or-wrong-ir)
- [Debugging a Runtime Crash or Wrong Result](#debugging-a-runtime-crash-or-wrong-result)
- [When it fails in CI but passes locally](#when-it-fails-in-ci-but-passes-locally)
- [Debugging a validate_slots Panic](#debugging-a-validate_slots-panic)
- [Debugging a Scope Analysis Bug](#debugging-a-scope-analysis-bug)
- [Read the fix back against the cause](#read-the-fix-back-against-the-cause-before-you-build-either)
- [Reading a defect — the shapes that keep recurring](#reading-a-defect--the-shapes-that-keep-recurring)
- [Using the Test Framework for Quick Iteration](#using-the-test-framework-for-quick-iteration)
- [Open work](#open-work)

---

## Interactive debugging (`loft debug`)

**Before adding `println`s to a loft program: there is a debugger.** It stops the
program at a line and lets you read and change the live frame — which is what
print-and-re-run is a slow approximation of.

```sh
loft debug prog.loft:12            # break at line 12, drop into the `(dbg)` prompt
loft debug prog.loft:12 --lib lib/ # a program whose `use` resolves through --lib
```

At the prompt: type a **name** (or any expression) to evaluate it at the frame ·
`name = <expr>` to CHANGE a local and carry on with the new value · `:vars` to
re-show the frame · `:step` / `:next` / `:finish` (into / over / out) · `:continue`
· `:watch <expr>` to run until an expression changes · `:undo` / `:redo` to walk
your edits · `:help` · `:quit`. Verbs also work bare (`step`), except when the
frame has a local of that name — then the local wins, so `n` and `c` read your
variables rather than stepping.

It is **driveable non-interactively**, which is what makes it usable from a
script or an agent rather than only by hand:

```sh
printf ':vars\ntotal\n:next\ntotal\n:continue\n' | loft debug prog.loft:12
```

For a scripted session with structured output, use the NDJSON RPC surface
(`loft debug prog.loft --rpc`) — breakpoints with conditions, `eval`, `setValue`,
stepping and tracepoints over stdio. The **`loft-debug` skill** § *The agent debug
surface* is the canonical guide with a worked example; the wire contract is
[plans/16-debugger/PROTOCOL.md](plans/16-debugger/PROTOCOL.md). Order matters
there: `launch` loads, `run` starts, and breakpoints go between them.

**A local of a generic type** shows its instance beside a literal naming the template —
`g: Grid<integer> = Grid{cells:[1,2],w:1}` — because the literal is what an evaluated
expression is seeded from, and `Grid<integer>{…}` is no expression the parser reads; the
seed carries the same annotation.  A frame inside a generic function is named by the
function (`peek`), not by its instance's key.

**What `:vars` shows, and the two markers.** A paused frame lists every local in
**lexical scope** at that line. A local in scope whose value the frame does not
hold is still listed, with the reason instead of a value:

| shown | means | what to do |
|---|---|---|
| `step = <unset>` | in scope, but its assignment has not run yet on this path | break one line later |
| `i = <reused by step>` | in scope, but its stack slot now belongs to `step` | break one line earlier |

The second is not a bug: the slot allocator is **scope-blind**, so two locals in
one scope share a slot whenever their live ranges do not overlap (in
`for i in 0..4 { step = i * 10; total = total + step; }`, `i` and `step` are the
same four bytes). Once `step` has been written, `i`'s value no longer exists
anywhere in the frame — so the debugger names that rather than printing the slot's
contents under `i`'s name. Reading such a local explains itself, and editing it is
refused. Compiler temps (`__work_N`, `i#index`) are hidden; `:vars all` shows them.

Remaining rough edges are tracked in
[@PLN120](plans/120-debugger-shape/README.md).

Editors get the same engine over DAP (`loft-dap`); see [@I91](../features/I91.md).

---

## Moved: The `LOFT_LOG` preset guide, the debug dumps and function-level IR / trace inspection

See [DEBUG_TRACE.md](DEBUG_TRACE.md) for the `LOFT_LOG` preset guide, the debug dumps and function-level IR / trace inspection.

## Fast iteration loop — `make iter`

For day-to-day "fix one bug, run one test" cycles:

```
make iter TEST=p197                        # all p197* tests
make iter TEST=p194 TFILE=issues           # only p194* in tests/issues.rs
make iter TEST=introspect TFILE=exit_codes # only introspect* in tests/exit_codes.rs
```

`make iter` runs `cargo test` filtered to `$(TEST)`, optionally
restricted to one test binary via `$(TFILE)`.  Defaults to the
**dev profile**, which is specifically tuned in `Cargo.toml`:

- `[profile.dev]` `opt-level = 1` (basic inlining)
- `[profile.dev.package.loft]` `debug-assertions = false`
  (skips the hot-path `Store::addr` / `keys::store` guards that
  add ~270x overhead to interpreter-heavy tests)

Measured here:

| Scenario | Dev profile | Release profile |
|---|---|---|
| Warm cache, no source change | ~0.3s | ~0.3s |
| Single-file edit, incremental rebuild | **~2.4s** | ~26.8s |
| Cold rebuild after `make clean` | ~30s | ~60s |

For most edits, dev profile is **~11x faster** on the inner
debug loop.  Tests that depend on release-only behaviour
(parallel timing windows, perf assertions) take `PROFILE=release`:

```
make iter TEST=par_throughput PROFILE=release
```

Sharing cache with `make test` / `make ci` (both release) means
switching profiles forces a one-time rebuild.  Within a single
debugging session, pick one and stay on it.

`make iter` cleans `tests/dumps/` and `tests/generated/` before
running — they pin per-test codegen output, and stale fixtures
across profile/test-set changes can produce bogus errors
(e.g. `attempt to add with overflow` from u16::MAX placeholder
positions).  This mirrors what `make test` already does.

### `mold` linker (committed default on Linux)

`.cargo/config.toml` activates `mold` on `x86_64-unknown-linux-gnu`.
Linker time is a small fraction of the rebuild (LLVM codegen
dominates), so the direct speedup is modest (~1s).  The bigger
win is a **unified cache**: every `cargo` invocation from this
checkout uses the same `RUSTFLAGS`, so alternating between
`cargo build`, `cargo test`, and `make iter` shares one cache
key.  Without this pin, ad-hoc `RUSTFLAGS=...` overrides force
rebuilds.

First-time setup on Linux x64: `sudo apt-get install mold`.
The CI workflow installs mold on its ubuntu runner.  macOS and
Windows ignore the config (different target triples) and use
their platform-native linkers.

---

## Read the fix back against the cause, before you build either

The cheapest check in this file, and it costs one line of thought. Having written *"the cause
is X"* and *"the fix is Y"*, read Y against X ALONE and ask: **does Y touch the site X names?**

Two failures that check catches, both measured on 2026-09-10 within one hour, both by people
who had the right diagnosis in hand:

- **The fix that cannot reach the cause.** A diagnosis named the SEED — a whole-body walk that
  arms a fact before any branch structure exists — and the proposed cure edited the RECONCILE,
  which runs after. Both halves were locally plausible and sat two paragraphs apart in one
  message, so reading it did not catch it; the rung was built and measured BYTE-IDENTICAL on
  all 19 cells. A green build reads as "the cure is not needed" exactly as easily as "the cure
  works".
- **The fix whose PREMISE was never probed.** A cure turned on *"on the path that ran, these
  two name one record"* — and in loft the plain bind COPIES (`binding.md (B-Copy)`), so they
  never do. The design was sound given its premise and no amount of reviewing the design could
  find that; one probe (`x = a; x.id = 99;` then read `a`) settles it.

So the order is: name the cause, name the site, then check the fix lands ON that site and that
every *"these two are the same X"* in it has been probed rather than assumed. The matrix below
is what settles the question either way — but it is expensive, and these two checks are not.

## Moved: The boundary-matrix runner, the fixed-axis check and counting before timing

See [DEBUG_PROBES.md](DEBUG_PROBES.md) for the boundary-matrix runner, the fixed-axis check and counting before timing.

## Moved: The `--introspect` CLI and the `LOFT_AUDIT_PASS1` refusal audit

See [DEBUG_INTROSPECT.md](DEBUG_INTROSPECT.md) for the `--introspect` CLI and the `LOFT_AUDIT_PASS1` refusal audit.

## Debugging a Parse Error or Wrong IR

1. Add `LOFT_LOG=static` and run the failing test.
2. In the output, find the function that contains the wrong code.
3. Compare the emitted IR (`Value` tree) against what you expect.
4. If the IR is wrong: the bug is in the parser. Search for the relevant `Value`
   variant in `src/parser/` and trace through `parse_single` or `parse_operators`.
5. If the IR is correct but the bytecode is wrong: the bug is in `src/state/codegen.rs`,
   in the `value_code` branch for the relevant `Value` variant.

---

## Debugging a Runtime Crash or Wrong Result

1. Reproduce with the smallest possible loft program (isolate to a single function).
2. Add `LOFT_LOG=minimal` and run. Find the last opcode executed before the crash or
   wrong result.
3. If the opcode is a memory access (`set_int`, `get_int`, `set_long`, etc.) and the
   `store_nr` is a large or unexpected value (like 60 or 0x3C), the DbRef on the
   stack is garbage — the bug is in scope analysis or codegen, not in the opcode.
   Switch to `LOFT_LOG=ref_debug` to find where the bad DbRef was created.
4. If the opcode itself is wrong (wrong opcode for the operation), check
   `src/state/codegen.rs` and the `Stack::operator` delta table in `src/stack.rs`.

### A repro that cannot separate the candidate causes is a coincidence

Reproducing the reported symptom is not the same as reproducing the reported BUG,
and an issue that names two failures usually offers two causes to tell apart.  So
before reading anything, ask what the probe would look like under each — if the
answer is "the same", the probe has not started work yet.

Two things make this concrete:

- **Find the control the environment already gives you.**  loft#1061 filed a placed
  library answering EMPTY and panicking on a large return, as one defect with a size
  threshold.  `--native` is NEVER placed, and it answered empty too — one run, and
  the wire is innocent for half the symptom.  They were two defects sharing a
  symptom.
- **A bug's own mechanism can fake its reproduction.**  The same probe, put in a
  scratchpad, "confirmed" the empty answers on both backends — because loft runs a
  script with CWD = the SCRIPT's directory, so `git` ran outside the repository and
  the library flattened the failure to `""`.  That IS the bug, reached by accident
  from the wrong direction, and it looked like confirmation of the wire theory.
  Put a probe that touches the filesystem or a subprocess **inside the tree it is
  about**.

The general form: *whose* explanation does this run rule out?  A cell that no
hypothesis fails is not evidence, which is the same reason [the matrix
protocol](../../CLAUDE.md) requires a hand-computed expected value per cell rather
than agreement between two binaries.

### When the crash will not repeat — the crash report file

On SIGSEGV / SIGABRT / SIGBUS, `src/crash_report.rs` prints the last opcode, its
bytecode position, the function, and the loft source line. It writes that to
stderr **and to a file**, because a build that pipes stderr through a filter
otherwise discards the one diagnostic that cannot be regenerated: the run that
produced it is by definition the run that will not repeat (loft#717 lost exactly
this, and all that survived the pipe was the header line).

| | |
|---|---|
| Default location | `.loft/loft-crash-<pid>.txt` when a `.loft/` directory already exists (the `loft test` case), else `<tmp>/loft-crash-<pid>.txt` |
| Override | `LOFT_CRASH_FILE=<path>` |
| Turn it off | `LOFT_CRASH_FILE=` (empty) — stderr only |

Nothing is written unless a crash actually fires, and the directory is never
created (a run that does not crash leaves no trace). The stderr report names the
file it wrote, on a line after the diagnostic — so if you see the report but no
such line, the write failed and stderr is all there is.

Reading it back: the `pc:` value is a bytecode position, which
`LOFT_LOG=static` maps to source. If the report says `(none — crash outside
interpreter)`, the fault was not in an opcode — look at `--native` code or a
library call instead.

**`at:` is a lower bound, not an answer — read the `pc+` suffix.** The span table
holds one entry per statement and none at all for regions no statement produced,
and the lookup takes the last entry at or before the crashing pc. When something
is recorded nearby, that IS the site and the line prints bare. When nothing is,
the lookup reaches arbitrarily far back, so the report says how far:

```
  at:      /…/default/05_coroutine.loft:18:27  (nearest span, pc+280 — NOT necessarily this line)
  at:      (no source span covers this pc)
```

Unqualified, that first line sent loft#806's reader into coroutine code the
program never calls. A confident wrong location is worse than none: silence makes
you look, an answer sends you away. So treat any non-zero `pc+` as "the nearest
thing recorded", and `fn:`/`op:` as the reliable pair.

**The `  at file:line:col` line of an internal PANIC is a different half of the
same table, and it is exact.** A panic — the store-lock write guard, a broken
runtime invariant — goes through the Rust panic hook rather than the signal
handler, and prints no `pc+` suffix, so a reader has nothing to discount. It
therefore prints only when a recorded span COVERS the crashing pc, and stays
silent otherwise; the span table carries each entry's pc RANGE so that question
can be answered rather than guessed. Before that it inherited like the signal
path but read like an answer, putting a `#lock` write on the line of the
arithmetic above it and — with nothing preceding it in the user's file — in
`default/05_coroutine.loft` (loft#1262). A wrapped construct still resolves: a
call is wrapped, which is why a failing `assert` and a `panic` still name their
own line.

So the two halves differ on purpose. The signal report keeps the nearest span
because it can label it (`pc+280`); the panic hook drops it because it cannot.

**`last op:` names the opcode**, resolved through a table the interpreter
publishes once per process (`crash_report::set_op_names`) — a signal handler
cannot borrow the definitions table, so the names are made `'static` up front.
The number alone (`op=249`) identifies nothing; the name is what points at a
subsystem, and on loft#806 `OpAppendStackText` did in one line what a matrix of
19 probes had not. Cross-check it against `LOFT_LOG=minimal`, whose trace ends at
the same op by an independent path — that agreement is what calibrates the
reading.

### An unexplained SIGSEGV in a package: suspect the auto-built cdylib

`--interpret` does **not** mean no native code ran. A library package with
`[library] compile = "native"` is auto-compiled to `<pkg>/native-auto/*.so` and
dispatched into even when the script interprets, so `gdb bt` on the core belongs
before any interpreter theory.

The generated cdylib hardcodes type-table **indices** and field **offsets**, so
it is valid only against the exact type table it was generated from. Two
defences keep it honest, and they fail differently:

- The artifact's FILENAME carries the caller's type-layout fingerprint (#715), so
  two contexts never name the same file.
- The artifact also DECLARES the layout it was built for
  (`loft_type_layout_fp_v1`), and the adopter verifies it before use (loft#717).
  A mismatch rebuilds rather than dispatching.

The second exists because the first is an argument, not a check: it holds only
while the fingerprint keeps covering every layout difference and while nothing
else can put a file at that name. When an argument like that fails, the artifact
is not slightly wrong — it resolves indices against a foreign table, so reads
land at wrong offsets and the crash surfaces arbitrarily far from the cause.
That is why the verification is worth a `dlopen`: the failure it prevents is
unattributable by construction.

Suspect this whenever a crash is intermittent, appears under a parallel sweep,
and does not repeat afterwards — the artifact is rebuilt on the next run, so the
evidence deletes itself. `rm -rf <pkg>/native-auto` and re-running is not a
diagnosis; it is destroying the only copy of the thing that crashed.

---

## Moved: Random-looking faults, store-ownership bugs and the `target-da` calibration run

See [DEBUG_STORES.md](DEBUG_STORES.md) for random-looking faults, store-ownership bugs and the `target-da` calibration run.

## When it fails in CI but passes locally

Same commit, same command, opposite result — so the difference is *state*, and the
suspect list is short. A build tree accumulates artefacts CI never has, and one of
them can supply exactly the thing the code fails to find, which turns a real bug
into "works on my machine" indefinitely.

Check, before re-running anything:

```bash
git status --ignored --short target/ | head     # stray artefacts in the build tree
find target -maxdepth 3 -type l                 # symlinks — the quiet ones
```

The worked example: the nightly ASan sweep failed on both runners while the
identical `cargo +nightly nextest … -Zsanitizer=address --target x86_64-…` command
passed locally, including the full 1667-test sweep. The reason was a
`target/x86_64-unknown-linux-gnu/release/default` **symlink to the repo's stdlib**,
left in the tree weeks earlier. It papered over a real defect — `project_dir()`
could not resolve the project root for a `--target` build ([INTERNALS.md §
`project_dir`](INTERNALS.md#project_dir), loft-lang/loft#638) — and it made the
first three hypotheses *unfalsifiable*, because every local control ran against a
tree where the missing thing was present.

Two rules follow:

- **Reproduce by construction, not by re-running.** Build the layout the failing
  environment has (here: copy the binary into a synthetic `<root>/target/<triple>/release/`)
  and vary ONE axis. That produced the CI failure line character-for-character in
  seconds, after ~20 minutes of full-sweep runs had proven nothing.
- **When a local control passes, ask what the local tree is supplying.** A green
  control is only evidence if you know the environment it ran in — otherwise you
  have measured the symlink, not the code.

### Compare the PASSING side, not just the failing one

**A differentiator found by comparing the passing side is worth more than any number of
hypotheses about the failing side.** A hypothesis about why the red job is red must be
disproven one at a time and each costs a round trip; a job matrix where something passes hands
you an axis for free, and one reading of it can retire a whole family at once.

Worked example, loft#1406's macOS leg. Four hypotheses had been offered for
`reclaim_spares_live_and_fresh_files` failing under the ASan gates — the sentinel pid reading
as a live process GROUP, the byte accounting losing a removed file, the sanitizer changing the
allocation shape, the temp directory differing — and reading the failing job could not separate
them. The per-job conclusions could:

| job | result |
|---|---|
| ASan interpreter leak gate (**ubuntu**) | ✅ |
| ASan UAF/OOB sweep (**ubuntu**) | ✅ |
| ASan interpreter leak gate (**macos**) | ❌ |
| ASan UAF/OOB sweep (**macos**) | ❌ |

ASan is on the PASSING side too, so the sanitizer is not the variable — the platform is. That
one comparison retired every "the sanitizer changed X" hypothesis at once, including the two
that were being actively worked, and it cost a single API call. The same reading also showed
the failure is a plain ASSERTION with no sanitizer report at all, which the job NAMES had
concealed.

**The corollary is about instrumentation, not diagnosis.** When a leg cannot be reproduced
locally, a round trip costs a day — so the unit to optimise is *does this run ANSWER the
question*, not *does it narrow it*. Rather than a fifth hypothesis, make the failing assertion
carry its own evidence: the decision INPUTS the code branched on, and the state it left behind.
Then force the assertion to fire once locally, so you know the message is not vacuous — a
diagnostic nobody has seen fire is a hypothesis about a diagnostic.

⚠ **And read the per-JOB log, not the run.** `gh run view --log` / `--log-failed` returns empty
for these runs; `gh api repos/<owner>/<repo>/actions/jobs/<job-id>/logs` works, and
`gh run view <run> --json jobs` gives the per-job conclusions the table above is read from. A
run that reads green overall can carry a red leg, and a job NAME can describe a gate rather
than the failure it caught.

## Debugging a validate_slots Panic

`validate_slots` panics in debug builds when two variables with overlapping live
intervals share the same stack slot. The panic message includes both variable names,
their slot range, and their live intervals.

1. Identify which function and which two variables conflict.
2. Add a minimal reproducer to `tests/slot_assign.rs`.
3. Check whether the live intervals truly overlap (can both variables be live at the
   same time?) or whether `compute_intervals` is computing a conservatively wide range.
4. If the overlap is real: a bug in scope analysis assigned the same slot to two
   simultaneously-live variables. Check `scopes.rs::copy_variable`.
5. If the overlap is spurious (a sequential block reuse): the exemption in
   `find_conflict` may need to be extended.

---

## Debugging a Tricky Compiler Bug (use logging first)

For non-obvious bugs — wrong use counts, unexpected variable lifetimes, closure leaks,
dead-assignment warnings that fire or don't fire — **always add targeted debug logging
before attempting a fix**.

Reasoning alone about multi-pass parser/compiler state is unreliable; logging shows
exactly what is happening.

Pattern:
1. Add `eprintln!` to the tracking function closest to the symptom (e.g. `in_use`,
   `track_write`, slot-assignment helpers).
2. Run the failing test and read the output to confirm your hypothesis.
3. If the call site is still unclear, add `std::backtrace::Backtrace::capture()` at the
   suspicious point and print it. This pinpoints the exact source location.
4. Fix the root cause, then **remove all debug prints before committing**.

Example: when investigating why a dead-assignment warning stopped firing, adding
`eprintln!` to `in_use` and `track_write` immediately revealed an extra `uses` increment
from a captured variable re-read, and the backtrace pointed to the exact `parse_var` call.

### When interp codegen is opaque or hangs — read the native-generated Rust

`LOFT_LOG=static` shows the bytecode, but a *codegen* bug (wrong channel, missing
cast, a termination test that never fires) is usually easier to see as Rust than as
bytecode — and if codegen itself loops, there is no complete bytecode to read.

`loft --check <file>` runs the **native** backend, which emits readable Rust and stops
at the `rustc` diagnostics. The generated source persists at `loft_native_*.rs` in the
build temp dir (`$LOFT_TMPDIR`, default the system temp; the path is printed in any
`E0xxx`). It encodes the same for-loop / yield / dispatch logic the interpreter
compiles to bytecode, but as named-variable Rust — so a type mismatch, a wrong
sentinel, or a doomed loop condition is visible directly.

Reach for this especially when a process **hangs**: `gdb` attach (`ptrace_scope`) is
blocked in this sandbox, and `perf` needs `perf_event_paranoid <= 2` (the default is 4;
`make profile` names the sysctl) — without it you cannot backtrace or sample a live hang. The generated Rust — or env-gated counter-panics
(§ above) — is the substitute.

Worked example (#401): an `iterator<float>` for-loop hung the interpreter at codegen.
The native `.rs` showed the loop as `let var_x: f64 = coroutine_next_i64(..); if
!var_x.is_nan() { … } break;` — a NaN value-sentinel termination reading an i64
channel — which exposed the root (the value-sentinel only terminates when the
element type's null sentinel matches the i64 transport's) in one read, after gdb/perf
and hand-placed counters had all stalled. Confirm the mechanism this way **before**
editing: a "fix" applied to a hypothesis (there, a guessed return-type change) was a
no-op that cost a rebuild.

---

## Debugging a Scope Analysis Bug

Scope analysis bugs are the hardest to diagnose. The gap between the wrong IR
insertion and the runtime crash is large.

Strategy:
1. Use `LOFT_LOG=ref_debug` to capture all allocation and free events.
2. Look for a `free` event on a DbRef whose `store_nr` does not match any live
   allocation — that is the double-free or wrong-store free.
3. Search backwards in the log for the `alloc` event for that DbRef. The function and
   variable name tell you where the wrong free was inserted.
4. In `src/scopes.rs`, find the `get_free_vars` or `exit_scope` call that produced
   the wrong `OpFreeRef` / `OpFreeText`, and fix the scope assignment for that variable.

---

## Reading a defect — the shapes that keep recurring

The matrix-first protocol in CLAUDE.md says how to *measure* a defect. These are the reading
errors that survive it, each one measured here rather than imagined. They are ordered by how
expensive each was.

**"Needs design" is a claim about your SEARCH, not about the defect — re-test it.** Declaring a
fix impossible without new surface is a hypothesis about the solution space, and it deserves the
same scepticism as a hypothesis about a cause. Measured on loft#1476: several closure records
adopt one store, exactly one must CASCADE to release the capture, and it must be the one that
escapes — a per-run fact against a marker set at compile time. The op surface was checked for a
non-cascading free and had none, so the conclusion was a new IR node plus per-run ownership.
Every step was true and the conclusion was wrong. The cure needed per-run **reach**, not per-run
ownership: let every record own, and NULL the left-behind record's capture slots before freeing
it, so its cascade follows nothing. No new node, no new op, no witness. The tell is that the
search had fixed the CASCADE as given and looked for a way to control it; the move was to change
what the cascade can REACH. So when routing something as needs-design, write down the primitive
you believe is missing, then ask whether the invariant could be met by changing a different term.

**Lifting a refusal RUNS never-run code.** A shape that did not compile has never been scored, so
the first green build of it is not a regression check — it is a first measurement. Two defects
this cycle were found this way and both were invisible until the refusal above them was gone
(one leaked only under `LOFT_STRICT_STORES=1` while the value was right). After removing a
refusal, run the newly-reachable cells under the instruments before believing the value — a guard
that suddenly compiles is a guard whose cells have never been measured.  The worked example is
loft#1479: right answer on both backends, clean on `--interpret`, and one leaked record on
`--native` that only `LOFT_STRICT_STORES=1` sees, found because `make falsify` refused to score
the tree.

**A baseline must FORK BEFORE the work you are attributing.** To answer *"is this mine?"* the
control build has to lack the suspect change. A commit inside your own branch's lineage cannot:
it already contains everything you are asking about, so *"identical on X and on my tip"* only
rules out what came after X. Measured 2026-09-08 with three checkouts in flight: a worktree at
the session's own tip-ancestor was used to call four defects pre-existing, and at the true
merge-base three of them were regressions — including two red corpus tests and a silently-wrong
capture defect, one of which the session's own earlier notes had already bisected to its own
commit. `git merge-base <mine> <theirs>` names the honest baseline; build it in its own worktree
with its own `--target-dir`, and run a positive control first, because a scratchpad binary
without `--path <worktree>` cannot find `default/` and greps as a clean run.

**A fix that makes the surviving defect QUIETER is a cost, not a neutral.** After a partial fix,
read what the still-failing cells now PRINT. The same session's fn-ref width fix turned a
capture defect's symptom from `4294967398` — visibly 2^32 plus the right answer — into a
plausible `103`, with both leak instruments silent because the store being read is live and
merely wrong. And backing out the incident-shaped half of that fix, keeping only the general
one, left thirteen matrix cells wrong with several converting a SIGSEGV into a quiet wrong
answer. A general fix that trades a loud failure for a quiet one can be worse than the bug; that
measurement is what made both halves ship together.

**The nested `{ … }` block is the forgotten spelling of every ARM fix.** When a delivery or
store-lifetime defect is fixed for an `if` / `match` ARM, the same defect is almost always still
live in the plain nested value block — measured twice inside one day, from two unrelated arm
fixes. loft#1493 (an arm yielding a struct's collection field answered EMPTY) has loft#1494 as its
nested-block sibling, a use-after-free by a different mechanism; and loft#1491 (an arm binding the
return buffer from a call leaks one store per call) has the identical per-call leak in
`{ v = head(n); v }`. The reason is structural: the delivery machinery is organised per CONSTRUCT
— a function tail, a branch arm, a bind, a `for` — and the plain nested block is the construct
with no delivery of its own, so it inherits nothing and each per-construct fix leaves it behind,
while `(F-Block)` promises a block yields its tail *wherever the block stands*. After any arm-side
fix, re-run the repro with the arm replaced by a bare `{ … }`. And check the CONSUMER axis
separately: loft#1494 had five red consumers (a return delivery, a call argument, a struct-literal
field, an operator, a field write) and two green ones, and the green ones were green because they
SINK their copy into the block — so a per-consumer cure would have had to be written five times
and would still have missed the sixth. That is what makes the BLOCK, not any consumer, the
chokepoint.

**A walker on a statement list matches more than the construct you meant.** When rewriting the
ARMS of a construct, keep the arm's node kind out of the same `match` as the construct's — called
on a statement list, the walker will also match that node kind standing ALONE and rewrite
something that was never an arm. Measured on loft#1496: the arm-voiding walk matched
`Span | If | Block`, meaning `Block` as the arm reached through the `If`, and also fired on a bare
`{ … }` in statement position. A keyed literal reached through a CAPTURE arrives as exactly that —
a block whose ops build STRAIGHT INTO its destination (@PLN93 build-into-target, loft#1326) — so
dropping its tail and voiding its type destroyed the build and the collection read `0xDEADBEEF`.
The cure is to split the walk: one function recognises only the construct and hands each arm to a
second, which is the only one that rewrites, so the arm's node kind is reachable only THROUGH the
construct. ⚠ **A green `make ci` and a clean `find_problems --changed` both passed over it; the
`LOFT_POISON=1` sweep found it in 90 seconds** — that sweep is not optional after touching shared
parser or codegen machinery ([CI_BUDGET.md § What runs when — today](CI_BUDGET.md#what-runs-when--today): `make ci` does not run it). And bisect such a regression by
disabling each candidate fix in place (`if false && …`, an incremental rebuild, one failing file)
rather than by building historical commits: three ~15s cycles named it.

**A "statement or value?" question is not decidable where the construct is built.** Three
arm-side discriminators for *is this branch a statement* were each disproven by building the
counter-example (loft#1496): the arm's expected `result` being `Void`, the arm's tail being a
`Value::Drop`, and un-dropping the tail instead. All three are carried by the else arm of
`if n > 0 { …; return b.items; } else { head(n) }`, whose value IS the function's — so each cure
made that function return nothing. The fact that works is *does anything FOLLOW the construct*,
and it is known only at the enclosing level: in `parse_block_inner`'s statement loop, past the
point that breaks out at `}`. When a defect turns on value-vs-statement position, look for the
parser point that has already seen what follows rather than trying to infer it from the
construct's own type or lowering — both are set before that is known. And note the interpreter
balances its eval stack against a block's declared TYPE, so a discard has to be total: an arm
keeping either the value or the type alone is off by one in that half's direction, and corrupts
every local live across the branch.

**One question with several DECODERS: the defect sits in whichever one the failing route
consults.** Three times in one cycle (loft#1444, loft#1474, loft#1477), all in the neighbourhood
of *"which closure records does this return deliver?"* — a fn-ref variable's free read one
decoder while the record local's free read a stale note; a tail return read a walker that
handles `Value::FnRef` while an explicit mid-body `return` routed through one that answers in
variables and has no `FnRef` arm at all. The defect is invisible from every route that consults
a correct decoder, so the search is not "where is the wrong answer" but "how many places answer
this question, and do they agree". A related structural warning from the same cycle: the
`ir_walker_audit` modes measure DESCENT into child-bearing shapes, so a walker missing a LEAF
arm like `FnRef` is invisible to them by construction — a traverser is checked, a value
CLASSIFIER is not.

**Start at the PRODUCER of a wrong fact, not the consumer where it surfaces.** When the bug is
a lie in the data — a dep, a type, a flag that disagrees with runtime — the crash is at the
reader and the cure is at the writer. loft#457's dep said `out` borrows `__vdb_1` while at
runtime `out` held the adopted `__ref_N`; the lie surfaced at the free as a use-after-free, and
a session of patching the free side (witness-pairing, dep-strip, explicit free, a narrowed
predicate) only grew complexity. The fix was in the return delivery, so the dep stopped lying.

**After two feature streams JOIN, the defect is in their PRODUCT and in neither factor — so
probe the seam, because a green gate on the join says nothing about it.** 2026-09-05 joined
loft#1361 (a whole-tuple bind COPIES its heap member) with loft#1362 (a whole-value copy MOVES
the drop).  Each branch was green, `make ci` on the join was green, and `t = (s, 5); u = t`
released one resource TWICE on both backends with nothing said.  Neither guard could see it:
the tuple guard carried no `OpDrop` and the drop guard carried no tuple, so the broken cell was
the one cell that needed both.  The cheap instrument is a probe whose cells cross the two
features deliberately; the expensive alternative is meeting it in a consumer.

**Attribute a joined defect across FOUR builds before deciding whose it is.**  The same
session: 2026.8.0 released once everywhere, branch A alone was worst (four shapes doubled, one
tripled), branch B alone was clean — because without A's copy one record wears both names —
and the join healed three of the four.  Read from the join alone it looked like the pick had
broken something; the four-build matrix showed it was A's own regression that B's work had
mostly absorbed.  Building the two parents costs two compiles and settles a question that
otherwise gets argued.

**A leak on one backend can be a wrong ANSWER on the other, and the leak is the harmless-looking
half.** loft#1081 was filed from `--interpret` as "a vector-valued `if` leaks a store per
evaluation". The same program on `--native` — the DEFAULT backend — printed the third call's
values for all three live bindings, silently, because native's allocator hands the just-freed
slot straight back while the interpreter retains it. A leak report describes what the detector
could see. Always run the other backend before accepting a leak as the whole finding.

**One symptom can have two mechanisms, and the first fix turning most of the matrix green is
not evidence you found the cause.** loft#879's real axis was an optional aggregate return
whose result stays a temporary, with the reported `??` incidental — peeling `Optional` fixed
the discarded call, the argument case, the vector case and the loop, and **the filed repro
still leaked**, because the `??` half was a second, independent defect. Re-run the ORIGINAL
repro after the matrix goes green.

**An assertion that fires can be reporting a WRONG ANSWER, not the invariant it names.** A
debug-assertion is written to catch one property, so the report describes that property and
nothing else — and the shape that trips it is then read as a question about the assertion.
loft#1241 was filed that way: a store-span check firing on a local the lowering had elided, with
the answer measured as *"right — `len=3` on both backends"* and the remaining question presented
as a choice between two ways to reshape the check. Both proposed cures would have silenced it.
One `for` around the same append and the answer was 3 where the program says 7 — at every
iteration count, because the rewrite had folded the append out of the loop (loft#1243). The
assertion was the only thing in the tree that had noticed. **Before treating an assertion as a
question about itself, sweep its shape over CONTROL FLOW** — a loop around it, a branch not
taken, the same statement at two nesting depths. An issue's own "the answer is right" is a
measurement of the cells its author ran.

**A rewrite's guard list can be complete on DATA and empty on CONTROL FLOW.** The move-elision's
`ready` filter asked seven questions — does the source escape, is the destination disturbed or
cleared, is the container allocated first, is the destination unique — and every one of them is
about values. None asked whether the statement it was DELETING runs as often as the code it was
keeping, and its prescan walked the whole body, so an append inside a loop was folded exactly
like one written beside the declaration. When reading any pass that MOVES or DROPS a statement,
count the two kinds of guard separately: the question "is this value still valid there?" and the
question "is that place reached the same number of times?" are answered by different machinery,
and a list can be exhaustive in one and empty in the other.

**A bound belongs to where the value LANDS, not to the type that named it.** Two expression
forms can name the same target type and put the result in different places, and a bound
attached to the named type then reaches one of them wrongly. `e as u8?` and `e as u8 ?? d`
both name `u8` and both lower through the same checked-cast helper — but the first stays a
`u8?`, whose sentinel is reserved, and the second is discharged to a non-null `u8`, which
reserves nothing. Narrowing the helper's bound to the reserved range was right for the first
and silently turned `255` into the default for the second (loft#1249) — a value the published
`hex_field` asserts, so it broke a shipped library. **When a fix makes a shared helper's
answer depend on a property the callers do not currently pass, that property is the new
parameter** — deriving it inside the helper from what it already has is the same guess in a
place the caller cannot see. And when NO caller can supply it either, that is the finding:
the same attempt moved to the store seam and hit the harder half — `Type::Optional` on a
write target means *"this slot holds null"* for a `u8?` local and *"this read may miss"* for
an element of a non-null `vector<u8>`, so one predicate cannot serve both and the fix was
withdrawn rather than shipped half-right.

**Two coverage instruments each caught a different half of that, and neither was looking for
it.** `scripts/matrix_axes.py` reported `A9 evaluation count` missing `loop`; writing the
loop cell needed a non-nullable control, which could only be spelled with the coalesced cast,
which is what exposed the first half. `scripts/revalidate_libs_local.sh` — the shipped
libraries, which `make ci` says nothing about — caught the second on a real consumer after a
full green gate. **Run both before believing a semantic change**: a green `make ci` is not
evidence about the language's users.

**And an A/B against a published library must resolve its source by the release TAG.**
`scripts/revalidate_libs_local.sh` does that by construction — it reads each package's tag from
the registry matrix and `git archive`s that tree — so prefer it, or copy that resolution. A
hand-rolled check picks a clone by PATH instead, and a sibling `…-main` beside the tagged one
can already carry the rename the diagnostic fires on: measured, a check of `regex` extracted
from `loft-libs-core-main`, which carries no `regex-v0.3.1` tag, so neither binary warned and
the result read as *"old and new agree, so it is not mine"* — from a tree where the construct
under test no longer existed. Grep the extracted source for that construct BEFORE running
either binary. An absence is evidence only once the instrument is shown able to produce the
presence, which is what the `*_INJECT` levers do for a detector.

**When you write a predicate over a type, read that type's own doc-comment for the shapes it
says it takes.** `target_holds_null` asks whether a store's target is a nullable slot, keyed on
what the place is read OUT of, and it handled a variable, a field and an element correctly on
the first try. It missed `&vector<u8>` — a write through a `&`-parameter — and fell through to
the answer meant for the other shapes, so a fully-opaque `255` written into a non-null
`vector<u8>` came back `null` and every proxy test in published `assets` failed (loft#1249).
The field it reads, `AssignPlace::parent_tp`, is documented as *"`Reference(S)` for `s.f`, **`&S`
for the same write inside a `&`-parameter**, `vector<τ>` for `v[i]`"* — the missing case is the
second of the three it names. **The enumeration you need is usually already written where the
data is declared**; a predicate built from the cases you happened to probe will match them and
nothing else, and the shapes it misses are the ones the type system spells differently for the
same thing.

**A precedent transfers along the MECHANISM, not the problem statement.** Before reusing a
neighbouring solution, ask what its unit of work is — per member, per record, per call, per
path, per pass — and whether yours has the same one. `keyed_group_clear` solves the same
*context* problem as loft#1152 (the emit site lacks the parent type) and does not transfer: a
clear is per-member and static, the write it was wanted for is per-record and dynamic.

**Read the code BESIDE the defect, not only the defect.** The sibling in the same function that
solved the same hazard first usually states the rule and the method in its own comment.
Measured twice in three days: `do_tret_bind`'s comment spelled out the pass-2 promotion rule
that `do_if_acc` was missing (loft#1099), and the `others`-link comment in `Stores::field`
named declaration order as the thing a group must not depend on, one level below the pairing
test that still depended on it (loft#1158).

**A defect has a SCALE, and a comfortable repro silently changes it.** loft#710's persisted-store
size defect (file = arena capacity, not content) produces byte-identical files at 200 records ×
40 coords, so every assertion written at that size passed on the broken binary too; the reported
125 × 2312 showed 1.84×. Allocator, cache and page behaviour are hidden axes. Replay the
reporter's exact parameters through the pre-change binary before trusting a guard.

**The filed scope is usually a fraction of the defect, in both directions.** An issue reports
the shape its author hit. Take the reported spelling as one cell, not as the boundary — and
equally, do not assume the boundary is wider than measured: loft#1158 predicted its fix would
need a second part (making the vector the record holder whatever the declaration order) and the
measurement falsified it.

**The cure is often one construct over.** `map`, `filter`, `reduce`, `all`, `count_if` and the
comprehension all ended their loop on a null ELEMENT and hung forever once a value-struct bind
deep-copied into a record that is never null. The `for` STATEMENT terminates on LENGTH, which is
why it was the only correct row in the report — and it became the shared home rather than a
sixth repair.

**A design question derived from what a program DOES is only as good as the lowering under it.**
Behaviour under-determines mechanism, and two different mechanisms can produce the same reading
from different starting states. `h += other_h` on a keyed collection was read as *"merges"* from
an empty destination and as *"silently drops the write"* from a populated one, and those two
readings were argued between checkouts as a design question — should `+=` merge two collections?
One `loft introspect` dissolved it: the statement lowers to `b = a`, a plain assignment. It
neither merges nor drops; it REBINDS, aliases the source, orphans the destination's store, and
destroys whatever the destination held. There was no design question, only an undocumented
aliasing rebind that no rule admits. **Read the lowering before you take a behavioural
observation as a premise** — it costs one command, and it is the cheapest check that can retire a
whole debate.

**A differential sweep can only see shapes the CORPUS contains.** *"The differential is clean"* is
a statement about the corpus, not about the change. The same whole-corpus sweep blessed both a
refusal and its narrowing on the keyed-merge question above, because nothing in the tree merges
two keyed locals — so the instrument was equally blind to both answers. Pair a sweep with a
hand-built cell for the shape you are actually changing, and treat a clean sweep over a shape the
corpus lacks as no evidence at all.

**A flag cleared at the start of a parse cannot survive its own left-hand side.** The shape is
`self.flag = false; parse(); read self.flag`, and it breaks when `parse()` RE-ENTERS the same
function for a sub-expression: the nested entry runs the clear again and erases what the outer
one had already recorded. `last_place_discharge` is cleared so an earlier statement's answer
cannot be read as this one's, and `parse_assign` is re-entered for every index and call
argument a left side contains — so `b.d? += […]` kept its answer only because nothing is parsed
after its `?`, while `h?[k] = v` reached the place check having forgotten its `?` and was
refused as an explicit coalesce (loft#1214). The cure is to SAVE on entry and RESTORE on exit,
which confines each nesting level to its own answer instead of letting the innermost win. Worth
grepping for whenever a per-statement flag is read after a parse that can recurse.

**Check that the precedent you are about to copy is itself sound.** A fix that says *"do what
the working twin does"* rests on that twin being right, and it is cheap to ask. loft#1214's
keyed materialisation was about to copy the vector local's, so the vector local's was
measured first: it mints its backing UNCONDITIONALLY at the append site, so a loop re-executes
it and every earlier iteration's elements are thrown away (loft#1220 — `len` 1 instead of 3, on
the shipped build). The copy would have inherited it. Two minutes on the control turned up a
`sev:high` `silent-wrong` defect and changed the fix from *"copy the twin"* to *"guard the mint
on the destination actually being null"*, which is what the rule said in the first place.

---

**A negative result is only as good as the tree it was measured on.** A candidate fix
rejected early can have been rejected by a DIFFERENT bug still live in the same subsystem: a
widening was tried, answered wrong, backed out and written into an issue as a negative
result; two commits later the real cause of that wrong answer — a slot-ordering defect in the
same emitter — was fixed, and the widening re-applied on top was correct all along. The
retraction was wrong too: the over-free later blamed on the same widening reproduced with it
REVERTED. When a fix lands in a subsystem, re-run every candidate backed out of it before
trusting the reason it was backed out; and before writing a negative result into an issue,
run the failing cell with the change reverted — if it still fails, the change was never the
cause.

**A repro can contain the WORKING form of the same construct, and that form can repair the
defect.** loft#1125 was filed with a dense `index<IS[k]>` local beside the nullable one that
failed; the dense local runs the pre-registration walk that sizes the element struct, so the
nullable one inherited a correct layout and the file as filed printed the right answer.
Removing the dense twin refused the file instantly — the real axis was *no dense twin of this
element type anywhere in the program*. When a filed repro is green, DELETE its sibling cells
before concluding it was fixed, and give every cell of the guard its own element type and its
own name, with a header sentence saying why: a guard built the natural way (one struct, a
dense cell beside every nullable cell) passes on the broken build.

**A guard that can only fire on one word size names where it can SPEAK, not where the
corruption is.** `Store::checked_offset` raises `Store offset overflow` from
`isize::try_from(rec * 8 + fld)` with `u32` inputs — an offset that always fits an `i64`, so
the raise is dead on every 64-bit target and live only on wasm32. loft#950 was filed as *"the
`--html` page traps; the interpreter, `--native` and `--native-wasm` are all green"*, which
reads as a browser-specific defect. The same corrupted `rec` on a 64-bit build computes a
representable offset and reads whatever lies there — the silent-wrong half of the same bug.
Before believing "only the browser", ask whether the other targets have a guard that could
have spoken; `LOFT_STRICT_STORES=1` is the one that speaks on all of them.

## Using the Test Framework for Quick Iteration

The `code!` and `expr!` macros in `tests/testing.rs` let you write a loft program
inline in a Rust test:

```rust
#[test]
fn my_feature() {
    expr!("my_expr_result").result(Value::Int(42)).run();
    code!("fn main() { assert(1 + 1 == 2, \"math\"); }").run();
}
```

Use `.error("expected error message")` to assert on compile-time diagnostics.
Use `.warning("expected warning")` for non-fatal diagnostics.

For end-to-end tests on `.loft` files, add to `tests/docs/` and the `wrap.rs`
runner will pick it up automatically.

---

## Two false failures that look exactly like real ones

Both waste a bisect if you take them at face value.  The rule underneath is the same:
**a surprising red is a suspect environment before it is a suspect commit** — check the
cheap environmental cause first, because each of these mimics a deterministic bug.

**A stale server process on the port.**  A networked test
(`tests/engine_host_connector.rs`, the `eh_*` family) failed in `make ci`, then failed
standalone three runs in a row at an *identical* 15.80s, and it post-dated a green run
— every tell of a real regression.  It was two leftover servers under
`target/test-tmp/.loft/cache/` holding the ports; the test then waits out its deadline.
Killing them made it pass in 3s.  So before bisecting one of these:

```bash
pgrep -af "target/test-tmp/.loft/cache/eh_"      # stale servers from an earlier run
```

⚠ **Run it AFTER a gate as well as before, because the reap is unconditional.**  Measured on
both checkouts 2026-09-07: a gate that finished `ALL GATES PASSED` left `eh_s5_*` and `eh_s7_*`
alive behind it, one pair still running 1h40m later.  Nothing reaps a server that outlived the
run that started it — `make sweep-scratch` reclaims the artefacts, not the processes.  A GREEN
run is exactly the case where nobody looks, and the orphan is then charged to whoever runs
next.  Kill by PID after matching `readlink /proc/<pid>/cwd` against your own checkout; a
`pkill -f` pattern took out a peer's own process group ([CODE.md](CODE.md)).

**Identical timing across runs is the tell** — that is a deadline expiring, not logic
failing.  Real logic bugs vary by a few ms; a deadline does not.

**The installed binary is not always a "before" oracle.**  `$(which loft)` is only a
pre-change reference if it was installed BEFORE the change.  A session used it to
conclude a consumer-reported fault was pre-existing; the binary turned out to be dated
*after* that morning's commits, and the fault was in fact a regression introduced by
them.  `ls -l --time-style=long-iso $(which loft)` first, and when it is not older than
the work, build the parent commit in a worktree instead:

```bash
git worktree add /tmp/pre <commit>^
cd /tmp/pre && CARGO_TARGET_DIR=/tmp/pre-target cargo build --release --bin loft
ln -s /tmp/pre/default /tmp/pre-target/release/default   # it loads default/ beside the binary
```

Related: **never run `find_problems.sh --bg` while building in the foreground** — they
share `target/`, and the contention produces failures that vanish on a settled tree.

---

## Bounding a run — `--timeout` / `LOFT_TIMEOUT` (@PLAN49)

**loft has no process-wide default timeout, by design.** Long-running programs —
servers, game loops, anything that should run until interrupted — must be able to
run unbounded, so we will not add a default. **Testing is the exception:** `loft
test` / `--tests` arms the watchdog at 300s automatically (a hung test or looping
compile in the suite can't be killed interactively).

When you run loft **ad-hoc** in an agent session — a `/tmp` probe, a one-shot
script, and especially `--native` (it shells out to `rustc`, which can hang on a
pathological program) — **bound it yourself**, or a runaway hangs the session:

```bash
LOFT_TIMEOUT=60 loft --native prog.loft     # env form — arms at startup, is the floor
loft --timeout 60 prog.loft                  # flag form — re-arms; 0 disables
LOFT_TIMEOUT_GRACE=5 LOFT_TIMEOUT=60 loft …  # grace before the hard kill (default 2s)
```

Mechanics (`src/timeout.rs`): `arm(secs, grace)` spawns a `loft-watchdog` thread
that sleeps to `secs + grace`, prints a breadcrumb, and **process-aborts** — so it
bounds the WHOLE process: the `--native` compile, the interpreter loop, everything.
The breadcrumb names the loft `fn`, its `file:line`, and the `entry` it was reached
from (under `--tests`, the test) — see [LOFT_TEST.md § Output format](LOFT_TEST.md#output-format) for the format.
`arm` is idempotent (first deadline wins) and `secs == 0` leaves it disarmed (the
default for ad-hoc runs — hence the hang risk). `LOFT_TIMEOUT` is read before argv,
so it is the floor; an explicit `--timeout` only re-arms if nothing armed yet.

Rule of thumb: **server/long-task run → unbounded; test or throwaway probe →
always pass `LOFT_TIMEOUT`.**

---

## Moved: The tracker-tag indexer and the branch review viewer

See [DEBUG_VIEW.md](DEBUG_VIEW.md) for the tracker-tag indexer and the branch review viewer.

## Open work

Diagnostic tooling enhancements surfaced by recurring debug
sessions across @PLAN22's 02d sub-phases (Sept-Oct 2026).
Each row is a focused, single-commit improvement; collectively
they would have shaved 10-20 hours of `eprintln!`-and-rerun
diagnosis time across @PLAN22 phases 02d-iii through 02d-vii.
Listed in ROI order (highest leverage first).  The rows that shipped are in [DEBUG-history.md](DEBUG-history.md).

| Tool | Effort | Where it would have helped | Notes |
|---|---|---|---|
| Moment-of-urge matrix hook (settings.json) | XS | Every "rushed fix without a matrix" episode — the #354 session's three cascading non-matrix fixes (hoist-everything leak, `let _ =` break, `callee_forwards` fragility); static doc text + memory entries were loaded and still lost to momentum | The only trigger the harness GUARANTEES: a PostToolUse hook on Bash sets a session flag when output matches `FAIL\|SIGSEGV\|panicked\|leaked\|assertion failed`; a PreToolUse hook on Edit/Write touching `src/**/*.rs` injects one line while the flag is set — "test failure seen this session: does a probe matrix with expected values exist?". Fires exactly at the urge-to-fix moment, where doc-reading has already decayed. No classifier, no blocking — a reminder injection only. Implement via the `update-config` skill. |

**Why DEBUG.md and not a plan**: each row is independent
(no cross-dependencies), each ships in a single commit, and
the work doesn't have phases — it's classic light-flow
infrastructure.  Per `loft-plan-workflow` skill, plans are
for genuinely multi-phase initiatives with shared design.

**Cross-cutting motivation**: the loft compiler has multiple
passes (parse-1, parse-2, scope analysis, codegen) that each
transform types, slots, and IR.  Mismatches between passes
manifest as runtime panics with thin context.  The current
debug story relies heavily on `LOFT_LOG=full` which is
high-volume and requires reading bytecode + cross-referencing
codegen.rs.  More targeted log modes that focus on specific
subsystems (locks / types / slots / captures) reduce the
cognitive load per debug session.

## See also
- [../DEVELOPERS.md](../DEVELOPERS.md) — Developer guide: pipeline overview, quality requirements, feature proposals
- [TESTING.md](TESTING.md) — Test framework, `code!` / `expr!` macros; [RUNNING_TESTS.md](RUNNING_TESTS.md) — LogConfig debug presets
- [PROBLEMS.md](PROBLEMS.md) — Known bugs with severity, workarounds, and fix paths
- [SLOTS.md](SLOTS.md) — Slot assignment design (for the slots-dump enhancement)
- [LIFETIME.md](LIFETIME.md) — Dep tracking and scope-based freeing (for the dep-graph enhancement)
- [SLOTS.md](SLOTS.md) — Variable scoping and slot assignment details
