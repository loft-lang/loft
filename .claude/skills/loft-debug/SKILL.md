---
name: loft-debug
description: Operational companion for running a loft bug down on this box — LOFT_LOG presets, dump files, find_problems flow, the live-debugger RPC surface (loft debug --rpc — scripted breakpoint/eval/setValue sessions), the native-backend env gotchas that cause FALSE failures, and the operational-safety rules for builds/processes. Routes to the matrix-first protocol (CLAUDE.md) for the METHOD; this is the MECHANICS. Apply when reproducing, investigating, or verifying a loft crash, wrong-result, or codegen bug — and whenever about to sprinkle println/probe prints into a loft program (the debugger session replaces them).
user-invocable: false
---

# Loft Debugging Reference (operational)

The **method** lives elsewhere and is always-loaded — this skill is the **mechanics**:
how to actually run a bug down here, the env traps that fake failures, and the
irreversible moves not to make.

## Method — read first, don't duplicate here

- **Matrix-first protocol** — `CLAUDE.md` § Debugging policy. Don't fix on first read;
  build the `/tmp` boundary matrix on `--interpret`; find the real boundary (the filed
  scope is usually wrong); fix at the chokepoint enforcing the *invariant* (no
  narrower / no wider); verify BOTH backends at the end.
- **Run the matrix with `scripts/probe-matrix`** (`init` scaffolds a probe dir; cells
  carry `// @EXPECT:` / `// @CONTROL` headers; `--baseline <main-worktree-binary>`
  labels each failure REGRESSION vs PRE-EXISTING; the validity rules — hand-computed
  expectations, no vacuous cells, control stays red — are hard errors).  Usage:
  `doc/claude/DEBUG_PROBES.md` § Boundary-matrix runner.
- **Composition axes** (what to vary in the matrix) — `python3 scripts/matrix_axes.py file
  <probe.loft>` names which axis values your cells reach (`cross A B` for never-crossed
  pairs); the vocabulary is `doc/claude/plans/README.md` § The composition axes.
- **Brittleness vs bugs** — `doc/claude/DESIGN_VERIFICATION.md` § C1 (the real target
  is robust algorithms; a green matrix can still be brittle).
- **Prior attempts on THIS mechanism** — `doc/claude/formal/`, and grep it by the MECHANISM
  (the op, the pass) rather than the symptom. For store lifetime and return buffers it is
  the record of what was tried: `ownership.md` holds the rules and `ownership-history.md`
  writes each reverted fix up, with the measurement that killed it, names where the closure decided the
  unsound step actually is, and cites the `tests/scripts/` cell now guarding it — so you
  inherit a working control — and when an entry names a FALSE PREMISE, search for the other
  sites that hold it. The cost of skipping it, worked through: the `loft-codegen` skill
  § Zeroth. `IMPLEMENTATIONS.md` indexes what is merged.
- **Investigation plans** (heavyweight) — `doc/claude/plans/_INVESTIGATION_TEMPLATE.md`.

## Running a bug down — mechanics

- **First, one tracker probe**: `gh issue list --search "<symptom keywords>"` —
  a bug worth fixing is often already filed (consumers file fast, often minutes
  before an agent starts on the same symptom blind). An existing row hands
  you scope + workaround for free: `scripts/claim.py take N` before touching code (a bug
  you find and fix at once: `claim.py file …`), and the fix commit must claim it (below).
- Single file, interpreter: `cargo run --bin loft -- --interpret file.loft`
- Native: `cargo run --bin loft -- --native file.loft`
- ⚠ **The default backend is `--native` wherever rustc is available** (a downloaded release
  interprets). For the SEEING loop ALWAYS pass
  `--interpret` explicitly — strides/types are IR operands the interpreter surfaces in
  seconds, whereas `--native` pays a rustc compile per probe (that cost belongs at the
  final verify, not the loop).
- **`LOFT_LOG=`** presets (full table: `CLAUDE.md` § `LOFT_LOG` quick reference / `doc/claude/RUNNING_TESTS.md`
  § LogConfig): `minimal` (exec trace — cleanest for runtime bugs), `static` (IR +
  bytecode, fastest for codegen), `crash_tail:N` (last N lines, flushed on panic),
  `ref_debug` (stack snapshots after Ref/CreateStack), `variables` (the per-fn var
  table — name/type/scope/slot), `fn:<name>` (one function). `LOFT_DUMP_DEPTH` /
  `LOFT_DUMP_ELEMENTS` tune the inline struct/vector dumps.
- **Dump files**: a failing wrap/native test writes `tests/dumps/*.txt` — full IR +
  bytecode + execution trace; the root cause is almost always visible there. (See
  `doc/claude/RUNNING_TESTS.md`.) NEVER `git bisect` / `git checkout HEAD -- <file>` to
  investigate (CLAUDE.md § Debugging policy) — read the dump and reason.
