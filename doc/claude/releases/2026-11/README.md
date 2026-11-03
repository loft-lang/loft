<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `2026-11` — release state (opened mid-cycle 2026-10-16)

> The record of ONE release cycle — its blockers, the evidence each gate produced, and the
> decisions taken.  The process every cycle follows lives in
> [RELEASE.md](../../RELEASE.md); the index of cycles in [releases/README.md](../README.md).

Opened at the cycle's **halfway point**: `make release-checklist ARGS="--version 2026.11.0
--phase mid"` is the view, read on `tuxedo-release-2026-11-mid` (forked from the join tip
`1a4ff58d4`, which carries `origin/main` at `46ec215e0`).  `2026.10.0` shipped on
2026-10-01; this cycle ships at the start of November, gated on stability.

## The halfway view — what each `[mid]` row reads

| row | state | evidence |
|---|---|---|
| `A-audit` | ✅ | four rustls advisories (RUSTSEC-2026-0346..0349) against 0.23.45; `cargo update -p rustls` → 0.23.46 (`f14aef881`).  Warnings left: `memmap` 0.7 and `ttf-parser` unmaintained, `zip` 2.4.2 yanked (the whole 2.x line is; 9.x is the live major — an API move, not a bump) |
| `A-docs-lint` | ✅ | 111 over the baseline, all in the files the join split out of main.rs and friends (the count-keyed baseline cannot see a move).  The 51 stamps and the real narration rewritten; the narration pattern no longer fires on the word `removed` alone, which `@FR-H-Removed` makes domain vocabulary; baseline re-pinned at 4069 (`a9d4063a1`) |
| `A-ci` | ✅ | the runner's gate (`ci.yml`, ubuntu, every job) green on `fb74cee79` — run 38942581765; the commits after it change only the release record and the revalidate-libs workflow, re-gated on the final sha.  The gate on `6b80c93b1` (run 38927571189) had been red on five tests: the platform census twice (the sibling's new `scripts/after.py`), the two adopt mint counts (moved by the loft#2021 fix) and `doc/examples.js` behind the corrected chapters — the sibling's `ed31795cd` cures the first four, the branch is rebased onto its tip `b32951d9b`, the examples regenerated.  Local `make after-fix` under a sibling build read the drop gate and the socket tests red on contention alone (`REFUSED` with no reason, `r899a.bin` deleted under it) — the runner is the verdict |
| `A-reference-review` | ✅ | 11 chapters re-read against the interpreter (probes, not prose): 17-libraries carried the pre-#1855 model in five sentences, 25-generics declared `<`/`==` where the interface declares `compare`, 24-json promised one `#errors` entry per error — fixed in `bb4da4cbe`; 40/40 at their source |
| `A-skills-review` | ✅ | 12 skills read on the three axes; the untrue claims (engineering-rigor's rebase licence, loft-write's `fn Op…` and divide-by-zero Warn, loft-test's unsharded leg, loft-plan-workflow's closed canon) fixed in `bb4da4cbe`; 13/13 at their sources |
| `M-doc-validation-mid` | ✅ | part 1: 51 libraries moved since their watermark, every one with zero `pub fn` lines changed — dismissed as the method allows, watermarks bumped, the stale `audio_bus` row dropped; part 2: gendoc on the runner, `scripts/doc_review.py` 267/267 signed off; part 3: 20 `@F` entries re-read, 11 corrected on the tracker and the shadow regenerated |
| `M-monthly-bugs` | read | narrow-int/width is the one RISING class; `Parser::nstore_unwrap_report` reads NO EFFECT on null/sentinel (10.8 % → 18.6 %) — a premise to re-open |
| `M-liveness` | read | one stale rationale (re-justified); chronic reds named below |
| `M-ignores` | ✅ | every reason's job exists and the sweeps job was green on the 2026-10-16 nightly |
| `M-close-plans` | ✅ | plans#178 and #183 already closed |
| `M-file-sizes` | read | `src/parser/mod.rs` (27 205) and `src/parser/control.rs` (23 123) are the picks |
| `M-falsify-receipts` | read | 117 receipts the census cannot classify (108 `hand-measured`) |
| `M-doc-review` | ✅ | PLANNING.md under DOC_CONTRACT in the code-free loft#2029 (913 → 220 lines; the shipped entries verbatim in PLANNING-history.md; baseline re-pinned) |
| `M-ops-census` | read | 343 operators declared, 315 emitted by some program, 28 unexercised, 0 orphan |

## Red on main's tip when the pass opened, and what this branch does about each

- **The optional audit row** (all three CI legs): 276 in QUALITY.md, 275 measured — the join re-measured it.
- **Two Windows tests** (`ctor_literal`, `fresh_text_hold`): `--native-emit /dev/null` — `NULL_SINK`.
- **Nightly native ASan**: `tests/scripts/10-match.loft` absent at `46ec215e0` — the join's `1c7e3c440` put the five programs back.
- **Nightly doc index hygiene**: 28 decisions "unguarded" because the job never checked out the language suite — `checkout-suite` added.
- **Nightly LOFT_POISON**: `1994-…` w2 reads freed memory — filed as loft#2022, a duplicate of loft#2021: the sibling's `64068fc63` (a returned branch arm that borrows a lifted argument copies what it hands up) cures w2, w2g and w2h under poison; closed.
- **Nightly debug-assertions**: `issue_328` leaks a store — filed as loft#2023, a duplicate of loft#2017: the sibling's `cdc57da9a` (a nullable local rebound from its own field frees the store it built) is red on its parent and green on itself under the nightly's recipe; closed.
- **Dependency advisories**: rustls, cured.
- **The crate layers and the platform census after the join**: `literal_host`, `self_replace` and `zero_count` sat above their readers, `stable_hash`/`source_hash` were read from `cache` up into `compiled_stdlib`, and `--native-emit /dev/null` in eight tests emitted nothing on Windows — `loft::platform::null_sink()` here, the layer moves taken as the sibling resolved them.
- **The features-shadow derived check** read `git status` and failed on the bot's own regeneration — dropped from `scripts/derived_artefacts.json` with a note.
- **revalidate-libs** (chronic, 9 of 14): E0514 twice over — rlibs a toolchain bump left in the cached `deps/` (the toolchain is in the cache key now), and the library legs compiling with the runner image's rustc against the build job's rustup-stable rlib (the legs install the same stable now).  Run 38943370117: 45 of 45 legs green.  Its `unreleased` job stays red on two `loft-libs-core` branches, `174-f5-crypto-foreign` and `174-f6-cbor-spans` — @PLN174's finished library halves, 20 commits behind that main and conflicting on `base64_to_bytes`'s contract (main: an ordinary vector; the branch: a read-only foreign view) — the owner's pick, then a PR or a delete.
- **browser-threads** (chronic, 7 of 14): `par-scaling-bench.sh` 147 % < 150 % and `par-ui-responsive.sh` — timing bars on a shared runner.  The UI gate's bar now reads the part of each frame `par` decides, the serial compile-and-dispatch cost subtracted (loft#2125); the scaling bar is open.
- **consumer-main-health** (chronic): dryopea ×14, moros/crawler ×8 — the consumers' own agents, per their `LOFT_HANDOFF.md`.

## Found by the reviews and filed

loft#2024 `**` on integers answers a `single` (silent-wrong past 2^24) · loft#2025
`variant-field-unchecked` over-fires on a `name:` binding · loft#2026 `lost-write` misdiagnoses
a write through null · loft#2027 C80 claims a run-time Warn no site emits · loft#2028 the two
JSON parse forms disagree about `#errors`.

## What the cycle still owes before the tag

`M-file-split` (the two picks), the two @PLN174 library branches decided, the `[cand]` sweeps on the
candidate, and loft#2024..#2028 fixed.

## The pre-tag pass (opened 2026-11-02 on `tuxedo-release-2026-11`)

**The release waits for @PLN210** (owner, 2026-11-02): 2026.11.0 ships WITH the C performance
work that brings the test programs under 3x their Rust twins.  The release RULE is **no routine
above 5x** (owner, 2026-11-03); a routine over 3x is filed as a bug (a separate rule, and a release
may ship with open bugs), and 2x is the nice-to-have (the plan's stretch target: every library
bench routine under 2x on the C ship lane, gcc and clang).  That work is in development on
`laptop-c-perf`; the tag candidate is cut after it joins.  Until then this branch works the rows
that do not depend on the candidate: `[pre]` and `[mid+pre]` rows are finished here, the `[cand]`
rows (`M-valgrind`, `M-wasm`) and the release-window rows wait for the candidate.

Forked from `tuxedo-pr2366-next` @ `fa32f6013`: PR #2366 (the 2026-11-02 join and the open-issue
batch) plus the fixes for its red steps.

