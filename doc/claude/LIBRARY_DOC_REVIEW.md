<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library documentation review — monthly by-hand protocol

> Origin: [@PLN141](lib_plans/141-library-worked-examples/README.md) (worked
> examples). Run once per monthly release cycle, alongside the
> [RELEASE.md](RELEASE.md) checklist. This is a **hygiene ratchet, not a gate** —
> it never blocks a release; the automated `check_doc_drift.sh examples` gate
> does that.

## Why a by-hand pass exists

> ⚠ **The gate BLOCKS inside loft and ADVISES in a library repo**, and the asymmetry is
> deliberate. `examples` and `examples-index` are the only two checks that span
> repositories: a library's CI checks loft out as `loft-src` and runs *loft's* script
> against the library, so the rules arrive from whatever loft `main` happens to be. That
> bit in both directions before it was tiered — `exindex` landed in loft on 2026-08-18 and
> reddened loft-libs-game's next PR for a file it never touched (last green run
> 2026-08-17), and switching a *library checkout's branch* turned loft's own run red with
> two dangling tags, a failure with no bad commit in either repo. A gate whose rules change
> under you, from a repo you do not control, lands its red on whoever opens the next PR.
>
> It follows this repo's own diagnostic rule: **a diagnostic gates if and only if ignoring
> it can produce a wrong result** ([CLAUDE.md](../../CLAUDE.md) § Two diagnostic tiers). A
> dangling doc citation is a broken link; it cannot. Inside loft — which owns the generator
> *and* the feature-doc citations, with no cross-repo coupling — it still gates, and the
> scanner's own selftests gate everywhere, because a scanner that stops following its
> documented rules is loft's bug whoever runs it.
>
> **Advisory does not mean quiet.** The findings go to the library PR's job summary in
> full — the same place a failing test writes its excerpt, so it is as visible as a red
> tick without being able to block a merge.
>
> **Advisory costs you a local pass/fail, so one command gives it back:** `make
> examples-preflight REPO=<library>` gates the citation faults CI reports and exits
> non-zero on them, without demanding the (no longer committed) index.
> `EXAMPLES_GATE=hard` restores full blocking for a repo that wants it.
>
> ⚠⚠ **`examples-index.tsv` is not committed in a library repo at all — CI builds it.**
> The index is DERIVED and its generator is in loft, so a committed copy there can only
> rot: it cannot be regenerated where it sits, and "regenerate it" names a command that
> repo does not have. CI emits it per run (`check_doc_drift.sh emit-examples-index`),
> folds it into the job summary and uploads it as an artifact. **A derived file that is
> never committed cannot be stale** — that retires the failure mode rather than
> downgrading it. loft keeps its own committed copy: loft owns the generator, and a
> greppable offline index is what the agent development model runs on.


The automated gate (`scripts/check_doc_drift.sh examples`) catches the two failures a
machine can see: a worked-example tag that **dangles**
(cited, but no test carries it) or **duplicates** (one tag on two functions). It
cannot see the two failures that actually rot a library's docs over time:

- **Staleness** — a `///` doc comment that still resolves and still reads
  cleanly, but no longer describes what the function *does now* (a parameter
  changed meaning, the return contract shifted, a behaviour note went out of
  date). The prose is internally consistent, so nothing flags it; only a human
  reading it against the current body notices.
- **Example quality** — a cited example that is valid and runs, but is no longer
  the *clearest* demonstration: a better real call site has since appeared in a
  consumer, or the tagged test drifted to exercise an edge case rather than the
  common path.

Both need judgment. This protocol is the monthly pass that supplies it, kept
cheap by a **watermark + changed-since worklist** so each month reviews what
actually moved, not all ~350 public functions.

## Cadence and scope

- **When:** once per monthly cycle (the `YYYY-MM` branch), before tagging the
  release. Libraries are not release-coupled for *publishing* (RELEASE.md § What
  forces a release), but their docs share the monthly beat for *review*.
- **Who:** one reviewer per pass — a human, or an agent steered through the steps
  below. Splitting libraries across passes is fine; the watermark carries state.
- **What:** the whole distribution — the loft stdlib (`default/`), the in-tree
  libraries (`lib/*`), and every package in the registry. `make libraries-review`
  names the population and says which part of it this pass owes; you never pick
  the list by hand.
