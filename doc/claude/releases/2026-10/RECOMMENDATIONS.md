<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `2026-10` — recommendations from the evaluation of 2026-10-01

> A read-only evaluation of `main`'s September (41 merged PRs, 543 issues filed) and of
> `laptop-superinstructions` up to `de33f7cf`, bundled for the agents finishing this cycle.
> Each item says what is wrong, how it was measured, the next step, and how to tell it is
> done.  The cycle's state lives in [README.md](README.md); this file is its worklist from
> outside.  Items are in pick-up order inside each section.

## Already handled — do not redo

- **The stub library pages.** A `gendoc` run on a box without the package cache turned 36
  `doc/lib-*-src.html` pages into "not on this build box" stubs.  Re-rendered in `d715c972`;
  `gendoc` refuses to replace a rendered page with a stub (`86ef3757`) and checks the cache
  first (`54ddd855`); CLAUDE.md names `make doc`.
- **dryopea red in `consumer-main-health`** (every test file "parse errors", four days).
  Known to the owner: it needs this release to be fixed easily.  `release.yml` runs the check at
  the tag without gating the draft, and `A-consumers` is the owner's call.

## Before the tag

### R1 — the switch-ab nightly is red on a guard that lacks its `@catches` header

- **What.** `tests/scripts/a-disturbed-keyed-view-keeps-the-value-at-its-bind.loft` guards
  the callee-disturb cure, but has no `// @catches: LOFT_NO_CALLEE_DISTURB` header, so
  `scripts/switch_ab.sh` reports its moving output as a DEFECT.
- **Evidence.** switch-ab on `main` was red on 09-30 and 10-01; on 10-01 the failing job is
  `ab (LOFT_NO_CALLEE_DISTURB)`: "1973 programs, 1 defect(s) … 6 caught".  The diff is the six
  `advice:` lines saying `ge` was copied out of `vs`, gone with the switch on.  The other
  programs that guard this cure carry the header (`tests/scripts/164-callee-disturb.loft:3`).
- **Next.** Run `scripts/switch_ab.sh LOFT_NO_CALLEE_DISTURB` and confirm that the moving output
  is the restored defect, not noise; then add the header in the form `164-callee-disturb.loft`
  uses.
- **Done when.** The same run lists the file as CAUGHT and ends "0 defect(s)".

### R2 — `#1841` is `sev:high` and open

