<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Development workflow — the record

The dated incidents behind the rules in [DEVELOPMENT.md](DEVELOPMENT.md),
[JOINING.md](JOINING.md) and [REVALIDATE_LIBS.md](REVALIDATE_LIBS.md).  Those docs state each
rule as it holds now; this file keeps the measurements that justify them, oldest first.  A
record doc: its dates and hashes are the point.

## 2026-05-13 — the opcode bootstrap

Adding `OpIncRc` for @P259 hit the 255-op limit of the fixed-size `OPERATORS` array, and
`fill.rs` had to be patched by hand (array size, placeholder identifier, stub body) before
`regen_fill_rs` could run.  The array entry also had to sit at its parse-order position.
Two improvements were proposed:

- **A (built the same day):** `OPERATORS: &[fn(&mut State)]` is slice-typed and the
  parse-time op-code assert is gone, so regeneration grows the array and writes the new body
  in one pass.  This removed three steps of what had been a ten-step procedure.
  `n9_generated_fill_matches_src` and `fill_rs_up_to_date` still catch a stale `fill.rs`.
- **B (not built, S effort):** let a failing `fill_rs_up_to_date` regenerate and re-compare
  instead of printing the command, or turn `regen_fill_rs` into a `build.rs` step.  The
  test-side variant carries less risk; `build.rs` is cleaner but runs codegen at build time.

## 2026-06-24 — the diverged branch

A long-lived branch accumulated about 65 commits across five topics (@PLN87, sandbox,
formalization, a parser fix, a CI change) without rebasing while `main` advanced.  @PLN87
reached `main` as a squash while the branch kept its individual commits and built on them.
Different patch-ids on a diverged base meant git could not drop the duplicates; every parser
commit collided, and spinning off sub-branches multiplied the surface.  The only clean
escape was to cherry-pick the genuine delta onto a fresh `main`, which is the reconciliation a
regular rebase does a little at a time.  Source of JOINING.md's first rule and of
DEVELOPMENT.md § Stay close to `main`.

## 2026-08-19 — the owner directive on PR size, and the invisible library break

The owner: *"I will never PR one or a few issues, it takes a lot of time because we cannot
stack PR's on gh"*.  The same owner asked that a PR never be proposed or hinted at, because
that pressure is why PRs were held off.

The same day nine published libraries lost their entire public surface to one resolution
rule and every branch gate stayed green for a full day: `revalidate-libs` runs on
`pull_request` and on `push` to `main` only.  That produced the scratchpad-copy rule and later
`scripts/revalidate_libs_local.sh`.

## 2026-08-21 — rebasing onto a squash that carried 41 of our commits

A peer's PR squash-merged 41 of this branch's commits under new hashes.  Patch-id dedup
dropped the 32 that survived unchanged; the edited ones conflicted and were then applied ON
TOP of the version already in `main`, defining `WorkerState`, `WORKER_FATAL` and
`take_worker_fatal` twice (`E0428`, `E0119`).  Testing "already upstream" before resolving
took the survivors from 9 to 4.  `git rebase` reported success on the tree that did not
compile.

## 2026-09-02 — `--theirs` and a merged count