- **The other half of the pass:** the feature catalogue (`@F`/`@I`) rides the same
  monthly beat through `make features-review`, with `make features-check` as its
  pre-flight. Same two questions, same non-gate status — the halves differ only in
  what they review, so run both and treat the union as one worklist.

## The pass — per library

### 0. Pre-flight (automated — must be green first)

```bash
scripts/check_doc_drift.sh examples     # no dangling / duplicate citations
make features-check                     # feature-catalogue shadow in sync (if touched)
make libcatalogue                       # the catalogue builds without breakage
```

`make libcatalogue` is a hard precondition for step 1, not just hygiene: the library
aid reads the snapshots it writes, and those are a **local build by design** (@PLN112 —
a committed copy silently lagged `origin/main`). Skip it and the "what moved" answer is
last month's, which is worse than no answer.

Green here is **necessary, not sufficient** — it means no cheap failure remains,
so the manual budget goes entirely to staleness and quality.

### 1. Generate the worklist

Two steps, coarse then fine. First, which libraries does this pass owe anything?

```bash
make libraries-review
```

It answers the only two questions a program can: what is **structurally missing** (a
library with no watermark row, so it has never been reviewed; one whose source cites no
worked example and carries no `examples-exempt.tsv` verdict; a watermark row naming a
library that no longer exists), and which reviewed libraries have **moved** since the
commit their watermark records — with the commit count and how many `pub fn` lines
changed, so "three commits, zero signatures" can be read and dismissed in seconds. It
is a report: it never fails, and it never judges whether a doc is *good*.

Then, for each library the aid put on the list, the per-function worklist:

```bash
scripts/doc-review.sh --since <its-watermark-commit> <library-tree>
```

It prints three things: a **coverage** count (a health signal, not a target —
most functions are self-evident and correctly carry no example), the **citation
inventory** (every `// Example:` site), and the high-signal part — every public
function whose **signature changed** since the last review. A changed signature
is the number-one source of a stale doc.

### 2. Re-read the changed docs (staleness)

For each function on the changed-since list, read its `///` doc against its
current body: do the description, each parameter's meaning, the return contract,
and any inline example still match what the code does **now**? Doc edits are XS —
**fix drift on the spot**.

### 3. Fill the highest-value coverage gap (examples)

From the *uncited* public functions, pick the ones a reader "knows exists but
cannot use from the signature alone" — the ratchet's rule (@PLN141 § Scope
discipline).  **This step is where @PLN141's tail lives now that the plan is
closed:** a package that lands after a rollout owes a verdict like any other, and
step 1's "owe a worked-example verdict" list is what surfaces it.  ⚠ That list is
built from a local snapshot, so a stale one omits exactly the newest packages —
the aid states its age and warns past two days; refresh with `make libcatalogue`
before trusting the worklist. Author a worked example for **one or two per pass** (a tagged test,
or a citation to a real consumer call site). The ratchet only goes up; there is
no sweep, and a function whose use is self-evident is left alone.

### 3a. Write a guide for a library that owes one

**This step is where @PLN149's tail lives now that the plan is closed.**  The four
documentation tiers, the guide contract and the rendering all shipped; what remains is
CONTENT, one library at a time — and it never finishes, because a package published next
month owes a guide too.  A plan that cannot reach a done state is a standing practice
wearing a plan's clothes, so it lives here instead.

Step 1's aid reports the queue rather than a document listing it:

```
  -- guides (Tier 1: docs/*.loft, run by the library's own CI on both backends) --
    16 have one · 25 owe one · 0 not measured
```

⚠ **Three states, and the third is the one that matters.**  The `guide` flag is recorded
by `scripts/refresh-unreleased.py`, whose cache reuses any entry whose sha has not moved —
so an entry written before the field existed carries no flag.  Counted as *no guide* it
invents a worklist; counted as *has one* it hides real gaps.  Neither is a measurement, so
"not measured" is its own count and names the refresh.  The flag backfills on the next
ordinary run (it is one directory listing, independent of the API extractor), so the third
state should be 0 in practice; a non-zero one means the snapshot has not been refreshed.