- **What.** [loft-lang/loft#1841](https://github.com/loft-lang/loft/issues/1841): `--native`
  panics generating a generic that formats its type variable when another generic calls it at
  its own variable.  Filed 10-01; no fix on `main` or on this branch.
- **Next.** The bug-filing policy says an open issue is fixed before the release.  The issue
  has the repro; [loft-lang/loft#1840](https://github.com/loft-lang/loft/issues/1840)
  (`sev:low`, a lost-write lint miss) is the other open bug.
- **Done when.** Both carry `fixed-pending-merge`.

### R3 — a PR merged with a red check, and `main` went red behind it (owner note)

- **What.** #1838 was merged while its PR run (36859560546) had `Feature catalogue` red.  The
  push run on `main` (36863297716) then failed `Clippy`, `Feature catalogue` and `Test` on
  ubuntu, macOS and windows `[rest-c]`, until #1839 landed about an hour and a half later.
- **Next.** The owner's call: make `Feature catalogue` a required check, or read the head's
  check runs before merging.  Nothing for an agent to change.

## The evaluation method

The variety of methods is what finds the bugs: of the 543 issues filed from 09-01 to 10-01, 515 are
`hit-by:loft` and 242 are `silent-wrong`.  So the items below do not add process to any
method; they make the methods' results comparable and keep a red result from going unread.

### E1 — record which instrument found each bug

- **What.** `Found-via: #N` records lineage (which issue led to this one), not the method that
  found the bug.  So `make bug-review` can classify bugs by mechanism but cannot say which
  instruments still pay and which have dried up.
- **Evidence.** Issues filed per week: 189, 71, 53, 125, and 105 in the three days to 10-01.  Without the instrument, a new
  method's first-week spike reads the same as a regression an old method found.
- **Next.** One `found-by:<instrument>` label, set at filing time, with a fixed vocabulary in
  [.github/LABELS.md](../../../../.github/LABELS.md): the rule-led walk, a matrix probe, the
  reference review, the skills review, the ops census, the perf portal, the debug-assertions
  gate, switch A/B, the script twin, a consumer, the nightly.  `make bug-review` then reports
  yield per instrument per band.
- **Done when.** The next bug review prints a per-instrument column.

### E2 — every scheduled red result lands in the tracker

- **What.** `.github/workflows/advisory-failures.yml` files the `ci-advisory` issue for `CI`
  and `Nightly checks` only.  `switch-ab`, `consumer-main-health`, `lib-main-health`,
  `registry-validation` and `revalidate-libs` turn red with no issue.
- **Evidence.** R1 has been red for two nights with no issue.
- **Next.** Add those workflows to the advisory path, one issue per workflow, closed when
  that workflow's scheduled run turns green.
- **Done when.** A red switch-ab night produces an issue without anyone filing it.

### E3 — a check reports how much it skips

- **What.** A check that exempts files can pass on exactly the files it does not read.
  `tests/doc_hygiene.rs:3514` (`registry_derived`) exempts `doc/lib-*.html` from the
  generated-pages drift test, which is how the stub pages above passed CI.  `gendoc` now guards
  that case itself; the exemption is still invisible.
- **Next.** A check with an exemption prints the number of items it skipped (and lists them on
  request), so a growing exemption shows up as a number that moves.  Start with
  `doc_hygiene`'s.
- **Done when.** The drift test's output names how many pages it did not compare.

### E4 — read bug counts against a denominator

- **What.** A raw weekly count cannot separate "looked harder" from "the code got worse",
  and a percentage floor such as `make rule-coverage`'s can fall when the code grows even
  though no rule lost its guard.
- **Evidence.** Rust in `src/` grew from 307k to 426k lines in September (+39 %).  The best
  convergence signal already exists: the `contract:` labels on September's issues read 444
  settled to 73 strained (14 % strained).
- **Next.** With E1, report bugs per instrument per week; keep the strained share as the
  headline; read the rule-coverage floors with the rule count beside them.

### E5 — one evidence ladder for every kind of claim (lower priority)

- **What.** Rules, design decisions, features, doc claims and subjects each measure their
  evidence differently.  [SUBJECTS.md](../../SUBJECTS.md) already ranks evidence by strength;
  extended, one ladder could cover all five: a guard falsified on its defect, a both-backend
  guard with a hand-computed expectation, runs in CI, two-backend agreement only, a cited site
  only, `UNVERIFIED`.
- **Why lower.** It does not find bugs; it answers what nobody has checked yet.  The owner
  values the variety of methods more, so this waits until E1–E3 are in place.

## @PLN179 — scripts in loft

### P1 — the originals stay until the plan closes (owner ruling, 2026-10-01)

- **Ruling.** Every original Python/bash script stays, and its callers keep calling it, until
  the plan is finished; there is no rush to swap.  This also keeps every gate script judged by
  a program that is not the loft under test, which matters most for the gate scripts in tiers
  T3–T5.
- **State.** The tree already does this: `Makefile:687` runs `scripts/wasm_bundle_stamp.sh`,
  `tests/doc_hygiene.rs` regenerates the ignore baseline with `tests/dump_ignored_tests.py`, and
  `tests/engine_host_connector.rs` loads `scripts/wasm_bundle_stamp.sh`.
- **Next.** [The plan's README](../../plans/179-scripts-in-loft/README.md) § "A script's name"
  says "a port is a swap under the same name", which reads as if a caller moves when a port
  lands.  Add one sentence: callers move when the plan closes, not before.

### P2 — the findings register counts two closed findings as open

- **What.** Findings `003` (loft#1773, the interpreter segfault) and `012` (loft#1776, the stale
  rlib) still read `status: open` and `holds: unprobed`, although both issues are closed.
  Neither has a probe, so `scripts/script_recheck.sh` cannot move them.
- **Evidence.** [REASONS.md](../../plans/179-scripts-in-loft/REASONS.md) reads `OPEN: 13`;
  with these two closed it is 11.
- **Next.** Give each a probe from its issue's repro and run the recheck, or close both from
  their issues' state.
- **Done when.** `REASONS.md` reads `OPEN: 11` and both findings name the closing fix.

### P3 — choose the next ports by the surface they newly exercise

- **What.** The six ports so far are all text and file processing under the `report` contract.
  The plan has recorded 15 findings so far, and further text scanners would mostly repeat them.
- **Next.** Take the next ports from the tiers that exercise what no port has yet: T3 (git, gh,
  curl — 50 files), T4 (cargo — 27), T5 (runs a program of its own choosing — 58), per the
  generated [WORKLIST.md](../../plans/179-scripts-in-loft/WORKLIST.md).
