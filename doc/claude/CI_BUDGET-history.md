<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# CI_BUDGET-history.md — the timeline behind CI_BUDGET.md

The measured record behind [CI_BUDGET.md](CI_BUDGET.md): dated incidents, the sharding and placement experiments, the compliance readings.  Sections keep their original headings.

---

>
> **And the gate's minutes here are CONTENTION, not work — measured the same day.**  The
> 20-minute budget cancelled a `make ci` at 1 935 of 4 864 tests; its JUnit record named six
> tests at 128–287 s each.  Run alone on the quiet box they took 21–78 s (215 s for all six
> against ~1 150 s in the gate, an inflation of 4–13× per test — the JUnit trap this document
> already describes, confirmed on a second machine).  One `loft --interpret` process costs
> ~60 ms here, one bare `fork+exec` 9–10 ms (a Mac without an ES agent: ~1 ms; a Linux
> guest on this box: 0.25 ms).  Then the thread count itself: the same 243-test slice
> ran in **390 s at 14 threads and 325 s at 7** — halving the threads was 17 % FASTER,
> because each nextest thread spawns its own `rustc`/`loft` processes, the last four threads
> land on the four efficiency cores, and every spawn pays the agent.  `CI_NPROC` therefore
> defaults to half the logical cores on macOS (`CI_JOBS=<n>` overrides; 10 is untested).
> The rest of the answer for this box is a Linux guest: the existing Lima instance (vz,
> Ubuntu 25.10, ext4, the repo cloned INSIDE the guest disk — never built from the
> read-only virtiofs `~` mount) measures a 40× cheaper exec and has the toolchain; it was
> resized to 8 CPUs / 32 GiB for a gate run.
>
> **A RAM volume buys nothing here — measured, so nobody tries it again.**  One emitted
> program through `rustc -O` (the native corpus's unit of work, 3 000 lines) with `-o` and
> `TMPDIR` on APFS: 409–410 ms; on a 1 GiB `hdiutil` RAM volume: 406–409 ms (3 rounds × 5
> compiles each).  The compile is CPU, link and code-signing, not disk — and it is 2× the
> 0.20 s the same unit costs on Linux, which is the spawn tax on rustc's own children.

---

> **Status: DESIGN (2026-07-26).** Measured against real runs, not estimates:
> PR run `30203257738` and nightly run `30191161652`. Every duration below is
> observed. The owner's rule is the premise: **a normal PR must settle within
> 20 minutes**, and anything that pushes past it gets parallelised or moved to a
> slower cadence until we are consistently back under.
>
> **Compliance reading (2026-08-10): not met at 31.6 m; the cause is now measured and
> the untried lever has been PULLED — awaiting real-run confirmation.** See
> [§ Where the 31 minutes actually are](#where-the-31-minutes-actually-are-2026-08-10--measured-and-one-axis-untried):
> a single job is the whole critical path, its members' `rustc` storms make the
> `heavy-serial` group ADDITIVE with the rest of the suite when caches are cold (which
> CI always is), and both recorded sharding attempts split across that group rather than
> along it. The PR leg is now split ALONG that boundary; the projection is ~21.5 m, and
> `cargo nextest archive` was evaluated and rejected (it serialises the build ahead of
> the shards). Do not record this as met until measured — the two prior attempts each
> looked sound on paper and bought fifteen seconds.
>
> **Compliance reading (2026-08-09): the rule is not met, and has not moved since
> this design was written.** Wall-clock to settle for the last 13 completed
> `pull_request` runs of `ci.yml` — created→updated, which is what an author
> actually waits — is **24–34 minutes, median ~31**. The one run under 20 minutes
> failed early, so it is not evidence of a fast path. Reproduce with:
> `gh run list --workflow=ci.yml --limit 40 --json createdAt,updatedAt,event,conclusion`.
> The per-job analysis below already records both test legs exceeding 20 minutes;
> what this adds is that the whole-run figure has stayed there, so the
> "parallelise or move to a slower cadence until we are consistently back under"
> half of the rule is still owed work.


---

**Measured, on two checkouts, after a full day of reading it as the verdict.** `result.txt`
held all of these at once:

```
error: this function has too many arguments (8/7)      src/scopes.rs
error: doc list item without indentation               src/generation/mod.rs
error: could not compile `loft` (lib) due to 2 previous errors
     Summary [ 362s ] 4537 tests run: 4537 passed, 35 skipped
CI-RESULT: ALL GATES PASSED
```

The clippy phase failed, the run continued into the tests, and the success line was emitted
anyway. Every "4537/4537, ALL GATES PASSED" reported that day was measuring the TEST phase
alone, on both checkouts independently.

### The cause: one `;` split the chain in two

The recipe is a single `&&` chain from `rebuild-native-cdylibs` to `nextest`, and the verdict
hangs off its tail. Two bookkeeping lines inside it were `;`-separated:

```make
./target/release/loft cache warm --from tests >> result.txt 2>&1 && \
gates=$(CI_LIVE_GATES); jobs=$$(( ... )); [ $$jobs -lt 2 ] && jobs=2; export NEXTEST_TEST_THREADS=$$jobs; \
echo "make ci: tests on ..." >> result.txt && \
cargo nextest run --profile ci >> result.txt 2>&1 && \
echo 'CI-RESULT: ALL GATES PASSED' >> result.txt || \
{ echo 'CI-RESULT: FAILED ...' >> result.txt; ...; exit 1; }
```

A `;` **terminates** the `&&` list before it. So the shell saw two independent lists: everything
from `fmt` through `cache warm`, whose exit status was discarded, and then the nextest phase,
which is the only thing `CI-RESULT` ever reported. A red `fmt` or `clippy` did not merely fail
to fail the gate — it SKIPPED every phase after it in the first list (clippy, doc-drift,
label-guard, the five builds, the target-surface check) and the run went straight to the tests.

That also explains the tell: between the failing phase's output and the nextest banner,
`result.txt` contains nothing at all — no `Finished` lines from the five builds that were
supposed to run in between.

Both bookkeeping lines are now brace groups joined with `&&`, so the chain is one chain
(`{ gates=…; jobs=…; export …; } && \`). Verified both ways on the same tree, with one
formatting violation present: before, `CI-RESULT: ALL GATES PASSED` after a full 4581-test run;
after, `CI-RESULT: FAILED` in seconds, at the `fmt` phase. And the control — a clean tree still
reaches `ALL GATES PASSED`.

**The old workaround was also incomplete, which is how this was found.** `grep -c "^error"` was
the documented compensating check, and a `rustfmt` diff does not print `error` — it prints
`Diff in <path>`. A formatting violation therefore passed the local gate under BOTH documented
checks and was caught by GitHub's `Format` job instead. Reading a verdict through a grep only
covers the failure spellings you thought of; the fix is that the verdict means something again.


---

This is the gate-level twin of GUARDS.md § How a guard reads green: a channel that reports
success while measuring nothing. It cost nothing on the day it was noticed because the two lints
were cosmetic — but nothing about the mechanism was limited to cosmetic lints, and for as long as
it stood, a green `make ci` was evidence about the test suite and about nothing else.


---

## The gate builds the test binaries TWICE — and the other structural minutes (@PLN159, 2026-09-08)

Measured on the PR run `34055973906` (2026-09-06) and a local `make ci` of 2026-09-07, and
recorded in full in [plans/159-gate-efficiency.md](plans/159-gate-efficiency.md).  The suite's
isolated cost is seconds — `loft_suite` 5.8 s, `native_scripts` 6.1 s warm, one corpus program
through `rustc` 0.20 s — and the gate's minutes were structural:

| where | what | cost |
|---|---|---|
| every CI shard, every local gate | `[profile.test] debug = 1` differed from dev's default 2, so `cargo build --all-targets` and nextest built all 267 test binaries TWICE (cargo shares artifacts between profiles with equal VALUES, never by name) | 4m05s per shard, 1m13s locally |
| the local loop | `find_problems.sh` ran `--release` while `make ci` and CI run the dev/test profile — a third build of the same binaries per source edit | one release test build per edit |
| the heavy shard | `native_scripts` (219.6 s) in a single-slot group, plus the 3.7-min cache save, on the critical path | 29.3 min shard |
| every commit | `native_artifact_cache_key` folded the git HEAD, so every commit rebuilt every hand-written package cdylib (`cdylibstale`) although none depends on the loft crate | one cold `cache warm` per commit |
| two gates on one box | each at half the threads, both ~2×, plus the OOM and load-flake reruns | 10 → 19 min |
| the iteration loop, every edit | `find_problems.sh` rebuilt the release rlib and both wasm rlibs — three NON-incremental whole-crate builds — before a `--subject parser` run that links none of them; the dev profile is incremental (a comment edit 3 s, a one-function edit 22 s) and the release profile is not (42–55 s, every edit) | 158 s ahead of 13 s of tests |

One of these was a correctness hole, not only a cost: the loop's rebuild step refreshed the
release LIB and never the release BINARY, so the 29 test binaries that spawn
`target/release/loft` measured whatever `make ci` last built — a binary from an unrelated
earlier run, or from a tree temporarily reverted for a falsification (loft-c1 hit the
consumer-side twin the same morning: a 25-minute valgrind sweep whose binary a rebuild
replaced under it).  Nothing tied the artefact to the source that was supposed to have
produced it.

What changed: `[profile.dev] debug = 1` (one build); `find_problems.sh` without `--release` (one
profile); a `corpus` PR shard and per-shard cache saves; the package-cdylib key without the git
HEAD; `make ci` takes `/tmp/loft-gate.lock` (one gate at a time, `LOFT_GATE_PARALLEL=1` opts out)
and runs the diff's subjects first (`scripts/nextest_priority.sh`, an order, never a selection);
`find_problems.sh --changed`; `find_problems.sh` rebuilding only what the selection links
(the dev rlib always, the release binary and the wasm rlibs only for the binaries that use
them); and `doc_hygiene::every_test_binary_matches_a_subject`.  Each
phase's proof — and the numbers the next PR run must confirm — are in the plan file.


---

**Fixed (loft#1504) by closing the descriptor in the CHILDREN, not by releasing the lock.** The
whole chain after the lock now runs in a subshell ending `) 9>&-`: the parent shell keeps fd 9,
so mutual exclusion is unchanged, and every process the gate spawns gets its copy closed.
Measured four ways before it was applied — a child without the redirection holds the lock after
its parent exits (the bug, reproduced standalone); with it the lock is free; the parent still
holds it while the run continues (so the gate still serialises); and closing an unopened fd 9 is
harmless, which is the `LOFT_GATE_PARALLEL` path. A SUBSHELL rather than a `9>&-` per command
because the per-command form is an allow-list, and a step added later would silently drop out of
it. Bounding the spawned children's own lifetime (a `LOFT_TIMEOUT` on each) is a separate
hardening and deliberately NOT done as an allow-list: there are **37 spawn sites of the loft
binary across 11 test files** (`engine_host_kernel.rs` alone has 14), inside **59 `.spawn()` sites
across 30 files** overall — so it wants a shared spawn helper, not dozens of edits that drift
apart. Both counts are worth carrying: the smaller one is what a `LOFT_TIMEOUT` could bound, and
the larger one is what can inherit a descriptor, since inheritance does not care which binary the
child is.


---

⚠⚠ **RE-MEASURED 2026-08-28 — the table below is STALE by three orders of magnitude, and it
sent one round of work down the wrong path.** The pair now runs in **0.139 s together**:

```
PASS [0.112s] stdlib_whole_data_round_trip      ← was 136.4 s
PASS [0.128s] stdlib_load_compares_equal_to_fresh
Summary [0.139s] 2 tests run
```

They are not self-skipping — `--no-capture` shows the real work
(*"710 definitions, 1053573 bytes of JSON, 710 names re-resolved"*). Something between
2026-08-21 and now made them ~1000× faster and nobody re-measured the number this document
recommends acting on. Acting on it: `make ci` was changed to exclude the pair locally, then
**reverted the same hour** once measured — it removed coverage to save 0.14 s.


---

**Where the time actually is.** `ir_schema_roundtrip`, in the general pool:

| test | alone, 24 cores free |
|---|---|
| `stdlib_whole_data_round_trip` | **136.4 s** |
| `stdlib_load_compares_equal_to_fresh` | **135.6 s** |
| `tests_scripts_round_trip` | 67.3 s |
| the other five | < 19 s combined |

That is real work, and two tests at ~136 s running concurrently are the **wall-clock floor of
any schedule** — no amount of parallelism goes below it. The cost is the whole-stdlib JSON
round-trip, not the parse (`cached_default()` is a `OnceLock`, but nextest gives every test
its own process so it never carries across; the parse is under a second —
`stdlib_definitions_round_trip` is 0.7 s). `stdlib_whole_data_round_trip` serialises the
entire stdlib **three times**: `json`, `data_to_json(&back)`, and the comparison.

**The serial groups are not the answer, on this box either.** The `[test-groups]` note in
`.config/nextest.toml` says widening buys ≈0 on `macos-latest`; the obvious objection is that
a 24-core box is not a 3-core runner, so it was re-measured. Same eight binaries, serial vs
widened: **122.5 s → 103.5 s**, 19 s, on a leg that is not the critical path and in isolation.
Against 572 s that is noise, and it re-opens the starvation flake the group exists to close.

**So the options are, in order:**

1. **Do not run `make ci` to iterate** — it is a pre-push gate, not an inner loop. Use
   `scripts/find_problems.sh --bg` (designed for exactly this: start it before editing) or a
   targeted selection. Costs seconds, not minutes.
2. **Shrink the round-trip fixture** — round-trip a representative slice every run and the
   whole stdlib on a schedule. Cuts the floor directly.
3. **Make `data_to_json` faster.** It is product code, so the win is not only in CI, and
   serialising the same `Data` three times in one test is a test-side saving available for
   free.

⚠ 2 and 3 are not equivalent: 2 reduces what is checked, 3 does not. Prefer 3 if the
serialiser turns out to be the cost, and measure it before choosing.


---

**Why it had to be written down.** Every instruction on this box tied the gate to an EVENT —
"before committing", "before every commit", "the pre-commit check" — and none said what to do
after it went red, so each fixup commit read as a new tree owing a new gate.  Measured
2026-09-28: four consecutive gates on one change, ended by rustfmt (after 279 s, because the
gate rebuilt the native fixtures and wasm rlibs before running fmt — fmt now runs first), by
clippy's `too_many_lines`, then by a derived audit row and a fixture test.  The pre-flight
catches the first and the third before a gate is queued; `recheck` covers the fourth in the
time of its two tests.


---

## Where the 31 minutes actually are (2026-08-10) — measured, and one axis untried

Per-job wall-clock on the last green PR run (`31359676983`). **One job is the whole
critical path**; everything else finishes inside it:

| job | wall |
|---|---|
| **Test (ubuntu-latest)** | **31.6 m** |
| ASan UAF/OOB gate (ubuntu) | 14.2 m |
| markdown viewer smoke | 6.8 m |
| stack_align_guard sweep | 6.7 m |
| Browser build + probe | 6.5 m |
| doc index hygiene | 6.3 m |
| everything else | ≤ 2 m |

Inside that job: **Test 22 m**, Build 4.3 m + 1.7 m + 0.7 m, cache save 2.3 m. So the
test step alone exceeds the 20-minute rule, and the macOS leg this document analyses is
already gone from the PR matrix.

### The measurement that matters is COLD, and that is what the earlier attempts missed

Locally, in the SAME warm tree, back to back:

| selection | wall |
|---|---|
| the `heavy-serial` group only (78 tests, 7 binaries) | 68 s |
| everything else (3837 tests) | 102 s |
| both together | 113 s |

Warm, the two overlap: adding the serial group to the rest costs ~11 s. Cold — no
`native-auto/` artifacts, no `.loft` caches, which is **every CI run** — the same three
selections measured 226 s, 174 s and 398 s. **Cold they are ADDITIVE**: the group's
members each spawn a storm of concurrent `rustc`, so while one runs it owns the machine
and nothing overlaps with it, however parallel the rest is.

That single fact explains both recorded sharding failures above, and it is why the axis
they split on was wrong. Splitting by test COUNT and splitting by DURATION both
scattered `heavy-serial` across shards, so both shards contained a machine-owning
member and neither could overlap anything. **The split that has not been tried is the
one along the group boundary itself** — every `heavy-serial` binary in one job, the
other 3837 tests in the other. Cold, that is `max(226, 174)` instead of `226 + 174`:
roughly 43% off the test step, ~9 minutes off the critical path.

### The second lever that ISN'T: `cargo nextest archive` was evaluated and rejected

The obvious companion — build once into an archive, have both shards run
`--archive-file` against it — looks like it removes the duplicated build. It does not
help the number this document is about, and it was dropped before implementation.

**It trades wall-clock for runner-minutes, and wall-clock is the constraint.** An
archive job must FINISH before either shard starts, so the build stops being concurrent
with anything:

| scheme | critical path |
|---|---|
| today, one job | 9 m overhead + 12.5 m + 9.6 m = **31.1 m** |
| two shards, each builds (concurrently) | 9 m + 12.5 m = **21.5 m** |
| build-once archive, then two shards | 10 m + 13.5 m = **23.5 m** |

Two shards each building are two builds on two runners at the SAME time, so the second
build is free in wall-clock and costs only minutes. The archive converts that free
parallelism into a serial prefix. It is the right tool when many cheap shards amortise
one build; with two shards, one of which is a 12.5-minute serial floor, it is a loss.

**And it cannot carry this suite anyway.** Measured: the archive builds in 57 s and is
266 MB, and `CARGO_BIN_EXE_loft` does survive it (`exit_codes` ran 26/26 from a fresh
`--extract-to` with `--workspace-remap`). But the whole `heavy-serial` group fails from
one:

```
build_shared_cdylib: cdylib compile failed ...
error[E0463]: can't find crate for `libloading` which `loft` depends on
```

The auto-native tests shell out to `rustc --extern loft`, which needs loft's dependency
rlibs from `target/release/deps` — 1412 rlibs, 1822 MB locally (9.0 GB for the whole
directory). An archive that carried them would be a multi-hundred-MB artifact uploaded
once and downloaded twice, spending the transfer time the scheme was meant to save.

### What was implemented (2026-08-10)

The split, without the archive. `changes` now emits `legs` instead of `os`, and on a
pull request that is two ubuntu entries carrying a `shard` key:

- `Test (ubuntu-latest) [heavy]` — the `heavy-serial` group, 77 tests
- `Test (ubuntu-latest) [rest]` — the other 3860

Verified to be an exact partition before pushing (`3937 = 77 + 3860` on the PR filter,
`3939 = 77 + 3862` on push) rather than assumed — nextest has no `test_group()`
filterset predicate as of 0.9.138, so a shard boundary must be spelled as `binary(...)`
terms, and `scripts/ci_test_filter.py` reads that membership out of
`.config/nextest.toml` so the two spellings cannot drift.

Push and nightly keep ONE job per OS. The split stops at the PR path because `needs`
cannot address individual matrix cells: with three OSes in the matrix, a single
aggregate result would red all three required contexts over one platform's failure.
On a PR the matrix is ubuntu-only, so the aggregate is exactly the two shards, and
`test-pr-gate` re-publishes the required `Test (ubuntu-latest)` context from it —
chosen over renaming the required contexts, which would be a repo-settings change that
blocks every open PR the moment it lands.

**This projection is ~21.5 m, which is close to the rule but not under it, and it is a
projection — it must be read off real runs before it is believed.** The floor is the
heavy shard: `max()` of the two sides cannot beat the serial group, so the next lever is
shrinking that group, not splitting further. One such cut already landed with this work
— `n3_use_native`'s two tests ran the same 12-context `rustc` loop twice and were folded
into one, 58.9 s → 33.1 s.

Two further levers are identified and deliberately NOT pulled yet, because the split has
to be measured on its own before another variable is added:

1. **Move the cache SAVE off the long pole.** The 2.3 m save now runs at the end of the
   `heavy` shard, i.e. on the critical path: `6.7 + 12.5 + 2.3 = 21.5`, while `rest`
   finishes at 16.3 m with ~3 m of slack. Saving from `rest` instead would put the pole
   at 19.2 m — under the rule. It is not done here because `rest` does not produce the
   native-compile artifacts that make the group's warmth, so it trades a measured
   overhead for an unmeasured regression in the very step being optimised. Decide it
   with the run-over-run trend, not with this arithmetic.
2. **Shrink `heavy-serial` itself.** 77 tests, one slot. Any member that does not
   actually need the whole machine is pure floor.

### What has already been taken out

`n3_use_native` was the largest single member at **58.9 s**, because two of its tests
ran the SAME twelve-context `rustc` loop — the pre-existing artifact-bound test and the
loft#831 shim-survival guard added beside it. Folded into one test asserting both
invariants over one loop: **58.9 s → 33.1 s**. In a group whose members cannot overlap
with anything, that 26 s is 26 s off the critical path rather than 26 s of slack.

### Do not re-derive this from a warm run

The warm and cold figures differ by 3.5x on the same selection, in the same tree, with
no change but the state of the caches. A local run that starts warm says the serial
group is nearly free; CI is never warm. Wipe `**/native-auto/` and the loft caches
before measuring anything intended to describe CI, and say which state a number came
from — the same discipline `scripts/test_speed.py` enforces for the test-speed report,
and for the same reason.


---

Two measurements, both from `gh run list`, decided this:

- **A scheduled run starts when GitHub gets to it.** The `ci.yml` daily is `cron: 0 3`
  and its last twenty runs STARTED between 03:34 and 14:45 UTC — 07:34, 08:45, 09:36,
  13:24, 14:45 on consecutive days. It also tests whatever `main` is at that moment.
  So "wait for tonight's nightly" is a wait of unknown length for an answer about an
  unknown commit.
- **The nightly-only legs are where a merge is found red.** The required checks on
  `main` are `Test (ubuntu/macos/windows)`, `Clippy` and `Format`, and on a PR the
  macOS and Windows legs are placeholders; the round-trip pair, the oracle and the
  browser asyncify/render tests are off the PR path by design (§ The rule that decides
  placement). Push-to-main's full matrix was red on each of the last eight merges, the
  daily on twenty of twenty, and the reds were macOS/Windows-only tests plus a codegen
  invariant in the debug-assertions gate — deep-internals changes that pass the ubuntu
  PR gate and fail on the legs that only run after the merge.

---

## Where the time goes — measured

### The PR critical path is the test suite, not the build

| leg | total | build steps | **test step** | nextest wall | tests |
|---|---|---|---|---|---|
| `Test (macos-latest)` | **31m40s** | 8m20s | **22m18s** | 1152s | 3481 |
| `Test (ubuntu-latest)` | **21m46s** | 5m05s | **14m30s** | 734s | 3481 (1 flaky) |
| `ASan UAF/OOB (ubuntu)` | 11m39s | — | — | — | — |

Both legs exceed the 20-minute rule; macOS by 11m40s. Build is **not** the
problem — it is a ~8m (macOS) / ~5m (ubuntu) floor. Test execution is 70 % of the
macOS leg.

### Two tests dominate everything

Time by test binary (macOS / ubuntu):

| binary | macOS | ubuntu | tests |
|---|---|---|---|
| `ir_schema_roundtrip` | **866s** | **744s** | 8 |
| `codegen_emitter` | 268s | 215s | 21 |
| `exit_codes` | 251s | 207s | 24 |
| `native` | 240s | 134s | 10 |
| `deliver_wasm` | 216s | 235s | 17 |
| `html_gl_imports` | 107s | 113s | 1 |

And inside `ir_schema_roundtrip`, **two tests are the whole story**:

| test | macOS | ubuntu |
|---|---|---|
| `stdlib_load_compares_equal_to_fresh` | **379s** | **308s** |
| `stdlib_whole_data_round_trip` | **375s** | **308s** |
| `tests_scripts_round_trip` | 83s | 99s |

Those two are 754s / 616s — about 65 % of the largest binary, and **~11 % of all
per-test time on their own**.

### MEASURED AFTER PHASE 1 — the model below was wrong, keep reading

Phase 1 landed (#646) and the result contradicted the prediction:

| leg | before | after | predicted |
|---|---|---|---|
| macOS | 31m40s | **25m35s** | ~20m ❌ |
| ubuntu | 21m46s | **24m26s** | ~15m ❌ |

The macOS `Test` step did fall 22m18s → 16m12s — exactly the pair being removed —
but the leg is still 25m35s, and ubuntu did not improve at all.

**Why the "floor" model misled.** `build + slowest single test` is a true LOWER
BOUND but not the predictor. The leg is governed by **total work ÷ effective
parallelism**: ~1950s of remaining test work at ~2 effective threads ≈ 16m15s,
matching the observed 16m12s almost exactly. Removing the two slow tests lowered a
floor the leg was never resting on. Use the work÷parallelism model below; the
slowest-test figure only tells you when sharding is pointless.

**And the serial group is a red herring.** `heavy-serial` (`max-threads = 1`) looks
like the obvious target — widening it locally takes the `native` binary 32.8s → 9.6s.
It is not the critical path:

```
serial chain, all 7 members:  288s
general pool:                ~1662s at ~2 threads ≈ 831s     ← the binding chain
```

Widening it buys ≈0 wall-clock on CI and re-opens the builder-vs-victim starvation
flake it exists to close (a rustc storm starving a websocket test into a timeout).
The stale claim that `multiplayer_v2` needs a fixed port 7878 is corrected in
`.config/nextest.toml`: P231 moved v2/v3 to `pick_free_port()`. They still cannot
leave the group, but for the *starvation* reason — and they would first need v5's
`JOIN_CAP` bounded drain, which they do not yet have.

### The floor nobody can shard away

A parallel suite cannot finish faster than its **slowest single test**. Today
that is **379s (6m19s)** on macOS. So:

```
minimum PR leg  =  build floor  +  slowest single test
macOS           =  8m20s        +  6m19s   =  14m39s   (before any other test runs)
ubuntu          =  5m05s        +  5m08s   =  10m13s
```

Sharding buys nothing below that line. **Any plan that leaves those two tests on
the PR gate is capped at ~15 minutes on macOS**, which leaves almost no room for
the gates we want to adopt. They are the first thing to move.


---


### A. Move the two stdlib round-trip tests to nightly — biggest single win

`stdlib_load_compares_equal_to_fresh` and `stdlib_whole_data_round_trip` verify
that the whole parsed stdlib survives a serialise/deserialise cycle byte-identical.
That is a **format-stability** property: it breaks when the IR schema or the
serialiser changes, which is rare and always deliberate.

- **PR keeps** `tests_scripts_round_trip` (83s/99s) — the cheap canary that still
  fails if round-tripping breaks at all.
- **Nightly gains** the two exhaustive ones.
- **Effect:** removes 754s/616s of work and drops the sharding floor from 379s to
  **194s** (macOS) / 139s (ubuntu).

Risk: an IR-schema diff that breaks only the exhaustive pair lands and is caught
that night rather than on the PR. Acceptable — it is precisely a "format did not
change by accident" check, and the canary still guards the common case.

**LANDED — but only half of it, for two months.** The exclusion went into
`ci_test_filter.py`'s `PR_ONLY` list, which is applied under
`if event == "pull_request"`. So the PR path got the win and **push-to-main and
nightly ran the pair TWICE**: once inside the contended suite, once again in the
dedicated `Stdlib round-trip` step that exists to run them "in parallel with
nothing else". Both this section and that step's own comment described the
intended single run; the filter never implemented it.

The contended copy is the expensive one, and on ubuntu it *was* the critical path:

| leg | contended (in the suite) | alone (dedicated step) |
|---|---|---|
| ubuntu | 493s + 484s, inside a 1306s suite | 232s + 233s |
| Windows | 501s + 500s | 269s + 269s |

It is also what turned the Windows leg red. Contended, the pair rides nextest's
600s `slow-timeout`: 545/533/571/589/501s over 08-01..08-09, then three
consecutive pushes pinned at the cap once the suite's total load grew 16 %
(1844s → 2141s) on a 1 % test-count rise. Measured **isolated**, the pair did not
get slower across those same commits (76.6s → 79.9s on one box), so the timeout
was a symptom of the double-run, not of a slow test — raising the cap would only
have hidden it.

Fixed by excluding the pair on *every* leg (a `DEDICATED_STEP` list applied
unconditionally). The step that now solely carries them also gained
`!cancelled()`: with no `if:`, it is skipped whenever an earlier step fails, so
across all three red pushes format stability was verified **nowhere** — the one
guarantee this section is about, silently absent exactly when the suite was
broken.

⚠ **A third copy exists — the local `make ci` — and it should STAY.** The exclusion lives in
`ci_test_filter.py`, which only the workflow invokes, so `make ci` still runs the pair. That was
briefly "fixed" on 2026-08-28 and reverted within the hour: re-measured, the pair costs **0.139 s**
(see § A LOCAL `make ci` above), so excluding it locally removed coverage for nothing. The CI-side
exclusion still earns its place — there the pair rode a 600 s slow-timeout when double-run — but
the local gate should keep running it until a fresh measurement says otherwise.

### B′. Duration-balanced sharding — TRIED, MEASURED, REVERTED

**Result: 24m11s versus 24m26s unsharded — fifteen seconds, for double the runner
minutes.** Reverted. The projection below said ~12.3 min; here is why it was wrong,
because the reason is more useful than the number.

| shard | tests | work | wall | parallelism |
|---|---|---|---|---|
| 1 — the 7 heavy binaries | 871 | 1778s | 526s | **3.38** |
| 2 — the other 151 | 2608 | 1261s | 733s | **1.72** |

Shard 2 had **less work and took longer**. Two mistakes compounded:

1. **The per-binary work estimates were taken from the pre-phase-1 run** and were
   ~50 % low: shard 1's real work is 1778s, not the 1183s the split was balanced
   on. The "perfectly balanced 1183/1083" partition was never balanced.
2. **Parallelism is a property of the TEST MIX, not the runner.** Splitting
   scattered the single-slot `heavy-serial` group (`n2_cdylib`, `n3_parity`,
   `n3_use_native`, `multiplayer_v2/v3`, `wasm_debug_relay`) across both shards
   while `native` — its biggest member — landed in the other one. The shard left
   holding the most serial constraints and the least work became the long pole.

The general lesson, now paid for twice: **a split cannot beat the serial structure
inside the set it is splitting.** Any future attempt has to move the serial group
as a unit, or fix why those binaries are serial at all — not partition around them.

And sharding multiplies the fixed cost it can never amortise: ~5m build + ~3.5m
cache restore/save = **~8.5 min per job before a test runs**, paid once per shard.

### What is left, in order of measured size

1. **~8.5 min fixed overhead per job.** `scripts/sccache_env.sh` exists and is
   unused in CI; the cache restore+save steps are ~3.5 min. This is now the biggest
   single item on the PR path and it is *not* a placement problem.
2. **The `heavy-serial` group itself** — not to widen it (measured, see
   `.config/nextest.toml`), but to reduce why its members must serialise.
3. Nothing else measured above a couple of minutes.

<details><summary>The original 2-way projection, kept for the record</summary>

The earlier failure (below) was **hash** partitioning, which balances by test
*count*. Two things changed since: phase 1 removed the two 308s tests, so no single
test is large against a shard (biggest ~139s vs ~1130s of budget), and this splits
on **measured duration per binary**.

| | work | projection |
|---|---|---|
| shard 1 — the 7 heaviest binaries | 1183s | ~6.1 min test + 6m10s build |
| shard 2 — the other 151 | 1083s | ~6.1 min test + 6m10s build |
| **wall** | | **~12.3 min**, from 24m26s |

Shard 2 is the *negation* of shard 1, so a new test binary runs there automatically
— it fails safe, and only the balance drifts. Exact-match `binary(=x)` keeps
`native` from swallowing `native_ext`/`native_loader`. Verified as an exact
partition: 871 + 2608 = 3479 tests, zero overlap.

3-way was computed (~10.3 min) and rejected: it saves 2 further minutes while the
6m10s build becomes 60 % of each shard.

</details>

Sharding renames the matrix jobs, so a `Test (ubuntu-latest)` **aggregator** carries
the branch-protection context and passes only when every shard passed — the
required check keeps its exact meaning with no settings change.

### B. Sharding by HASH — tried, measured not to work

**Do not partition a set that holds a single-slot test group.** Hash partitioning of the
WHOLE suite was implemented and reverted, and the reason is recorded in `ci.yml`'s matrix
comment.  The `rest` legs are the exception that proves it: every single-slot group lives
whole in `heavy`, so `rest` has none to scatter, and it is split `--partition slice:i/3` —
a ROUND-ROBIN deal of the list, because slow tests cluster (one binary's cases sit side by
side) and a hash can land a cluster on one leg.  `scripts/ci_timing.py` reports each leg's
balance on every PR.  The original finding:

> *Hash-partitioning was measured to NOT help: it balances by test COUNT not
> duration and can't split a single slow test, so the few slow integration tests
> piled into one shard (macOS shard1 6m vs shard2 20m) — the wall-clock stayed at
> the slow shard while runner-minutes doubled.*

This doc's first draft proposed 2–3 shards and projected ~14m/~11m. Those numbers
assumed **even** splitting; nextest partitions by count, so with a 379s test in the
set one shard simply inherits it and the wall-clock does not move. The measurement
above is the counter-example, and it predates this design.

What the prior art actually points at — *"the real long pole is those slow tests,
attack them directly"* — is section **A**, which is why A is phase 1 and this
section is a warning rather than a plan.

If duration-balanced splitting is wanted later, it has to be **by binary** (assign
`ir_schema_roundtrip`, `codegen_emitter`, `exit_codes`, `native`, `deliver_wasm`
to their own job), because that is balanced by hand against measured time. Note
each such job re-pays the build floor (~8m macOS), so two jobs cost ~16m of build
to save ~6m of test — worth it only after A, and only if A alone leaves us over.

### C′. macOS leaves the PR matrix — IMPLEMENTED

Measured asymmetry (running only the platform-sensitive families on macOS) does
**not** reach the rule: the platform-agnostic families total only ~412s of the
~1950s macOS test work, so trimming them leaves ~21m. The build floor alone is
8m26s — 44 % of a 20-minute budget — before one test runs.

So macOS moves to the footing **Windows has had since the per-PR Windows leg was
dropped**: full suite on push-to-main and the daily schedule, a non-blocking
placeholder on PRs. The trade is only honest because of what #646 added — the
macOS-specific risk is ARM-only *memory* corruption (@P383), and Miri-macOS
(1m29s) and the ASan-macOS UAF/OOB sweep (7m34s) now gate every PR. **A
pure-interpreter test cannot fail on macOS alone; a memory bug can, and those two
catch it — for 9 minutes instead of 25.**

Residual risk, stated plainly: a macOS-only *functional* break (not memory) now
surfaces on the merge commit rather than the PR. That is one merge of exposure,
never a release, and it is the same bet already taken for Windows.

**ubuntu then becomes the binding leg at 24m26s** — still over the rule, and its
own problem: build ~6m10s + test 15m35s, with effective parallelism ~1.9 despite 4
cores, because the subprocess-heavy tests (each spawning rustc) cannot overlap.
That is the next thing to attack, and it is a *work* problem, not a placement one.

### C. Asymmetric platform coverage (superseded by C′)

macOS is ~50 % slower than ubuntu for identical work and duplicates it exactly.
Options, in order of preference:

1. **Full suite on ubuntu per PR; macOS runs the platform-sensitive families**
   (`native`, `codegen_emitter`, `exit_codes`, `deliver_wasm`, `html_*`) — the
   ones where an ARM/Darwin difference can actually appear. Full macOS stays
   nightly. Saves ~10 minutes of the worst leg.
2. Keep both full but shard macOS harder (B).

Option 1 is the honest one: a pure-interpreter test cannot fail on macOS alone —
and when it did (@P383), it was a *memory* bug the ASan-macOS leg is there to catch.

### D. Adopt the cheap nightly gates onto the PR — free under the ceiling

Once A–C put the critical path near ~14 minutes, these all fit **in parallel**
without touching it, and each one can be broken by a diff:

| gate | cost | catches |
|---|---|---|
| Miri hard-UB ×2 | 1m15s / 1m07s | hard UB in the store/unsafe layer |
| ThreadSanitizer | 2m57s | `par` data races |
| POISON arena-UAF | 3m43s | store-internal use-after-free |
| Native-backend ASan | 3m57s | codegen-only memory bugs |
| Debug-assertions | 5m46s | violated internal invariants |
| ASan UAF/OOB **macOS** | 10m38s | the @P383 class (ARM-only) |

**Stays nightly:** ASan interpreter leak gate (23m29s / 25m28s — over the ceiling
by itself, and deliberately the slow single-threaded pass), toolchain matrix
(tests rustc drift, not the diff), `registry-validation`, `revalidate-libs`,
library health, stale-plan audit, `lib-branch-report`.

### E. Build floor (secondary)

macOS spends 8m20s building before a test runs. `scripts/sccache_env.sh` exists
and is unused in CI. Worth measuring after A–D; it is the next constraint once the
suite stops dominating, not before.


---

## Phasing

| phase | change | effect | status |
|---|---|---|---|
| **1** | move the two stdlib round-trip tests to nightly (A) | floor 379s → 194s, frees 754s/616s | **IMPLEMENTED** |
| **2** | ~~shard both legs~~ | — | **DROPPED** — measured not to work (B) |
| **3** | adopt the six cheap gates (D) | large coverage gain, no wall-clock change | **IMPLEMENTED** |
| **4** | daily digest + narrowed auto-issues | one place to look, no ticket noise | **IMPLEMENTED** |
| **5** | asymmetric macOS (C) / sccache (E) | the next lever, *if* phase 1 leaves us over | measure first |

Phase 1 is the one that moves the number; phase 3 is what the headroom buys.
Phase 5 is deliberately NOT pre-committed: the honest next step is to read the
macOS leg after phase 1 lands and see whether it is under 20 minutes. If it is,
nothing more is needed; if it is not, C (asymmetric macOS) is the lever with the
best ratio, because macOS duplicates ubuntu exactly and costs ~50 % more to do it.


---


- **A** — `ci.yml`'s `Test` step excludes the pair on `pull_request` only, and a new
  `Stdlib round-trip (nightly)` step runs exactly those two on push-to-main and the
  schedule, on every platform in the matrix. Verified with `nextest list`: the
  nightly expression selects exactly 2 tests, and the PR filter takes
  `ir_schema_roundtrip` from 8 tests to 6 with `tests_scripts_round_trip` retained.
- **D** — `miri.yml` gained a `pull_request` trigger rather than having its jobs
  copied into `ci.yml`; cadence is per-job via `if: github.event_name != …`. One
  definition, no drift. (Copying was the alternative and it is exactly the mistake
  the library-CI unification had just finished undoing.) On a PR the ASan job runs
  **macOS only** — `ci.yml` already gates every PR with an ubuntu ASan job.

---

## Two hours of waiting on one verified unit (2026-09-29)

The loop clause and `(R-ExitVector)` (fb214dd7) were verified in minutes — the matrix on
both backends, the pins, the sabotage, clippy — and then sat for two hours: three full
`make perf-check` runs of ~30 min each on the lima box (the first found a real crash in
hex_shape, loft#1757; the second failed on a stale bare rlib; the third had no baseline to
compare with), two clause-off A/B runs (the first measured only the drawing lane, because
with the switches off the census flags a different set), then the curated suite — each held
in the foreground with the box idle to keep the measurement quiet.  The A/B that answered
the question took fifteen minutes on seven flagged libraries: `check_request` +18 % and cbor
`decode` +17 % without the clauses, no hex_* routine moved 15 %.  The owner's verdict: too
much testing for one speed improvement.  CI_BUDGET.md § What a compiler change needs before
its commit is the rule that came out of it.

## Sizing the superinstruction checks

Measured 2026-09-29.

The interpreter's operand fusion (superinstructions) got its guard, two planted defects, a
1,876-script fused/unfused A/B, a round-trip check and a full suite.  Only the guard and one
plant were owed: a wrong operand position breaks nearly every loop, which the guard shows on
its first run.  The fast stack path the same day did reach every interpreted run and every
instrument's checked path, so its one full suite was owed.  CI_BUDGET.md § Sizing the checks
for a performance change is the rule that came out of it.
