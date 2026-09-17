<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `2026-10` — release state (opened mid-cycle 2026-09-16)

> The record of ONE release cycle — its blockers, the evidence each gate produced, and the
> decisions taken.  The process every cycle follows lives in
> [RELEASE.md](../../RELEASE.md); the index of cycles in [releases/README.md](../README.md).

Opened at the cycle's **halfway point**, not at prep: `make release-checklist` marks the rows that
`[mid]` runs meaningfully at (overall stability), and reading them now is early warning — a row
bound to the tag candidate is re-measured on the candidate and only ticked there.  `2026.9.0`
shipped on 2026-09-05; this cycle ships at the start of October, gated on stability rather than on
a feature set.

## The mid-cycle audit — `--version 2026.10.0 --phase mid`, measured on `0b8bccef914b`

Cargo.toml still names `2026.9.0`, which is tagged, so the checklist is asked for the next version
explicitly.  The bump itself is a release-window step: it makes `A-registry-prev` live, and that row
is **already green** — `2026.9.0` reached the signed registry index, so the bump has nothing to wait
for.

**Automatic: 3 of 8 pass, 1 could not run, 4 failing.**

| row | state | what it says |
|---|---|---|
| `A-main` | ✅ | HEAD contains `origin/main` (`e5dcf2deb`) |
| `A-registry-prev` | ✅ | the previous release reached the signed registry index |
| `A-ignores` | ✅ | 34 `#[ignore]`s, each carrying a rationale |
| `A-clean` | ❌ | uncommitted changes — a tag must name a committed tree.  Mid-cycle this is the join in flight (the regenerated browser bundle and stamp, and the library pages `gendoc` rebuilds from local checkouts); it is a blocker only at the tag |
| `A-ci` | ❌ | the tree moved after the last green `make ci`; the join's own gate run is what clears it |
| `A-reference-review` | ❌ | 3 of 40 reference chapters owe a read (`make reference-review`) — the `A-pdf*` rows cannot see a chapter that is merely untrue |
| `A-skills-review` | ❌ | 11 of 11 agent skills owe a read (`make skills-review`) — a skill quoting last month's procedure steers every session that loads it |
| `A-validator-dryrun` | ❓ | never ran; UNKNOWN is not a pass |

**Manual: 0 of 12 done.**  Three of those twelve are bound to the tag candidate (`M-valgrind`,
`M-leaks`, `M-wasm`) and are measured when there is one — see *The cadence split* below, which
corrects this paragraph: it first listed nine docs as "eight bound to the candidate" and named
four rows the next section then treated as mid-cycle work.  The rest are the mid-cycle work
proper, and these three are the first things this cycle owes:

- **`M-perf-pass`** — `make speed`, and `python3 bench/compare.py` per library against
  `formal/performance.md`'s bar.  ../loft2 has just narrowed `D-perf-1` to the four bench rows still
  over it (picked into this cycle), so the pass starts from a known list rather than a survey.
- **`M-liveness`** — `make release-liveness`: suppressions justified by closed issues, gates that
  quietly stopped firing, checklist items never run in any recorded cycle.
- **`M-falsify-receipts`** — `make falsify-review`: which guards can still be re-validated, and how
  quickly.

## What the cycle carries so far

- `2026.9.0` shipped 2026-09-05 (the `2026-09` record).
- **Merged since** (PR #1542, squashed as `e5dcf2deb`): C124 — a `const` value reaches only a
  `const` parameter, decided by the signature — with loft#1540's view half and its
  function-reference half (`fn(const T)`); C121–C123; @PLN162's close; @PLN163's copy leases;
  @PLN164 through B1 and C4; @PLN165 phase 0; loft#1535.  Issues #1535–#1540 closed on that merge.
- **On `tuxedo-advisory-2026-09-16`, not yet merged**: the catalogue tag for `src/lease.rs`; a
  nullable iterator's `null` init writing its slot (the Debug-assertions and `LOFT_VERIFY_STACK`
  nightly gates); the join of ../loft2's `D-heap-7` close, `D-tup-10` and three `D-perf-1`
  re-measurements with @PLN164's B2 units 2–3 and loft#1541's browser-kernel fix; @PLN164 B2's
  placement candidate asking its return shape before the ownership question (which closed the
  Debug-assertions gate for a SECOND reason); the stdlib reference dropping the `both` receiver
  C123 retired; @PLN162 citing `@I78` by its real prefix; and SLOTS.md naming the allocator that
  runs.
