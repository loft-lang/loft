<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 172 — Documentation workflow: one contract, enforced by tools, reviewed on the existing beat

## Status

**Active — Phases 1–3 done; Phase 4 (the burn-down, one doc per `[mid]`) is the standing work.**

**Phase 4, the 2026-10 cycle's D-review — DEVELOPMENT.md (2026-09-28):**
- **Size:** 1565 → 452 lines; lint findings 24 → 1 (a literal `FIXED` in quoted runner output).
- **Split by question:**
  - [JOINING.md](../JOINING.md) — rebasing onto a squash, joining sibling checkouts, resolving
    and verifying a join; it now also holds the commit-classification ladder, which had lived
    only in agent memory.
  - [REVALIDATE_LIBS.md](../REVALIDATE_LIBS.md) — does a change break a shipped library.
  - [INTERMEDIATE.md § Adding an operator](../INTERMEDIATE.md#adding-an-operator) — the
    opcode bootstrap, with the `make surface-gen` step it lacked.
- **History out:** the dated incidents went to
  [DEVELOPMENT-history.md](../DEVELOPMENT-history.md).  Among them is the unbuilt auto-regen
  idea for `fill_rs_up_to_date` (improvement B).
- **Contradictions with CLAUDE.md resolved to CLAUDE.md:**
  - "always branch from `main`" (rule 5 says: from the tip of in-flight work);
  - open a PR before the local test run;
  - bugs as PROBLEMS.md rows;
  - sprint branches and `{id}-{name}` branch names;
  - merging `origin/main` into a branch that is behind.
- **Stale facts corrected against the tree:**
  - the five-job CI table (`main` requires Test ×3, Clippy and Format; the rest is CI_BUDGET.md);
  - the manual gate list (`make ci` now builds the wasm and no-default-features variants and the
    fixture cdylibs);
  - the commit-message style (the log states outcomes as present-tense sentences, not
    `{scope}: {imperative}`);
  - the Co-Authored-By model;
  - the routing table (COMPILER.md has no § Open work; `lib_plans/future` became plan issues).
- **Baseline:** only the touched files' rows were re-pinned, so other branches' timeline growth
  in `formal/` stays visible as new.

**Phase 4 — TESTING.md split by question (2026-09-28).**
- **4602 lines became six working docs, each under the ceiling, plus the record:**
  - TESTING.md (705): where a test goes and how to write it — the harness, the suites, the gates;
  - RUNNING_TESTS.md (662): running the suite and reading a failure, LogConfig included;
  - GUARDS.md (586): does a guard catch its defect — `make falsify`, receipts, and the
    ways a guard reads green, as rules grouped by kind (the 73 incidents are in the record);
  - RUN_BOUNDS.md (260), TEST_ENVIRONMENTS.md (417), LOFT_TEST.md (252);
  - the diagnostic tiers moved to DIAGNOSTICS.md, their subject's home;
  - TESTING-history.md holds every moved incident verbatim.
- **Stale facts corrected against the tree**, among them: six `tests/*.rs` entry points and the
  `generated/` workspace that no longer exist; `wrap.rs` tolerating unclaimed warnings (it fails
  them); a `pkill -f` recipe the shell rules forbid; `make test` and `make loft-test` doing
  other than stated; the LOFT_LOG preset table missing six presets; two failure-kind causes
  ("Undefined type" is not definition order; a range query is one interval, @FR-Slice-KeyedIter).
- **Pointers:** every citation of a moved section names its new doc — 112 `tests/scripts`
  headers, CLAUDE.md, skills, scripts and comments.

**Phase 4 — DESIGN_DECISIONS.md, register and record (2026-09-28).**
- **4381 → 966 lines.**  Every entry keeps its heading, so every anchor and every
  "DESIGN_DECISIONS.md C86" citation still resolves.  Each entry now states the decision, its
  reason and what would reopen it; the question, evaluation and amendments moved verbatim to
  DESIGN_DECISIONS-history.md under the same heading.
- **Folded in:** amendments that changed a decision (C68 reversed, C77 superseded by C86, C110
  by C126), so the register states what holds now.
- **Defects fixed:** two entries shared C67; the processor-arithmetic one is now C129 and its five
  citations moved.  C91 carried a placeholder link to C101 from before C101 existed.
- **Open, found here:** C98 is not what ships — a bare `use lib;` wildcard-imports; the entry
  records the owner call as open.  C124's warning → error step has no tracker.

**Plan docs are out of the burn-down (owner, 2026-09-28).**  A plan's docs are kept under the
contract by that plan's own agent, so `make docs-lint`'s report, baseline and worklist and
`make file-sizes` leave `plans/` and `lib_plans/` out; the edit hook and the PR gate still
apply.  DOC_QUALITY § Maintainer docs rule 14 and DOC_CONTRACT rule 39 carry it.  This also
settles the open `orphan` question for `plans/`.

**Phase 4 — PERFORMANCE.md split by the same rule (2026-09-28).**
- **5078 lines became three docs:**
  - PERFORMANCE.md (769 lines, working): how to measure, where the numbers are, open work;
  - STARTUP_CACHE.md (250 lines): its own question;
  - PERFORMANCE-history.md (the record, size-exempt): the benchmark tables and comparisons,
    every design, the analyses, and the drawing-pass baseline.
- **The two status tables disagreed with each other and with the tree.**  N4 was listed open
  but shipped; BUILD1 and P7's `reserve` were listed open but delivered.  The working doc's
  single table was re-derived from the code, with the evidence per row.
- **References follow their section.**  The ones that mean open work name the working doc;
  a stale "P1 blocked" in ROADMAP.md was corrected.

**Phase 4 — QUALITY.md split into working doc and record (2026-09-28, owner-steered).**
- **The owner's rule, now DOC_QUALITY § Maintainer docs rule 4:** everything normal work needs
  lives in compact files, and history is split off one way — the `<doc>-history.md` companion.
  Rule 2 now exempts such a companion from the size ceiling (`<!-- size-exempt: … -->`).
- **QUALITY.md went from 12,892 lines to 169, with 0 history lines.**  It holds the open
  non-defect decisions, the three gated ratchets, what remains of the duplication thread, the
  instruments, and the store-lifetime residuals.
- **QUALITY-history.md is the old file moved intact,** so every anchor survives and ~90
  citations repoint mechanically.  The ratchet tables left behind pointers.
- **Tools and tests follow the content:**
  - `campaign_review.py` reads its walks from the companion;
  - the two Tier guards read the companion;
  - the open-table and ratchet gates stay on QUALITY.md.
- **Two open items that existed only as prose were measured:**
  - The vector/tuple `null`-first-arm chain still answers `null` silently, bare form included.
    Filed as loft#1711 (`silent-wrong`, `wa:clean`).
  - The omitted-field "third spelling of absent" had been settled by `D-Opt-Zero`; its guard
    now pins the answer.
- A stale "D-bind-38 stays open" in formal/binding.md was corrected; the old QUALITY.md was
  restating that register.

**Phase 4, the 2026-10 cycle's second pick — RELEASE.md (2026-09-28, on the owner's repeated
ask to implement the plan):**
- **Size:** 1211 → 702 lines; lint findings 38 → 8 (the rest are `timeline` false positives:
  dates inside an anchor, "no longer" describing present staleness, literal state words).