Write **one per pass**, and pick by who is reached: a library other packages depend on is
one more readers arrive at, and the aid's dependants column ranks that.  The contract is
[LIBRARY_AUTHORING.md § 2c](LIBRARY_AUTHORING.md) — five parts, `main` calls every section,
green on both backends with identical output.  Two rules that are easy to skip and are what
make a guide worth trusting:

- **Measure every asserted number before writing it.**  A guide is a running program, so a
  recalled value is a red run at best and a confidently wrong page at worst.
- **Falsify it.**  Invert one load-bearing assertion, confirm the file goes red, restore.
  A section whose asserts cannot fail reads as verified and is not — which is exactly the
  defect that made `main` calling every section a rule.

### 4. Spot-check example quality (freshness)

For a **rotating handful** of already-cited functions (the inventory from step
1), open the cited test: is it still the *clearest* demonstration of the common
path? Has a new consumer usage become a better example? Re-point or improve if
so — a worked example is only worth its tag while it teaches.

### 5. Record and route findings

Fix XS/S (doc edits, a clearer example) in the same pass. File M+ — a function
whose doc drift reveals an actual behaviour bug, or a gap that needs design — as
a GitHub issue per the [bug-filing policy](../../CLAUDE.md) (`Fixes #N`), never
inline. Don't scope-creep the review into unrelated fixes.

### 6. Bump the watermark

