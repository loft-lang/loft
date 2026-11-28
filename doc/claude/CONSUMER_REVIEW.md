<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Consumer review — the monthly pass over the projects built on loft

A project written in loft keeps the workarounds of the loft it was written against: the
defect that closed, the form that was slow, the routine a library gained later.  This page is
the SPEC of the monthly pass that removes them — which projects it covers, the four checks it
runs on each, what it leaves behind, and the tools it needs.  **State: designed; the tools of
§ Tools are not built.**  The evidence that it is needed is
[@PLN231](plans/231-against-rust/README.md) R11: two consumers steer their code by sixteen loft
issues, every one closed, and by speed notes that measure false on main.

It is run FROM this repository, once per release, against that release's binary, over every
project of the census — one sitting, so a routine two projects wrote twice is seen as one
finding.  It is a REPORT, never a gate; the one gate is the census guard (§ The census).

## What the pass is for

| A project still carries | Because | The check |
|---|---|---|
| a workaround for a defect or a restriction that loft does not have | nothing tells a project that an issue closed | 1 — lifted |
| a rule in its notes about loft that was never an issue, or does not hold | a belief is written once and never re-measured | 2 — beliefs |
| a form rewritten for speed, or a habit that reads as the idiom | the language once punished the natural form | 3 — canon |
| a routine the standard library, a published library or another project already has | the author did not look, or the routine arrived later | 4 — reinvention |

The fourth is the one the games need most: a game's agent writes the clamp, the grid walk, the
easing curve and the rectangle test it needs where it needs them, and three games then carry
three of each.

## The census

**One file names every project the pass covers, and every other list reads it.**  The home is
the `hit-by:` table of [.github/LABELS.md](../../.github/LABELS.md): a project that can hit a
loft defect is a project that carries workarounds.  It gains the columns the pass needs:

| column | meaning |
|---|---|
| project, repository | as today |
| kind | `game`, `tool`, `service` — check 4 compares within a kind first |
| gate | whether `consumer-main-health` runs it nightly |
| reviewed at | the release whose pass it last received |

- **Everything else derives from it**: the nightly gate's matrix, the weekly read's consumer
  column, the lint sweep and the switch A/B workflows carry the list today, each its own copy,
  and they disagree (a project with a label and no gate; projects named in plans with neither).
- **The census guard**: a workflow or script that names a consumer the table does not, and a
  `hit-by:` label in use on the tracker with no row, each fail a tree guard.  That is what
  makes the table a census: the tracker says which projects exist, the table says which were
  reviewed, and the difference is printed.
- **The bench is project zero.**  The consumer benches were lifted from the projects and can
  carry their workarounds; a canon that holds a habit confirms it in every project compared
  with it.  The pass runs over `bench/` first.
- **The libraries are not in it.**  They are this stream's, and their pass is
  [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md)'s; the checks below apply to them there.

## The four checks

Each check has a SOURCE (what is true now), a way to find candidates in a project, and an
OUTCOME per candidate.  A candidate is settled by a probe — a few lines of loft, both
backends, the answer computed by hand — never by reading.

### 1 — Lifted: the release's fixes and features against the project

- **Source.**  The issues closed since the project's `reviewed at` release, each with the
  `Workaround` field its report carries, and the release's CHANGELOG entries.  The fix's cell in
  the language suite is the form that works now.
- **Candidates.**  (a) every `loft#N` in the project's notes, its `CLAUDE.md` and its code
  comments whose issue is closed; (b) the workaround's SHAPE searched in its code — the inline
  index where an accessor would be, the loop variable prefixed per function, the helper
  declared above its caller — from the closed issue's workaround text.
- **Outcome.**  The note's line retired and the workaround undone, with the project's own
  tests green.  A natural form that fails is a REGRESSION: an issue with the project's
  `hit-by:` label and `regression`, and the workaround stays until it is fixed.

### 2 — Beliefs: the project's notes about loft, re-measured

- **Source.**  The project's own notes: every sentence that says what loft does, cannot do, or
  does slowly.
- **The rule, the one LOFT.md's own limitations follow**: a statement about loft is a claim,
  and a claim cites the probe that shows it.  A line with an issue is check 1's; a line with
  neither issue nor probe is UNVERIFIED and gets a probe written here.
- **Outcome.**  Per line: `holds` (the probe shows it; if it is a defect, it is filed), `lifted`
  (the probe runs clean; the line goes), or `cost` (it runs but slower — check 3 measures it).
  The probes are kept, so the next month re-runs them instead of re-deriving them.

### 3 — Canon: the hot paths against the performance programs

- **Source.**  The bench routines of [PERF_PORTAL.md](PERF_PORTAL.md): each is written in the
  natural form ([GOALS.md § F](GOALS.md)), hash-checked against a Rust twin and measured
  against the bar, and `(Perf-Teach)` forbids rewriting one for speed.  They are the worked
  answer to "how is this written".