- **Split by question:** [RELEASE_PUBLISH.md](../RELEASE_PUBLISH.md) takes how a release is
  tagged, built, published and made installable:
  - the draft-first mechanics and the registry splice;
  - the bundles (old step 10);
  - what the tag pipeline proves;
  - the reference-PDF checks;
  - the two install layouts;
  - reproducible builds, as the current state.
- **History out:** [RELEASE-history.md](../RELEASE-history.md) takes the dated measurements:
  - the never-completed splice;
  - the clippy census;
  - the reproducible-build investigations;
  - the waiver story;
  - the past cycle themes.
- **Stale facts corrected:**
  - `checklist.json` is committed, not git-ignored;
  - the release gate calls six workflows, not "five";
  - the PDF ships inside each bundle and is not a separate release asset;
  - the safety gate's `0.8.4 / 1.0.0` framing is gone;
  - the out-of-scope list now names the current applications, not retired demo IDs.
- **Pointers:** every pointer into a moved section follows it, including `release.yml`'s
  comments and `release-checklist.py`'s printed pointers.
- **Side finding:** `every_markdown_link_resolves` does not check `#fragment` anchors; five
  links from PLANNING.md and a finished plan into ROADMAP.md name anchors that no longer exist.

**Phase 3, as built (2026-09-25):** two rows in `scripts/release-checklist.py`.
- **`A-docs-lint`** (automatic, `mid pre`) fails on growth since the baseline, and also on a
  burn-down left un-pinned, because headroom would hide the next regrowth.  It was falsified
  both ways.
