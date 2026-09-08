<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 159 — Gate efficiency: one build, one profile, diff-first, and a cdylib key that ignores the loft build

Tracker: [`@PLN159`](https://github.com/loft-lang/plans/issues/159) · `subject:loft` ·
value **Q** · **Effort: M · Design: ~** · opened 2026-09-08.

## Status

In progress (2026-09-08) — A, B, D, G and H are implemented and proven on this box; C is
implemented with its premise CORRECTED (below); E1/E2 are implemented with the partition
proven locally and the wall-clock still owed to a real PR run; F is open.  Every number
below was read off a real run between 2026-09-06 and 2026-09-08 and names its source; none
is a projection except where the word appears.  [CI_BUDGET.md](../CI_BUDGET.md) stays the reference doc for
where gate time goes: this plan's measurements move there when a phase closes, and its
phase 5 ("sccache / asymmetric macOS — measure first") is superseded by this plan.

## Goal

A fix is validated by ONE build of the test binaries, in ONE profile, with the diff's own
subjects first — and every gate keeps its full coverage: the `nextest list` before and after
each phase is identical.

## Composition matrix

Not applicable: no language surface changes.  Every phase's proof is a count or a
wall-clock read off the same run shape before and after.

## What was measured

**The suite is not slow; the gate is.**  Isolated costs, from the `@speed` annotations
(reference-machine seconds, serial) and a direct timing:

| item | isolated |
|---|---|
| `wrap::loft_suite` — all 1206 scripts, interpreter | 5.8 s |
| `native::native_scripts` — the same corpus, `--native`, warm cache | 6.1 s |
| `features::features_examples_interpret` — 125 examples | 3.5 s |
| one corpus program through `rustc` (20 KB generated, opt-level 0) | 0.20 s, of which the link 0.05 s; mold vs lld 0.02 s |

Against that, the two gates a fix waits on:

| gate | wall | source |
|---|---|---|
| local `make ci`, 12 of 24 threads, a second gate live | nextest 561.7 s + ~4 min of builds | `result.txt` 2026-09-07 |
| local `make ci`, idle box, `ci` profile | 153 s | `.config/nextest.toml`, 2026-08-21 |
| PR critical path (`Test [heavy]`) | 29.3 min | run 34055973906, 2026-09-06 |
| PR settle time, last week | 29–52 min; push-to-main 48–98 min | `gh run list` |

**Where the heavy shard's 29.3 min are** (job 101548309931): `Build` 5.0 (`cargo build
--all-targets`, dev) · release lib+bin 1.9 · wasm rlib 0.9 · `Test` 16.8 = **4m05s of
`Finished test profile`** before the first test + 764.8 s of nextest over 121 tests, of
which `native_scripts` alone is 219.6 s · `Save cargo cache` 3.7.  The `rest-b` shard shows
the same 4m04s test-profile build inside its `Test` step.

**The double build, verified locally.**  One `make ci` left two `wrap` binaries in
`target/debug/deps`: `wrap-0e2f…` at 197 MB (dev profile, 20:02) and `wrap-76c0…` at 93 MB
(test profile, 20:05) — 545 binaries, 35 GB, for 267 test crates.  Cause: `[profile.test]
debug = 1` while dev keeps the default 2; cargo hashes profile VALUES, not names, so two
profiles with equal settings share artifacts and these two do not.  Proof of the mechanism:
`CARGO_PROFILE_DEV_DEBUG=1 cargo build --test wrap` wrote `wrap-76c0…`, the test-profile
hash, rather than a third artifact.

**Contention, not test count.**  In the 2026-09-07 JUnit the 17 tests over 60 s sum to
1745 s of 4241 s total wall, yet `features_examples_interpret` (3.5 s alone) read 164 s and
CI_BUDGET.md records a 0.9 s test at 230 s.  The inflation is the compile-spawning tests —
`codegen_emitter` 389 s / 22 tests, `exit_codes` 344 s / 32, `html_embed` 284 s / 4 on
`rest-b` — each `rustc` or wasm build taking the whole box while 24 tests wait.  None of
their 49 spawn sites has a content cache; the corpus runner does (`native_cache_key`:
generated source ⊕ rlib bytes ⊕ package rlibs) and skips its compiles on a hit.

**What grows.**  Test binaries: 68 (2026-06) → 177 (2026-08) → 261 (2026-09); tests 3481
(07-26) → 4719 (09-07); the script corpus 239 → 613 → 1206.  Each new binary is a compile
plus link in four builds (dev, test, release for `find_problems`, clippy `--all-targets`);
each new compile-spawning test is 10–60 s of CPU that does not parallelise.  Each new
corpus file is ~5 ms.

**How often a re-gate has an unchanged runtime.**  Of the last 300 commits, 215 touch
`src/`, `default/`, `lib/` or the Cargo files; 85 do not.  Within a session CI_BUDGET.md's
measured afternoon had three of five gate cycles end on a docs-side item — the cycles a
content cache would make free.

**The fingerprint churn (corrected 2026-09-08 — filed as a hole, measured as waste).**
`native_artifact_cache_key` (`src/cache.rs`) folded the FFI ABI, RUSTFLAGS, `LOFT_VERSION`
and `BUILD_ID`, the git HEAD.  The first reading of this plan called that a dirty-tree hole
for the generated `loft_auto_*` cdylibs.  It is not: those are content-addressed by
`loft_build_fingerprint` (the rlib's bytes) in their file NAME (`cached_or_build_shared_cdylib`,
loft#715), so they move on every rebuild, committed or not.  The HEAD-keyed stamp governs only
a package's HAND-WRITTEN `native/` crate — and no such crate in the tree, the fixtures or the
registry depends on the `loft` crate (every `native/Cargo.toml` names loft-ffi, loft-ffi-macros,
loft-ffi-build only), so a codegen change cannot reach it.  What the fold bought was nothing;
what it cost was a rebuild of every package cdylib on every commit — each CI run's
`cdylibstale` events, a cold `cache warm` after every local commit, and a stamp ping-pong
between the three checkouts sharing `~/.loft/build-cache`.  The comment at `src/main.rs`
describing a content hash was wrong either way.  Phase C therefore drops `BUILD_ID` from the
key instead of adding a dirty hash to it; the value category falls from S to Q.

## Sub-arcs

`Verify` names the comparison that goes RED if the phase is done wrong.  Every phase also
carries the standing proof: `cargo nextest list -E "<the leg's filter>" | wc -l` is the
same number before and after.

| Item | Source | Verify | E | Status |
|---|---|---|---|---|
| **A** — one build: `[profile.dev] debug = 1` so dev and test share artifacts | `Cargo.toml` `[profile.test]` / `[profile.dev]` | after `cargo build --all-targets`, `cargo nextest run --no-run` prints no `Compiling`; one hash per test binary in `target/debug/deps`; the next PR run's `Finished test profile` line inside `Test` under 10 s (today 4m05s) | XS | Shipped 2026-09-08: `cargo test --no-run` after `cargo build --all-targets` compiled nothing (0.19 s), and the dev build reused the test-profile `wrap-76c0…` |
| **B** — one profile for the local loop: `find_problems.sh` drops `--release` | `scripts/find_problems.sh:177` | touch one `src/` file, run `--subject parser` then `make ci`: exactly one `Compiling loft` for the lib across both logs; the 60 s per-test budgets hold (they already hold in this profile on CI's 4 vCPUs) | XS | Shipped 2026-09-08: after `touch src/lexer.rs`, `--subject parser` built the lib once and both gate build steps compiled nothing |
| **C** — the package-cdylib key without the loft build: `native_artifact_cache_key` = FFI ABI ⊕ RUSTFLAGS ⊕ version; the four comments that described the old key corrected | `src/cache.rs`, `src/extensions.rs`, `src/main.rs`, `tests/features.rs`, `Makefile` | `cargo build` in `tests/lib/native_pkg/native` stays Fresh across the loft source edit that is this phase (0.17 s, no `Compiling`: cargo's own fingerprint says the crate does not depend on loft); `loft cache warm --from tests` rebuilds every package cdylib ONCE (new key) and reports them current on the next run and after the next commit; the unit test pins the key's inputs | S | Shipped 2026-09-08 — the after-commit `cache warm` read is owed |
| **D** — diff-first: `find_problems.sh --changed` from `SUBJECT_PATHS`; `make ci` passes a generated `--tool-config-file` with nextest `priority` overrides for the diff's binaries | `scripts/test_subjects.sh` (`SUBJECT_PATHS` is defined and read by nothing), `.config/nextest.toml` | a planted failure in a diff-touched binary is reported inside the first minute of nextest (today: whenever the pool reaches it, ≥ 5 min); `--changed` on a diff under `src/parser/` prints the `parser` filter | S | Implemented 2026-09-08; the planted-failure timing is owed |
| **E1** — the native corpus as its own shard: `binary(native) & test(native_scripts)`, heavy = serial groups minus it | `scripts/ci_test_filter.py`, `ci.yml` `changes` job | the four shards are an exact partition (counts sum to the unsharded count, zero overlap — the discipline the existing split records); the real PR run's heavy job under 20 min | S | Implemented 2026-09-08; partition proven (120 + 1 + 1457 + 3130 = 4708, zero duplicates); the wall clock is owed to the PR run |
| **E2** — the cache save off the long pole | `ci.yml` `Save cargo cache` (`shard == 'heavy'`) | measured, not assumed: one run saving from `rest-a` and the run after it — `Build` shows deps Fresh and `~/.loft/build-cache` warm; if heavy-only artifacts go cold, split the save (target from `rest-a`, build-cache from heavy) | XS | Implemented 2026-09-08 as the split: `target` from `rest-a`, the corpus binaries from `corpus`, the build cache from `heavy`, all three restored everywhere; the run-after read is owed |
| **I** — the iteration loop builds only what the selection needs (added 2026-09-08 when the owner ranked this box's agents above the PR clock).  Measured in B's proof: after a one-file edit, `--subject parser` spent 158 s in `rebuild_native_cdylibs` — the release rlib (158 s) and the two wasm rlibs (99 s, in parallel) — before 13 s of tests, and a parser test uses none of them: the loft the tests spawn is the test-profile binary (`CARGO_BIN_EXE_loft`, 152 binaries), which resolves its rlib beside itself; the release rlib serves only the 29 binaries that spawn `target/release/loft`, and the wasm rlibs only the html/wasm suites.  So: always `cargo build --lib` (dev, ~1 s, the uplift beside the debug loft); the release lib AND `--bin loft` only when a selected binary spawns the release loft (today's loop rebuilt the release lib but never the binary those 29 spawn — a stale-binary verdict in the loop); the wasm rlibs only when a selected binary is a wasm/html one; curated and full keep everything | `scripts/find_problems.sh` `rebuild_native_cdylibs`, `scripts/test_subjects.sh` | after `touch src/lexer.rs`: `--subject parser`'s timing summary shows no release or wasm row and its wall is the dev lib compile plus the tests; `--subject wasm` still rebuilds both wasm rlibs and advances `target/release/loft`'s mtime; the curated run is unchanged | S | Open — next |
| **F** — a content cache for the compile-spawning tests.  Surveyed 2026-09-08: the tests spawn the loft CLI (`--native` / `--html`), not `rustc` — `store_persist_loft` 44 sites, `native` 36, `exit_codes` 7, `html_wasm` 8, `html_embed` 4; direct `rustc` only in `n3_use_native` (11) and `native` (3).  So the lever is loft's own program cache, whose entry is keyed on the script's PATH plus the lib dirs (`program_cache_paths`, loft#930): every probe gets a fresh temp name, so every run is a miss and the cache only grows (TESTING.md § Scratch hygiene: 13 GB in one test's dir).  F1: key the bundle on the script's CONTENT hash + its directory + the lib dirs, so a byte-identical probe hits under any name while the manifest still catches sibling drift.  F2: the direct `rustc` sites through the corpus runner's `native_cache_key` | `src/cache.rs` (`program_cache_paths`), `tests/n3_use_native.rs`, `tests/native.rs` | F1: two back-to-back runs of `exit_codes` with no source change — the second run's `[loft-timing]` shows program-cache HITS and its JUnit wall drops to run cost; then a one-byte `src/` edit — every probe rebuilds (build signature moved); and the loft#930 cell stays red (two lib trees, one script, two bundles).  F2: spawn count = artifact count via `LOFT_TIMING`, then zero on the re-run | M | Open |
| **G** — gate queue across checkouts: `ci-run.sh start` takes a box-wide flock; `CI_LIVE_GATES` throttle stays as fallback | `scripts/ci-run.sh`, `Makefile:520` | two gates started 5 s apart on two checkouts: the second's `result.txt` header records the wait, the first runs at 24 threads, peak load ≤ a solo gate's | XS | Implemented 2026-09-08 as a queue by default (`/tmp/loft-gate.lock` in `make ci`), `LOFT_GATE_PARALLEL=1` for the old throttle — open question 1 stands for the owner |
| **H** — subject-coverage guard + growth rule | `scripts/test_subjects.sh` (`unmatched_binaries` is printed, never checked), `TESTING.md` | a `doc_hygiene` test that `unmatched_binaries` is empty goes red on a synthetic `tests/zz_probe.rs`; TESTING.md states the rule (guards to the corpus; new Rust tests join an existing binary; a compile-spawning test shares its binary's fixture) | S | Implemented 2026-09-08: 80 of 261 binaries matched no subject, the map now reaches all 261, the guard is strict |

## Phase ordering

**The owner's priority (2026-09-08): this box's agents, not the PR clock** — most of an agent's
time goes to waiting on validations.  So the local phases rank first: A, B, I, D, G, H; then
F; E only shortens the PR.

1. **A then B**, the same day: XS each, and the win every gate sees (about 4 min per CI
   shard on the critical path, about 1 min per local gate, half the disk under
   `target/debug`).  Cost of A: dev binaries carry line tables only; backtraces keep
   file:line and `profile.profiling` is untouched.
2. **C before F.**  F extends the fingerprint machinery; it must not inherit the hole.
3. **D**, then **E1/E2** — independent of each other; E waits for one real PR run per
   change so the arithmetic is confirmed, as CI_BUDGET.md insists.
4. **F** last of the code phases, one binary per step, each with its own comparison.
5. **G/H** whenever; G needs the owner's answer first.

Projection, to be replaced by the measurement: heavy shard 29.3 → ~18 min after A + E1 +
E2; a docs-only local re-gate from ~10 min to under 1 min after A + F.

## Open design questions

1. **G: queue or throttle as the default?**  Today two gates on one box each run at half
   the threads and both take ~2× (CI_BUDGET.md: 10 → 19 min).  A queue finishes the first at
   1× and the second at the same 2×, and removes the OOM and load-flake reruns — but an
   agent waiting on a lock is idle.  Owner's call; the plan implements whichever, with the
   other reachable by a flag.
2. **F: what is in the key for a wasm build?**  The corpus key is generated source ⊕ rlib
   bytes ⊕ package rlibs.  A `--html` build also depends on the wasm rlib and on
   `binaryen`'s `wasm-opt` version; the helper folds both, and the falsification cell
   (rebuild after a wasm-rlib change) proves it.
3. **E2: what does the heavy shard's cache actually warm?**  Decided by the run after the
   move, not by this table.

## Measure-first items (not phases)

- **Batching the corpus compiles.**  At 0.2 s per file the fixed per-invocation cost is
  most of it; 20 scripts per `rustc` would cut `native_scripts` several-fold on a 4-vCPU
  runner.  On 24 cores the corpus is already seconds, so this is CI-only, and E1 may make
  it unnecessary.
- **The flake-rerun multiplier.**  One branch on 09-01 needed 6 CI runs, the 09-02 join PR
  4.  Out of scope here (the flake list is its own work), but it multiplies every minute
  this plan saves.
- **`cargo check` for the `--no-default-features` compile gate** (27 s locally): the
  defect class it exists for is a resolution error `check` reports, but `check` misses link
  errors; not worth the risk for 27 s.

## What this plan does NOT do

- Remove a test from any leg.  Every phase's proof includes the identical `nextest list`.
- Touch the nightlies, the release gate, or the serial groups' starvation reasoning
  (`.config/nextest.toml`).
- Adopt sccache: evaluated and dropped, `target/` is restored per run already (`ci.yml`
  restore step); the 5-min `Build` is loft and its 267 dependents recompiling against a
  changed lib, which no cache avoids.
- Cache VERDICTS.  The tests whose run is long enough for that to matter are the
  non-hermetic ones (browser boots, server sessions, the shared `~/.loft` registry); their
  inputs are not in any computable key.  Artifacts are cached; the run is real.

## See also

- [CI_BUDGET.md](../CI_BUDGET.md) — where gate time goes; this plan's measurements land there.
- [TESTING.md](../TESTING.md) — the harness, the corpus runners, the binary cache (§ Binary cache).
- `.config/nextest.toml`, `scripts/ci_test_filter.py`, `scripts/test_subjects.sh`,
  `scripts/find_problems.sh`, `scripts/ci-run.sh`, `build.rs`, `src/cache.rs`.
- [`@PLN156`](https://github.com/loft-lang/plans/issues/156) — release evidence (the gates
  must prove they ran); this plan makes the same gates cheaper, never thinner.
