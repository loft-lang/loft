<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN45 — Doc-hygiene auto-fix

## Status

**FINISHED (closed 2026-09-25).**  What shipped:
- **Phase 0, `make plan-move FROM=… TO=…`** (`tools/indexer/rewrite_links.py`): it computes every
  rewrite from the rename, including incoming links, outgoing links at the new depth, and
  repo-rooted spellings in code, tests and scripts.  Checked on three moves in a scratch
  worktree: a plan into `finished/` (57 rewrites), a plan across trees (68, including the
  `tests/*.rs` fixture paths into it), and a single doc into a new subdirectory (100).  After
  each, the index validator and the drift checker read clean; a bare `git mv` as the control
  broke three links, and both reported them.
- **Phase 1, `make doc-fix`** (`tools/indexer/fix_broken_links.py`, rewritten): a link is
  repaired only when exactly one tracked path ends in every segment it names; anything else is
  flagged for a person.  Its first run found 37 broken links the drift checker never looked
  at (it checks plan links only) and the index's bucket had missed.  Six were repaired and 31
  fixed by hand: libraries that left the tree now point at their `loft-libs-*` repo, renumbered
  plans at their new numbers, and prose placeholders are now code spans.  One more (the
  `02_images.loft` link) had a wrong `../` depth, which the fixer then repaired itself.
- **Phase 5 changed form:** a gate replaces the opt-in auto-fix mode.
  `tests/index_hygiene.rs::every_markdown_link_resolves` fails on any repairable or flagged
  link and names `make doc-fix` as the cure; it was falsified with a planted gone target and a
  planted misplaced one.  A test that mutates the tree was never the right shape (ground rule 1).
