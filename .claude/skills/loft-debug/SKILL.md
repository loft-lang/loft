---
name: loft-debug
description: >-
  The instrument panel for running a loft bug down: USE THIS while reproducing, narrowing or
  verifying a crash, a wrong answer, a hang, a leak, a native compile error or a backend
  disagreement — and INSTEAD of adding println or eprintln probes. Reach for it when you
  need to SEE what the program does: which line wrote a variable's type and its deps
  (LOFT_LOG=type_timeline), what a function's variable table holds (LOFT_VAR_TABLE), which
  free released a store a later read touched (LOFT_POISON, LOFT_STRICT_STORES), what the
  frame holds at line N (loft debug prog.loft:N, scripted with --rpc), where a hang loops
  (LOFT_MAX_OPS), a leak against a runaway length (the store-heap ceiling), what a dump in
  tests/dumps says. Also the native env gotchas that fake a failure, the build loop and
  find_problems. The method — matrix first, hypothesis, falsify — is engineering-rigor's;
  this skill is the mechanics.
user-invocable: false
---

# Loft Debugging Reference (operational)

The **method** — matrix first, hypothesis, falsify — is `CLAUDE.md` § Debugging policy and the
`engineering-rigor` skill.  This skill is the **mechanics**: which instrument answers which
question, the env traps that fake a failure, and the live debugger.

## Ask the instrument, not a print — question → instrument

Reach for these before reading code or adding an `eprintln` (homes: CLAUDE.md § Environment
switches, [RUN_BOUNDS.md](../../../doc/claude/RUN_BOUNDS.md)):

- *Which line gave this variable its type or its deps?* `LOFT_LOG=type_timeline:<var>`,
  with `LOFT_TIMELINE_BT=1` for the stack; it shows pass 1 and pass 2 separately, which is
  where a dep that "keeps coming back" usually comes from.
- *What does the function's variable table really hold?* `LOFT_VAR_TABLE=<fn>` — every
  dep resolved to `name(index)`, with OWNS / skipfree flags.
- *Which free released the store a later read touched?* `LOFT_POISON=1` (freed bytes read
  as 0xDEADBEEF, so a lucky read becomes a loud one) with `LOFT_STRICT_STORES=1` (names
  the violation, and every store never freed, at exit).  A probe that prints the right
  answer is not clean until it passes under both.
- *Where does this hang loop?* `LOFT_MAX_OPS=<n>` set low, in a debug-assertions build
  (RUN_BOUNDS.md § Hang guard) — it prints the last sixteen ops.
- *A leak, or a runaway length?* The store-heap ceiling's report under `--tests` /
  `LOFT_MEMORY_LIMIT` names the type and splits one store from many.
- *What does the frame hold at line N?* `loft debug prog.loft:N`, scripted with `--rpc`
  (§ The agent debug surface below).
- *What did the parser resolve at compile time?* `LOFT_TRACE=<category>` (§ Parser tracing
  below).

## Method — where it lives

- Matrix-first protocol: `CLAUDE.md` § Debugging policy; run the matrix with
  `scripts/probe-matrix` (usage: `doc/claude/DEBUG_PROBES.md` § Boundary-matrix runner —
  `--baseline <main-worktree-binary>` labels each failure REGRESSION vs PRE-EXISTING) and
  check its axes with `scripts/matrix_axes.py` (vocabulary: `doc/claude/plans/README.md` §
  The composition axes).
- Brittleness vs bugs: `doc/claude/DESIGN_VERIFICATION.md` § C1 — a green matrix can still be
  brittle.
- Prior attempts on THIS mechanism — grep `doc/claude/formal/` by the mechanism (the op, the
  pass), not the symptom; for store lifetime, `ownership-history.md` is the record of reverted
  fixes and the cells guarding each closure.  Why: the `loft-codegen` skill § Zeroth.
- Heavyweight investigations: `doc/claude/plans/_INVESTIGATION_TEMPLATE.md`.