- **Candidates.**  The project's hot functions (`make profile` on its scripted run), each
  mapped to its portal mechanism class — keyed, vector-build, alloc-temp, record-field, call,
  float-kernel, text-scan and the rest — and compared with that class's routines.
- **Outcome.**  One of three.  The project's form IS the class's form: nothing.  It differs and
  the natural form's row is under the bar: the rewrite goes.  It differs and the row is over
  the bar: a defect by [@C136](DESIGN_DECISIONS_PLATFORM.md), filed, the rewrite kept until it
  closes.
- **No pattern found** — the shape has no routine in any class: a bench row is ADDED, the
  routine with its Rust twin, under [@PLN158](https://github.com/loft-lang/plans/issues/158).
  The project gets its pattern and the shape comes under the bar.

### 4 — Reinvention: a routine that already exists

- **Source.**  Three, in this order: the standard library ([STDLIB.md](STDLIB.md)), the
  published libraries (`make libcatalogue`, the generated `LIBRARIES.md` — never a clone), and
  the other projects of the census.
- **Candidates.**  Every function of the project against those three, by name and by
  signature shape first, then by reading the short list: a routine whose body does what a
  catalogue routine's description says.  Functions under a few lines that only rename a call
  are candidates too — a wrapper is a routine the author did not find.
- **Outcome.**  One of four.  **Use it**: the project calls the library, its copy goes.  **The
  library lacks an edge** the copy has: a library gap, and library work is this stream's
  (CLAUDE.md § Dogfood loop) — the library gains it, then the project uses it.  **Two projects
  share a routine no library has**: it is extracted into the library of its subject, once, and
  both use it.  **It only looks alike**: recorded with the reason, so next month does not
  re-ask.
- **Why from loft**: the catalogue, the libraries' sources and every project are readable
  from one place only here, and a duplicate across two games is invisible from inside either.

## What a pass leaves behind

- **One report per project per release**, in the cycle's record:
  `releases/<cycle>/consumers/<project>.md` — a table per check, one row per candidate with
  its probe and its outcome.  A record doc: it keeps its dates and measurements.
- **The probes**, beside the report, re-run by the next pass.
- **Issues**: a regression (check 1), a defect a belief turned out to hold (2), a routine over
  the bar (3), each with the project's `hit-by:` label.
- **Bench rows** (3) and **library changes** (4), each landed the way that kind lands.
- **The watermark**: the census row's `reviewed at` moves to the release — after the report is
  committed, never in the same commit (SKILLS_REVIEW.md § Do it early).

**Who changes the project.**  This stream reads a project's tree and never writes to it
(CLAUDE.md § Dogfood loop), and never runs `loft test` inside it; probes run from a scratch
package that points at its sources by path.  The report is the hand-over: the project's own
agent applies it, and its `reviewed at` moves when the report is committed, not when the
project has changed — the next pass finds what was left.

## The numbers

| number | read | stable when |
|---|---|---|
| closed-issue references standing in the projects' notes | the weekly read, a column | 0 a week after a release |
| unverified belief lines (no issue, no probe) | the pass | 0 |
| speed rewrites kept for a row over the bar | the pass, with their issues | each closes with its issue |
| reinvented routines per project, and routines extracted to a library | the pass | the first falling, release over release |
| projects behind the current release | the checklist row `M-consumer-review` | 0 |

## Tools

To build, each a `scripts/` program run through one `make consumer-review` target, in the shape
`make skills-review` has:

| tool | does |
|---|---|
| `consumer-review` (no argument) | the census: each project, its `reviewed at`, what owes a pass |
| `… lifted <project>` | check 1's candidates: closed `loft#N` references with file and line; the release's closed issues with their workaround text |
| `… beliefs <project>` | check 2: the notes' lines about loft, each with its issue, its probe or `unverified`; runs the kept probes on both backends |
| `… canon <project>` | check 3: the hot functions of a profiled run with their portal class and that class's routines |
| `… reinvented <project>` | check 4's short list: project functions against the stdlib, the catalogue and the other projects, by name and signature |
| `… --done <project>` | moves the watermark |

The tools produce candidates; the reading, the probes and the outcomes are the pass's.  The
census guard and the weekly read's column are the two pieces that run without a pass.

## Open decisions

1. **Who applies a report** when a project has no active agent: this stream by exception, or
   the report waits.
2. **The reader's sample** — one file per project per month read by a fresh agent with the
   current language reference and no history, for the forms nobody thought to write.  Whether
   it is part of the pass or of its own.
3. **Projects with no `hit-by:` label yet** (named in plans and commits only): a label and a
   row each, or one `hit-by:other` row.

## See also

[plans/231-against-rust/README.md](plans/231-against-rust/README.md) ·
[LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md) · [SKILLS_REVIEW.md](SKILLS_REVIEW.md) ·
[BUG_REVIEW.md](BUG_REVIEW.md) · [PERF_PORTAL.md](PERF_PORTAL.md) ·
[RELEASE.md](RELEASE.md) · [STABILITY_METHOD.md](STABILITY_METHOD.md)
