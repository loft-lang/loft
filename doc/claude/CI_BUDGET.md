<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# CI budget — what runs when, where the time goes, what to split

> **Where the 20-minute rule is ENFORCED.** In the LOCAL gate, not in GitHub CI.
> `make ci` carries a hard cancel at `CI_BUDGET_SECS` (default 1200s), implemented in
> `scripts/ci_budget.sh`; a run that passes it is killed, `result.txt` gets
> `CI-RESULT: CANCELLED`, and the message says to CUT THE WORK rather than raise the number.
> The reason it is a cancel and not a report: a gate that grows is a gate that stops being run —
> measured 2026-09-12, a local `make ci` on macOS passed **45 minutes**, which is long enough
> that the honest response is to skip it, and a gate nobody runs protects nothing.
>
> GitHub CI's sharded legs previously carried `timeout-minutes: 20` and no longer do; they take
> the same 90 the unsharded legs take, which is a HANG guard rather than a budget. A hosted
> runner's wall time is not the diff's to control — a slow runner, a cold cache or a queue makes
> the same commit red or green — and the cancel threw away the leg's output, the one thing that
> would have said why it ran long.
>
> **The same rule holds for a NIGHTLY job that outgrows its limit: the limit stays, and the job
> does less repeated work** (owner's direction).  The valgrind sweep was cancelled at
> its 120 minutes on every run.  The cure was measured, not assumed: a per-file timing column
> showed the stdlib being re-parsed under memcheck in every one of 1 753 runs and a handful of
> store-ceiling guards running into the per-run limit.  Parsing once (a warm bundle) and planning
> the runs from a plain pre-pass took the work from 20 046 s to 11 913 s — TEST_ENVIRONMENTS.md § Recipe
> (valgrind).  Profiling the slow files also found four defects, a quadratic interpreter path
> among them, which a longer limit would have kept hidden.
>
> ⚠ Two gates on this box roughly double each other's wall time, so a sibling checkout running
> its own gate is the one case where `CI_BUDGET_SECS=... make ci` is the right answer for a
> single run. That is a fact about the box, not about the diff.
>
> **A macOS box is not the Linux gate's twin, and four facts bit.**  Its
> `/bin/sh` is bash 3.2: a single quote inside `"${x:+…}"` is read as an opening quote (the
> recipe died with *unexpected EOF while looking for matching `'`*), and `BASHPID` does not
> exist.  It ships no `flock` (`brew install flock`; the recipe runs UNSERIALISED without it,
> saying so).  It has no `/proc`, so `scripts/gate_lock.sh` can tell HELD from FREE but never
> name the holder — it answers `HELD_UNKNOWN` there and its self-test SKIPS, stated, because
> its cells are /proc verdicts.  And this box runs an endpoint-security agent (Kandji's ES
> system extension) at a steady 25–37 % of one core, which taxes every exec and file event —
> the spawn-heavy phases (rustc per test program, the scratch sweep) more than a hot loop —
> so a timing taken here beside a build is not a timing.
>
> ⚠ A mid-run kill can tear the debug rlib (`undefined symbol: anon.*.llvm.*` out of
> `libloft.rlib`); recover with `cargo clean -p loft`. The cancel message says so.

## `CI-RESULT` measured only the TEST phase — cause found and FIXED (2026-09-01)

**`CI_MAX_FAIL` — how many test failures a gate collects before it stops (default 5).**
`make ci CI_MAX_FAIL=1` is the old fail-fast behaviour and `CI_MAX_FAIL=all` runs the whole
suite whatever happens; the run's own header line in `result.txt` says which it used.

The default moved off 1 because of what a stop-at-one gate costs when a tree has SEVERAL
independent failures: each ~20-minute gate reports exactly one, and the count is learnable only
by fixing and re-running. Measured 2026-09-08 across the two-checkout join — three consecutive
gates, each cancelled at a different first failure: a stale `doc/pkg` browser bundle, then a
golden mismatch, then a whole-corpus keyed-store regression that had been present the entire
time and was the serious one. Two full gates bought no information about it.

Not `all` by default either: a genuinely broken tree fails thousands of tests and then spends
its full wall clock saying what the first screenful already said. Five answers the question the
count is actually being asked — *is this red one defect or several?* — and a run that wants the
complete list asks for it.

The verdict is the whole `&&` chain: a red `fmt` or `clippy` fails the gate and skips no later phase.  Before 2026-09-01 a `;` split the chain and `CI-RESULT: ALL GATES PASSED` measured the test phase alone; the incident is in the history companion.  Read `CI-RESULT` in `result.txt`, not the exit status of whatever launched the gate.

**And a bare `cargo clippy` will not show you what `make ci` denies.** `make ci` passes
`-D warnings`, so lints that print as *warnings* in an ad-hoc `cargo clippy --release
--all-targets` are *errors* there. An eight-argument function sat as a visible warning through
several ad-hoc lint checks and a dozen full-gate runs before anything reported it as a failure.
Run `cargo clippy --release --all-targets -- -D warnings` when you want the answer `make ci`
will give.

## Do not rebuild under a running suite — and the open gate-efficiency work (@PLN159)

**The same hazard, one step smaller: do not REBUILD while a background suite runs.**  The gate
lock stops a second `make ci`; nothing stops a `cargo build` in the same checkout, and the running
suite links what it finds rather than what it started with.  Measured 2026-09-16 on
`157-native-4x`: rebuilding the release binary and the cdylibs under a `find_problems.sh --bg` run
produced **eleven** failures — three `par_nested`, five `n3_parity`, `ownership_drop_gate`, a
`poison_claim` TIMEOUT at 300 s — none of which reproduced on a clean run of the same commit, and
all of which read as ownership regressions rather than as what they were.  One real failure hid
among them.  So: do every build, `cargo fmt` and `clippy` leg BEFORE arming the suite, and while
it runs touch only files nothing is compiling — docs, plans, an issue.  This is the sibling of
PERFORMANCE.md's *never run perf or a bench beside the gate*, and of the same @PLN159 cause: a test
binary and the artefact it links have to come from one source state.

### Open work (from @PLN159, closed 2026-09-08)

| item | what | trigger / size |
|---|---|---|
| **E — read the PR** | On the first real PR run after the split: the four shards' walls (`gh run view --json jobs`), the `Finished test profile` line inside `Test` (expected seconds, was 4m05s), and the run AFTER it restoring warm (`Build` shows deps Fresh, the corpus and build caches restored).  Record here; the projection was heavy 29.3 → ~18 min | the next PR; XS |
| **F′ — a cache for the wasm builds** | `--html` / `--native-wasm` write `<dir>/.loft/<stem>.wasm` with no key and recompile every run (`html_embed` 60 s per test isolated).  A `<stem>-<hash>` entry keyed like the native cache (generated Rust ⊕ wasm rlib CONTENT ⊕ `wasm-opt --version` ⊕ flags); seam: a lookup after `prog.rs` is emitted at `src/main.rs` "Compile to wasm32-unknown-unknown cdylib", a publish after `wasm_bytes` is read before "Assemble HTML".  Pays only on a run whose wasm rlib did not move — a docs- or tests-only re-gate — never on a fix iteration; a `--native` probe needs no cache (0.3 s cold, and the emitted Rust embeds the script's path, so a content key is a per-name key) | when docs-only re-gates are measured to matter; S |
| **G — the gate lock's default** | `make ci` queues behind another checkout's gate (`/tmp/loft-gate.lock`); `LOFT_GATE_PARALLEL=1` restores the throttle.  Queue finishes the first gate at 1× and the second at the same 2× the throttle gave both, and removes the OOM and load-flake reruns; an agent waiting on the lock is idle | the owner's call; XS to flip |
| **Pin the artefact a long gate measures** | A sweep that consumes `target/release/loft` records its hash at start and fails loudly when it moves (or copies the binary aside); the gate lock is `make ci`-only and does not stop a `cargo build` beside a sweep | when the next sweep is contaminated; S |
| **The crate split** | The 22 s a one-function edit costs is the 325 k-line crate's frontend; a workspace (parser · typing · store · runtime · codegen · cache/registry) gives cargo's crate-level "unchanged, not rebuilt" — the true analogue of a Makefile's per-file rule | H; a plan of its own |

## A LOCAL `make ci` is ~10 min, and it is two tests (2026-08-21)

This document is about the CI runner. A developer's complaint is different — *a local
`make ci` costs ten minutes and blocks iteration* — and it has a different answer, so it is
recorded separately rather than folded in.

**A red gate is not re-run to verify its fix (owner).** Once a gate has named the
tests it failed, what is left to check is those tests: rerun them alone
(`cargo test --release --test <binary> <name>`, or `find_problems.sh --changed`) and push.  A
test whose own retry passed in the gate (`TRY 2 PASS`) is rerun alone too, never by
restarting the gate.

**Measured on 24 cores.** Full run: **572 s**, of which `cargo nextest` is ~478–572 s and the
three builds ~130 s. So the test step is the whole question.

**The thread count is capped by MEMORY as well as cores.** `make ci` sizes its
build and test parallelism as `nproc / live-gates`, floored at 2 — and now also capped at
`MemAvailable / 0.7 GiB` (`CI_MEM_JOBS` in the Makefile): a thread's peak is roughly 0.7 GiB
(rustc for native fixtures, the release build's codegen units), so sizing by cores alone
over-commits a small-memory box into swap — measured on a 14 GiB laptop, 20 threads drove
swap use from 7.6 to 10.3 GiB mid-gate while a browser and rust-analyzer held their usual
residency. `MemAvailable` is read once at gate start and already discounts that residency;
the gate's banner says when the cap bit (`memory-capped`). GH runners are RAM-rich per core,
so CI itself never throttles.

**When a gate DIES, ask who signalled it before asking why.** Two `make ci` runs ended on
2026-09-04 with `make: *** [Makefile: ci] Terminated` — SIGTERM, so not the kernel OOM
killer or `systemd-oomd`, which send SIGKILL and print `Killed` — with no OOM record in the
journal and every checkout's tooling killing only by recorded pid. Both had been started as an
agent tool's background task, which makes the gate a child of that tool's process tree and
lets anything that stops the tree stop the gate. `scripts/ci-run.sh start` is the launcher for
a reason: it detaches the gate (`setsid nohup`), records the signal a wrapper receives, and,
with `strace` on the PATH, runs `make` under a signals-only trace so the sender's pid, uid and
`si_code` land in `target/gate-signals.log`, beside a process-table snapshot taken the moment
`make` dies (`target/gate-killer-snapshot.txt`). `scripts/ci-run.sh status` then answers
KILLED with the sender named, instead of a verdict-less `result.txt`.  Two more instances
2026-09-07 (one 5 s in, one 7 min in, both launched as agent-tool background tasks; kernel
journal, `systemd-oomd` and `systemd-tmpfiles` all clean) — the pattern is the harness's
process tree, not the box, and `ci-run.sh start` is the launcher that survives it.

**A SECOND gate on the box turns native tests RED, not slow.** With
`LOFT_GATE_PARALLEL=1`, or with a sibling checkout's gate live while yours starts, the two
storms of `rustc` meet and the LINKER is what gives: a native cell fails with `native compile:
error: linking with `cc` failed: exit status: 1`, the harness reports it as a failed test, and
the verdict names a subject nothing is wrong with. Measured over four runs of one tree on one
night: `keyed_fast_paths::the_general_paths_give_the_same_answers_on_native` failed in the two
gates that overlapped another gate (all 14 cells, every one a link failure — one of those
windows also killed the sibling's own gate with `3227/5263 tests were not run due to signal`),
and PASSED in the two that did not, while passing standalone at load 37 in between. So a lone
native red in a loaded gate is a claim to re-run before it is a defect to chase; the
discriminator is a gate on a quiet box, and it costs one run. The load average at the START of
your gate is what predicts it — `uptime` before `ci-run.sh start`.

**A gate that reports `QUEUED behind another gate` may be queued behind NOTHING.**
`make ci` serialises with `exec 9>/tmp/loft-gate.lock` followed by `flock 9`, and fd 9 is
INHERITED by every process the gate spawns — including the long-lived `loft` server children some
tests start (`tests/engine_host_kernel.rs`'s `run_s3_scenario` spawns one per scenario). When such
a child outlives its gate — its `Guard` drop never runs, which is exactly what the paragraph above
describes happening to harness-child gates — it is reparented to init and keeps holding the lock.
Every later gate on the box then blocks in `flock 9` indefinitely and reports only *"QUEUED behind
another gate on this box"*, naming a gate that no longer exists. Measured: an orphan from an
08:48 run held the lock while both checkouts' gates sat queued for 48 and 20 minutes with a
one-minute load average of **0.05** — nothing was compiling, in either tree.

The tell is that the holder is not a gate: `fuser -v /tmp/loft-gate.lock` names a `loft`
process, and `ls -l /proc/<pid>/fd/9` points at the lock file. Clear it by killing that pid
specifically — never by pattern, which reaches the sibling checkout's live gate. **The load
average is the cheap discriminator** — a real queue has a busy box behind it.

**And the reporting was the other half — the 90 minutes went to a MESSAGE, not to the lock.**
Closing the descriptor stops this cause; it does nothing for the next one, because the waiter
could not tell a real queue from a stuck lock and said the reassuring thing either way. Three
defects stacked in six lines: `flock 9` waited unbounded, the *"QUEUED behind another gate"*
line asserted a fact nothing verified, and it printed ONCE and then went silent — so a static
line read as normal while both boxes idled. Meanwhile every reader of `.ci-running` already
gated on `kill -0` (seven sites: `ci-run.sh`, `box-claim.sh`, `Makefile` ×3). The LOCK was the
one place that never asked, and it is the one that hung.

So the question has ONE home, `scripts/gate_lock.sh` — `state` · `why` · `doctor` · `selftest`
— consulted by the Makefile's waiter, by `ci-run.sh status`, and by `ci-run.sh doctor` alike,
because a second decoder of it is what produced the defect. It answers `FREE`,
`HELD_LIVE <pid> <cwd>` or `HELD_ORPHAN <pid> <cwd>`:

* the waiter re-reports every 60 s (`flock -w 60` in a loop) with the holder and the load. It
  deliberately does NOT time out and fail — a slow box must not become a failed gate; what it
  may not do is be silent.
* `ci-run.sh status` splits the two answers that need OPPOSITE responses. **QUEUED** (a live
  gate in another checkout holds it) keeps `wait` waiting, and now says so truthfully instead
  of reporting `RUNNING` while nothing compiles. **BLOCKED** (nothing running accounts for the
  lock) exits 3, so `wait` stops instead of hanging as before.
* `ci-run.sh doctor` is the front door for *"why is my gate not running"*: fd holders with
  their ppid and cwd, orphans flagged, the load, each checkout's claim, and a verdict.
  Enumerating that by hand is what this cost twice in one session.

⚠ **The holder record never decides.** `gate_lock.sh claim` writes `<pid> <cwd>` after
acquiring, but the state is derived from the KERNEL — a holder is accounted for when its
`ppid != 1` (an orphan is reparented to init) or a live `.ci-running` names it. The record only
supplies a human label. That distinction was measured the hard way while building this: the
first version read a MISSING record as "orphan", and loft2's gate — mid-`cargo-nextest`, no
record because it ran the older Makefile — was confidently reported as an orphan whose pid
should be killed. A verdict that tells an operator to kill a live gate is worse than the
silence it replaces, so the live-holder-without-a-record case is a permanent cell in the
selftest (`make ci` runs it beside the other script self-tests; cells 3 and 4 are the two
observations that look identical and need opposite verdicts).

⚠ **`make -n ci` RUNS the gate — it is not a dry run.** The whole `ci` recipe is ONE
backslash-continued line, and it contains `$(MAKE)`; make executes a recipe line mentioning
`$(MAKE)` even under `-n`, so the entire chain runs. Measured 2026-09-10: `make -n ci` as a
"does the recipe parse" check took the box lock, wrote the holder record and started
compiling — while an unrelated gate's `result.txt` was still the current one, so the run
appended into a file whose header names a DIFFERENT run, which is this document's own
stale-verdict hazard arriving by a new route. To check the recipe's shape, read it, or run
`make --dry-run` on a target that does not recurse.

**The verdict line names the failing TEST and how many, not the first `error[` in the file.**
`ci-run.sh` used to take `grep -m1 "^error|FAIL \["`, and a cargo error always comes BEFORE the
test run, so a gate whose only failure was `doc_hygiene::quality_optional_table_matches_the_audit`
reported `error[E0425] … generate_register_from_loft_with_bridges` — a registry package the
branch had never touched, 1500 lines above the real failure (loft#1448).  Both agents on this box
triaged that line and went looking at the cdylib.  **Read the failing test, not the captured
detail**, and the verdict now says it: `FAILED 1 test(s) — loft::doc_hygiene::quality_optional…`.

The COUNT is the other half, because it splits two reds that need opposite responses.  **FAILED
with 0 test failures is the toolchain, the box or the target dir** — a stale `libloft.rlib`, a
corrupt `target/debug/incremental`, a full disk — **and FAILED with a count is the code.**  That
distinction was re-derived from error text three times in one evening before it went into the
line.

**Kill a background sweep by its PARENT, and the parent is not named after the work.**
`scripts/valgrind-sweep.sh` runs its memchecks under `xargs -P N`, so killing every
`valgrind.bin` just lets the `xargs` start the next batch, and a `pkill -f valgrind` never
matches the process that owns the queue.  Worse, if the launching shell is gone the `xargs` is
reparented to init, so it survives being aimed at through its own session.  Find it with
`ps -eo pid,ppid,pgid,cmd | grep "[v]algrind"` and kill the PPID the children share.  The
general shape: a name-based kill can only find processes whose name you already know, and a
work queue's parent shares no name with its work.

**A background gate whose SUBJECT keeps moving measures nothing, and nothing in the foreground
says so.**  A valgrind sweep left running against `target/release/loft` while that binary was
rebuilt five times for an unrelated fix produced 250 rows of results about no particular build.
The rule — do not rebuild while a gate runs — is easy to hold for a foreground command and easy
to forget for one that is already detached, which is exactly when it costs the most.  Pin a long
sweep to its own worktree (`git worktree add --detach <dir> HEAD`, build there, run there) so the
main checkout stays free to iterate.

**A gate makes its own room.**  A full disk fails the NATIVE corpus with `FAIL
unknown-mode` after `low space` lines and the linker with `cannot find lib<dep>.rlib`, both of
which read as code faults; every gate therefore runs `scripts/disk_headroom.sh` first, which
reclaims loft's own scratch, the incremental caches and — with no gate alive — this checkout's
gate scratch until 20 GB is free, and refuses below 2 GB (RUN_BOUNDS.md § Scratch hygiene).  By
hand: `make disk-headroom`.

**Run a 19-second triple FIRST when the change touches parser diagnostics, guards or docs.**
`make ci` stops at its first failure, so each cycle surfaces exactly ONE new problem and costs
the full ten minutes to do it. Measured over one afternoon's work on the nullable-collection
cluster: five consecutive cycles, each ending on a different thing — a stale audit row in
QUALITY.md, a flaky browser test, a cdylib rebuild that blew a 60 s per-test budget, a real
corpus breakage, and a stale `doc/examples.js`. Three of those five are caught by

```bash
cargo nextest run --release -E 'binary(doc_hygiene) + binary(wrap) + binary(issues)'
```

which takes **19 s**. It does not replace `make ci` — the corpus breakage and the cdylib budget
are only reachable from the full run, and `Stores::find`'s unit test was found by nothing else
— but it converts three of the five ten-minute cycles into one twenty-second one.

Two specific traps behind that list, both worth knowing before they cost a cycle:

* **`doc/examples.js` is a tracked SHADOW of `examples/*.loft`.** Editing an example without
  re-running `loft --interpret scripts/build-playground-examples.loft` leaves the playground
  serving the old text, and `doc_hygiene::doc_examples_js_is_up_to_date` fails. Regenerate in
  the same commit.
* **Never run cargo beside a live `make ci`.** They share a target directory, and the collision
  surfaces as a mold link error — `undefined symbol: anon.<hash>.llvm.<hash>` against a stale
  `libloft.rlib` — which reads exactly like a real link failure and is not. A clean tree with
  nothing else building is the only valid run, and
  the wrapper's own exit code is not the verdict — it has been observed as 0 on a run whose
  `result.txt` said FAILED, so read `CI-RESULT` in `result.txt` rather than `$?` of whatever
  invoked it (a pipe into `tail`, for instance, reports `tail`'s status). `CI-RESULT` itself
  is trustworthy again — see § `CI-RESULT` measured only the TEST phase.

⚠⚠ **The "slowest tests" list is a trap, and reading it is how this went wrong the first
time.** JUnit `time` is WALL clock, so on a saturated machine it counts *waiting*:

| test | in the full run | alone |
|---|---|---|
| `deliver_reconstructs_nested_value_in_js` | **89.4 s** | **0.9 s** |

Summing those wall times gave "4696 s of CPU" and a confident, wrong conclusion that
`deliver_wasm` was the biggest cost. It is ~1 s a test. **Anything derived from JUnit `time`
under load measures contention, not work** — isolate before believing it.

**So the local ten minutes is NOT two tests, and there is no single hot test at all.** Isolating
the slowest entries from a contended run on an idle box:

| test | in the contended run | alone | inflation |
|---|---:|---:|---:|
| `pln10_n2_cdylib_text_wrapper_returns_owned_string` | 280 s | **14.0 s** | 20× |
| `dhtml_vector_arg_gl_host_import_is_emitted` | 234 s | **19.1 s** | 12× |
| `a_declared_font_reaches_the_emitted_page` | 230 s | **0.9 s** | **255×** |

The cost is STRUCTURAL: ~4 474 tests of which a large share shell out to `rustc` or link a
cdylib, saturating the cores, so wall clock is set by how much else is running rather than by any
one test. The two levers that follow are behavioural, not code:

1. **Do not run two gates at once.** A second checkout's `make ci` took this one from ~10 min to
   **19 min** (load 42 on 24 cores) and, the same morning, triggered the `systemd-oomd` kill that
   ended a session. Check `pgrep -af "make ci"` and its cwd first.
2. **Do not use `make ci` as the iteration loop — nor as the answer to its own red.**
   `./scripts/find_problems.sh --subject <name>` is seconds; the full gate runs once per change
   whose reach you cannot bound, and a red one is followed by `scripts/ci-run.sh recheck`, not a
   restart (§ After a red gate).  Measured cost of getting this wrong: six full gates in one day
   on a three-line change, and four in one afternoon on fixes to constants.

⚠ The general lesson is the one this document already teaches about JUnit `time` and did not
apply to itself: **a recorded measurement is a claim with a date on it.** Re-measure before
acting on one, especially when it is the number that decides what to optimise.

### A cloud session: the disk is the limit, not the CPU (2026-09-28)

A Claude Code on the web session runs on a fresh container with four cores, 15 GB of memory
and a FIXED writable allowance, and every gate that died there died of the disk.  Measured
in one session: the machine idle (load 0.02), a release build of `loft` in 2–3 minutes —
and three runs killed mid-link, each leaving a log that ended without a verdict because the
`echo exit` that would have written one failed too.  The symptoms to recognise, all of them
disk: `ld terminated with signal 7 [Bus error]`, `No space left on device`, loft's own *low
space in /var/tmp/loft-test-scratch* warning, a `--native` cell failing with a code-shaped
message (CLAUDE.md § `make sweep-scratch`).  What filled it:

| consumer | measured |
|---|---|
| `target/debug/deps` — every test binary of every dependency hash, 382 of them | 16 GB (`issues`: 225 MB, 96 MB of it debug sections) |
| `target/debug/incremental` — rebuilt by every `cargo check`/`clippy` pass | 5 GB per pass |
| `make falsify`'s control build, one full release target per ref | ~2 GB each, kept under `~/.cache/loft-falsify` |
| `/tmp/loft_native_*`, the test scratch, `.loft/` caches beside probe files | hundreds of MB |

Three things make it reliable, and the first two need nothing from the person:

1. **The session-start hook** (`.claude/hooks/session-start.sh`, web sessions only): sets
   `CARGO_INCREMENTAL=0` (each target is built once per session, so the cache is pure
   cost), removes the previous session's incremental cache and falsify control builds,
   runs `make sweep-scratch`, and prints the headroom.
2. **`find_problems.sh` refuses a run that would not fit.**  A curated or full run needs
   about 20 GB free (`LOFT_GATE_MIN_FREE_GB`, `0` switches it off); short of that it exits 2
   before building anything and names what fits: a `--subject` run, the space to reclaim,
   or the GitHub gate.  Note the trap that reaches this guard: `--changed` widens to the
   whole curated set whenever the diff touches `src/main.rs` or `src/lib.rs`, which every
   test binary depends on — a two-line edit there is a 16 GB run.
3. **The full gate runs on GitHub, not here.**  The section below has the dispatch; a
   session without `gh` uses the GitHub MCP tool (`actions_run_trigger`, `run_workflow`
   on `ci.yml` with `os=ubuntu-latest`, then `actions_get` on the run).  Locally, keep to
   `--subject <name>` and single `cargo test --test <name>` runs, and read `df -h /`
   before each — the allowance never grows back except by deleting.

### When the local gate is unreliable, run the same gate on GitHub (2026-09-08)

A local `make ci` is one process tree on a shared laptop.  On 2026-09-08 the waiter for
one was killed for memory while the gate ran on at 10 of 14 GiB used, and a gate under that
pressure can die the same way (SIGKILL, no verdict) — a run that ends without a verdict is
not a gate, however far it got.  The owner's rule: **do not fight the box; the identical
gate runs on GitHub against the pushed commit.**  `ci.yml` carries a `workflow_dispatch`
trigger, so a branch needs no PR to be gated:

```bash
git push origin <branch>                    # a dispatch runs the commit GitHub holds
gh workflow run ci.yml --ref <branch> -f os=ubuntu-latest
gh run list --workflow ci.yml --branch <branch> --limit 1      # its run id
gh run watch <run-id> --exit-status         # or poll: gh run view <run-id>
```

`os=ubuntu-latest` is the local gate's twin: the PR matrix is Linux-only (macOS and
Windows are placeholders there), and so is `make ci`.  One dispatch per push: the
workflow's concurrency group cancels an in-progress run on the same ref (every ref but
`main`), so a second dispatch after a follow-up commit CANCELS the first — which is the
right outcome (the newer commit supersedes), but a cancelled run is no verdict for the
older one.  A dispatch takes the push-to-main
path rather than the PR path, so it also runs the non-PR extras (the stdlib round-trip, the
differential oracle) — strictly more than `make ci`, in ~20 min.  `os=all` adds macOS and
the ~30-min Windows leg; reach for it when the change touches a platform seam.  `make
release-gate` is the heavier sibling (every nightly against one commit, 60–90 min) and is
release evidence, not a branch gate.

**The inner loop can die the same way, and earlier than the tests (2026-09-21).**  A
`find_problems.sh --subject store` run that had passed two hours before was killed for
memory while still COMPILING — a subject links tens of test binaries in parallel, and the
link step is where the memory goes.  Two things had changed, neither in the repo: a
session's scratch directory under a RAM-backed `/tmp` had grown to gigabytes of finished
clones, and an editor's language server was holding more.  So before a heavy local run on
a box where `/tmp` is a tmpfs, read `free -h` AND `df -h /tmp` (a full `/` is the other
half — `make sweep-scratch`), delete scratch whose durable copy exists, and bound the run:
`CARGO_BUILD_JOBS=4 NEXTEST_TEST_THREADS=4 ./scripts/find_problems.sh --subject <name>`
finishes later and finishes.  A killed run is no verdict, exactly as above.

What a GitHub run cannot do: measure a ratio (`make speed` is a report and stays local;
the native/Rust ratio gate, `scripts/native_ratio.sh --gate`, runs LAST in the local
`make ci` and is deliberately not mirrored in ci.yml, because a shared runner's timing is
noise and a flaking timing gate is one people learn to ignore) or read this box's scratch.  What it does that a local
run cannot: run cold, on a machine nobody else is using, and leave a verdict that
`release-checklist` can read by sha.  Local tooling (`scripts/ci-run.sh`,
`find_problems.sh --subject`) stays the inner loop; the dispatch replaces only the final
`make ci`.

## After a red gate: recheck, do not restart

**The rule.** One full gate per change whose reach you cannot bound.  When it goes red, fix what
it NAMED and run `scripts/ci-run.sh recheck`; do not start another gate.  A recheck builds on the
last gate in this tree (`.ci-gate-head`, written by `start`) and runs, each timed:

* the pre-flight — `cargo fmt --check`, QUALITY.md's audit rows against
  `ir_walker_audit.py`, `check_doc_drift.sh` (`scripts/gate_preflight.sh`, ~15 s);
* both clippy variants the gate runs, when Rust changed since the gate;
* EXACTLY the tests the gate failed (`.ci-failed`, nextest `binary_id(=…) & test(=…)`);
* `find_problems.sh --changed <gate sha>` — the subjects the change since the gate touches.

A green recheck writes `.ci-recheck`, and `ci-run.sh status` shows it beside the gate verdict
for as long as it describes HEAD.  The push then names both: *full gate on `<sha>`, and a
recheck of the change since.*

**When a new full gate IS owed** — the fix's reach is not known, so `--changed` cannot map it:
a new refusal or warning, a type-inference or coercion change, a new op or builtin, a change to
shared lifetime/codegen machinery.  `--changed` maps a PATH to a subject, and a change to what the
compiler ACCEPTS can break a cell in a subject it never names (a new refusal broke three tests
in `codegen` from a `.loft` under `doc/claude/plans/`, 2026-09-24).  A fix to a constant, a
fixture, a derived row, formatting or a lint attribute is not that.

**The pre-flight** runs in `ci-run.sh start` and refuses to queue a gate that would stop on one
of its checks (`CI_NO_PREFLIGHT=1` skips it; `CI_PREFLIGHT=full` adds `doc_hygiene` and
`frontend_counts` through nextest — seconds of tests, minutes of compiling when the release
test binaries are stale, so not the default).

## The rule that decides placement

**The PR gate tests what THIS DIFF can break. The nightly tests what THE WORLD
can break.**

Toolchain drift, published-library rot, registry decay and time-passing checks
are "the world": running them per-PR says nothing about the diff, costs the
author minutes, and trains everyone to ignore red. Memory safety, codegen
correctness and internal invariants are "the diff": they must fail on the PR that
introduced them, when the author still has the context to fix it.

A second rule follows from the first: **a gate that cannot fail because of a diff
does not belong on a PR, however cheap it is.**

## What runs when — today

| cadence | jobs | trigger |
|---|---|---|
| **per PR** (`ci.yml`) | full suite ubuntu + macOS, ASan UAF/OOB (ubuntu), `stack_align_guard`, browser build+probe, Clippy, Format, Doc hygiene, CodeQL (`codeql.yml`, scoped by `.github/codeql/codeql-config.yml`), feature catalogue, contract-goldens drift, API compat, several advisory doc jobs | `pull_request` |
| **push to main** | everything above **plus the real `Test (windows-latest)` leg** (~53 min) | `push: main` |
| **nightly 04:00** (`miri.yml`) | Miri ×2, ASan UAF/OOB ×2, ASan interpreter leak ×2, POISON arena-UAF, STACK-SHADOW frame-slot gate, TSan, native-backend ASan, debug-assertions, valgrind memcheck sweep (release binary, both backends), release-gate sweeps (the ignored ownership fuzz replay + SI-2 check), toolchain matrix (beta+nightly), doc index hygiene, library health, stale-plan audit | `schedule` |
| **nightly 04:30** | `registry-validation` (scope `tip`) — each published package's NEWEST stable installed + tested on both backends, 42 legs | `schedule` |
| **Sundays 05:30** | `registry-validation` (scope `full`) — EVERY non-yanked published version, 164 legs.  The nightly only ever validated the tip, so 121 of 164 versions were checked by nothing (loft#1462, the gate hole behind #1448).  Weekly rather than nightly because a rotted OLD release breaks nobody until somebody pins it | `schedule` |
| **nightly 06:17 + on `src/**`,`default/**`** | `revalidate-libs` — ONE suite pass per published lib, scored twice: does it still compile + test (a break is the freeze's business), and is it RELEASE-READY — RED on any warning, since a lib carrying them fails its own `LOFT_DENY_WARNINGS` CI. **Advisory, never a required check**; blocking on a release | `schedule`, `push`, `pull_request` |
| **nightly 07:00** | `lib-branch-report` — unmerged branches across the library repos | `schedule` |
| **daily 03:00** | the Windows leg (mirrored onto PRs as the non-blocking `Windows (daily)` check) | `schedule` |
| **daily 05:00** | `browser-threads` — the threaded-wasm browser leg | `schedule` |
| **daily 05:45** | `lib-main-health` — published libs against their own `main` | `schedule` |
| **daily 06:15** | `consumer-main-health` — each consumer's tip (moros, dryopea, Moros-Economy-Development, crawler) against loft main, every package on both backends; a stability meter (STABILITY_ROADMAP.md § The owner's directive) | `schedule` |
| **daily 06:45** | `switch-ab` — every both-backend rewrite off against on (`scripts/switch_ab.sh`) over the corpus and the public consumers; `(R-Switch)`'s landing gate | `schedule` |
| **Mondays 06:00** | `repro-build` — reproducible-build check (weekly, not nightly) | `schedule` |
| **per PR + push to main** (`daily-build.yml`) | the four RELEASE bundles — `x86_64-pc-windows-msvc`, both Apple targets, `x86_64-unknown-linux-musl` — each through `scripts/make-release.sh`, smoke-tested by running a program out of the unzipped bundle, uploaded as a 30-day artifact.  Called the **daily build** because that is the term users know; the cadence is per PR, which at one or two PRs a day comes to the same thing.  **Advisory, never a required check** — a bundle leg must never block a merge the gate itself passed.  `concurrency: cancel-in-progress`, so a re-push replaces its predecessor rather than queueing.  Exists because release bundles are attached to `v*` tags ONLY, so between monthly releases the only way to run current loft was to compile it — which needs a machine that survives `codegen-units=1` on this crate, and a contributor whose laptop does not is what prompted it | `pull_request`, `push: main`, `workflow_dispatch` (one triple or all) |
| **library repos** | one `library-ci` per repo, all callers of `library-ci-reusable.yml`, and **`ci / <package>` is a REQUIRED check on every repo's `main`** (41 contexts, one per package; `strict` off, so a PR need not be rebased onto a moved `main`, and `enforce_admins` off, so the owner's direct pushes to `main` still land — the way every library fix reaches it today): the per-package test matrix, plus a repo-level **`unreleased work`** job — a branch ahead of the default branch with no PR, a PR nobody has touched, or a `loft.toml` version the registry has never seen, each red after 14 days without activity (`scripts/unreleased-work.py`, `stale-days` to tune) | `push: main`, `pull_request` |
| **on demand only** | `ci-probe` (where CI time goes) and `gate-probe` (re-runs the debug-assertions sweep and the browser UI gate on a real 4-vCPU runner, each beside a cell proving it can still FAIL). Measurement, never gates, never on a PR | `workflow_dispatch`, or push to the `ci-probe` / `gate-probe` branch |
| **on demand — the release evidence** | `release-gate` — every row above that is a nightly (`ci.yml` full matrix incl. Windows + round-trip + oracle, `miri.yml` all gates, `revalidate-libs`, `browser-threads`, `repro-build`; `registry-validation` is deliberately not a leg, since the registry is never release-coupled and a package's own defect turned the gate red twice) called as reusable workflows against ONE commit, ending in one `verdict` job that is red if any leg is not `success` — advisory PR jobs included, and a leg red for a reason outside the candidate WAIVED on the record (`make release-checklist ARGS="--waive <leg> --note '…'"`). `make release-gate` dispatches and waits; `make release-checklist` reads the run for HEAD's sha. Never on a PR, never scheduled, never tags (§ The schedule is not a clock) | `workflow_dispatch`, or push to the `release-gate-probe` branch |

## The schedule is not a clock — the release gate (2026-09-04)

The schedule gives no evidence about a merge (a scheduled run starts when GitHub gets to it, on whatever `main` is then; the legs that run only in the nightly are where a merge is found red).  The measurements are in the history companion.  One more:

- **Measured 2026-09-07, three at once.** A local `make ci` returned `ALL GATES PASSED`
  (796 s) on a tip whose nightly had THREE red gates: `LOFT_POISON` (a nested tuple's
  copy freed the source's vector), `debug-assertions` (attribute indices tagged as frame
  variables in `Definition.returned`) and `ASan UAF/OOB (macos-latest)` (`pid_alive` knew
  only procfs, so nothing was provably dead off Linux).  All three were regressions from
  one join, all three were green on `main` the morning before it, and none is in the
  local gate's path.  **So a green `make ci` is not evidence about POISON, the
  debug-assertions gate, or any macOS leg** — the same shape as the shipped libraries,
  which `make ci` also says nothing about ([REVALIDATE_LIBS.md](REVALIDATE_LIBS.md)).  Two of the three needed a
  config the box cannot run at all (macOS) or does not build by default
  (`-C debug-assertions=on`, which `[profile.dev.package.loft]` strips), so the way to
  ask before a merge is `gh workflow run miri.yml --ref <branch>` — a dispatch runs the
  FULL nightly set, wider than the push-triggered run that produced the reds.

- ⚠ **Two of those gates need no dispatch at all — they are a MINUTE each on this box**, and
  not knowing that is what makes the round-trip look like the only option.  Both are plain
  `nextest` runs of the ordinary corpus with one env var, and the nightly's own recipe is the
  whole of it:

  ```bash
  LOFT_POISON=1       cargo nextest run --release --lib --test issues --test wrap                         --test strings --test frame_vars -E 'not test(library_suite)'
  LOFT_VERIFY_STACK=1 cargo nextest run --release --lib --test issues --test wrap                         --test strings --test frame_vars -E 'not test(library_suite)'
  ```

  Measured 2026-09-09: **57 s** and **69 s**, 2011 tests each.  Run BOTH before pushing a change
  to shared store-lifetime or codegen machinery, because `make ci` arms neither and
  `find_problems --changed` arms neither — the class they cover is a freed store that is READ,
  which no ordinary run reports, and a frame slot nobody wrote.  A same-session regression was
  caught exactly here on the sibling branch: a per-arm capture release destroyed a record
  handed to the caller through a `&fn(…)` LINK, which no arm can witness, and `1443` read
  `0xBEEF` under poison while every other gate was green.

  The two that genuinely need CI are the macOS legs and the debug-assertions gate, for the
  reasons above — a config this box cannot run, and one it does not build by default.

The **release gate** (`release-gate.yml`) is the deliberate counterpart: the six
nightlies called as reusable workflows (`workflow_call`, the pattern
`library-ci-reusable.yml` already uses) against one commit, with a `verdict` job that
reads every leg's result from `needs` and is red on anything that is not `success`.
Three properties are load-bearing:

- **It cannot drift from the nightly**, because it does not restate the nightly — it
  calls it. A gate added to `miri.yml` is in the release gate the same commit.
- **A called workflow sees its CALLER's event**, so each nightly takes the path its own
  `workflow_dispatch` takes (full matrix, non-PR extras) with no `mode` plumbing through
  its conditions. The one input that exists, `miri.yml`'s `from_gate`, keeps `notify`
  and `daily-status` with the schedule: a candidate run must neither open nor
  auto-CLOSE the nightly's tracking issue. The concurrency groups of `ci.yml`,
  `revalidate-libs.yml` and `browser-threads.yml` carry `github.workflow` (the caller's
  name) so a gate leg neither queues behind nor cancels a standalone run on the same ref.
- **What a PR shows as advisory is blocking here.** A called workflow's result is
  `success` only if every job in it succeeded, so the seven advisory `ci.yml` jobs count
  without a list of names to keep in step.

It is the release's evidence (`A-release-gate` on `make release-checklist`, keyed by
HEAD's commit), not a replacement for the schedule: a red nightly is still fixed the
day it appears, or the gate finds a month of them at once. Cost is the nightlies' own
— `ci.yml` 37–82 min, `miri.yml` 22–34 min, the rest under 15 — in parallel, so about
an hour and a half of wall clock for a release that happens monthly.

## What to optimise and split
A–E (moving the stdlib round-trip pair to nightly, the sharding attempts, macOS leaving the PR matrix, the cheap nightly gates on the PR, the build floor) are implemented, dropped or superseded; their record is in the history companion.

### F. The cache budget — a cache that is never restored is a cost, not a cache (2026-09-28)

**Rule: everything `ci.yml` saves in one run must fit GitHub's 10 GB per-repo cache
budget, with room for the nightlies — and it is saved on `main` only.** Over the budget,
GitHub evicts least-recently-used entries, so an over-budget run evicts its own caches
before the next run restores them. A branch can restore only its own caches and
main's, so a branch save helps that branch alone and evicts main's, which every run uses.

Measured on run 36421513313 and main's 36420537678: a PR run saved **24.8 GB**, a main
run **39 GB** (`Linux-cargo` 13.8, `macOS-cargo` 9.8, the corpus binaries 9.1, three
advisory jobs' own `target` caches ~3.5 each). No run restored a cargo cache — every
leg's `Build` compiled all 474 crates cold (331 s), and the release, wasm-rlib and
warm-up steps rebuilt on top of that, **~14 of the ~24 min per leg before a test ran**.

What `ci.yml` does since:

- **Saves on `main` only** (`github.ref == 'refs/heads/main'`); every other run restores.
- **`target/loft-native-cache` is out of the Linux/macOS cargo cache.** The compiled
  native test binaries are 9–13 GB, and their key includes the `libloft.rlib` content
  hash (`tests/native.rs` `cache_key`), so any commit touching loft's source misses
  all of them. The `corpus` shard's own cache of them is gone for the same reason.
  Windows keeps its copy: that cache holds nothing else large.
- **`index-hygiene` and `viewer-smoke` restore the `test` job's cache read-only.** They
  build the same release `loft`; their own 3.5 GB caches only competed for the budget.
  A restore matches only a save with the identical `path:` list, so theirs is a copy.
- **The save key is the restore step's `cache-primary-key`.** A second
  `hashFiles('**/Cargo.lock')` at save time also hashed the gitignored
  `tests/fixtures/libs/*/native/Cargo.lock` the warm-up step creates, so the saved key
  never matched the next run's exact-prefix restore key.

**The other workflows follow the same rule.** Every cache step is a `restore` plus a
`save` that runs on `main` only and not after an exact hit, so a nightly refreshes its
cache and a PR, a tag or a probe branch reads it without writing. `branch-gates`
(never on main), `api-compat` (PR only) and the nightly `index-hygiene` in `miri.yml`
restore ci.yml's `test` cache read-only instead. The measured offenders were the
nightly hygiene cache (3.3 GB), `macOS-v2-cargo` (6.1 GB from a branch push) and one
357 MB `branchgate` entry per pushed branch. Two exceptions: `ci-probe.yml` measures
a cold→warm pair on its own `probe-*` namespace and still saves from its branch, and
`library-ci-reusable.yml` runs in the calling library's repository, against its budget.

To check it holds: `gh api repos/loft-lang/loft/actions/cache/usage` after a main run, and
the `Restore cargo registry and build` step of the NEXT PR run reading `Cache restored
from key`, not `Cache not found`.

## The daily overview

Two problems today: a red nightly is **undifferentiated** (six nights of
`registry-validation` failure looked identical to the five before it, so nobody
reached the cause), and **every** non-success files a GitHub issue — which is how
a jq crash in a matrix selector became a ticket.

### Narrow the auto-issue to the blocking class

`notify` currently watches nine gates and files on any non-success. It should file
**only** when the language itself is unsound:

- Miri hard-UB · POISON arena-UAF · ASan UAF/OOB · debug-assertions

Everything else reports without ticketing. Once those four are gated per-PR
(section D of the history companion), a nightly red in them means something slipped past the PR gate —
exactly the case worth interrupting a human for.

### One "Daily status" run, not an issue list

A single scheduled workflow, after the others, writing **one job summary**:

- every gate with conclusion + link
- the library warning dashboard (`lib_warning_scan.py collect`)
- `registry-validation` result, per package
- in-flight library branches
- anything INCONCLUSIVE — a gate that did not run proves nothing

Its own conclusion goes red **only** on the blocking class, so a README badge for
that one workflow answers "is anything blocking?" without opening anything. Cost
is API calls — well under a minute.

### Reading a red nightly — the day it appears

The release gate does not change the daily discipline: a red nightly is fixed the day it
appears, because the legs that run only there — macOS, Windows, the oracle, the sanitizer
and invariant gates — are where a deep-internals change is found red, and each unfixed one
masks the next. Three things make the reading honest:

- **Read the run's ref before debugging it** — `gh run view <id> --json headBranch,headSha`.
  The nightly runs `main`, and a commit on your branch may already have closed it: loft#1133
  was auto-filed from the debug-assertions gate and did not reproduce on a working tree whose
  fix had landed sixteen minutes after the nightly started. "Cannot reproduce" reads as
  flakiness when it is a fix you already have.
- **Build a control at that sha without touching your tree** — `git archive <sha> | tar -x
  -C <dir>`: no worktree, no branch, no index change. Confirm the failure there with the
  gate's exact command, copied byte-for-byte from the workflow yaml; then attribute the
  fixing commit by reading the diff, and verify by running the same command on your tree.
- **A separate `CARGO_TARGET_DIR` needs the stdlib beside the binary.** Every test that
  SPAWNS the loft binary fails with *"cannot load standard library"* until
  `ln -sfn <repo>/default <target>/release/default` — four harness artefacts read as
  findings before that was known.

### What "implemented" means here
- **Phase 4** — `notify` files an issue only for the gates whose red means *the language
  is broken*, never from a PR; `daily-status` writes the single digest.  Which gates those
  are is **not repeated here**: each job in `miri.yml` carries `# @nightly-class:
  unsound|report` as the first line of its block, and `notify.needs`, `daily-status.needs`
  and the digest's closing sentence are all derived from it, with
  `doc_hygiene::nightly_gate_classes_drive_every_list_that_reads_them` failing when they
  disagree or when a job carries no class at all.  The rule that motivates the marker is
  the one the job list always stated — a gate absent from `needs` reads as green and
  AUTO-CLOSES the issue — and the marker exists because prose did not enforce it: `valgrind`
  was in no list at all, so when memcheck went red on 2026-09-07 the filed issue named only
  `poison,debug-asserts` and the next green run would have closed it with valgrind still
  failing.  @PLN154's `stack-shadow` costs ~2x the in-process interpreter
  corpus (67-93 s against ~50 s for `loft_suite` locally, the spread being box contention), needs no sanitizer and no nightly
  toolchain, and covers the residence POISON cannot describe: poison needs the slot to hold
  a distinguishable byte pattern, and a recycled frame slot holds a plausible one.

## See also

- [TESTING.md](TESTING.md) — the test framework; [RUNNING_TESTS.md](RUNNING_TESTS.md) — `LOFT_LOG`, targeted-suite map
- [DEVELOPMENT.md](DEVELOPMENT.md) — workflow and where changes land
- [PERFORMANCE.md](PERFORMANCE.md) — runtime benchmarks (not CI cost)
- [CI_BUDGET-history.md](CI_BUDGET-history.md) — the dated measurements and experiments behind this doc
