<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Joining branches

How to bring another line's work into this branch without losing any of it: a rebase onto a
`main` that squash-merged your commits, or a join of a sibling checkout's branch.  Several
checkouts of this repo work in parallel and pick from one another, so a join is a routine
task.  Each rule below was measured on a join; the incidents are in
[DEVELOPMENT-history.md](DEVELOPMENT-history.md).  Why to stay close to `main` in the first
place: [DEVELOPMENT.md § Stay close to `main`](DEVELOPMENT.md#stay-close-to-main--rebase-rigorously).

## Contents
- [The script](#the-script)
- [Does this tree already have that change?](#does-this-tree-already-have-that-change)
- [Rebasing onto a squash that carried your commits](#rebasing-onto-a-squash-that-carried-your-commits)
- [Joining a sibling checkout](#joining-a-sibling-checkout)
- [Resolving a conflict](#resolving-a-conflict)
- [Verifying a join](#verifying-a-join)

## The script

`scripts/join.py` is the mechanical half of this page; the judgment below stays yours.

```bash
scripts/join.py survey [SRC …]   # per source, per commit: pick NEW/PARTIAL, skip the rest
scripts/join.py apply            # cherry-pick source by source, `cargo check` after each
scripts/join.py rederive         # re-measure every derived artefact on the union (--commit)
scripts/join.py verify           # the cheap checks a gate stops on: pre-flight, clippy ×2, …
scripts/join.py guards           # the tests/scripts guards the join brought in, both backends
scripts/join.py run [SRC …]      # all five, stopping at the first decision; then the gate
```

A SRC is a ref or `name=<sha>`; with none, every `origin/*` branch that moved in the last two
days and has commits this tree lacks.  The survey classes each commit by the ladder below, in
this order: `DERIVED` (it touches only derived artefacts), `DUP` (an earlier source carries its
subject), `HERE` (its diff applies in reverse to this tree, file by file), `SUBJECT` (this
branch has a commit of that subject — a sibling's pick of our own work, resolved against its
tree), `SQUASHED` (a squash on the base lists it as a `* <subject>` line), `WIP` (a `wip` /
`fixup!` / `squash!` checkpoint, which ends what the join takes from that source), `REVERTED`
(a pending commit and a later one that exactly inverts it — the self-reverting run below, skipped
whole), else `NEW`, or `PARTIAL` when only some of its files are here.

`apply` resolves a conflict itself only in a derived artefact (either side, re-derived later)
and in an append-only changelog (both sides); a conflict in anything else STOPS with the file
named, and `apply` resumes after your `git cherry-pick --continue`.  `rederive` reads its list
from `scripts/derived_artefacts.json` — each artefact's paths, the inputs that make it stale,
its writer and its check — and refuses to re-pin a count that GREW beyond the growth each
source pinned on its own tree, naming the source or, for the `@FR-N-Shape` ratchet, the new
opaque shape tests.  A new derived artefact is one entry in that file.

## Does this tree already have that change?

Ancestry answers about COMMITS; a join asks about CHANGES, and the two diverge after the first
cherry-pick.  The ladder, cheapest first, none sufficient alone:

1. `git merge-base --is-ancestor <sha> HEAD` — blind after any pick.
2. `git show <sha> | git patch-id --stable`, compared on both sides — blind after a pick that
   RESOLVED a conflict, and blind against a SQUASH, whose one patch is the sum of its inputs.
3. Subject match — finds those, but cannot tell a rework from the original it replaced.
4. Grep the tree for the change's own new names — the only real test.

Run it as a table over the whole source branch before picking anything.  The commits you
already carry are usually a contiguous PREFIX, which turns a join into a range instead of one
decision per commit.  Verifying a peer's "missing" claim by re-running the peer's method can
only confirm it; change the instrument ([CODE.md § shell](CODE.md)).

**Against a pre-squash ref, rungs 1 and 2 invent work.**  `git cherry` reports every
constituent of the squash as missing.  The tell: `git merge-base --is-ancestor <squash> <ref>`
answers no.  What survives a squash is file content, so compare trees:
`git merge-tree --write-tree origin/main <branch>` against `origin/main^{tree}` — identical
means the branch is a no-op however many commits it is "ahead".  Where that conflicts, score
`git diff origin/main...<branch> --name-only` on the files missing from `main`, especially
under `tests/`.

**A fetch reporting `(forced update)` on a sibling's branch is often your own stale ref.**
After you rebase onto a squash-merged `main`, your remote-tracking refs still sit on the
pre-squash chain, so a peer's ordinary push arrives as a forced update.  A bare `git push`
cannot force; establish whose ref is stale before reporting overwritten work.

## Rebasing onto a squash that carried your commits

1. **Test "already upstream" BEFORE resolving a conflict.**  Patch-id dedup drops the commits
   the carrier left unchanged, but one it EDITED conflicts, and resolving it applies your
   version on top of the one already in `main` — duplicate definitions, a tree that does not
   compile.  Build the skip list from the squash's own body
   (`git show --format=%b -s <squash> | grep '^\* '`); subject matching there is approximate,
   so a build is still the verdict.
2. **A completed rebase is not a verified rebase.**  `git rebase` prints *Successfully rebased*
   on a tree that does not compile, and `git status` is clean.  Build, and re-run the change's
   own probes, between the rebase and the push.
3. **Where both sides touched one region, keep `main`'s side** — it is almost always the later
   state of the same work.  Keep both sides only in append-only files such as `CHANGELOG.md`,
   where the two sides are independent entries.

## Joining a sibling checkout

- **Build after EACH source, not once at the end.**  A join can hold a defect neither branch
  could see: one gives an enum a new variant and makes every reader handle it, the other adds
  a reader, and only the union fails to compile.  `cargo check --all-targets` between sources
  names the source that caused it.
- **Skip a source's self-reverting run whole.**  A revert written against the source's tree,
  replayed onto a tree that closed the same issue another way, deletes the JOINING branch's
  fix.  The test: `git diff <commit-before-the-run> <revert>` is empty.  Run it before choosing
  a range, and cut the range around the run.
- **New commits from a source are a new join.**  Commits taken after a join that passed the
  gate need the gate again, not only a build, a ratchet and a bundle stamp.

## Resolving a conflict

- **`git checkout --theirs <file>` takes the WHOLE file, not the conflicted hunk.**  Every
  other change your side made to that file is reverted with it, and the result reads as a
  resolution.  Resolve a source file by editing the markers and keeping both halves; use
  `--ours`/`--theirs` only on generated artefacts, which you then regenerate.  The tell: a diff
  far larger than the conflict was.
- **A number both sides changed is a MEASUREMENT, not a merge.**  An audit row or a site census
  holds two true numbers, and neither is the merged tree's.  Resolve to either side to unblock,
  keep a list, and re-run every instrument once at the end in one commit, so the baseline in
  the diff is the receipt: `scripts/join.py rederive` runs every artefact in
  `scripts/derived_artefacts.json`, and `scripts/join.py verify` the checks beside them.
- **`git show --cc` cannot audit a join.**  A combined diff prints only lines that differ from
  BOTH parents, so a conflict resolved by taking one side wholesale — the commonest resolution
  — appears nowhere in it.  Recompute the merge instead: `git merge-tree --write-tree <p1> <p2>`
  names every conflicted file and writes both sides' markers; check each resolution against
  `git show <parent>:<file>`.  A conflicted file that IS in the `--cc` output was edited by
  hand; one that is NOT was chosen silently and is the one to read.  A derived artefact has no
  side to choose: re-derive it.

## Verifying a join

`make ci` is not the whole verification.  A join of store-lifetime or codegen work also needs:

- the `LOFT_POISON` / `LOFT_VERIFY_STACK` sweeps ([CI_BUDGET.md](CI_BUDGET.md));
- the shipped libraries ([REVALIDATE_LIBS.md](REVALIDATE_LIBS.md));
- the debug-assertions variant, where a `cfg(debug_assertions)` item that only one side's
  types reach fails to compile:
  `RUSTFLAGS='-C debug-assertions=on' cargo build --release --target-dir target/dbgassert --lib`;
- `scripts/feature_coverage.sh --check` and `make optional-ratchet`.