- **Phase 6:** [DEVELOPMENT.md § Moving a doc, and repairing links](../../DEVELOPMENT.md#moving-a-doc-and-repairing-links).
- **Phases 2–4 retired.**  They were fixers for drift classes the checker reports clean.  Two
  of them (3, the ROADMAP fixer, and 4, the closure annotator) assumed that closing a plan
  moves its directory into `finished/`; a plan now closes as an issue and its directory stays
  where it is.  Phase 2's class is warn-only, and choosing an effort letter for "two weeks" is
  a judgement, not a rewrite.

The design below is the record.  Opened 2026-05-18.  Driver: PR-212 cycle.  Every move
of a doc / plan directory (e.g. `lib_plans/57-regex/` →
`lib_plans/57-regex/`) triggers a cascade of broken-link fixes
across the tree that take 3-5 grep + sed iterations to fully
chase down.  Each iteration costs a CI round trip
(~10 min) when caught remotely instead of locally.  Auto-fix
turns the chase into a single command.

Currently `tools/indexer/fix_broken_links.py` exists for the
phase-09 `broken_links` bucket from `scan.sh` and handles common
off-by-one `../` repairs.  This plan extends that auto-fix
discipline to the full `check_doc_drift.sh` surface AND wires it
into `tests/index_hygiene.rs::index_hygiene_clean` as an
opt-in repair mode.

## Goal

Single command `make plan-move FROM=<old> TO=<new>` performs a
directory move AND rewrites every affected link (incoming +
outgoing + relative-depth adjustments) in one atomic operation.
The drift checker stays clean after each move; no cascade of
fix-up commits.

Secondary: `make doc-fix` repairs any drift that crept in by
other means (manual link edits, plan promotions, etc.), printing
what it changed and exiting 0 if the post-fix state is clean.

## The dominant failure mode — directory moves

Empirical observation from PR-212 (the trigger for this plan):
**every drift cascade so far has been a directory move.**  Each
one followed the same pattern:

```
git mv doc/claude/lib_plans/57-regex/ doc/claude/lib_plans/57-regex/
git add -A && git commit
git push
# ... CI fails 10 min later, broken links surface ...
grep + sed for incoming links  (1 commit)
push, CI fails 10 min later
grep + sed for outgoing root-doc links  (2nd commit)
push, CI fails 10 min later
grep + sed for outgoing sibling-plan links + reverse refs  (3rd commit)
```

Three round-trips × 10 min = **30 min wasted per move** chasing
predictable fixes that the move itself fully determines.  Auto-
fix collapses this to one atomic command.

Other drift classes (stale `is current` claims, ROADMAP
mismatches, etc.) are minor compared to the move-driven cascade.
Phase 0 alone closes the dominant failure mode; phases 1-5 catch
the long tail.

## Effort + design

- **Effort:** M (cross-tier: shell + Rust + design)
- **Design:** ~ (partial — phase layout clear, fix-shape per
  drift kind needs per-row investigation)
- **Last touched:** 2026-05-18

## Why now

The PR-212 cycle is the trigger.  Sequence of broken-link
cascades from a single `lib_plans/57-regex/` →
`lib_plans/57-regex/` move:

1. Initial move + commit (`3b6aab5c`) — hit broken links from
   8 sites in 26-match-peg + 1 in finished/35-branch-review-viewer
   + 3 in phase-07 doc.  Fixed.
2. Run drift checker: clean.  Push.
3. CI fails — broken outgoing links in 01-regex/README.md that I
   missed because they reference root docs (`../../../LOFT.md`)
   not relative siblings.  Fixed in commit `c1212566`.
4. Run drift checker: clean.  Push.
5. CI passes ubuntu+macos … but a third miss surfaces (sibling
   `../03-lazy-stdlib/` and reverse refs from
   `future/03-lazy-stdlib/README.md`).  Found by the SAME drift
   checker that already said "clean" — because the relative path
   from `lib_plans/57-regex/` to `lib_plans/59-lazy-stdlib/`
   isn't a string the grep could pre-flag without resolving each
   candidate.

Every iteration was the same shape:

```
git mv plans/<from> plans/<to>
# (forgot to update incoming links from doc X)
# (forgot to update outgoing links to sibling Y)
git commit; git push
# CI fails 10 minutes later
# grep+sed
git commit; git push
# CI fails 10 minutes later
# repeat
```

An auto-fix that **computes the link-rewrites from the rename
itself** would have caught all five iterations in the first
commit.

## Categories of fixable drift

The drift checker reports six classes (`scripts/check_doc_drift.sh`
divides them by `=== <kind> ===` headers).  Per-class fix
strategy:

| Drift class | Auto-fixable? | Strategy |
|---|---|---|
| **Broken plan links** (paths) | **YES** | For each broken `[text](relative/path)`: try the canonical resolution heuristics from `tools/indexer/fix_broken_links.py` (off-by-one `../`, missing `../`, sibling-not-cousin), pick the unique candidate that exists.  Reject ambiguous matches (multiple candidates) for human triage.  Existing logic — extend it. |
| **Time projections** | **YES** | Replace calendar-time phrases (see the pattern list in `check_doc_drift.sh::check_time` — week-range / compound-week / month-window variants) with the effort-letter equivalent (`MH`, `H`, etc.) per a calibration table.  Single-word substitutions; ambiguous phrasings → flag for human. |
| **Stale 'is current' claims** | **NO** | Genuine semantic decisions — has the feature been retired or just renamed?  Human-only. |
| **ROADMAP plan-state cross-check** | **YES** | Two sub-cases: (a) plan moved to `finished/` but ROADMAP still has it → remove ROADMAP row; (b) ROADMAP cites a plan that no longer exists → flag for human (the row's whole purpose may have moved).  Plan dir on disk is the source of truth. |
| **Suspect refs from normal docs to finished/deferred plans** | **PARTIAL** | If the linking-doc's context contains a "closed by" / "shipped via" / "historical" phrase within the 3-line tolerance window → add a `(closed YYYY-MM-DD)` annotation that satisfies the existing tolerance regex.  Otherwise → flag for human (the reference content may belong somewhere else entirely). |
| **lib/ hygiene** (missing READMEs etc.) | **NO** | Architectural decision — does this library deserve a README?  Human-only. |

So 3 of 6 are fully auto-fixable, 1 is partial, 2 are human-only.
Conservatively the auto-fixer would close ~60-70% of drift items
without manual intervention; the rest get a clear "needs human"
report.

## Phases

| # | Phase | Effort | What ships |
|---|---|---|---|
| 0 | `00-move-rewriter.md` — Move-aware link rewriter | S | New `tools/indexer/rewrite_links.py` (or `.loft` if regex lib is up) that takes `<old-path> <new-path>` and rewrites every incoming `[text](<old-path>)` to `[text](<new-path>)` PLUS every outgoing link in the moved file to use the new relative depth.  Invoked from a `make plan-move OLD=… NEW=…` wrapper.  This is the single biggest PR-212-cycle saver — would've made the regex promotion a one-commit operation. |
| 1 | `01-doc-fix-paths.md` — `make doc-fix` broken-link auto-fix | S | Extend `tools/indexer/fix_broken_links.py`'s pattern matching to cover all six off-by-one shapes the drift checker flags.  Wire `make doc-fix` Makefile target to invoke it; report counts (`fixed: 12 / flagged: 3 / failed: 0`); exit 0 only when post-fix state is clean. |
| 2 | `02-time-projections.md` — Time-projection rewriter | XS | Add a `rewrite_time_projections.py` that maps the calendar-time phrases the drift checker flags (see `check_time` in `scripts/check_doc_drift.sh` for the regex set) to effort letters per the calibration table.  Reports each substitution.  Refuses ambiguous phrasings. |
| 3 | `03-roadmap-fixer.md` — ROADMAP cross-check fixer | S | For the two sub-cases above (finished plan still on ROADMAP, ROADMAP cites missing plan), apply the unambiguous side (remove ROADMAP row when plan in `finished/`).  Flag the other side. |
| 4 | `04-closure-annotator.md` — Closure-narrative annotator | S | For "Suspect refs from normal docs to finished plans": detect the linking-doc's context, insert a `(closed YYYY-MM-DD)` annotation derived from the plan's `git log -1 --format=%cs`.  Avoid touching rows that already have a tolerance phrase. |
| 5 | `05-test-runner.md` — Test-runner integration | S | Extend `tests/index_hygiene.rs::index_hygiene_clean` to invoke `make doc-fix` when `INDEX_HYGIENE_AUTOFIX=1` env var is set.  On success, re-run the assertion against the fixed state.  Opt-in (default off) so the test still fails loudly when run without the env var — the auto-fix is for `make doc-fix` workflow, not silent test-time mutation. |
| 6 | `06-closeout.md` — `make ci` integration + docs | XS | `make ci` doesn't run auto-fix (it's read-only).  `make doc-fix` runs the fixers + drift check.  Doc the workflow in `doc/claude/DEVELOPMENT.md`. |

Phase files (`00-*.md` through `06-*.md`) get created when each phase opens.  Inline-listed here rather than linked because the files don't exist yet (would otherwise show as broken `broken_links` entries in `index/tags.json`).

## Ground rules

1. **Auto-fix never silently mutates files outside an explicit
   `make doc-fix` invocation.**  Test runs report failures; only
   the dedicated fix command writes.
2. **Every fixer reports what it changed.**  Output: per-file diff
   summary + classify (`auto-fix safe`, `auto-fix ambiguous (skipped)`,
   `human-only`).
3. **Idempotent.**  Running `make doc-fix` twice produces the same
   tree as running it once.
4. **No new dependencies beyond what's already in tree.**  Python
   3 (existing — used by `fix_broken_links.py` and `migrate.py`);
   bash if needed (existing).  When `lib/regex/` ships per
   [`lib_plans/57-regex/`](../../lib_plans/57-regex), the
   fixers MAY be re-implemented in loft; not a prerequisite.
5. **Reuse drift-checker's classification.**  The auto-fix doesn't
   re-implement drift detection — it consumes
   `check_doc_drift.sh`'s output and applies fixes by class.
   Keeps the two tools coherent.

## Acceptance

Single test:

```bash
# Open with a clean tree, then run a plan-rename:
git mv doc/claude/lib_plans/57-regex doc/claude/lib_plans/01-regex
make doc-fix
make doc-check  # must report clean
make ci         # must pass
```

Should produce: a single commit's worth of changes (the
rewrites), zero broken links, zero stale references.

## Cross-references

- [`scripts/check_doc_drift.sh`](../../../../scripts/check_doc_drift.sh) — the detector this plan's auto-fixers consume
- [`tools/indexer/fix_broken_links.py`](../../../../tools/indexer/fix_broken_links.py) — existing partial auto-fix for the `broken_links` bucket; phase 1 extends it
- [`plans/42-tracker-index/07-loft-native-scanner.md`](../42-tracker-index/07-loft-native-scanner.md) — sibling effort (port bash scripts to loft); auto-fix would ideally live in loft once `lib/regex/` ships, but ships in Python initially to avoid blocking on that
- [`lib_plans/57-regex/`](../../lib_plans/57-regex) — once the regex MVP lands, the fixers can be re-implemented in loft (~50% size reduction expected)

## Value category

**Q — Internal quality.**  Pure dev-velocity multiplier; closes a
class of self-inflicted CI churn.  No user-facing surface.
