
# Development Workflow

Step-by-step process for taking a PLANNING.md item from backlog to merged.

**Session start:** Review [CLAUDE.md](../../CLAUDE.md) at the project root — it contains the project overview, architecture, branch policy, and documentation index.

**Who develops loft:** almost entirely AI coding agents, steered by the owner (who prioritized documentation and tooling above writing code). Everything needed to work on loft is in this repo, so the project has no single point of failure — see [BUS_FACTOR.md](BUS_FACTOR.md).

---

## Contents
- [Branch Naming](#branch-naming)
- [Development Phase — Single WIP Commit](#development-phase--single-wip-commit)
- [Validation Against CODE.md](#validation-against-codemd)
- [Structured Commit Sequence](#structured-commit-sequence)
  - [Step 1 — Tests with `#[ignore]`](#step-1--tests-with-ignore)
  - [Step 2 — Code Changes](#step-2--code-changes)
  - [Step 3 — Enable Tests](#step-3--enable-tests)
  - [Step 4 — Structural Refactors](#step-4--structural-refactors)
  - [Step 5 — Documentation](#step-5--documentation)
- [Splitting High-Effort Items](#splitting-high-effort-items)
- [Bytecode Economy](#bytecode-economy)
- [CI Validation](#ci-validation) — local gate (before every commit) + remote CI (after push)
- [Commit Message Style](#commit-message-style)

---

## Branch Policy — Main is Read-Only

**Direct commits to `main` are not allowed.**

`main` is the release branch; every commit on it must be releasable.  All
development happens on feature branches and reaches `main` only through a
reviewed, CI-green pull request.

Rules:
- Never `git commit` directly on `main`.
- Pushing commits is OK by default — unless there's an open PR on the branch
  that the push would disturb.  For a long-lived working branch with no open
  PR, push freely after each green-CI commit so the remote stays in sync.
  When the branch has an open PR, do NOT push without the user's explicit
  consent (force-pushes / rebases / unexpected commits disrupt review) — the
  one exception is a fix for a blocking failure (red CI, a broken build, a
  failing required check), which you may push without asking, because it
  unblocks the PR rather than disrupting review (it cannot merge while red).
  Check `gh pr list --head <branch>` before pushing if uncertain.
- **Never create a branch unless the user explicitly says "create a branch".**
  Do not create branches as part of a workflow, sprint start, or task planning.
  Work on the current branch and commit locally.  The user decides when to
  branch or open a PR.
- Never create a feature branch from another feature branch — always branch from `main`.
- Merging to `main` is done via a GitHub pull request, not a local `git merge`.

**The working branch is ONE accumulating PR unit.**  A long-lived work branch
(`<host>-work`) carries fixes for *everything* worked on in the cycle — the feature,
plus any bugs found along the way.  Two consequences:
- **Fix bugs on sight, don't leave them on the floor.**  A bug discovered while doing
  other work (paths warm, repro cheap) is fixed in the same branch, not filed and
  deferred — a left-behind bug only hampers later work.  (Reinforces "default is FIX,
  not file".)
- **PR the whole branch, not a surgical slice.**  When asked to "PR the <feature>", the
  PR is the entire work branch as one review unit — do not cherry-pick a subset onto a
  fresh branch.  If the branch is BEHIND `origin/main`, bring it current by **merging
  `origin/main` in** (rebase conflicts against squash-merged duplicates), reconciling
  conflicts to keep both sides' features; where both branches converged on the same fix,
  take the mainline's canonical form and keep any local improvement on top.
- ⚠ **`git checkout --theirs <file>` takes the WHOLE FILE, not the conflicted hunk.**  Every
  other change your side made to that file is reverted with it, silently, and the diff you
  review afterwards looks like a resolution rather than a loss.  Measured twice here: a
  2026-09-02 join resolved `tests/docs/25-generics.loft` that way and dropped 101 lines of
  chapter for 32 — the `<T, U>` restriction, the note that generic structs do not exist, an
  empty-vector caveat and its assertion — and the branch's own log already carried the same
  lesson from an earlier pick (*"Three chapters re-read after the release picks moved them,
  and all three had lost something"*).  Resolve a conflicted SOURCE file by editing the
  markers and keeping both halves; reserve `--ours`/`--theirs` for generated artefacts, which
  you then REGENERATE rather than trusting either side.  The tell that you took a whole file
  by accident is a diff far larger than the conflict was.
- **A count that both sides changed is a MEASUREMENT, not a merge.**  Where a conflict is a
  tracked number — an audit row, a site census — re-run the tool on the merged tree instead of
  picking a side or splitting the difference.  The same join found `678 | 324 | 5 | 349` and
  `678 | 325 | 5 | 348`, and the merged tree was neither.

---

## Sprint Branches

Development is organized into **sprints** (see [ROADMAP.md](ROADMAP.md) for
the sprint plan).  Each sprint gets **one branch** containing up to ~4 items.
The branch is merged to main via a single PR when all items pass CI.

### Why sprints, not per-item branches

- A sprint groups related items that touch overlapping files (e.g. PKG.1 +
  PKG.2 + PKG.6 all touch `compile.rs` and `main.rs`).
- Fewer PRs = less CI wait time and merge churn.
- Each commit within the branch is still one coherent item (test + code +
  enable), so `git log` stays bisectable.
- **Owner directive: a PR is never one issue, or a few.** *"I will never PR one or a
  few issues, it takes a lot of time because we cannot stack PR's on gh"*
  (2026-08-19). GitHub has no stacked pull requests, and the branch policy already
  requires new work to branch from the TIP of unmerged in-flight work — so a second
  PR cannot be reviewed or merged independently, it sits behind the first one's merge
  clock. **Every PR therefore serialises the whole stream**, which is a bigger cost
  than the CI round it also pays (~20–30 min).

  **Opening a PR is the owner's call, and not a subject to raise.** Do not propose one,
  hint that the work is "ready" for one, or treat a finished issue as a milestone that
  wants one — that pressure is why the owner holds off (2026-08-19). Fix, gate, push,
  and say what is done.

  One consequence to handle LOCALLY rather than by reaching for a PR: `revalidate-libs`
  — the gate that compiles every published library against this loft — triggers on
  `pull_request` and on `push` **to `main` only**, never on a work branch. So a language
  change that retro-breaks shipped libraries is invisible on the branch. On 2026-08-19
  that was nine libraries losing their entire public surface to one resolution rule,
  green on every branch gate for a full day. The cure is to run a library's suite from a
  **scratchpad copy** after any resolution or diagnostic change (a suite run inside a
  consumer's tree writes `native-auto/` and `.loft/` and is not read-only).

  **`scripts/revalidate_libs_local.sh` is that, for the whole registry** — one library is
  the advice the incident produced and the gate is all 42. It reads the matrix from
  `../loft-registry/index.json` through `scripts/revalidate_matrix.py`, which is the
  workflow's source too, extracts each release TAG with `git archive` so the sibling clones
  are never written to, runs the suite, and re-classifies a failure exactly as the workflow
  does. Run it after any `src/**` or `default/**` change that a library could notice.
  `--self-test` first if you are about to trust a green: it injects a compile break and a
  runtime break and asserts the two are reported DIFFERENTLY, and checks the shared matrix
  policy on inputs each of its rules has to act on. A SKIP is not a pass — it means that
  repo is not cloned beside this one.

  **It answers a SECOND question now, apart from the first: can each library be built and
  RELEASED as it stands?** A library carrying warnings cannot — its own CI runs
  `LOFT_DENY_WARNINGS=1`, so a release cut from it fails and the next PR to that repo is red
  before its author has typed anything. So the run prints a `RELEASE-READY` /
  `NOT RELEASE-READY` verdict naming the packages, and carries it in the exit code.

  ⚠ **The two verdicts are apart because only one of them is your change's fault.** A
  COMPILE-BREAK is a language change retro-breaking a shipped library — the freeze's
  question, and yours to answer. `NOT RELEASE-READY` is a fact about the ecosystem that was
  true before you started; the closing line says so, so a red never sends you looking for a
  regression you did not cause.

  ⚠ **Red on warnings EXISTING, not on the set growing.** "Can it ship" is a question about
  the absolute state; a delta answers a different one. On the CI side this is the
  `Release-ready` STEP inside each library's own matrix leg in `revalidate-libs.yml` — one
  pass per library, reading the log that pass already wrote, so the answer costs nothing.
  It is **advisory and must never become a required check**: `main` requires Test ×3,
  Clippy and Format, and a library's warning debt is information, never a veto on a loft
  PR. On a RELEASE it is blocking, which is `release-gate.yml`'s standing rule —
  *informational on a diff, blocking on a release* — and exactly right for a question that
  is literally "can these be released".

  ⚠ **Why it is a verdict and not a dashboard line**, in one sentence, because the dashboard
  already existed: it printed the dirty set for weeks inside a GREEN check — measured over
  `revalidate-libs`'s own history, **2 → 11 libraries in eight days, every run green** — and
  the first anyone heard of it was a red check in the library repos on code those authors
  had not touched, which is exactly what the step's own comment predicted. A report whose
  default state nobody notices is not a report.

  **A clean run reads `42 pass, 0 runtime/env, 0 skipped, 0 COMPILE-BREAK` and exits 0.**
  It did not until loft#1315: the matrix policy was written twice, once in the workflow and
  once here, and the local copy was missing the workflow's skip of the `loft` package. That
  package is the COMPILER — its `tests/` is this repo's own suite and holds fixtures that
  are deliberately not standalone programs — so the re-classifier read 26 of its 400 files
  as a language break and the summary said `1 COMPILE-BREAK` on a tree with nothing wrong
  in it, against the very binary the package was published with. The exit status was
  therefore 1 on every run, a real break read `2 COMPILE-BREAK` — one character from the
  baseline everyone had learned to ignore — and the closing sentence about the freeze
  printed on every green run. **If your run is not zero, read the rows: a number whose zero
  is not zero measures nothing.** The two readers now share one policy file, which also
  settled three quieter disagreements between them (the known-broken map, the `subpath`
  default, and whether a YANKED version may be validated — it may not).

  ⚠ That self-test earned its place immediately: it found the shipped gate misclassifying.
  `loft --dump` WRITES a `tests/.loft` cache directory beside the file it compiles, the glob
  `*.loft` matches that name, and `find` streams through a process substitution — so the
  re-classification loop could be handed the directory it had just created, fail to `--dump`
  it, and report a **runtime/environment failure as a COMPILE-BREAK**: a shipped library
  falsely accused of a freeze violation, on any package with two or more test files. Both
  copies now use `find -type f`.

  So **bundle many subjects into ONE PR**, even unrelated ones — a docs/tooling
  stream, a compiler soundness fix and a language feature ride together. A branch the
  owner asks for is a branch to ACCUMULATE on, not one to PR when its first issue is
  done. Do NOT default to proposing a split; only split when the owner explicitly
  wants an independent clock for one item. Run the local gate (`make gate` / `make ci`)
  once, so the single PR clears in one round instead of bouncing.

### Stay close to `main` — rebase rigorously (the 2026-06-24 lesson)

The instinct to partition work into clean per-topic branches feels tidy but is the
direct cause of merge hell.  **Build everything on the ONE working branch — mixed
topics are fine — and rebase it on `origin/main` OFTEN** (after every merge into
main, not once at the end).  Four reasons this is the discipline, not a preference:

1. **You don't lose track of the work.**  One branch is one whole; many branches
   scatter it and you can no longer see (or reason about) the change as a unit.
2. **Overlapping work merges cleanly.**  You reconcile small and continuously,
   instead of one big painful merge where a diverged base collides commit-by-commit.
3. **`git diff main` becomes a real refactor compass.**  Close to main, the diff
   tells you *exactly* what you changed — am I going the right direction? — and lets
   you cleanly **revert any file to a known-good baseline** (`git diff main -- f`,
   `git show origin/main:f`).  A diverged branch poisons that diff with merge noise,
   so you lose BOTH the compass and the clean revert.
4. **The compass survives others' work.**  After a rebase onto a `main` that now
   carries someone else's slightly-related fixes, your `git diff main` is *still* your
   delta on the up-to-date base — you absorb their fixes for free and keep comparing
   cleanly.  Skipping the rebase is what makes their work and yours un-mergeable later.

**The cautionary tale (what NOT to do):** a long-lived branch accumulated ~65 commits
across 5 topics (@PLN87 · sandbox · formalization · a parser fix · a CI change) without
rebasing, while `main` advanced.  @PLN87 reached `main` as a SQUASH (one PR commit) while
the branch kept the *individual* @PLN87 commits AND built more on top of them.  Different
patch-ids + a diverged base ⇒ git cannot auto-drop the duplicates, every parser commit
collides, and spinning off sub-branches multiplied the surface.  The only clean escape was
to cherry-pick just the genuine delta onto fresh `main` — exactly the reconciliation a
regular rebase would have done a little at a time.  Rebase early, rebase often.

**When you DO rebase onto a squash that carried your commits, two mechanical rules
(2026-08-21).**  A peer's PR squash-merged 41 of this branch's commits under new hashes:

1. **Test "already upstream" BEFORE resolving a conflict, not after.**  Git's patch-id
   dedup silently drops the commits that survived the carry unchanged — 32 of the 41 here
   — but any commit the carrier EDITED has a different patch, so it conflicts and then gets
   force-applied ON TOP of the version already in `main`.  That is not a merge artefact
   you notice: it defines `WorkerState`, `WORKER_FATAL` and `take_worker_fatal` twice and
   the tree stops compiling (`E0428`, `E0119`).  A resolver loop that resolves first and
   skips second feeds duplicates through commit after commit; ordering the skip test first
   took the survivors from 9 to 4.  Build the skip list from the squash commit's own body
   (`git show --format=%b -s <squash> | grep '^\* '`) and treat subject matching as
   approximate — some content lands under a different subject, so a build is still the
   verdict.
2. **A completed rebase is not a verified rebase.**  `git rebase` printed *Successfully
   rebased* on the tree that did not compile, and nothing in git was going to say
   otherwise — `git status` was clean, the log looked right, `--force-with-lease` would
   have pushed it.  Build, and re-run the change's own probes, BETWEEN the rebase and the
   push.

Resolving the conflicts themselves: where both sides touched one region, `main` was almost
always the LATER state of the same work — the carrier had corrections on top, so a residual
this branch still recorded as "open" was already "closed" there.  Keep `main`'s side for
those.  Keep BOTH sides only for append-only files like `CHANGELOG.md`, where the two sides
are independent entries rather than two versions of one.

### Joining sibling checkouts — the branch is the unit, not its tip

Three checkouts of this repo work in parallel and continuously cherry-pick from one
another, so "join `../loft2`, `../loft3` and `origin/<branch>`" is a routine task rather
than an event.  Four things about it are not obvious, and each was measured on a join.

**Classify every source commit before picking any.**  The ladder is in
[CLAUDE.md](../../CLAUDE.md) and in the memory note it points at: ancestry is blind after
the first pick, `patch-id` is blind after the first CONFLICT-RESOLVED pick, subject match
finds those but cannot tell a rework from what it replaced, and grepping for the change's
own new names is the only real test.  Run it as a table over the whole source branch —
the commits you already carry are usually a contiguous PREFIX, which turns three joins
into three ranges instead of seventy-six decisions.

**A source branch's own REVERT makes a sequence NET-ZERO — skip it whole.**  A branch that
tried a mechanism, measured a peer's as sufficient and withdrew its own leaves a
self-cancelling run in its log.  Picking it is worse than wasteful: the revert was written
against ITS tree, where the withdrawn work was the only implementation, and replayed onto
a tree that closed the same issue by another route it deletes the JOINING branch's fix
instead.  Measured 2026-09-09 — four commits (a pick of this branch's own fix, two of the
source's, and the revert) where `git diff <commit-before> <revert>` is EMPTY.  That diff
is the test: run it before choosing a range, and cut the range around the run.

**A DERIVED row is re-measured on the join, never carried.**  Two branches that each
lowered a count hold two true numbers and neither is the merged tree's; the same trap has
cost QUALITY.md's audit row a false figure on eight consecutive joins.  Resolve such a
conflict to either side to unblock, keep a list, and re-run every instrument once at the
end — `ir_walker_audit.py optional --write-ratchet`, `o_proxy_check.py`,
`rule_tags.py check` — in one commit, so the baseline in the diff is the receipt.

**Build after EACH source, not once at the end.**  A join can hold a defect neither branch
could see: one branch gives an enum a third variant and makes every reader dispose of it
explicitly, the other adds a reader, and only the union fails to compile.  Neither
branch's gate could have gone red.  `cargo check --all-targets` between sources attributes
such a failure to the source that caused it instead of to the whole join.

**The gate is not the whole verification of a join.**  `make ci` arms neither the
`LOFT_POISON` / `LOFT_VERIFY_STACK` sweeps ([CI_BUDGET.md](CI_BUDGET.md) — a minute each on
this box) nor the shipped libraries (`scripts/revalidate_libs_local.sh`, below).  A join of
store-lifetime and codegen work is exactly the change class both cover.

### Sprint branch naming

```
sprint-{N}-{short-description}
```

Examples:
- `sprint-1-pkg-infrastructure`
- `sprint-2-stdlib-extraction`
- `sprint-4-http-client`

### Sprint workflow

**Every sprint branch MUST start from a merged, up-to-date `main`.**
If the previous sprint's PR has not been merged yet, wait for it.
Never branch from another feature branch.

**Do not create the branch yourself.**  Wait for the user to say
"create a branch" — then follow the naming convention below.

```
1. Merge the previous sprint's PR (wait for CI green)
2. (user creates branch from main)
3. For each item in the sprint (up to ~4):
   a. Write tests with @EXPECT_FAIL / @EXPECT_ERROR
   b. Implement the code change
   c. Remove annotations, verify tests pass
   d. Commit: "{ID}: {description}"
4. Update all relevant documentation (see checklist below)
5. cargo fmt && cargo clippy --tests -- -D warnings && cargo test
6. (user says "push" → git push -u origin {branch})
7. (user says "create PR" → gh pr create)
8. Wait for CI green on all 3 platforms
9. (user says "merge" → gh pr merge --squash)
```

### Announce each step — MANDATORY

**State the name of every step as you start or finish it.**  This applies to
the sprint workflow, individual items, and sub-steps within each item.
Always include the issue/item ID when one exists.

Examples:
- "Starting H4.1: HttpResponse struct"
- "Starting H4.1: writing test for http_get"
- "Finished H4.1 — interpreter + native tests pass"
- "Starting: clippy fixes for loft_register_v1 refactor"
- "Finished: clippy clean, 0 warnings"
- "Starting step 5: documentation updates"
- "Finished step 6: CI green, 548 passed"

**Why:** silent progress is invisible progress.  The user cannot see tool
calls in real time — they only see text output.  Naming each step gives the
user a running status line so they know where things stand, can interrupt
early if the plan is wrong, and can resume efficiently if context runs out.

**Why this matters:** branching from an unmerged feature branch creates
a dependency chain.  If the earlier branch needs changes during review,
the later branch must be rebased — causing merge conflicts and wasted
work.  Sequential merges keep the history linear and each PR reviewable
in isolation.

### Item limit per sprint

**Target: ~4 items per branch.** This keeps PRs reviewable (<500 lines of
non-test code) and limits blast radius if something goes wrong.  A sprint
with fewer than 4 items is fine — never pad a sprint to reach the target.

If an item turns out larger than expected, split the sprint: merge what's
done, create a new branch for the remainder.

### Documentation updates — MANDATORY per sprint

**Every sprint must update all documentation affected by its changes before
the PR is created.**  Documentation is not a follow-up task — it ships with
the code.  **Never create a separate docs branch or PR** — documentation
commits belong in the same sprint branch as the code they describe.

#### Checklist (step 5 in the sprint workflow)

Run through this list before pushing.  Skip items that are clearly unaffected.

| Document | Update when… |
|---|---|
| `doc/claude/CHANGELOG_TECHNICAL.md` | Always — add a detailed entry under `## [Unreleased]` for every change (internal phase/opcode/slot detail welcome) |
| `CHANGELOG.md` | When a change is user-visible — add a friendly, jargon-free entry under `## Unreleased`.  Entry-level programmers are the audience |
| `doc/claude/ROADMAP.md` | Sprint items were completed or reprioritised |
| `doc/claude/PLANNING.md` | Items were completed (remove) or new items discovered (add) |
| `doc/claude/PROBLEMS.md` | Bugs were fixed (mark resolved) or **any new bug found during the sprint** (add with reproducer) |
| `doc/claude/CAVEATS.md` | Edge cases were fixed or **any new workaround discovered** (add with test reference) |
| `doc/claude/TESTING.md` § Coverage Gaps | Test coverage improved or new gaps identified |
| `README.md` | New user-facing features, CLI commands, or examples added |
| Relevant feature design doc (reference doc or active plan README) | Implementation diverged from design, or phases completed |
| `doc/claude/STDLIB.md` | New stdlib functions or types added |
| `doc/claude/LOFT.md` | Language syntax or semantics changed |
| `doc/claude/INTERNALS.md` | New opcodes, state changes, or native functions added |
| `.claude/skills/loft-write/SKILL.md` | New patterns, caveats, or conventions for writing `.loft` files |

**Filing bugs is not optional.** Every workaround, test simplification, or
failure encountered during the sprint — even if worked around — must be
filed in PROBLEMS.md or CAVEATS.md with a reproducer.  Unfiled bugs get
rediscovered in future sprints, wasting time.

**Why this matters:** stale documentation causes wasted time in future
sessions.  Claude reads these docs at session start — if they describe
features that don't exist yet or omit features that do, the first 10 minutes
of the next session are spent rediscovering the current state.  Keeping docs
in sync with code is cheaper than reconstructing context later.

---

## Branch Naming

For non-sprint branches (bug fixes, documentation, one-off tasks), use
item ID + short suffix:

```
{id}-{short-name}
{id}-{id}-{short-name}        # two items
```

IDs use the single-letter prefix scheme: `l1`, `p1`, `p1-1`, `a6`, `n2`, `r1`, `w1`.
Phase sub-steps use the dot notation lowercased: `p1-1`, `p1-2`, `a6-3`.

Examples:

| Planning item(s) | Branch name |
|---|---|
| L2 — Nested match patterns | `l2-nested-match-patterns` |
| P1.1 + P1.2 + P1.3 — Lambda expressions (all 3 phases) | `p1-1-p1-2-p1-3-lambda-expressions` |
| A6.1 — Stack slot assign_slots standalone | `a6-1-assign-slots-standalone` |
| N2 + N3 + N4 — output_init/output_set/format fixes | `n2-n3-n4-output-fixes` |

Branches are created from the tip of `main`.  **Do not create branches
yourself** — wait for the user to ask.  When they do:

1. Commit any uncommitted work on the current branch first.
2. `git checkout main && git pull`
3. `git checkout -b {branch-name}` (only when the user says to)

Never use `git stash`.  Never create a feature branch from another
feature branch.

---

## Development Phase

For **trivial one-file fixes** (e.g. a single clippy suppression, a doc typo),
work directly without a structured commit sequence — just run the local CI gate
before committing.

For **all planned items** (anything in PLANNING.md with an ID), follow the
[Structured Commit Sequence](#structured-commit-sequence) below.  Do not collapse
a planned item into a single amending WIP commit; bisectability and item-traceability
require separate commits for tests, implementation, and docs.

Verify locally at any point using the full CI gate:

```bash
make ci       # fmt → clippy → test; stops at first failure; full output in result.txt
```

Keep the **installed** loft current — a stale one silently builds consumer libraries
against an old rlib.  Reinstalling needs no root:

```bash
make install-user-fast    # => ~/.local, native only (skips the wasm + html-mt runtimes)
make install-user         # => ~/.local, everything
```

Both verify that `command -v loft` is the binary they just installed, reporting an
absent *and* a shadowed `PATH` entry with the rc line for your shell.  `PREFIX=…`
retargets any install target; `sudo` is used only when the prefix isn't writable.

The order matters: `cargo fmt --check` and `cargo clippy --tests -- -D warnings` run
first so formatting and lint errors are fixed before the slower `cargo test` runs.
If `make` is unavailable, run the three commands manually in the same order:

```bash
cargo fmt -- --check                    # no formatting diff; run `cargo fmt` to fix
cargo clippy --tests -- -D warnings     # zero warnings, including test code
cargo test                              # all tests pass
```

---

## Validation Against CODE.md

Before committing, check new code against every rule in [CODE.md](CODE.md):

| Check | Command | Exception |
|---|---|---|
| No clippy warnings | `cargo clippy --tests -- -D warnings` | Skip pre-existing `too_many_lines` and `cognitive_complexity` violations in functions you did not write — fixing them would disrupt unrelated code and obscure the feature diff |
| Formatted | `cargo fmt -- --check` | None |
| Naming conventions | Manual review | `n_<name>` for global natives; `t_<LEN><Type>_<method>` for methods |
| Function length | `cargo clippy` | If **new** code you wrote triggers `too_many_lines`, move the refactor to Step 4 of the commit sequence rather than mixing it with the functional change |
| Null sentinels | Manual review | Any new numeric function returning null must use `i32::MIN` / `i64::MIN` / `f64::NAN`, never `0` |

The line-count and complexity exceptions exist because fixing these in files
touched incidentally by a feature would inflate the diff and make the real change
hard to review.  Such refactors belong in a dedicated commit (Step 4) if they are
necessary, or left for a separate cleanup task if they are pre-existing.

---

## Commit Rules

A branch may contain **any number of commits** as long as every commit satisfies the
local CI gate — see [CI Validation](#ci-validation) for the exact commands.  In short:

```bash
make ci
```

Run this **before every `git commit`** (including amends).  A commit that breaks
any of these must be fixed before the session ends.  Never rely on the remote CI to
catch failures that could have been caught locally.

### Commit structure

Each commit should be a coherent, self-contained change.  Good splits:

- Code change + its tests in one commit
- Documentation updates in a separate commit
- Refactors that don't change behaviour in their own commit

### Pushing `.github/workflows/` changes — use the SSH remote

GitHub rejects a push that creates or updates any file under `.github/workflows/`
when the credential is an **OAuth-app token lacking `workflow` scope**:

```
! [remote rejected]  <branch> -> <branch>
  (refusing to allow an OAuth App to create or update workflow
   `.github/workflows/ci.yml` without `workflow` scope)
```

In agent sessions the git credential helper is `gh auth git-credential`, whose
token carries `repo` but **not** `workflow` — so an HTTPS push of any CI-workflow
change is rejected.  This is **server-side**: disabling the command sandbox does
not help.  (History confirms it — past CI changes reached `main` via PR
**merges**, where GitHub applies the workflow file server-side, never via a
direct OAuth HTTPS push.)

The account is configured for SSH git operations (`gh auth status` →
`Git operations protocol: ssh`) and the SSH key pushes with full rights and no
per-scope gate.  So push workflow changes over SSH:

```bash
# one-shot, without touching the remote:
git push git@github.com:loft-lang/loft.git HEAD:<branch>
# or align origin once so every push uses SSH (matches the gh SSH default):
git remote set-url origin git@github.com:loft-lang/loft.git
```

Non-workflow changes push fine over the HTTPS/OAuth path; only files under
`.github/workflows/` need this.

### Document findings before committing

When implementing a feature, you often discover things not in the planning:
limitations, edge cases, incorrect assumptions, or new issues.  **Update the
relevant documentation before including it in the commit:**

- **PROBLEMS.md** — new bugs or limitations discovered during implementation
- **PLANNING.md** — **remove the completed item entirely** (both the section and
  the Quick Reference row).  PLANNING.md is strictly for future work; completion
  history belongs in git and CHANGELOG.md.  If only part of an item was done,
  update the section to describe what remains.
- **NATIVE.md** — design corrections found during implementation
- **INCONSISTENCIES.md** — new language quirks discovered

Include these documentation updates in the docs commit at the end of the branch.
Do not wait until later — findings are freshest immediately after implementation.

When multiple PLANNING items share a branch — **including the individual phases of a
multi-phase item** — **each item or phase gets its own separate commit sequence**.
Do not collapse them into one big commit.  A reader bisecting the history must be
able to pin the change to a single item or phase.  Mention the item ID in every
commit message that belongs to it (e.g. `P1.1: …`, `P1.2: …`, `N2: …`).

### Commit message style

```
{scope}: {imperative summary}  (≤ 72 characters)

{body: describe what the feature does in plain language.  Focus on the
user-visible or developer-visible effect, not the implementation.
Mention function or file names only when they clarify the scope.}

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
```

**Scope** is one of:
- `L1`, `P1`, `P1.1`, `A6`, `A6.2`, `N2`, `W1` etc. — planned item or phase
- `fix` — bug fix not tied to a planned item
- `docs` — documentation-only change
- `refactor` — behaviour-neutral code change

**Summary** starts with an imperative verb: *add*, *fix*, *implement*, *remove*,
*enable*, *warn on* — never *added*, *adds*, *implementing*.

**Body** explains what changed and why in clear sentences.  Avoid listing every
file or function touched — the diff shows that.  Use a function name only when
it is the thing being fixed or added (e.g. "fix `output_if` to emit typed nulls")
rather than as implementation detail.

**Good example:**
```
N6.1: implement vector iteration in codegen_runtime

Vector `for` loops now emit an index-based loop using a dedicated
`_iter` counter variable rather than relying on the interpreter's
generic iterate path.  This is the first of three N6 phases; sorted
and reverse iteration follow in N6.2 and N6.3.
```

**Bad example:**
```
N6.1: fix codegen_runtime.rs vector loop

Changed emit_for_vector() at line 412 to add _iter variable and emit
OpGetInt/OpSetInt for the counter. Added match on IterKind::Vector in
three places. Updated output_step() at line 531 to check _iter against
vec_len. Added OpBranchFalse at end of loop body.
```

### Documentation commit

The **last commit** on a branch updates documentation:

```
docs: {ID} — update CHANGELOG, PLANNING

- CHANGELOG: add feature/fix entry under Unreleased
- PLANNING: remove completed item section and quick-reference row
```

Review every file in `doc/claude/` for references to the feature and update as needed.

---

## Splitting High-Effort Items

Any item rated **Medium–High or higher** in PLANNING.md must be split into
sub-steps before work begins.  A sub-step is a change that:

1. **Passes all three CI checks on its own** (`make ci`).
2. **Has at least one test** that was written before the implementation (Step 1 of the
   structured sequence) and enabled immediately after (Step 3).
3. **Leaves the codebase in a better or equal state** — no sub-step may introduce a
   regression, a dead code path, or a half-working feature visible to loft programs.

### How to split

Look for **natural seams** in the planned work.  Good split boundaries:

| Seam | Example |
|---|---|
| Independent areas of the codebase | Parser change + runtime change → two commits |
| Phases of a larger design | A8 destination-passing: Phase 1 compiler, Phase 2 native rewrites |
| Feature flags / opt-in paths | Implement behind a `#[cfg(test)]` stub, then wire it in |
| Layers of correctness | Guard first (panic on bad input), full fix second |
| Subset of cases | Handle the common case first, edge cases in follow-up commits |

If no natural seam exists and the item genuinely cannot be split, document why in the
PLANNING.md item before starting.  This is the exception, not the rule.

### Update PLANNING.md before starting

When splitting a High or Very High item, **rewrite its Fix path section** in
PLANNING.md to list the sub-steps explicitly before the first commit lands.  This:

- Makes the plan reviewable before any code is written.
- Gives future sessions enough context to resume mid-item without re-deriving the plan.
- Forces a check that each sub-step is independently testable — if you cannot write a
  test for a sub-step, the split boundary is wrong.

Example: A8 (destination-passing for text-returning natives) was already split into
phases (compiler, native rewrites, format expressions, scratch buffer removal) in
PLANNING.md before implementation began.  Each phase is independently testable because
existing string tests catch regressions and new tests verify the new calling convention.

### Size budget

A single commit should rarely exceed **~200 lines of non-test Rust**.  If a sub-step
exceeds this, look for a smaller seam.  Large diffs are hard to review, hard to bisect,
and statistically more likely to contain regressions.

---

## Inserting Discovered Enhancements Into the Active Plan

Building real loft consumers (libraries, tools, viewers, indexers)
systematically surfaces gaps in the language and stdlib that toy
programs and the test suite never hit.  A consumer that needs a
missing feature has THREE choices at the moment of discovery:

  1. **Work around it in the consumer** — write extra loft code
     to dodge the gap, leave a `// loft gap: ...` comment.
  2. **Defer it to a separate plan / catalog** — file the gap
     somewhere central, keep building the consumer.
  3. **Insert a step into the active plan that fixes the gap
     itself, then resume the feature work** — language /
     stdlib gets sturdier; the workaround never enters
     shipped code.

**Default to (3) when the fix is XS or S** (under half a day).
For (3) the discovered enhancement becomes a sub-step of the
plan's CURRENT phase — landed BEFORE the next feature phase
opens.  The cost-of-context advantage is the whole reason:
you already understand the workaround site; the loft-side
fix is one or two file edits away in compiler / stdlib
territory you haven't paged out yet.

### When to default to (1) workaround + (2) defer instead of (3) inline-fix

Use (1) + (2) when ANY of these hold:

  - Fix needs design discussion (typer architecture,
    multi-file refactor, breaks ABI).
  - Fix is M+ effort (a day or more) and would push the
    feature phase past its planned-budget.
  - Workaround is genuinely cheap and the gap is
    documented (`<!-- loft gap: needs vector.sort() -->`
    inline + a row in the canonical home).

Default to (3) when the fix is small, the consumer
workaround is clearly inferior to having the feature, and
the consumer code that uses the gap is fresh in working
memory.

### How items are documented (one canonical home each, no duplicates)

When (3) doesn't apply, the gap goes to its CANONICAL home.
**Never invent a parallel catalog** — that creates the
"two places to keep in sync" problem and dilutes the action
surface (`./scripts/idx broken`, the broken-tag validator,
the open-issues fast index in PROBLEMS.md, etc.).

| Item shape | Canonical home | Where to scan for them |
|---|---|---|
| **Bug** (observable wrong behavior — codegen quirk, parser quirk, lexer rejection, runtime panic) | P-issue row in [PROBLEMS.md](PROBLEMS.md) | `./scripts/idx tag:@P<n>` per ID; bash one-liner `./scripts/idx all \| jq '...'` for the open set |
| **Stdlib gap** (missing fn / method / overload that fits the existing API surface) | `## Open work` row in [STDLIB.md](STDLIB.md) | grep / read STDLIB.md `## Open work` |
| **Compiler / language gap** (lexer / parser / typer change with surface-area implications) | P-issue row OR `## Open work` row in [COMPILER.md](COMPILER.md) | same as above |
| **Native codegen gap** | `## Open work` row in [NATIVE.md](NATIVE.md) | NATIVE.md |
| **New library** (independent package — process, fs_watch, regex, cache, …) | a [`loft-lang/plans`](https://github.com/loft-lang/plans) issue (`@PLN<n>`, labelled `subject:libs`) | `gh issue list -R loft-lang/plans --label plan` |
| **Big deferred feature** (M+ scale; needs its own design + multi-phase implementation; can't reasonably inline into the discovering plan's phase) | a `loft-lang/plans` issue (`@PLN<n>`) — goal, phases, acceptance in the issue body; no local plan slot | `gh issue list -R loft-lang/plans --label plan` |

The in-code workaround comment is MANDATORY regardless of
which choice (1 / 2 / 3).  Reference the canonical home so
the workaround stays self-explaining:

```loft
// @P276 — `s[i] ?? '<char>'` chain-compare trips rustc E0308 in
// native; remove `??` and rely on the surrounding `i < n` guard.
c = line[i];
```

```loft
// stdlib gap (STDLIB.md § Open work, "vector.sort"): no
// vector.sort() yet.  Use sorted<TagSlot[name]> as a sort
// proxy for now.
struct TagSlot { name: text not null }
```

When the canonical-home item ships, the workaround comment
gets removed in the same commit (the comment IS the
"unwound someday" handle).

### Schedule-to-fix lives in the active plan

The canonical home (P-issue / `## Open work` / lib_plans
slot) holds the **design / details / reproducer** — the
"what's broken and how would we fix it" answer.  The active
plan's sub-step list holds the **schedule** — the "we plan to
land this in this phase" commitment.

Two-part discipline:

  1. File the issue in its canonical home (per the routing
     table above).  That's where readers go to understand
     the bug.
  2. Add a sub-step row to the active plan's phase doc
     that schedules the fix — `<step #>` + `<one-line
     summary referencing the canonical home>` + `<files to
     touch>` + `<test name>`.  That's where readers go to
     see "is anyone going to actually fix this?"

The sub-step row doesn't duplicate the design — it points
at the canonical home and commits the active plan to
landing the fix.  Without the sub-step row, the
canonical-home entry can languish indefinitely; with it,
the issue has a scheduled landing.

This applies to NEWLY-discovered issues during a phase too:
file in canonical home, then append a sub-step row to the
SAME phase (10.<N+1>) before moving on.  The phase's
sub-step table grows in flight as the work surfaces sibling
issues; that's expected and right.

Items too big to inline as a single sub-step (L effort,
full design pass needed) get a `lib_plans/future/` slot
created AND a tracking row in the active plan's sub-step
list that says "track via [lib_plans/future/<NN>/](path)
— close this sub-step when that plan ships its first
phase."  Design lives elsewhere; schedule lives in the
active plan.

### Why this keeps memory + ROADMAP clean

- **Memory** stays small because every consumer-side
  workaround references its canonical home — there's nothing
  to "remember" beyond the inline pointer.  Your future self
  reads the workaround comment, sees `@P276` or "STDLIB.md §
  Open work", and goes there.

- **ROADMAP** doesn't grow a row per discovered gap.
  Discovered enhancements either get inlined into the active
  plan (option 3) or land in their canonical home (P-issue /
  `## Open work` / lib_plans).  ROADMAP rows POINT at those
  homes when scheduling — they don't duplicate the inventory.

- **Canonical homes already have action infrastructure**:
  `./scripts/idx broken` for tag refs, `make problems` for the
  open-issue list, `./scripts/idx incoming:STDLIB.md` for
  "what links here," the doc-hygiene gate for plan-link
  freshness.  Inventing a parallel catalog re-builds that
  infrastructure for one slice of the open work.

### When NOT to insert the fix (architectural caveat)

Inserting a stdlib / compiler fix into a feature plan's
phase is fine for XS / S work that lifts a workaround the
phase added.  It's NOT fine for:

  - Fixes that change observable language semantics (those
    need a feature plan of their own — design first, then
    the migration).
  - Fixes that touch unrelated subsystems (a fence-skip in
    `lib/markdown` shouldn't grow into a full markdown-
    extension arc).
  - Fixes that demand new tests beyond the consumer's
    smoke test (those want a focused commit + their own
    regression suite).

Use judgment.  The default is "fix it now if it's cheap and
in scope"; the override is "this is bigger than I thought —
defer to canonical home".

---

## Structured Commit Sequence

For each item (or each independent area of a single item) follow the commit order
below.  It is **not required for trivial one-file fixes** — the only hard
requirement is that every commit passes the three checks above.

When a branch contains multiple items **or multiple phases of one item**, repeat
the sequence once per item/phase before writing the shared documentation commit
at the end.  Each phase is treated as an independent item: it has its own test
commit, code commit, and enable-tests commit.

```
[P1.1 — Step 1] tests with #[ignore]
[P1.1 — Step 2] code change
[P1.1 — Step 3] enable tests
[P1.2 — Step 1] tests with #[ignore]
[P1.2 — Step 2] code change
[P1.2 — Step 3] enable tests
[P1.3 — Step 1] tests with #[ignore]
[P1.3 — Step 2] code change
[P1.3 — Step 3] enable tests
[Step 4] any refactors (shared or per-phase)
[Step 5] docs: update PLANNING, PROBLEMS, CHANGELOG for all phases
```

### Step 1 — Tests with `#[ignore]`

Add only the new test file(s) or test functions, with every new test marked
`#[ignore]`.  The `#[ignore]` annotation keeps CI green before the implementation
lands, while making the intent of the tests clear from the first commit.

```rust
#[test]
#[ignore = "P1.1: parser for lambda expressions not yet implemented"]
fn lambda_basic_parse() { ... }
```

Commit message:

```
P1.1: add lambda parser tests (initially ignored)

lambda_basic_parse, lambda_with_return_type, lambda_in_map_call.
All marked #[ignore] until the parser extension lands.
```

Verify: `make run-tests` must pass with the new tests reported as ignored, not failed.

### Step 2 — Code Changes

Stage only the implementation files.  If the feature touches multiple independent
areas of the codebase, split this step into one commit per area.  Common split
boundaries:

| Area | Typical files |
|---|---|
| Standard library | `src/native.rs`, `default/*.loft` |
| Database / runtime state | `src/database/*.rs` |
| Parser | `src/parser/*.rs`, `src/lexer.rs` |
| Bytecode generation | `src/state/codegen.rs`, `src/fill.rs` — see [Bytecode Economy](#bytecode-economy) |
| Scope and variable analysis | `src/scopes.rs`, `src/variables/` |

Example split for P1.2 (two areas):

**Commit 2a** — IR synthesis:
```
P1.2: synthesise anonymous def for lambda in compile.rs

Lambda expressions are lowered to a `Value::Def` with a generated
name. compile.rs emits the def-nr as an integer constant at the
call site. No codegen changes yet.
```

**Commit 2b** — codegen emission:
```
P1.2: emit def-nr for lambda in codegen.rs

codegen.rs recognises `Value::Lambda` and emits `OpPushInt` with the
def-nr, completing the compile-to-bytecode path for inline lambdas.
```

When there is only a single area, one commit is fine.

Verify after each commit: run `make ci` — all three checks must pass.

### Step 3 — Enable Tests

Remove the `#[ignore]` annotations from all tests added in Step 1.  No other
changes.

```
P1.1: enable lambda parser tests

All three tests now pass. Removes the #[ignore] markers added in the
initial test commit.
```

Verify: `make run-tests` must pass with zero ignored tests among the new ones.

### Step 4 — Structural Refactors

If the implementation introduced new code that violates CODE.md line-length or
complexity limits, extract the required helpers or split the functions here.
If no such refactoring is needed, skip this step entirely.

This commit must be **behaviour-neutral**: the test suite must still pass
unchanged after this commit.

```
Refactor: split parse_binary_operator — extract check_constant_zero helper

parse_binary_operator exceeded 55 lines after the L3 constant-zero check.
Extract the new check into its own function per CODE.md § Functions.
```

Verify: `make run-tests` unchanged; `cargo clippy --tests -- -D warnings` clean.

### Step 5 — Documentation

Documentation changes **must be in their own commit**, separate from code,
tests, and refactors.  Never mix doc edits with any of Steps 1–4.

Review **every file in `doc/claude/`** for references to the feature or affected
behaviour and update them as needed.  Common files to check:

| File | Update when |
|---|---|
| `doc/claude/CHANGELOG_TECHNICAL.md` | Always — add a detailed entry under Unreleased |
| `CHANGELOG.md` | When the change is user-visible — add a plain-language entry under Unreleased |
| `PLANNING.md` | Always — remove the item section and Quick Reference row |
| `ROADMAP.md` | Always — remove or update the row(s) for the completed item(s) |
| `RELEASE.md` | Gate criteria or release checklist changed |
| `PROBLEMS.md` | A known bug was fixed or a new one was discovered |
| `STDLIB.md` | A standard-library function was added or changed |
| `PACKAGES.md` | Library resolution or manifest handling changed |
| `INCONSISTENCIES.md` | A documented language inconsistency was resolved |
| Any other `doc/claude/*.md` | File explicitly describes the feature area |

Stage all files that required a change:

```
docs: P1 lambda expressions — update CHANGELOG, PLANNING, LOFT, STDLIB

- CHANGELOG: add P1 feature entry under Unreleased
- PLANNING: remove P1 section (all three phases complete)
- LOFT.md: document lambda syntax in the Declarations section
- STDLIB.md: document map/filter/reduce accepting lambda arguments
```

Verify: `make run-tests` still passes (documentation changes are non-functional).

---

## Bytecode Economy

**Never add a new opcode if the problem can be solved by composing existing
opcodes.**  New opcodes increase the `OPERATORS` array size, the opcode
dispatch surface, and the maintenance burden in `fill.rs`, `codegen.rs`, and
`02_files.loft`.

Before proposing a new opcode, check whether the compiler can emit a sequence
of existing opcodes to achieve the same result.  For example, `insert(v, idx,
elem)` reuses the existing `OpInsertVector` (creates space) followed by the
appropriate `OpSetInt`/`OpSetLong`/`OpSetFloat`/`OpSetSingle` (writes the
value) — no new opcode needed.

Only add a new opcode when:
- No existing opcode sequence can express the operation (e.g. a fundamentally
  new runtime primitive like `OpSortVector` that cannot be decomposed).
- Performance is critical and the overhead of multiple opcodes is measurable
  and unacceptable (document the benchmark).

**When you do add one**, follow the 10-step bootstrap procedure
below.  New opcodes require a bootstrap because `regen_fill_rs`
compiles `loft` to discover declared ops, and `loft` cannot
compile without the generated dispatch entries the regeneration
produces.

1. **Add any new Store/stores methods first.**  The `#rust"…"`
   bodies you'll declare next reference them.  E.g.
   `Store::get_u16_raw` / `set_u16_raw` must exist in
   `src/store.rs` before regen can compile their callers.
2. **Declare the opcodes in `default/01_code.loft`** with
   `fn OpName(...) -> ret;` plus the `#rust"…"` body.  Keep the
   declaration adjacent to the existing `Op*` family it extends
   (e.g. new `OpGetShortRaw` next to `OpGetInt4`) so regen output
   is readable.
3. **Regenerate**: `cargo test --release --test issues
   regen_fill_rs -- --ignored --nocapture`.  Overwrites
   `src/fill.rs` with canonical content derived from every
   `#rust"…"` body in `default/*.loft`.  No manual `fill.rs`
   prep is needed — `OPERATORS` is slice-typed (`&[fn(&mut State)]`,
   no fixed size) and the parse-time op-code assert was removed
   (2026-05-13, Option A); regen handles the array grow + the
   new function body in one pass from each new op's `#rust"…"`
   annotation.
4. **Rebuild dependents**:
   - `cargo build --release --lib` — refreshes the interpreter.
   - `cargo build --release --target wasm32-unknown-unknown --lib
     --no-default-features --features random` — refreshes the
     WASM rlib.  The freshness check in `tests/html_wasm.rs`
     catches this.
   - `(cd tests/lib/native_pkg/native && cargo build --release)`
     — refreshes the fixture cdylib.  Same freshness check in
     `tests/native_loader.rs`.
5. **Audit native codegen** (`src/codegen_runtime.rs`):
   regen_fill_rs does NOT touch this file.  `match parts` arms
   that enumerate every `Parts::*` variant get a non-exhaustive
   warning when a new variant is added — add the new arm
   manually.  For opcodes that add new `stores.method()` calls,
   mirror them in codegen_runtime.rs (look for parallel
   `OpGetInt4` / `OpSetInt4` handling).
6. **Run `native_dir` before committing**: `cargo test --release
   --test native native_dir`.  Pure native-mode test compilation;
   catches the silent-hang class of regression where every unit
   test passes but a native-compiled script hangs.  Do NOT
   commit based on unit-test success alone.

**Ordering constraint**: opcode number is determined by entry
order in `OPERATORS`, which `regen_fill_rs` derives from
declaration order in `default/*.loft`.  Reordering existing
opcode declarations invalidates every pre-compiled package that
embeds the old numbers — **never reorder existing op
declarations while adding new ones**.  Append at the end of the
relevant family.

### Friction history + remaining backlog

**Surfaced 2026-05-13** during @P259's commit-1 work: hit the
255-op limit when adding `OpIncRc`, had to manually patch
`fill.rs` (array size + placeholder + stub fn body) before
regen could run.  Position-sensitive too — the array entry had
to match parse-order position (OpIncRc declared in
`01_code.loft` → goes after `pre_alloc_vector`, NOT at the end
of the array).

| # | Improvement | Status | Notes |
|---|---|---|---|
| **A** | Slice-typed `OPERATORS: &[fn(&mut State)]` (no fixed size) + remove the parse-time op-code assert.  Regen now handles array grow + new fn body in one pass; staleness still caught by `n9_generated_fill_matches_src` and `fill_rs_up_to_date`. | **Shipped 2026-05-13** | Eliminates steps 3-5 of the old 10-step procedure (manual array grow, placeholder identifier, placeholder fn body). |
| **B** | Auto-regen on `fill_rs_up_to_date` failure: have the test attempt regen + re-compare instead of just printing the command, OR convert `regen_fill_rs` from `#[ignore]` to a `build.rs` step that runs on every cargo build | Open (S effort) | Test-side variant is safer than `build.rs` (no build-time codegen risk); `build.rs` is cleanest long-term but more invasive.  After B: editing `default/*.loft` just causes a rebuild that automatically refreshes `fill.rs`. |

---

## GitHub Issues and Releases — Hard Limits

**Issues follow CLAUDE.md § Bug-filing policy.**  A bug you are NOT fixing now is a
GitHub issue with a both-backend repro and its labels; a bug fixed in the same session
gets a regression test and no issue — an issue for it is a second copy of the data, stale
the moment it is created.  Plans are `loft-lang/plans` issues.  Design decisions and
standing state live in `doc/claude/`: an issue is a pick-up, never the home of a decision.

**Never trigger or automate a release.**  Every release requires a manual
validation phase (see [RELEASE.md](RELEASE.md)) that cannot be automated:
hands-on testing of pre-built binaries on each platform, review of the
CHANGELOG, and a deliberate version-bump decision.  Do not push release tags,
trigger release workflows, or draft GitHub Releases programmatically.

**Merging is squash or rebase, never a merge commit.**  The `Use branches` ruleset on
`main` requires linear history, so `gh pr merge --auto --merge` is refused with *"Merge
method merge commits are not allowed on this repository"* — while the classic
branch-protection API still reports `required_linear_history: false`, because the rule
lives in the ruleset (`/repos/<owner>/<repo>/rulesets`), so read both before calling the
refusal mysterious.  Use `--squash`.  A refused `--merge` arms nothing, so it is a safe
probe; a `--squash` on a PR whose required checks are green merges almost at once, so arm it
only when a merge is actually wanted.

---

## CI Validation

CI validation has two distinct phases: a **mandatory local gate** that must pass before
every commit, and the **remote CI** that GitHub runs after a push.  Most failures happen
because the local gate is skipped.

### Pre-existing vs. newly-introduced failures — always irrelevant

**A red CI gate is a red CI gate, regardless of who or what made it red.**
The working tree must be stable and usable after every commit.  It does
not matter whether a `cargo fmt --check` diff, a clippy error, a
`no-default-features` build break, or a test failure was already present
on the base branch before your work, was caused by a toolchain upgrade,
or was introduced by the current change.  If `make ci` is red when you
reach for `git commit`, **fix it first** — then commit.

The reasoning is flat: downstream contributors, CI runs, and future you
cannot distinguish "broken by this commit" from "broken by a prior
commit" from the working-tree symptom alone.  Leaving a red gate in
place forces every later contributor to diagnose it from scratch, and
turns every future `make ci` run into noise that hides genuinely-new
regressions.  A clean gate is a shared resource; "it wasn't me" is not
a reason to leave it dirty.

Practical shape:

- A toolchain upgrade surfaced new clippy lints on existing code → fix
  them (apply the suggestion, restructure the doc comment, or add a
  scoped `#[allow(...)]` with a comment explaining *why* the lint is a
  false positive).  Do not revert the toolchain.
- `cargo fmt --check` reports drift in a file you did not touch → run
  `cargo fmt` and commit the result.  The bundled drift lands alongside
  your intended change; flag it in the commit message so reviewers can
  recognise the stylistic churn.
- A pre-existing dead-code warning fires because of a build-profile
  cfg gate → gate the item with the same cfg, or `#[allow(dead_code)]`
  with a comment pointing at the gated call site.
- A test regresses on your branch *and* on `origin/main` → the fix
  blocks your commit either way.  If the root cause is clearly outside
  your work's scope, land a minimal repair in a separate preparatory
  commit (still on your branch) before the feature commit.

This policy is stricter than "don't introduce regressions."  It is
"leave the gate cleaner than you found it, every time."

### Local CI gate (mandatory before every commit)

Run all four checks and confirm they are clean **before** `git commit`.  Never commit
when any check fails — fix first, then commit.

```bash
make ci   # fmt → clippy → test in order; stops at first failure; output in result.txt
```

Or run the checks individually:

```bash
cargo fmt --check                              # 1. formatting
cargo clippy --tests -- -D warnings            # 2. pedantic lints as errors
cargo check --no-default-features              # 3. feature-gated code compiles
cargo test                                     # 4. all tests pass
```

**All four checks are required.** Skipping any one causes CI failures after push.

These are the same checks the remote CI runs.  Running them locally catches errors that
would otherwise only surface after a push, which cannot be taken back.

#### Common pitfalls

| Pitfall | Why it fails CI | How to avoid |
|---------|----------------|--------------|
| Running `cargo clippy` without `-D warnings` | Project uses `#![warn(clippy::pedantic)]` in `lib.rs` and `main.rs`; CI promotes pedantic warnings to errors | Always use `cargo clippy --tests -- -D warnings` |
| Skipping `--no-default-features` check | CI tests feature-gated builds; `#[cfg(feature = "...")]` on imports and functions must be correct for stripped builds | Always run `cargo check --no-default-features` |
| Running `cargo test` but not `cargo fmt --check` | `cargo test` does not check formatting | Run fmt check first |
| Adding `#[cfg(feature = "X")]` to `FUNCTIONS` table entries | Changing registration order causes `library_names` index mismatch — tests crash with "index out of bounds" | Use `#[cfg]` on array entries to preserve order but conditionally include them |
| New files with crypto/FFI constants | SHA-256 K-tables, base64 lookup tables trigger `unreadable_literal`, `many_single_char_names`, `cast_lossless` pedantic lints | Add `#[allow(clippy::...)]` on the specific function or constant |
| Stale WASM rlib after touching core sources | `cargo test` never rebuilds `target/wasm32-unknown-unknown/release/libloft.rlib`. A `--html` or `html_wasm::*` test will fail with rustc errors citing line numbers from an older source (e.g. `cr_rand_int` at the pre-migration position) or `E0599` on methods that were renamed. | After any change to `src/codegen_runtime.rs`, `src/ops.rs`, or the stack layout: `cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random`. Do NOT use `--features wasm` for the `--html` rlib — that pulls in wasm-bindgen and the resulting bundle imports from `__wbindgen_placeholder__`, breaking Node-stub instantiation. |
| Stale `tests/lib/native_pkg/native` fixture cdylib | The fixture `.so` is not rebuilt automatically by `cargo test`, `make ci`, or the CI workflow. A signature change to the fixture source (e.g. the C54 `*const i32` → `*const i64` swap) is invisible until `native_loader::*` tests mis-read memory and report "expected N, got M" from a pre-rebuild `.so`. | After editing `tests/lib/native_pkg/native/src/lib.rs` or after any Phase-migration change that shifts vector element layout: `cd tests/lib/native_pkg/native && cargo build --release`. |

#### When to run

- Before every `git commit` (including amends)
- Before reporting a branch as done
- After any stash pop or cherry-pick that brings in new code

#### Workflow: push first, test in parallel

To save wall-clock time, push the branch and create the PR **before** running
the local test suite.  CI starts immediately on the remote while the local
tests run in parallel:

```bash
git push -u origin <branch>       # 1. push
gh pr create --title "..." ...     # 2. create PR (CI starts)
cargo test                         # 3. local tests (runs in parallel)
```

This avoids waiting for local tests before discovering remote CI failures.
However, the full local gate (fmt + clippy + no-default-features) must still
pass **before** pushing.

If `cargo clippy --tests -- -D warnings` reports errors for violations that were already present on `main` and in
code you did not write, suppress them with `#[allow(...)]` on the specific function —
see [Validation Against CODE.md](#validation-against-codemd) for the exception policy.

### Remote CI / Pull Request

Pushing commits is OK by default once the local gate is clean (so the remote
stays in sync without the user having to ask each time).  Opening a PR
remains gated by an explicit user instruction.  When the branch already has
an open PR, do NOT push without the user's explicit consent — force-pushes /
rebases / surprise commits disrupt review-in-progress.  The one exception is a
fix for a blocking failure (red CI, a broken build, a failing required check):
push it without asking, because it unblocks the PR rather than disrupting it.

```bash
# OK by default after green local gate (no open PR):
git push

# Opening a PR — only after explicit user ask:
git push -u origin p1-1-p1-2-p1-3-lambda-expressions
gh pr create --title "P1: lambda expressions (all 3 phases)" \
             --body "Implements fn(params)->type block inline lambdas with map/filter/reduce integration."
```

The CI pipeline (`.github/workflows/ci.yml`) runs five jobs:

| Job | Command | Must pass |
|---|---|---|
| Format | `cargo fmt -- --check` | No diff |
| Clippy | `cargo clippy --tests -- -D warnings` | Zero warnings |
| Test (ubuntu) | `cargo check --no-default-features` then `cargo test` | Both pass |
| Test (macOS) | `cargo check --no-default-features` then `cargo test` | Both pass |
| Test (windows) | `cargo check --no-default-features` then `cargo test` | Both pass |

Do not merge until all three jobs are green on all platforms.  If a job fails:

- **Test failure on one platform only** — usually a path-separator or timing
  issue; reproduce with `cargo test` locally in a container or VM.
- **Clippy failure** — a lint that passes locally may become an error under
  `-D warnings` if it was suppressed or not triggered.  The Makefile's `make test`
  uses `-W` (warn only) and will not catch these.  Run
  `cargo clippy --tests -- -D warnings` locally, fix all errors, and push again.
- **Format failure** — run `cargo fmt` locally, verify with `cargo fmt -- --check`,
  amend the relevant commit, and push again.

---

## Renaming a Branch After Completion

When a branch ends up implementing different items than originally planned (e.g.
you started with `l2-nested-patterns` but ended up doing `l2-p3-nested-patterns-aggregates`
instead), rename the branch before pushing the PR so the name reflects the actual
work:

```bash
# Rename the local branch
git branch -m old-name new-name
```

If the branch was already pushed under the old name, the remote must be updated —
but only when the user explicitly instructs a push:

```bash
# Only on explicit user instruction:
git push origin --delete old-name
git push -u origin new-name
```

The branch name appears in the merge commit and PR title.  A misleading name
makes history harder to navigate.  Rename before opening the PR, not after.

---

## Debugging a Regression — MANDATORY APPROACH

### Never use `git bisect` or `git checkout HEAD -- <files>`

**`git bisect` is prohibited.**  It requires running tests against many historical
commits.  Claude cannot do this reliably: context windows are finite, intermediate
compile states are inconsistent, and the process almost always requires reverting
in-progress files — destroying multi-session work that is not yet committed.

**`git checkout HEAD -- <file>` to "reset and try again" is prohibited.**  This silently
discards uncommitted changes on the named files.  When a feature branch has several
files in flight (e.g. codegen, fill, debug, mod, scopes all modified together), resetting
individual files breaks cross-file invariants and produces a state that is harder to
debug than the original problem.

**The correct approach for every regression:**

1. **Write a minimal `.loft` reproducer first** — create a short script in
   `tests/scripts/` that triggers the bug.  Use `fn test_*()` entry points.
   If the test fails, add `// @EXPECT_FAIL: <message>` directly above the
   failing function so CI stays green while you work on the fix.  If it's a
   parse error, use `// @EXPECT_ERROR: <message>` instead.
2. Run the failing test with `LOFT_LOG=minimal cargo test --test <suite> <name>` and
   read `tests/dumps/<name>.txt` — the full IR, bytecode, and execution trace are there.
3. If the trace is too long, use `LOFT_LOG=crash_tail:50` to see the last 50 steps
   before the panic.
4. Read the 3–5 source files that the trace implicates.  Reason about the code path.
   The root cause is almost always visible within one careful read.
5. If you need to know what a recent commit changed, use `git show <sha>` or
   `git diff <sha>^ <sha>` — read the diff, do not re-run old code.
6. Fix forward.  Do not revert; do not bisect.
7. **Remove the `@EXPECT_FAIL` / `@EXPECT_ERROR` annotation** once the fix is
   verified.  The test must pass cleanly — `wrap.rs` will print `FIXED` for
   functions that pass despite having `@EXPECT_FAIL`, confirming the annotation
   can be removed.
8. **File pre-existing bugs surfaced during diagnosis BEFORE moving on.**  See
   the "Bug-filing during a hunt" section below.

---

## Bug-filing During a Hunt — MANDATORY

A bug hunt routinely surfaces *other* bugs that aren't the original report —
sibling shapes, latent issues flagged in comments, symptoms unrelated to the
active fix.  **The default is to FIX them, not file them.**

Why: bugs found during a hunt are the cheapest in the project's lifetime — the
code paths are loaded, the diagnostic infrastructure is warm, a reproducer is
within reach.  That is exactly why they should be *solved* on the spot (focused
fix + regression test), not turned into backlog.  Filing only documents a bug
*for later*, and "later" re-pays to re-derive the scope and repro you already
have.  (Origin — which commit, what history — is never worth recording; scope +
root cause in the *present* code are what you fix from.)

**File a P-issue only when you are NOT fixing it now** — its purpose is to
document the bug so it isn't re-discovered.  Two cases:

- The finding **blocks** the fix you're on → file a bookmark + use a workaround,
  keep moving, come back to it.
- It's **M+ / needs design** → route it to its canonical home
  (§ Inserting Discovered Enhancements above).

Inside an investigation plan, don't file — the plan's probes + cluster docs
already document every shape.  When you DO file a row (you're not fixing it now):

1. **Save the reproducer** to `/tmp/p_followups/p<N>_<slug>.loft` (one
   `.loft` file per finding).  Captures the smallest input that reproduces.
2. **Add a P-issue row** to [PROBLEMS.md](PROBLEMS.md) with: minimal
   reproducer text or `/tmp` path, observed behaviour on each backend (interp
   and `--native`), severity tier (S0/S1/S2), and the workaround.
3. **If user-visible**, mirror the row in
   [USER_FACING.md](USER_FACING.md).
4. **If the bug deserves CI lock-in** (most do not until they're being fixed),
   add a regression to `tests/scripts/` or `tests/issues.rs`.

Do **not** file a row for a bug you just *fixed* — the fix + its regression test
ARE the record.  And filing is **not** a license to scope-creep the active fix:
an unrelated bug Y you can't fix without derailing fix X is the "not fixing it
now" case — file Y (or pick it up next as its own focused change); bundle into
X's patch only when they share a single fix site.

The rule applies even when the bug looks obvious, narrow, or "clearly
unrelated."  Survey method that worked in past sessions:
`grep -E "(workaround|caveat|but the|but is|currently ignored|FIXME|TODO)"`
over the diff and the surrounding files surfaces self-flagged latent bugs;
running variant probes (different LHS, different scope, different element
type) surfaces sibling shapes; comparing the new fix against any symmetric
unfixed path surfaces parallel bugs that didn't get the symmetric fix.

---

## Closed-by-Decision Register

Before proposing a feature, fix, or language change, check
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).  It records
questions that have been evaluated and explicitly declined —
feature proposals (e.g. Rust-style literal suffixes), accepted
limitations (e.g. WASM `par()` sequential), and design choices
(e.g. closure capture by value).  The register exists so the
same questions don't resurface every session.

**Rules**:
- Closed items are **not** backlog.  They don't belong in
  ROADMAP.md's milestones, PLANNING.md's priorities, or
  QUALITY.md's active tables.  A short cross-reference to
  DESIGN_DECISIONS.md in an "Out of scope" section is enough.
- **Re-opening** requires new evidence (a concrete use case,
  incident report, or measurement) that wasn't available at the
  decision.  Put it at the top of the revived entry; don't
  silently flip.
- **Adding** a new entry requires the same rigor: question,
  evaluation, decision with date, and "revisit when" trigger.

When declining a proposal, strike it (`~~…~~`) in its source doc
and append a pointer to its new DESIGN_DECISIONS.md entry.  Keeps
the git history discoverable without cluttering active tables.

---

## See also

- [CODE.md](CODE.md) — Naming conventions, function-length rules, clippy policy, null sentinels
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — Closed-by-decision register (see above)
- [TESTING.md](TESTING.md) — Test framework, `code!` / `expr!` macros, LogConfig debug presets
- [PLANNING.md](PLANNING.md) — Backlog, version milestones, effort estimates
- [PROBLEMS.md](PROBLEMS.md) — Open bugs; update here when fixing a known issue
- [RELEASE.md](RELEASE.md) — Gate criteria and release checklist
