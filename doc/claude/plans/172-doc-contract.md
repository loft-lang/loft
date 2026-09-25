<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 172 — Documentation workflow: one contract, enforced by tools, reviewed on the existing beat

## Status

**Active — Phase 1 in progress.** Tracks [@PLN172](https://github.com/loft-lang/plans/issues/172).

Two things built elsewhere feed into this plan:

- **The `link` check already exists.** `tools/indexer/fix_broken_links.py` reads every tracked
  markdown file the way a renderer does (fences and code spans are not links). It repairs a
  link only when exactly one tracked path matches, and flags the rest.
  `tests/index_hygiene.rs::every_markdown_link_resolves` gates it (@PLN45). `doc_lint.py`
  calls that, not `check_doc_drift.sh`'s narrower plan-link check.
- **The contract shape is proven.** [LIBRARY_AUTHORING.md § The library contract](../LIBRARY_AUTHORING.md)
  states 28 library rules, one line each, each pointing at its home. It was written from a
  sweep of every place a rule was stated, and that sweep found about twenty contradictions,
  several of which hid real defects. DOC_CONTRACT.md is written the same way: sweep first,
  then the contract, then fix what disagrees.

---

It changes *where* the documentation rules live and *who* applies them. It does not add a
cadence: the `[mid]` and `[pre]` phases of the release checklist (RELEASE.md § cadence
markers) are the only two beats this work uses.

## The problem in one paragraph

The rules for good documentation exist and are mostly right, but they are enforced by
**recall**: an agent must remember to load the `doc-quality` skill, and often does not.
Loading it into the building agent is the wrong fix — it pays context on every task for a
job that belongs to a reviewer. Meanwhile the rules are split by surface (code comments in
DOC_QUALITY.md, user docs in USER_DOCS.md + API_SURFACE.md, libraries in
LIBRARY_DOC_REVIEW.md), and the third surface — `doc/claude/` itself — has no rules for
size, findability or history, which is exactly where CLAUDE.md (1,552 lines),
DEVELOPMENT.md (1,543) and RELEASE.md (1,182, with dated rulings in its body) show the
cost.

## The three moves

1. **One contract.** A short doc that states every standing documentation rule once,
   for all three surfaces, one line each, with a pointer to the doc that owns the detail.
2. **Rules a script can check leave the agent's head.** They run as a
   Claude Code hook on the file just edited, as the diff-scoped PR check, and as the
   `[mid]` report. One implementation, three trigger points.
3. **Rules that need judgment go to a reviewer, on the beat.** A fresh agent whose
   context is the contract + skill + the report's worklist, at `[mid]` and `[pre]`.
   The builder is never that reviewer.

Nothing in this plan puts new text into the builder's context. Its only exposure to
documentation quality is hook output, which is empty when the file is clean.

## What already exists (build on it, do not rewrite it)

| Piece | What it does today | Role in this plan |
|---|---|---|
| `scripts/lint_comments.sh` | Flags history stamps, change-narration, incident-subject comments in `src/` | The code-surface checks of the lint |
| `scripts/check_doc_drift.sh` | Broken plan links, time-projection language, stale "is current" claims in `doc/claude/` | The `doc/claude/` staleness checks |
| `tools/indexer/fix_broken_links.py` | Every broken relative link in tracked markdown: repaired when unique, flagged otherwise; gated by `every_markdown_link_resolves` | The `link` check |
| `scripts/doc_history_report.py` | How much of a contract doc is its own history; the `<doc>.md` / `<doc>-history.md` convention | The history check, extended from contract docs to all of `doc/claude/` |
| `scripts/doc_review.py`, `api_lint.py` | RELEASE.md § 0: per-section staleness of user-visual docs, stdlib API doc coverage with a ratchet baseline | The user-surface checks; the baseline/ratchet pattern to copy |
| LIBRARY_DOC_REVIEW.md, SKILLS_REVIEW.md | Watermark reviews on three axes — content, usability, conciseness | The by-hand reviewer pass; unchanged |
| `doc-quality` skill | The rules condensed, with the stamp-vs-pointer test | Shrinks; loaded only by the reviewer |
| `make release-checklist ARGS="--phase mid"` | Lists what is completable at the halfway point | Where the two new rows land |

The gap is not tooling. It is that these pieces have no common rule set, no shared
trigger, and no coverage of `doc/claude/` size and findability.

## Phase 1 — the contract (one PR)

Create `doc/claude/DOC_CONTRACT.md`. Target: under 80 lines. Format: one rule per line,
grouped by surface, each ending in the owner doc.

Rules it must carry (the current homes in brackets; the ones marked *new* have no home yet):

**Every surface**
- A doc or comment describes the thing as it is now. Change history goes to the commit,
  CHANGELOG_TECHNICAL.md, or `<doc>-history.md` — never the same file. [DOC_QUALITY,
  `doc_history_report.py`]
- Every fact has one home; other places point, they do not copy. [USER_DOCS, STABILITY_METHOD]
- Plain language: common words, short sentences, no idioms, a term explained on first use.
  [DOC_QUALITY § Write for every reader]
- Concise means shorter, not simpler. [DOC_QUALITY]

**Code comments** (`src/`, `lib/**/src/`)
- Present tense, why-to-use, what a caller may rely on; cite a `@FR-` rule where one exists.
  [DOC_QUALITY § The rules, CODE.md § Doc Comments]
- No plan tags, dates or phase references in comments. [`lint_comments.sh`]

**User-facing docs** (guides, API reference, comparison pages, library READMEs)
- Three tiers: what is there / how do I start / what is the signature. [USER_DOCS]
- Every example runs on both backends and is a test. [RELEASE.md § 0b]
- No temporal or hedge words in prose. [RELEASE.md § 0c]
- A library's guide lives in the library; this repo points. [USER_DOCS one-home rule]

**Maintainer docs** (`doc/claude/`) — *new*
- A doc answers one question. If a reader needs two headings to say what it is for, it is two docs.
- A size ceiling per file: **1000 lines, a hard ceiling** (owner, 2026-09-25). Under it,
  the file's internal structure carries the weight — headings a reader can navigate, one
  topic per section — so a long doc that is well sectioned is fine and a short one that is
  not still fails review. Formal rule docs and generated reports are exempt and say so in
  their header.
- Every doc is reachable from the CLAUDE.md index in at most two hops, and each index
  entry names the *start here* doc for its topic, not a list of peers.
- A dated ruling ("the owner ruled on …", "retired …") is a CHANGELOG_TECHNICAL entry,
  and the doc is edited to state the outcome as the current rule.
- Maintainer docs are allowed denser language, but not longer sentences. The plain-language
  bar is not a gate here; the size and history rules are.

Then:
- CLAUDE.md gets one line at the top of the docs index: *Writing or changing any doc or
  comment: DOC_CONTRACT.md is the rule set; the hook checks it.* The paragraphs that restate
  rules elsewhere in CLAUDE.md are cut to a pointer (this alone removes some tens of lines).
- The `doc-quality` skill is reduced to: load the contract, apply the stamp-vs-pointer test,
  read the worklist. Its description says it is for the *reviewer* pass.

## Phase 2 — the lint, as one script with three callers (one PR, a few days)

`scripts/doc_lint.py <path>...` — a thin driver over the existing scripts plus the new
`doc/claude/` checks. Output: one line per finding, `file:line: rule-id: message`, nothing
when clean. Exit code is a count, never a failure, unless `--gate` is passed.

Checks, by rule id, matching the contract:

| id | check | source |
|---|---|---|
| `history` | dated sentence, "retired / ruled / previously / used to / was" in body prose | `doc_history_report.py` extended, `lint_comments.sh` patterns |
| `stamp` | plan tag, phase, date in a code comment | `lint_comments.sh` |
| `size` | file over the ceiling and not exempt | new |
| `orphan` | `doc/claude/*.md` not reachable from CLAUDE.md within two hops | new; reuse the link walk from `fix_broken_links.py` |
| `two-h1` | more than one H1 in a doc | new |
| `hedge` | temporal/hedge words in user-facing prose | RELEASE.md § 0c |
| `link` | broken relative link | `fix_broken_links.py` |

**Caller 1 — the edit hook.** A `PostToolUse` hook on Edit/Write/MultiEdit, matching
`*.md` and `*.rs`, runs `doc_lint.py` on that one file and returns its output. The
builder sees a finding only when it just wrote one, at the moment it is cheapest to fix.
Registered in `.claude/settings.json` so it travels with the repo. Runtime must stay well
under a second; the walk for `orphan` is cached.

**Caller 2 — the PR check.** `doc_lint.py --gate --changed` over files in the diff:
a PR may not add a `history`, `stamp` or `two-h1` finding, and may not push a file across
the `size` ceiling. Existing findings in untouched lines are not the PR's problem. This is
the one place a documentation rule blocks, and it blocks only growth, so the standing rule
"a doc finding never holds a bug fix" (RELEASE.md § Pre-Release Documentation Review) stays
true for the corpus.

**Caller 3 — the report.** `make docs-lint` (all of `doc/claude/` + `src/` comments +
user docs) with a committed baseline `doc/claude/releases/docs-lint.baseline`, same shape
as `api_lint.py -c`. The report prints the count, the delta since the baseline, and the
worklist sorted by rule then file.

## Phase 3 — two rows on the existing beat (one PR)

Add to the release checklist:

| marker | row | evidence |
|---|---|---|
| `[mid]` | **D-lint** — `make docs-lint`: the count did not grow since the last baseline; baseline re-committed | the report in the cycle's `releases/` directory |
| `[mid]` | **D-review** — the reviewer pass: one `doc/claude/` doc brought fully under the contract this cycle (split, history moved out, index entry fixed), chosen from the top of the `size` + `history` worklist | the PR link |
| `[pre]` | **D-user** — the existing § 0 user-visual review, unchanged, now run with the reduced skill loaded | as today |

D-review is deliberately *one doc per cycle*, the same beat as BUG_REVIEW's one
generalization per month. The worklist orders the queue; the owner may override the pick.
The first three picks are already visible from the numbers: CLAUDE.md, DEVELOPMENT.md,
RELEASE.md.

The reviewer pass is a **separate agent session** started from a fixed prompt kept in
`doc/claude/DOC_CONTRACT.md § Reviewer pass`: load the contract and the skill, run
`make docs-lint`, take the top doc, do the split/move, open the PR. It never touches code
and never runs in the same session as feature or bug work.

## Phase 4 — burn-down (ongoing, no PR of its own)

Each `[mid]` retires one doc from the worklist. When `size` and `history` reach zero for
`doc/claude/`, the D-review row shrinks to the watermark pass LIBRARY_DOC_REVIEW.md already
describes, and the PR gate can widen from "no new finding" to "no finding in touched
files". Do not widen it earlier: a gate that fails on inherited debt is one people learn to
bypass.

## What this plan does not do

- It does not make plain language a gate anywhere. That stays a reviewer judgment.
- It does not reorder or renumber RELEASE.md § 0 — only adds rows and the skill note.
- It does not split any large doc up front. The lint decides the order; D-review does one
  per cycle.
- It does not move the deferred steps 5–7 (external-user validation); they wait for the
  signal RELEASE.md names.

## Open decisions for the owner

- ~~The `size` ceiling~~ — **decided 2026-09-25: 1000 lines, hard**, with structure judged
  by the reviewer below it.
- Whether `doc/claude/formal/` and `plans/` are exempt from `orphan` (proposed: `formal/`
  exempt via its README, `plans/` reachable via `plans/README.md`, so no exemption needed).
- Whether the PR gate lands in Phase 2 or waits one cycle behind the report, so the first
  baseline exists before anything can block.

## Done when

- DOC_CONTRACT.md exists, under 80 lines, and CLAUDE.md points to it in one line.
- The hook fires on a doc edit and is silent on a clean file.
- `make docs-lint` has a committed baseline and the `[mid]` view shows D-lint and D-review.
- The `doc-quality` skill is loaded by the reviewer session and by nothing else.
- Three cycles later, three docs are under the contract and the baseline count has fallen
  three times.
