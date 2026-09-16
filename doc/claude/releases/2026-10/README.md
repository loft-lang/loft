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

**Manual: 0 of 12 done.**  Eight are bound to the tag candidate (`M-valgrind`, `M-leaks`,
`M-ignores`, `M-wasm`, `M-docs-review`, `M-monthly-docs`, `M-monthly-bugs`, `M-libs`,
`M-close-plans`) and are measured when there is one.  Three are the mid-cycle work proper, and are
the first things this cycle owes:

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
  nightly gates); and the join of ../loft2's `D-heap-7` close, `D-tup-10` and three `D-perf-1`
  re-measurements with @PLN164's B2 units 2–3 and loft#1541's browser-kernel fix.
- **Open and labelled `fixed-pending-merge`**: #1541, whose fix rides the branch above.

## Decisions and questions for the owner

- **Nothing is ticked yet.**  A mid-cycle reading of a candidate-bound row is early warning, and
  recording it as evidence would be a tick against a tree that is not the candidate.
- **The version bump is not done here.**  `A-registry-prev` is green, so `2026.10.0` can be named in
  `Cargo.toml` whenever the owner wants the cycle's version to read untagged; until then the
  checklist is asked with `--version`.
- **Two red rows are a standing debt, not this cycle's work yet**: 3 reference chapters and all 11
  agent skills owe a read.  Both are continuous watermark passes, so they shrink by being done the
  week their source moves rather than on tag day.