- **Open and labelled `fixed-pending-merge`**: #1541, whose fix rides the branch above.
- **Filed this cycle**: loft#1544 — `./scripts/idx tag:@PLN<n>` answers an empty list at exit 0 for
  every plan, because the scanner indexes `@P` and `@PLAN` and has no `@PLN` arm.  `needs-design`:
  the two existing families validate locally, while a plan id is an issue number in another repo.

## The cadence split — `[mid]` now means COMPLETABLE (2026-09-16)

The halfway view reported **7/12 manual steps done** against a denominator no halfway run could
reach: five of the twelve could only be ticked on a tag candidate that does not exist yet.  A
tally that cannot be completed reads as permanent unfinished work, and a reader learns to skim
it — the same failure as a gate whose threshold has drifted from its subject.

`[mid]` now means *can be FINISHED here*, and a new `[cand]` marks what is worth RUNNING early
as warning but whose tick must name the tree that ships (`M-valgrind`, `M-leaks`, `M-wasm`).
`--phase mid` lists those apart and counts them nowhere.  `A-validator-dryrun` lost `mid` for the
same reason: rehearsing the registry splice needs an artefact that exists only at the tag, so at
halfway it can report nothing but exit 3, planting a permanent "never ran" in the view.  That
answers the open question below in the direction the phase's own definition requires.

The rows were classified on evidence, not on their titles: the seven a human actually finished
this morning are mid-completable by demonstration; `M-docs-review` is too (this record shows four
of its five steps done at halfway); and the three `[cand]` rows are the ones the script's own
comment already named, two of which say "on the TAG CANDIDATE" in their titles.  The mid view now
reads **7/10**, with `M-ignores` and `M-docs-review` as the two rows actually actionable today.

### New row — `M-file-sizes`, the file-shape census

Added at the owner's ask: *are our files too long for proper use, should they be split per
subject, and is the change history split from the documentation itself?*  `make file-sizes`
answers all three (`scripts/file-sizes.py` + the existing `scripts/doc_history_report.py`).

**Length is not the defect** — `DOC_QUALITY.md` rule 4 judges by content — so the report's verdict
is the SPLIT SIGNAL: the share of a file taken by its largest section.  A small share means
comparable subjects sharing a file (split it); a dominant one means a single subject that is
merely long (leave it).  Measured 2026-09-16:

- **Docs** — 1000 files, median 189 lines, 89 over the 700 bar, **36 reading as several
  subjects**: `PERFORMANCE.md` (32 sections, largest 17 %), `DESIGN_DECISIONS.md` (70, 6 %),
  `TESTING.md` (36, 20 %), `WASM.md` (22, 13 %), `PLANNING.md` (21, 22 %), `LOFT.md` (29, 16 %),
  `PACKAGES.md` (39, 22 %), `plans/157-native-4x-drawing/DESIGN.md` (56, 8 %).
- **Code** — 612 files, median 260, 96 over the 1200 bar, 45 as several subjects.  The giant
  parser and state files are **KEEP**: one `impl` block is 74–97 % of each, so they are one
  subject that is merely long.  The real candidate is `src/use_analysis.rs` (167 sections,
  largest 8 %); `tests/issues.rs` (1550, 1 %) is a test file, where that shape is the point.
- **History** — the split holds in general: 138 docs carry timeline lines, 5155 of 90 793 (6 %).
  The exception is the formal contract docs, where it concentrates — `formal/heap.md` is **71 %
  history with no `-history.md` companion**, the only high-share contract doc without one, and
  `binding.md` (52 %), `tuples.md` (59 %), `closures.md` (49 %) and `ownership.md` (35 %) all
  HAVE a companion and still carry that much.  **A companion existing is not evidence the history
  moved into it** — read the share, not the `yes`.

⚠ The first version of the report was wrong in a way worth recording, because it is the failure
this project keeps paying for: it divided the largest section by the WHOLE FILE while the sections
span only from the first anchor to EOF, so a doc with one late `##` scored a 2 % "largest section"
and read as several subjects.  It flagged `ownership-history.md` as *"1 sections, the largest only
2 %"* — a sentence that cannot be true.  A denominator that does not match its subject; 7 of 43
documentation recommendations were false before the guard (`len(secs) < 2` cannot be split) landed.

## Decisions and questions for the owner