- **`M-doc-review`** (by hand, `mid`) is one doc per cycle, ticked with the PR link.
- **D-user** is the existing `M-docs-review` row, now pointed at the reviewer session.
  RELEASE.md § Pre-Release Documentation Review names all three. Tracks [@PLN172](https://github.com/loft-lang/plans/issues/172).

**Phase 1, as built (2026-09-25):**
- [DOC_CONTRACT.md](../DOC_CONTRACT.md): 41 rules in 71 lines, each naming its owning doc
  and the gate or report that checks it.  It was written from a sweep of every place a
  documentation rule is stated.
- DOC_QUALITY.md now covers every surface.  Its § Maintainer docs is the repo home for rules
  that had lived only in agent memory, and for the 1000-line hard ceiling;
  `scripts/file-sizes.py` reports against 1000.
- The link check is a real gate: `every_markdown_link_resolves` moved to `tests/doc_hygiene.rs`,
  in the required matrix.
- The sweep's contradictions are resolved against the tree.  Among them:
  - the bug-filing home and the CHANGELOG heading;
  - the plan-file location and where a plan's status lives;
  - `api_lint`'s gate claim (its two findings are fixed, so the stdlib reads `0 active`);
  - the lint baseline path;
  - CODE.md against DOC_QUALITY;
  - `//` against `///`;
  - the library guide obligation;
  - the README's false `GENERATED` marker;
  - the doc-writer agent's stale test and date rule;
  - the doc-quality skill's rule numbering.
- CLAUDE.md points at the contract, and DRAWING.md is reachable.

**Phase 2, as built (2026-09-25):**
- `scripts/doc_lint.py`: nine rules (`stamp`, `narration`, `history`, `timeline`, `size`,
  `two-h1`, `orphan`, `hedge`, `link`).  Each pattern is read from the tool that already
  owned it, so the patterns cannot drift apart.  `fix_broken_links.py` gained the `scan()`
  function both tools call.  `history` is kept narrow on purpose: phrasing only a change
  story uses.  A hand-read sample of its hits was about 80 % real; the misses were method
  prose, and they are excluded.
- **Caller 1, the edit hook:** `.claude/settings.json` is per-machine and git-ignored (a
  2026-07-08 decision), so the hook travels through `scripts/install_claude_hooks.py`, which
  `make hooks` runs; it merges and is idempotent.  It was proved in a live session: a planted
  sentence came back as context, and the revert was silent.
- **Caller 2, the PR gate:** a step in ci.yml's `Doc hygiene` job, and `make docs-lint-gate`
  locally.  It compares against the base, never a baseline, so inherited findings cannot block
  it.  That is why it landed in this phase rather than one cycle behind the report (the open
  decision).  It was falsified four ways: a new stamp and a new history line exit 1; moved
  text, and a file already over the ceiling growing further, exit 0.
- **Caller 3, the report:** `make docs-lint` against `doc/claude/releases/docs-lint.baseline`,
  which pins counts per file and rule (510 rows), never line text.  `make docs-lint-baseline`
  re-pins it.
- The `doc-quality` skill is reduced to the reviewer pass, in the same change as the hook.

**One sequencing change:** the `doc-quality` skill is reduced to the reviewer pass in the
same change that installs Phase 2's edit hook, not in Phase 1.  Reducing it first would leave
a builder with neither the skill nor the hook.

**The reviewer's worklist, measured at Phase 1:**
- **Over the ceiling:** `make file-sizes` reports 55 docs over 1000 lines.  **CLAUDE.md is
  first**: most of its length is one section, `## LOFT_LOG quick reference`, which catalogues
  the `LOFT_NO_*` switches and belongs in NATIVE.md, DEBUG.md and DIAGNOSTICS.md.
- **More than one H1:** PACKAGES.md, NATIVE.md, TUPLES.md, THREADING.md and the two plan
  templates.
- **A restated register:** formal/README.md's areas table restates each chapter's `OPEN: n`,
  and says so itself.

Two things built elsewhere feed into this plan:

- **The `link` check already exists.** `tools/indexer/fix_broken_links.py` reads every tracked
  markdown file the way a renderer does (fences and code spans are not links). It repairs a
  link only when exactly one tracked path matches, and flags the rest.
  `tests/doc_hygiene.rs::every_markdown_link_resolves` gates it (@PLN45). `doc_lint.py`
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
- Whether `doc/claude/formal/` is exempt from `orphan` (proposed: reachable via its README, so
  no exemption needed).  `plans/` is settled: out of the report entirely (rule 14).
- ~~Whether the PR gate lags the report~~ — it did not need to: the gate compares a change with
  its base, so no baseline is involved and inherited findings cannot block (Phase 2).

## Done when

- DOC_CONTRACT.md exists, under 80 lines, and CLAUDE.md points to it in one line.
- The hook fires on a doc edit and is silent on a clean file.
- `make docs-lint` has a committed baseline and the `[mid]` view shows D-lint and D-review.
- The `doc-quality` skill is loaded by the reviewer session and by nothing else.
- Three cycles later, three docs are under the contract and the baseline count has fallen
  three times.