| row | state | evidence |
|---|---|---|
| `A-version` | waits | Cargo.toml says 2026.10.0; bumped at the candidate cut |
| `A-ci` | waits | the candidate's gate |
| `A-deviations` | open | D-clo-52 (loft#2284) and D-fact-1..3 (@PLN210) |
| `A-docs-lint` | open | 37 findings over the baseline |
| `A-reference-review` | open | 12 of 40 chapters owe a read |
| `A-skills-review` | open | one cited path no longer resolves |
| `A-pdf` / `A-pdf-version` | waits | rebuilt at the candidate |
| `M-doc-validation` | open | over what moved since the halfway pass |
| `M-windows-parity` | open | the report, read |

**Joined 2026-11-02 (2a4bd66e0, 3e4856ef4, d9cd80766 from loft2's 7e5b76b4a; was a nice-to-have):** the C backend's store upgrade (@PLN203 C3, the
`store_upgrade_owed_*` port), which `../loft2` is building.  Both halves are new this cycle — the C
backend and store upgrade each came after v2026.10.0 — so no release carries the gap and it is not
filed.  If the port is not in by the candidate, `store_load`'s doc says the upgrade is
interpreter-only so far, so 2026.11.0 does not promise it on `--native-c`.  When it joins it also lifts 54ce9e165's refusal (`loft_type_has_dbref` in `loft_image_write`):
a reference-holding type goes through the same converter.


## The candidate (2026-11-03)

**What ships is `main` @ `096397bba`** — PR #2374 squashed; its tree is the PR head `54576fabe`,
whose CI was green on every check (112), the informational nightly-health line aside.  The frozen
release gate ran on `692174cd7` (run 41600514860): green on every job but one Windows test whose
port pick collided with a sibling test's UDP port — fixed in `54576fabe`, which changes only that
test (`tests/c_emit.rs`).

**The post-freeze perf fixes stay out** (owner, 2026-11-03: "those routines are fixed today, but
I do not want to rerun every validation again because those will be broken on merging").  The
three routines over the 3x aim on the C ship lane — `wall_chain_walk` (hex_shape, clang 3.07x),
`parse` (arguments, clang 3.04x), `terrain_relief_pass` (hex_terrain, gcc 3.06x), loft#2379..#2381
— are under the 5x rule; their fixes reach `main` after the tag.

**The 5x rule holds** on the C ship lane (`bench/stats.py --lanes rust,cship`, every hash
agreeing): the 2026-11-02 full table of 126 routines, taken BEFORE the release's last two perf
batches (which only make routines faster), so an upper bound — gcc median 1.42x, worst 3.07x
(`terrain_relief_pass`); clang median 1.55x, worst 3.93x (zttext `materialise`); none above 5x.
Re-measured on the build with those batches, the worst is 3.08x (`wall_chain_walk`, clang).
Committed numbers: `doc/claude/plans/210-c-worst-to-2x/handoff-2026-11-03.md` § The numbers on
`laptop-c-perf`.  The rustc-native lane was not re-measured this cycle.

**D-fact-1..3 ship open** (owner, 2026-11-03: "ship with them, I am very much tasking the perf
agent right now to harmonize a lot of optimizations across the project, this will be part of
it").  `A-deviations` reads red on them by design: each is a fact computed apart from the one
routine `facts(kind, reach)` (`formal/facts.md`), so no program answers differently — a callee
fact with its own walk (D-fact-1), interval entry facts only in the rustc writer (D-fact-2, an
index check the C keeps), each twin kind keyed by hand (D-fact-3).  Their removal is loft#2351,
in the perf agent's harmonisation work after the tag.

**`A-ci` on the paperwork branch** (`4db9a938f`): the local `make ci` does not finish in its 20-minute
budget on this box under the sibling sessions' load (average 31-36; it reached test 5548 of 6540,
one test flaky, none failing), so the gate ran on a runner — `ci.yml -f os=ubuntu-latest`, run
41647930783: every job green but `Native static link`, whose runner was lost mid-step (no log).
That job's step, run here as CI runs it (`LOFT_NATIVE_CABI=0`, the same nextest filter): 111 / 111
passed, one test flaky on its first try.