Update the library's row in the table below — `reviewed through` to this cycle, `at
commit` to the ref this pass ended on (`git rev-parse --short HEAD` in that library's
repo). A library reviewed for the first time gets a **new row**; that is what moves it
out of the aid's "never reviewed" list.

Leaving `at commit` empty is not free: next month the aid can only say *reviewed, but
no commit recorded — nothing to diff against*, and that library falls back to a full
re-read. The watermark is the entire mechanism that keeps a quiet month cheap.

## Watermark table

"Reviewed through" = the last monthly pass that read this library's docs; "at
commit" = the ref that pass ended on — the baseline `make libraries-review` diffs
against to say what has moved since.

**A row means a review happened.** The libraries that have *not* been reviewed are
not listed here — `make libraries-review` derives that backlog from the actual
population (the in-tree trees plus every package in the registry snapshot), so a
placeholder row would be a second list of the same fact, drifting the moment a
library is published or renamed. Six such rows had already gone stale by 2026-08:
`lib/html` and `lib/markdown` had moved out to the `loft-libs-docs` repo, two more
were spelled differently from the tree they named, and two were prose standing in
for a list — a four-library cell, and a catch-all "registered libs (…)" row that
named six of thirty-four.

The **key column is machine-read** — one library per row, spelled exactly as the aid
keys it: the path for an in-tree tree (`default`, `lib/git`, `lib/lexer.loft`), the
package name for a published one (`graphics`). A key that matches nothing is reported
as a STALE ROW rather than ignored.

| library | reviewed through | at commit | notes |
|---|---|---|---|
| `default` | 2026-09 | `32700867` | @STD-001..012 authored across text / collections / JSON / files-IO; docs read while tagging. 2026-09: the only change since is two compiler-internal ops (`OpDistinctStore`, `OpRefAlias`), non-`pub` and undocumented by design — the reference chapter's promises are unchanged. 2026-09-04 re-read: loft#1348 removed every tracker tag from the published descriptions and rewrote the `min`/`max` section, `sum_of` and `File.format` for their readers; `Walkable`, `tree_walk`'s cap edge and the three interfaces documented; the `assert_eq` and `03_text` maintainer notes moved off the page. Each authored behaviour sentence was probed on both backends and holds |
| `lib/git` | 2026-09 | `2966e9b5` | @GIT-001..005 tagged to live uses in `scan.loft` + `refresh.loft`; 13 pub fns read while tagging. 2026-09: a query that cannot be ASKED now halts instead of answering `""` (loft#1061) and the doc above `git_query` and `branch` moved with the code |
| `lib/lexer.loft` | 2026-09 | `32700867` | @LEX-001 (matches/test/identifier), @LEX-002 (anchor/revert backtracking) — both tagged to live uses in `parser.loft` (`function`, `object`), exercised by the `16-parser` doc test; format-protocol/comment fns still owe examples (need a non-rendered demo). 2026-09: `Anchor.start` (a revert can land at a token's START), `split_token` (maximal munch undone for a nested `>>`) and `offset` (a stalled-loop guard) — each documented with its reason and with live callers in `lib/parser.loft`. 2026-09-04: `Lexer` itself documented (the token/keyword tables must be set before the first parse); no `pub fn` moved |
| `lib/parser.loft` | 2026-09 | `69af7b6a` | First review. One `pub fn` (`parse`), doc read against the body and found current; @PAR-001 tags the doc test `tests/docs/16-parser.loft`, which is the clearest call site there is. Its prelude load names `default/01_code.lav`, an extension this repo has never had — filed as loft#1339, not fixed here because dropping it renumbers `cur_file`. 2026-09-04 after loft#1339: the phantom `.lav` prelude load is gone, with the comment saying why a real one would regress `parse`'s count; no `pub fn` moved |
| `lib/code.loft` | 2026-09 | `32700867` | First review. 24 `pub fn`, no doc comment on any of them and no module header; header written naming what `Code` is, what `cur_arg` switches, and which half is reached. `deferred` in `examples-exempt.tsv`: the emitter half has no call site to cite. Two defects it hides — `null_value` emits `Boolean`, `blocks` is popped but never pushed — filed as loft#1340. 2026-09-04 re-read after loft#1340: `add_block`/`add_loop`/`add_if` record the header they open, `end_if` spans are consistent, `null_value` emits `Null`; `Code` and `Structure` documented. Still `deferred` — the emitter has no caller to cite |
| `lib/testlib.loft` | 2026-09 | `32700867` | First review. `exempt` in `examples-exempt.tsv` — a fixture for `tests/docs/17-libraries.loft` and `tests/diagnostic_reach.rs`, deliberately trivial, so a call site teaches nothing its signature does not. Docs read; nothing stale. 2026-09-04: `Point` and `Bag` documented as the fixtures they are; no `pub fn` moved |
| `lib/audience_crystal` | 2026-08 | `7786d28c` | @ACR-001..003 tagged to the `01-editor-helpers` test (picking inverse, incr editor loop, erase) |
| `lib/engine_host` | 2026-08 | `7786d28c` | @EHK-001..004 tagged to CI-spawned audience-demo kernels (run loop, broadcast, sync lanes, run_client drain); 37 pub fns read while tagging |

`7786d28c` is the commit that authored every in-tree worked-example tag (squash-merged
PR #971, 2026-08-18) — the 2026-08 pass *was* that tagging, so it is the pass's real
end ref rather than the `(bootstrap)` placeholder these rows carried first.

### Type descriptions — 2026-09 (loft#1342)

The 2026-09 pass found the published distribution documents its functions and not
its types: 34 `pub struct`/`pub enum` declarations carried no description, 33 of them
named in other public signatures of their own library (`Stage` in 111 of `stage`'s).
Every one now has a one-line description directly above its declaration, on the
`doc-types-2026-09` branch of each library repo that had one — `loft-libs-graphics`,
`-world`, `-net`, `-core`, `-assets` and `-docs` (`game` and `plugins` had none);
parse-checked as 19 packages, 0 errors. The count is taken over the REGISTRY's
published surface, so the `34 type` figure `make libraries-review` prints moves only
once those branches merge and the libraries republish: the branches are the fix, the
release is the owner's step.

## What this is NOT

- **Not a gate.** It never blocks a release — the `examples` gate blocks on
  dangling/duplicate; this is a report, like `make speed`.
- **Not a full re-sweep.** The watermark + changed-since worklist bound each
  pass to what moved. A month with no library changes is a five-minute pass.
- **Not a coverage mandate.** A low citation count is healthy when the uncited
  functions are self-evident. The target is "every *non-obvious* function has a
  *current, clear* example", never "every function has one".

## See also

- [@PLN141](lib_plans/141-library-worked-examples/README.md) — the worked-example
  mechanism (tag family, `check_doc_drift.sh examples`, `idx` ingestion).
- [DOC_QUALITY.md](DOC_QUALITY.md) — how the docs themselves should read.
- [RELEASE.md](RELEASE.md) — the monthly cadence this pass rides.
- `scripts/doc-review.sh` — the per-function worklist generator invoked in step 1.
- `make libraries-review` — the per-library worklist that picks what step 1 drills into
  (`scripts/check_doc_drift.sh libraries-progress`); `make features-review` is its
  feature-catalogue twin.