- **Six manual steps are ticked, and each is deliberately NOT candidate-bound**: `M-liveness`,
  `M-falsify-receipts`, `M-perf-pass`, `M-close-plans`, `M-monthly-docs`, `M-monthly-bugs`.  A
  mid-cycle reading of a candidate-bound row is early warning, and recording one would be a tick
  against a tree that is not the candidate — so `M-valgrind`, `M-leaks`, `M-wasm` and `M-libs` stay
  open by design.  `M-libs` is the one of those that only wants a quiet box.
- **The version bump is not done here.**  `A-registry-prev` is green, so `2026.10.0` can be named in
  `Cargo.toml` whenever the owner wants the cycle's version to read untagged; until then the
  checklist is asked with `--version`.
- **`A-validator-dryrun` has never run in any recorded cycle.**  It reports structural gates passed
  at exit 3, which is not the full verdict, so the checklist reads UNKNOWN rather than green.
  Whether the registry-validator rehearsal belongs mid-cycle or on the candidate is the owner's call.
- **CLAUDE.md's rule count was stale; on 2026-09-17 the owner had it REMOVED rather than
  refreshed.**  It read *"179 of 257 rules have no code representation"*.  A standing-instruction
  number is the owner's to move, which is why this record left it — that reservation is now
  DISCHARGED, so this line is history rather than a live decision.  The first cure refreshed the
  figure; the owner's call was that refreshing it only restarts the clock, because *a number in a
  committed file is stale the moment it is committed*.  So the position figure is gone from every
  prose home that carried one — **five files, seven passages**: CLAUDE.md, `STABILITY_METHOD.md`
  (where every one of its five numbers had moved), `COMPATIBILITY.md` § the honest bound,
  `formal/README.md` three times over (the coverage line, the backlog sentence, and the
  prefix-pair paragraph, which contradicted ITSELF at 21 and 23 before going stale), and
  `formal/IMPLEMENTATIONS.md`'s census table, whose *"cited from code today"* row was wrong by
  more than two orders of magnitude while reading as current.  Each now links to
  **`make rule-coverage`** (`rule_tags.py coverage`, added the same day).  Dated RECORDS keep
  their figures on purpose — `BUG_REVIEW.md`'s per-cycle rows, `QUALITY.md`'s phase narratives,
  the plan files, IMPLEMENTATIONS' counting progression and the changelog are evidence of what
  was measured then, and refreshing them would falsify the record.  What
  IS written down is the contract-1 FLOOR, which does not rot — a MINIMUM for the freeze rather
  than a target to stop at, recorded in `COMPATIBILITY.md` § The road to contract 1 beside
  `silent-wrong` → 0, `contract:strained` → 0, "open deviations as few as they can be made" and a
  LIGHT detection pass over every rule (named there, and not yet built): **70 % of rules carrying a code annotation,
  40 % an active guard**, stated once in `rule_tags.py` and overridable by env.
- **`M-docs-review` is four-fifths done and deliberately untickled.**  RELEASE.md steps 1, 2, 3 and
  8 are complete: PROBLEMS.md is uniformly closed (its `@P1`–`@P6` rows are a 3-column legacy shape,
  not open entries), PLANNING.md carries no done-in-place items, 0 of 93 docs are orphaned, and the
  clippy census measured 51 dead suppressions, 7 partly dead, 212 live, with 96 of 261 item
  attributes unjustified.  Step 4 is scoped below but not done, and one whole step outstanding is
  not a residue.
- **Step 4's dominant target, scoped and deliberately NOT attempted here**: `formal/heap.md` is 1389
  timeline lines of 1955 (71 %), and is the only high-share contract doc with no `-history.md`
  companion — binding, ownership, tuples, closures, layout and collections all have one.  The move
  is surgical: the file carries 35 `@FR-` rule definitions, 11 `D-heap-*` deviation entries and a
  stated `OPEN: **3**`, and RELEASE.md § 5b's rule is MOVE, never copy — a register living in two
  files defines its entries twice and reddens `doc_hygiene::every_rule_citation_resolves`, which is
  green today (`rule_tags.py check` exit 0 over 351 rules and 1347 citation sites).
- **The two doc-review debts are cleared, not standing**: the reference review is **40/40** chapters
  and the skills review **11/11**, both at their current sources.  Each pass found real defects
  rather than paperwork — the stdlib reference still teaching the retired `both` receiver, and
  `loft-write` instructing agents to write it.