A join resolved `tests/docs/25-generics.loft` with `git checkout --theirs` and dropped 101
lines of the chapter for 32: the `<T, U>` restriction, the note that generic structs do not
exist, an empty-vector caveat and its assertion.  The branch log already carried the same
lesson from an earlier pick (*"Three chapters re-read after the release picks moved them,
and all three had lost something"*).  The same join found an audit row at
`678 | 324 | 5 | 349` on one side and `678 | 325 | 5 | 348` on the other; the merged tree was
neither.  QUALITY.md's audit row carried a false figure on eight consecutive joins before
derived rows were re-measured on every join.

## 2026-09-09 — a revert replayed onto another tree

Four commits of a source branch (a pick of this branch's own fix, two of the source's, and
the source's revert of its own mechanism) had an empty `git diff <commit-before> <revert>`.
Replayed here, the revert would have deleted this branch's fix for the same issue.

## 2026-09-10 — the cadence revision

The directive's cost model counted only the serialisation a PR causes, not the one that
withholding a PR causes.  One branch reached its PR carrying **9 joins**: the walker-audit rows
re-measured five times, `falsified_docs.baseline` regenerated, the browser bundle rebuilt, and
the siblings paying the same re-derivation in their own trees.  Two join-only defects existed
only on that union.  The same day three local `make ci` runs, started to hold a PR opening,
were killed by a sibling's `pkill -f "make ci"` (no path in the pattern), costing about
50 minutes with the PR still unopened.  Outcome: one or two stable PRs a day stays, opening is
minutes once asked, and a branch joined twice by a sibling is overdue (CLAUDE.md § Branch
policy, rule 3).

## 2026-09-16 — `git cherry` against a pre-squash ref

`git cherry HEAD <pre-squash-tip>` reported **57** `+` commits, every one a constituent of the
squash that carried them.  Read at face value, "57 commits missing" triggers a destructive
recovery.  The tree comparison (`git merge-tree --write-tree`) showed the branch was a no-op.

## The join at `b5c179e5e` — what `git show --cc` hides

Eleven conflicted files; `--cc` showed ten resolutions, five of them files `--cc` never
mentioned, and three choices were the stale side: `main` already had `i64::from(i32::MAX)` in
`IntegerSpec::i32` and the join put `as i64` back (red at clippy on the next gate);
`IntegerSpec::u32` was resolved to a third form that neither parent had; and
`index/target_surface.json` went back to listing `store_load_url` as unavailable in the
browser, undoing a re-derivation on the same branch.

## loft#1315 — one policy written twice

The revalidation matrix policy lived in the workflow and in the local script, and the local
copy lacked the workflow's skip of the `loft` package.  That package is the compiler; its
`tests/` holds fixtures that are not standalone programs, so 26 of its 400 files read as a
language break and every clean run said `1 COMPILE-BREAK` and exited 1.  A real break read
`2 COMPILE-BREAK`, one character from a baseline everyone had learned to ignore.  Sharing one
policy file (`scripts/revalidate_matrix.py`) also settled three quieter disagreements: the
known-broken map, the `subpath` default, and whether a yanked version may be validated (it may
not).

The `--self-test` found a second defect in the shipped gate at once: `loft --dump` writes a
`tests/.loft` cache directory, the `*.loft` glob matched it, and the re-classification loop
reported a runtime failure as a COMPILE-BREAK on any package with two or more test files.
Both copies now use `find -type f`.

## The release-ready verdict

Before it was a verdict it was a dashboard line inside a green check.  Over
`revalidate-libs`'s own history the set of libraries carrying warnings grew **2 → 11 in eight
days, every run green**, and the first anyone heard of it was a red check in the library repos
on code their authors had not touched.

## 2026-09-23 — the letter of a rule is applied without asking

The owner's standing ruling: when a fix meets a rule whose letter and the code disagree, the
agent applies the letter.  The reason measured that week: the three rulings asked for
(loft#1600 block scoping, loft#1619 the once-taken range end, C127 narrow ranges) all went the
letter's way, and each waited on the owner's attention.

## 2026-09-24 — a stale registry clone

`../loft-registry` was 5 commits behind, so `revalidate_libs_local.sh` picked `graphics` 0.9.1
and `imaging` 0.3.2 and reported both as COMPILE-BREAKs, while `revalidate-libs.yml` on the
same commit was green with 0.9.3 and 0.3.3 (`imaging` 0.3.3 was published that day to cure
the break).  Two agents on the box reproduced each other's phantom breaks: agreement between
siblings is not independent evidence when the input is shared.  The A/B against `origin/main`
on that branch then answered main 0 / branch 1: `assets` 0.2.1, refused by loft#1660, whose
issue and `Contract: strained … (owner-ruled)` trailer had already named that blast radius.

## PR #1688 — new sibling commits after a gated join

A second batch of a source's commits, taken after a join that had passed the gate, was checked
by build, ratchet and bundle stamp only.  It carried four defects that only the union had, and
six gates went red through one test.