- **Tight loop**: `./scripts/find_problems.sh --subject <name>` (seconds) while
  iterating — one full gate per change of unknown reach, and `scripts/ci-run.sh recheck`
  after it rather than a restart (CLAUDE.md § Key commands); a CANCELLED gate takes
  `ci-run.sh start` again, which resumes it.
- **Full suite, detached**: `./scripts/find_problems.sh --bg` → `--peek` mid-run /
  `--wait` to block; structured summary on finish in `/tmp/loft_problems.txt`.

## The build loop — which binary, and what each build costs

`src/` is ONE crate of half a million lines: any one-file edit recompiles all of it, and the release
profile is not incremental.  So the loop is decided by which build answers the question
([CI_BUDGET.md § What a build and each check costs here](../../../doc/claude/CI_BUDGET.md#what-a-build-and-each-check-costs-here--the-ladder)
has the ladder; re-measure a number before it decides anything):

- **Iterate on the dev binary** — `cargo build --bin loft` (minutes), then
  `target/debug/loft --interpret | --native | introspect | --native-emit …`; it runs
  `--native` against the `deps/` rlib the same build wrote.  `make iter TEST=…` for one test.
- **`cargo check --bin loft` for a compile-only question** — never a build.
- **A release build (`cargo build --release --bin loft`, the costliest rung — CI_BUDGET.md) once per fix, at the
  end**: it refreshes `target/release/libloft.rlib`, which only the `cargo test --release`
  native suites link; `make check-rlib` says whether it is due.  Never one per hypothesis.
- **Name the cause from an artefact before any build** — the IR (`loft introspect`) or the
  emitted Rust (`--native-emit`) of the failing function, pointing at the node or line that
  is wrong.  A fix built on a hypothesis the artefact never showed is a build that may change
  nothing.
- **One heavy job at a time**: a build, a gate (`ci-run.sh start`), `make falsify` (two dev
  builds), the drop gate (every core for an hour), a native corpus census, a `cargo test` of
  many binaries.  Beside one another they slow every one of them and the one waited on most
  of all.
  While a build runs, read the next unit's code; wait on its pid or output file, never a
  sleep loop (CLAUDE.md § Never wait on a process by NAME).
- **Receipts last**: `make falsify` builds the working tree, so it runs after the fix is
  committed and while no edit is pending — not in the middle of a fix.

## The agent debug surface — `loft debug <file> --rpc` (drive a live session, don't println)

The @PLN16 debugger speaks NDJSON over stdio (the contract:
`doc/claude/plans/16-debugger/PROTOCOL.md`). One `printf` pipes a whole scripted
session — breakpoint → inspect the live frame → eval → edit → resume. A setBreakpoints
response without `verified` is a pre-2026-06 installed binary — drive `target/*/loft` from
this tree instead.

```sh
printf '%s\n' \
  '{"id":1,"req":"launch","file":"/abs/path/prog.loft"}' \
  '{"id":2,"req":"setBreakpoints","file":"/abs/path/prog.loft","breakpoints":[{"line":3,"condition":"n == 2"}]}' \
  '{"id":3,"req":"run"}' \
  '{"id":4,"req":"eval","expr":"a + n"}' \
  '{"id":5,"req":"setValue","target":"a","value":"100"}' \
  '{"id":6,"req":"stepOver"}' \
  '{"id":7,"req":"continue"}' \
  '{"id":8,"req":"disconnect"}' \
| loft debug /abs/path/prog.loft --rpc
```

- **Order matters: `launch` LOADS but does not run; `run` starts execution.** Set
  breakpoints between them. (`continue` before `run` just emits `terminated`.)
- **`setBreakpoints` matches files by BASENAME** (relative or absolute both work,
  including a `use`d module's file while launching the consumer; two same-named
  files in one program can't be told apart). The response carries
  `breakpoints:[{line, verified}]` — **check `verified`**: `false` means the
  breakpoint can never fire (no breakable code on that line, or a file the
  program doesn't use); don't wait on a stop that never comes.
- **Multi-lib programs**: append the usual lib flags —
  `loft debug src/x.loft --rpc --lib ../pkg/ …`.
- **`stopped` carries the whole frame**: function, line, and every local **in
  lexical scope at that line** — a full crawler `Sim` struct renders inline
  (vectors elided `…`); one event = the variables panel, drill down with `eval`.
  A local in scope that the frame does not HOLD carries a marker instead of a
  value: `"<unset>"` (its assignment has not run yet on this path) or
  `"<reused by step>"` (its stack slot now belongs to `step` — the allocator is
  scope-blind, so two locals in one scope share a slot when their live ranges do
  not overlap; break one line earlier to read it). Never mistake a marker for a
  value: `eval` on such a name returns `value:null`, and `setValue` refuses it.
- **`eval` runs against the paused frame** (`s.php`, `n * 100`, `len(v)`) over
  every local the frame **holds** — including one read later on the same line
  (`step` in `total = total + step`). A name
  that is out of scope, `<unset>` or `<reused by …>` returns `value:null`, not an
  error — check the `stopped` frame to tell which, and don't misread null as "the
  field is empty".
- **`setValue` edits the live run** (scalar / text / enum / struct field /
  nested path / vector element; a whole heap local is rejected by design) —
  execution resumes WITH the edit: probe a hypothesis by injecting the suspect
  value instead of rebuilding.
- **Conditional breakpoints** (`"condition":"p.x == 9"`) filter by frame; a
  tracepoint (`"log":"expr"` or `"log":["e1","e2"]`, `"stop":false`) streams
  `output{category:"trace"}` lines (`expr = value`) without pausing — structured
  trace beats sprinkled printlns. Log entries are EXPRESSIONS, not format strings.
- **Conditions and trace exprs obey the same rule as `eval`** — the usable names
  are the locals in lexical scope (an enclosing block's local works); a name the
  frame does not hold makes the condition **report and stop** rather than silently
  never match. The one unusable case: a name the `stopped` frame shows as `<unset>`
  or `<reused by …>`.
- Program stdout arrives as `output` events on the SAME pipe, never interleaved
  with protocol — parse line-by-line as JSON, switch on `event`/`id`.
- The interactive twin is `loft debug <file>` (the `(dbg)` prompt) — same engine,
  for a human; agents always prefer `--rpc`.

## Native-backend env gotchas — these fake FAILURES; rule them out before believing a native failure

1. **Default is `--native`** — see above; pass `--interpret` for the seeing loop.
2. **Toolchain mismatch.** The box's rustup default can differ from the repo's
   `rust-toolchain.toml`. Then `--native` fails
   `E0514 incompatible rustc`, *or* forcing a toolchain triggers a full from-scratch
   rebuild. Fix: run native from **inside the repo** so `rust-toolchain.toml`
   applies; if a snap `rustc` is first on PATH, prefix
   `PATH="$(dirname "$(rustup which rustc)"):$PATH"`. `find_problems.sh` self-heals
   the cross-toolchain-polluted `libloft_ffi-*.rlib` case before building; hand-fix
   only direct `--native` runs outside the script.
3. **Stale dependency rlibs.** After a rebase / `Cargo.lock` bump, a `--lib`-only build
   leaves dep rlibs stale → native tests fail `crate rustls / ureq / webpki / ring
   required to be available in rlib format, but was not found`. Fix: a **full**
   `cargo build --release` (NOT `--lib`) — the native harness links the whole dep
   tree. These are FALSE failures.
4. **Linker flakes under parallel native compiles** (seen as `rust-lld` SIGBUS in a
   tmpfs `/tmp` on Linux). Native tests flake under `find_problems`' parallelism.
   Confirm any native failure **serially**
   (`cargo test --release --test <bin> <name> -- --test-threads=1`) before trusting it.
5. **Stale `libloft.rlib`.** After changing `loft-ffi*/` or the `loft` crate's runtime
   code, run `make check-rlib` (the 1s pre-flight) before native tests — it says whether
   the rlib the native tests link is current; a stale one is a wave of false failures.

Pattern: a sudden wave of native-compile/link failures right after a rebase / dep /
toolchain change is almost always 2–5, not a regression. Rebuild fully + re-run
serially before believing them.

## Operational safety — the irreversible moves

- **Never `kill` / `pkill` a process you did not personally start.** A broad
  `pkill -f "cargo run …"` matches a SIBLING agent's identical command — it has killed
  another agent's build (and the killer's own). To stop your OWN background task, kill
  only its specific PID / task-id.
- **Never touch a sibling checkout of this repo** (a parallel agent's workspace). An
  `--out-dir` pointing into a tree that is not your working directory, in a process you
  are inspecting, means that process is NOT yours.
- **An anomaly right before a destructive command (kill / rm / force-push / overwrite)
  is a STOP, not a footnote.** Investigate the surprise first — there is no undo, and
  this is exactly where "don't act on partial sight" matters most.

## After the fix — routes

- **Re-probe the tracker with what you NOW know** (the fixed mechanism names
  better keywords than the filed symptom did), and put `Fixes #N` in the fix
  commit (or PR body) for every row your fix closes, beside its `Contract: settled|strained
  — <why>` trailer (CLAUDE.md § Bug-filing policy). The `Fixes` trailer drives the
  whole lifecycle — `fixed-pending-merge` on push, auto-close + label-strip on
  merge; never hand-label. Add the substantive comment: fixing commit,
  regression test, what verification actually found. When a PR opens, copy
  every `Fixes #N` into the PR BODY too — merges squash, and a merge-time
  edit of the squash message would otherwise kill the trailer (issue stays
  open, label goes stale).
- Found a *sibling* bug while debugging? Default is **FIX** it (cheapest bug you'll
  ever fix), not file it — `CLAUDE.md` § Bug-filing policy. File only if it blocks the
  task or is too big to fix now.
- Before marking `fixed-pending-merge`: the **done-gate** —
  `doc/claude/ISSUE_TRACKING.md` § The done-gate (class coverage + intent match;
  "would the requester file a slight variation?"). Name any residual you couldn't
  close; don't leave it silent.