## Running a bug down — mechanics

- **First, one tracker probe**: `gh issue list --search "<symptom keywords>"` —
  a bug worth fixing is often already filed (consumers file fast, often minutes
  before an agent starts on the same symptom blind). An existing row hands
  you scope + workaround for free: `scripts/claim.py take N` before touching code (a bug
  you find and fix at once: `claim.py file …`), and the fix commit must claim it (below).
- Single file, interpreter: `cargo run --bin loft -- --interpret file.loft`
- Native: `cargo run --bin loft -- --native file.loft`
- The compiled leg of a check is the C backend (`--native-c`, or `LOFT_NATIVE_BACKEND=c` for a
  plain `--native`); rustc-native only for an edit in what that writer emits.
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
- **Building**: which build answers which question (`cargo check`, the dev binary, one release
  build per fix for the rlib, one heavy job at a time, receipts last) is the checklist in the
  `loft-codegen` skill § Before the push; CI_BUDGET.md holds the costs.

## The agent debug surface — `loft debug <file> --rpc` (drive a live session, don't println)

The @PLN16 debugger speaks NDJSON over stdio (the contract:
`doc/claude/plans/16-debugger/PROTOCOL.md`). One `printf` pipes a whole scripted
session — breakpoint → inspect the live frame → eval → edit → resume. A setBreakpoints
response without `verified` comes from an installed binary older than the protocol —
drive `target/*/loft` from this tree instead.

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

1. **Default is `--native`** — pass `--interpret` for the seeing loop (§ Running a bug down).
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
| `call` | `Parser::call` after `find_fn` | "Which def did the parser resolve, and was it skipped as Generic?" |
| `field` | `Parser::field` after attr lookup | "Did method dispatch find the attribute, what's the receiver type?" |
| `generic` | `predict_generic_return_type` + `try_generic_instantiation` | "What did first-pass predict?  What did second-pass instantiate?" |
| `match` | `expect_match_arm_arrow` | "What arrow did the parser see at the arm boundary?" |

### Adding a new category — selective rule

Trace points are for **recurring** diagnostic vantages, not one-off probes: a
bug-specific `eprintln!` stays local and is removed before commit; a vantage you
expect to revisit becomes a `loft_trace!(category, …)` call.  The mechanics of
adding a category (field, initialiser, macro arm) are documented in
`src/trace.rs` itself — follow it, then extend the table above.

### When NOT to use `LOFT_TRACE`

- Runtime/bytecode debugging — use `LOFT_LOG` (RUNNING_TESTS.md § LogConfig).
- Loft-program-level logging — use `log_info` / `log_warn` /
  `log_error` from inside loft programs (STDLIB.md § Logging).
- A one-time question — an instrument above answers it first.  When a read resists twice
  and none does, add ONE env-gated `eprintln!` (CLAUDE.md § Debugging policy, step 4),
  committed around so it can be removed with the inverse edit, never with git.

## Safety and after the fix

- The kill / wait-by-name / sibling-checkout rules are `CLAUDE.md` § Key commands and § Git
  safety.  One more: **an anomaly right before a destructive command (kill / rm / force-push /
  overwrite) is a STOP, not a footnote** — investigate the surprise first; there is no undo.
- The `Fixes #N` + `Contract:` trailers and fix-not-file are `CLAUDE.md` § Bug-filing policy.
  Beyond it: **re-probe the tracker with what you NOW know** (the fixed mechanism names better
  keywords than the filed symptom did) and add every row the fix closes; leave a substantive
  comment on each (fixing commit, regression test, what verification found); when a PR opens,
  copy every `Fixes #N` into the PR BODY too, since a squash merge can drop the commit trailer.
- Before `fixed-pending-merge`, the **done-gate** — `doc/claude/ISSUE_TRACKING.md` § The
  done-gate (class coverage + intent match: "would the requester file a slight variation?").
  Name any residual you couldn't close.
