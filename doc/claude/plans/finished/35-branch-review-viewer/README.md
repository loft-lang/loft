<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN35 — BRANCH_REVIEW_VIEWER — a frozen loft binary for VM-based code review

**Status:** Active — promoted to current 2026-05-13.

A small loft application that runs in the user's development VM
and serves a branch-aware doc + code review dashboard to a
browser on the host (via SSH port-forward).  Built IN loft to
dogfood the server lib + @PLAN22 closures + 0.8.3 coroutines.
Compiled to a **standalone binary that ships with the repo and
runs decoupled from loft tree state** — so the user can review
loft-in-flight even when loft itself is mid-refactor or
mid-broken-build.

## Drivers

Two distinct audiences, both pointing at the same tool:

**Driver 1 — the user's own review workflow** (origin of this
plan).  Scrolling through chat-pasted code snippets is too
narrow for substantive review of in-flight work.

**Driver 2 — friend-onboarding (added 2026-05-13)** —
loft has crossed the threshold where developer friends with
some Rust / ML experience can be invited to try it.  But the
project is now ~250 markdown files in `doc/claude/` plus a
deeply-linked plan tree.  A friend who wants to grasp "what is
loft, what's done, what's planned, why does the codebase look
like this" cannot do that by `grep`-ing 20+ files manually.
The viewer becomes their **navigation surface**: open the
dashboard, click into the relevant docs, follow cross-doc
links naturally.  See [ROADMAP § Near-term focus —
friend-readiness](../../../ROADMAP-history.md#near-term-focus--friend-readiness-added-2026-05-13).

Three concrete needs surfaced from the personal-review angle:

1. **Read full files in context** — a 192 KB `PROBLEMS.md` or
   3.4 KB `PLANNING.md` cannot be reviewed via chat snippets.
2. **See what's changing on the current branch at a glance** —
   files touched, recent commits, diffs vs `main`, broken-link
   sentries.  Today this requires `git log --stat` /
   `git diff` / `git show` per file in the terminal.
3. **Follow cross-doc references** — the plan tree is densely
   linked (`[link](../22-mutable-closures/03-case-c.md#major-finding)`), <!--noindex-->
   and `make serve` (which runs `python -m http.server` on
   `doc/`) serves `.md` files raw — clicks 404 or download.

The viewer also doubles as a flagship test of three plans of
loft work that have shipped:

- **`lib/server`** (already shipped — `lib/server/src/server.loft`):
  HTTP routing, response builders, WebSocket support.  Viewer
  exercises `respond_html`, `respond_css`, `serve_dir`, plus
  the streaming-write story.
- **Plan-22 mutable closures** (shipped 2026-05-13, currently
  on `demo_dev`): route handlers expressed as closures
  capturing the viewer's shared state (template cache, file
  tree).  The "outer reads + writes seen by closure" semantics
  from the cell + auto-Reference machinery are exactly what a
  shared cache wants.
- **0.8.3 coroutines**: streaming responses for big files
  (192 KB `PROBLEMS.md` doesn't need to materialise as one
  contiguous text buffer).

If a viewer feature surfaces a loft gap that's worth filling
in the language itself, that's a **driver-level finding** and
gets filed against `plans/35-…` rather than worked around in
the viewer's loft source.

## Architecture — frozen-binary contract

The viewer binary lives at a stable path under
`tools/viewer/` with its own `loft.toml` package manifest.
Build is **deliberate, not automatic** — `make view-build`
compiles the binary; subsequent `make view` invocations run
the existing artifact.  The artifact is committed to the repo
(or, in a later phase, attached to releases).

This isolation gives three properties:

1. **Stability**: a broken `src/parser/foo.rs` change does not
   break the viewer.  The user can keep reviewing.
2. **Reproducibility**: the viewer was built against a known
   loft commit recorded in its `loft.toml`.
3. **Constrained surface**: the viewer can only use loft
   features that landed before its build commit.  This forces
   us to ship the viewer against a "release-quality" loft
   slice rather than chasing tip-of-tree.

When the user wants viewer updates, they `make view-build`
deliberately.  CI does not auto-rebuild.

## Ground rules

- **Loft-native source** at `tools/viewer/`.  No Python, no
  Node.js, no Rust outside what loft itself uses.  Every gap
  the viewer hits in loft becomes either a viewer-side
  workaround OR a loft-language enhancement (filed as a
  separate plan or P-issue with this plan as the driver).
- **Git access via wrapper script** — `tools/viewer/refresh.sh`
  dumps `git diff --name-status main...HEAD`, `git log
  --oneline -10`, `git status --short`, and per-file diffs
  to JSON / text files under `tools/viewer/state/`.  The
  viewer reads those files; refreshing state means re-running
  `make view`.  Loft has no subprocess primitive today and
  this plan does NOT block on adding one — see the
  [out-of-scope](#out-of-scope-deferred--separate-plans) section.
- **Markdown rendering minimal in v1, tables in v2** — the
  user's pain point with current tools is poor table rendering;
  proper tables are a planned phase, not v1.  See
  [phase 06](06-tables-design.md).
- **Single user, single VM, SSH-forwarded port** — no
  authentication, no multi-tenant concerns.  Default bind to
  `0.0.0.0` for SSH forwarding; `LOFT_VIEW_BIND=127.0.0.1`
  opt-out documented.
- **Zero JavaScript in v1** — plain HTML5 + CSS.  Sidebar
  collapsing via `<details>`, "diff toggle" via plain links.
- **Plain HTML + embedded CSS** — no template engine, no
  asset pipeline.  HTML is built via loft string concatenation;
  CSS embedded in the binary as a string constant.

## Phases

Each phase ships a usable viewer.  No phase requires the next
to be useful — if work pauses after phase 03, the user has a
working file browser with markdown rendering.

| # | Phase | Effort | What ships | Status |
|---|---|---|---|---|
| 0 | [Skeleton + binary build](00-skeleton.md) | XS | `tools/viewer/` script + Makefile, "hello world" binary, `make view-build` + `make view` + `make view-refresh` targets, frozen-binary contract documented (BUILD_NOTES.md records loft commit). | **Shipped 2026-05-13** |
| 1 | [HTTP server + static + tree](01-http-routes.md) | S | Server serves `/`, `/tree/<path>`, `/raw/<path>`, `/static/style.css`, 404.  Verified end-to-end (curl + browser).  Native compile blocked by @P262 + @P263 — ships interp-mode (`make view` invokes `loft --interpret`). | **Shipped 2026-05-13** (interp-mode) |
| 2 | [Code-file rendering with `<pre>` + line numbers](02-code-files.md) | XS | `/file/<path>` route renders any text file as line-numbered HTML; `<a id="L<n>">` per line for fragment scroll + `:target` highlight; HTML escape + tab → 4 spaces; binary-extension skip-list; markdown stub for phase 03; tree pages link files via `/file/`. | **Shipped 2026-05-13** (interp-mode) |
| 3 | [Minimal markdown subset](03-markdown-minimal.md) | M | Headings, bold/italic, inline code, fenced code blocks, links with relative-path resolution + GitHub-compatible heading slugs, horizontal rules, HTML comments.  ~250 lines added inline to `tools/viewer/src/main.loft`.  Surfaced two new compiler bugs (@P270, @P271) — filed.  Lists / tables / nested inline / images deferred to a follow-up phase. | **Shipped 2026-05-13** |
| 4 | [Git state via wrapper script](04-git-state-wrapper.md) | S | `tools/viewer/refresh.sh` dumps git state to `tools/viewer/state/*.json`; viewer dashboard reads JSON via the now-fully-wired JSON natives (P54 sprint complete) and renders branch header + changed-files list + recent-commits list + uncommitted-files list with status-badge styling.  `/commit/<sha>` shows the prebuilt diff.  Phase 05 (proper diff/commit views) builds on this. | **Shipped 2026-05-13** |
| 5 | [Diff + commit views](05-diff-and-commit.md) | M | `/diff/<path>` renders unified diff against `main` with hunk-coloured CSS (green +, red -, blue hunk header).  `/commit/<sha>` renders commit message + per-file diffs via the same renderer.  Top-right `[Rendered ¦ Diff vs main]` toggle on every file page (Diff link only when a diff exists).  Refresh script already dumps both per-file (state/diffs/) and per-commit (state/commits/) diffs from phase 04. | **Shipped 2026-05-13** |
| 6 | [Proper tables (forward-looking)](06-tables-design.md) | M | Full GFM table support — alignment, headers, body, nested formatting inside cells (bold/italic/code/links via render_inline).  Shipped via `lib/markdown` table renderer.  Multi-line cells + escaped pipes (`\|`) deferred — rare in loft docs; promote when a downstream consumer needs them. | **Mostly shipped 2026-05-13** (multi-line cells + escaped pipes deferred) |
| 7 | [Closeout — release binary, docs, retrofit](07-closeout.md) | S | DEBUG.md § "Branch review viewer (`make view`)" added; CLAUDE.md § Key commands already includes the row; CHANGELOG.md user-facing entry + CHANGELOG_TECHNICAL.md retrospective; ROADMAP.md updated; plan moved to `finished/`.  Retrofit: the viewer was used throughout 2026-05-13's bug-arc work (@P262→@P272) to read PROBLEMS.md / plan READMEs / cross-doc navigation — dogfooded continuously. | **Shipped 2026-05-13** |

**Phase boundaries are commit boundaries.**  Each phase lands as
its own commit on a focused branch (or directly on the
viewer's working branch).

## Acceptance — full plan

- `make view-build` produces `tools/viewer/bin/loft-view`
  (loft-compiled binary, ~stable size).
- `make view` runs the binary; user opens
  `http://localhost:8765/` via SSH port-forward and reviews the
  current branch end-to-end without dropping into the terminal.
- Worst-case file (`doc/claude/PROBLEMS.md`, 192 KB) renders
  in <2 sec on the user's hardware.
- All cross-doc relative links resolve to served URLs;
  GitHub-compatible heading slugs for anchor fragments.
- GFM-style tables render with column alignment in phase 6.
- Frozen-binary contract holds: a deliberately-broken `main`
  build does NOT prevent `make view` from working off the
  pre-built binary.
- DEBUG.md has the user-facing usage section (port-forward
  example, env var reference).
- CLAUDE.md § Key commands lists `make view` next to
  `make serve`.

## Risks

| Risk | Mitigation |
|---|---|
| Loft features the viewer needs surface gaps | Each gap becomes a P-issue or its own plan with this plan as the driver.  Viewer ships a workaround in the meantime (e.g. simpler markdown until a feature lands). |
| Frozen binary diverges from current loft, can't be rebuilt | Loft commits to backwards-compatible `lib/server` API surface.  The viewer's `loft.toml` pins a specific loft commit; rebuild against newer loft is a deliberate exercise. |
| Wrapper-script approach feels janky vs. proper subprocess | Acceptable trade-off for shipping in weeks not months.  If subprocess support lands in loft (separate plan), refresh-script approach can be retired. |
| Tables phase (06) blocks closeout | Phase 06 is its own milestone; phases 00-05 close as a usable v1 even if 06 is still in flight.  Plan can split into `35a` (v1 close) + `35b` (tables) if 06 takes too long. |
| User wants live reload sooner than expected | Coroutines + WebSocket already in `lib/server`; live-reload becomes phase 08 if needed. |

## Stretches (post-v1, listed for traceability)

These three stretches form a coherent arc: **curation engine**
(phase 08) feeds both a **live newcomer landing** in the
viewer and a **static dump** (phase 09) that can ship to
GitHub Pages.  **Public hosting** (phase 10) is the deployment
target.  Each can be picked up independently after v1 closes.

### Phase 08 — Curation engine + newcomer landing

The 250-file `doc/claude/` tree is overwhelming for someone
arriving at loft for the first time.  Phase 08 builds the
**curation engine** that classifies docs by status — and uses
its output to drive a `/welcome` landing page that gives
newcomers a guided path.

#### Curation classifier

A loft module that walks the project at startup and produces a
classified index:

| Bucket | Source | Status filter |
|---|---|---|
| **Open problems** | Rows in `PROBLEMS.md` table | Severity column does NOT contain "(closed)" |
| **Recently closed problems** | Same, inverse | Closed in last 30 days; sorted by close date (parsed from row text) |
| **Active plans** | `plans/[0-9]*-*/README.md` | Always considered active |
| **Future plans** | `plans/future/[0-9]*-*/README.md` | Always considered planned |
| **Deferred plans** | `plans/deferred/[0-9]*-*/README.md` | Distinct from future — won't do absent trigger |
| **Recently finished plans** | `plans/finished/[0-9]*-*/README.md` | mtime within last 60 days; sorted by close date |
| **Curated examples** | `examples/*.loft` | All listed |
| **Library packages** | `lib/*/loft.toml` | Names + one-line descriptions |

Each bucket produces a small JSON/loft-struct list the landing
page renders.  The classifier is deliberately loose with row
parsing — failures degrade to "show fewer items," not crash.

#### `/welcome` landing page

The page is structured around the **four questions a visitor
asks** — in priority order:

1. **What is loft and where is it going?** — vision + the
   near-term focus the project is pursuing right now.
2. **What did we finish recently?** — the "this is alive
   and shipping" signal.
3. **What interests me most?** — multiple entry points by
   interest area; visitors pick the path that fits.
4. **Where could I help?** — open work surfaced by
   approachability, not just priority.

Layout:

```
[loft]                              [Branch dashboard ▸]

────────────  Where is loft going  ────────────
   Two-paragraph elevator pitch — what loft is, what makes
   it different.  Link to root README + ROADMAP.

   Near-term focus: friend-readiness (4 gates)
   ▸ Plan-07 phases 5/6/7 — error messages
   ▸ Plan-35 phases 00-03 — the viewer you're using
   ▸ DX.2 — CI for package + native tests
   ▸ Plan-35 phase 06 — proper GFM tables
   (Source: ROADMAP § Near-term focus)

────────────  What we finished recently  ────────────
   Recently shipped plans (last 60 days)
   ▸ 22-mutable-closures (2026-05-13) — closures novices expect
   ▸ 15-closure-validation (2026-05-12) — regression matrix
   ▸ 31-html-export (2026-04-22) — loft → wasm in browser
   ▸ ... (more)

   Recently closed problems (last 30 days)
   ▸ P261 (2026-05-13) — vector-field assign appended
   ▸ P260 (2026-05-13) — closures hold live DbRef
   ▸ P259 (2026-05-13) — multi-factory cell ownership
   ▸ ... (more)

   Click any item → that plan's README or PROBLEMS row.

────────────  Pick your interest  ────────────
   Building games?
     ▸ lib/graphics/examples/25-brick-buster.loft
     ▸ Brick Buster gallery (live in-browser)
     ▸ plans/32-event-loop  (where games are headed)

   Servers + multiplayer?
     ▸ lib/server (HTTP + WebSocket)
     ▸ plans/39-tic-tac-toe (TTT v5 multiplayer)
     ▸ plans/finished/22-mutable-closures (closures power
       writable server state)

   Language design?
     ▸ doc/learn-loft.md (the syntax tour)
     ▸ DESIGN_DECISIONS.md (closed-by-decision register)
     ▸ INCONSISTENCIES.md (known asymmetries we live with)
     ▸ plans/28-error-messages (UX of compiler errors)

     Compiler internals?
     ▸ COMPILER.md / INTERMEDIATE.md / NATIVE.md
     ▸ The interpreter + native + WASM three-way story

   Performance?
     ▸ PERFORMANCE.md (benchmarks + optimization arc)

   Just want to try it?
     ▸ examples/hello.loft     (click → rendered)
     ▸ examples/structs.loft
     ▸ examples/match.loft
     ▸ doc/learn-loft.md       (30-min tutorial)

────────────  Where could I help  ────────────
   Open work classified by WHAT YOU NEED, not just by size.
   Some tasks are easy IF you have a specific environment;
   the same task is hard from the wrong machine.

   Each item below has 3 tags: [size] [setup] [skills].

   ▸ Setup tags — what hardware/OS makes this approachable
       [linux] / [macos] / [windows] / [any-os]
       [gpu] / [no-gpu]
       [claude-or-ai] (an LLM assistant materially helps)
   ▸ Skills tags — prior knowledge that helps
       [no-loft] (no loft familiarity required)
       [some-loft] / [deep-loft]
       [rust] / [ml-family]
   ▸ Size tags — XS / S / M / L

   ── Looking for an approachable starter? ──

   [S][windows][claude-or-ai][rust] — P229b: Windows
     multiplayer flake.  Reproduces only on Windows; needs
     someone with a Windows box (and Claude or similar AI
     pairs perfectly with the multi-file diagnosis work).
     Worth doing because it unblocks Windows users for the
     multiplayer story.

   [XS][any-os][no-loft][rust] — DX.2: extend GitHub Actions
     workflow with package + native test matrices.  Pure
     YAML + shell; design fully spec'd in plans/future/
     27-developer-experience.

   [S][any-os][some-loft][no-rust] — Add an example to
     examples/.  Pick a small program (TODO list, anagram
     finder, simple text adventure); ship it as one new file.

   ── Larger contributions? ──

   [M][any-os][some-loft][rust] — Plan-07 phase 6: per-site
     type-mismatch wording across parser/expressions /
     parser/control / parser/objects / parser/operators.
     Pure rendering work once Type::name is exhaustive
     (shipped 2026-05-13).

   [M][any-os][deep-loft] — Plan-35 phases 04+05: the
     viewer's git-state wrapper + diff/commit views.  This
     IS the viewer you're using — contribute by making it
     better.

   [L][linux][some-loft][ml-family] — lib_plans/future/
     01-regex: first lazy-stdlib library.  Multi-week scope;
     touches loft text-handling depth.

   ── Active P-issues (smaller scoped) ──
   See PROBLEMS.md for the full catalogue.

   Click any item → its full plan README, PROBLEMS row, or
   ROADMAP entry, with the design and context already laid
   out.  No need to "read 20 files" before starting.
```

The tag system makes the page **searchable by what
contributors actually have**.  A friend with a Windows box
and Claude scans the [windows] tag, sees @P229b is a perfect
fit, clicks through to the full PROBLEMS.md row + design
context, and starts.

The classifier in `tools/viewer/src/help_index.loft`
populates the tags from a hand-maintained mapping of
plan/issue → tags.  Initial mapping ships with phase 08;
contributors update tags as they discover the right
classification.  No automatic tagging from the open work
itself — the heuristic isn't reliable enough to be
algorithmic.

────────────  Where to get help  ────────────
   ▸ GitHub issues (general)
   ▸ This dashboard's branch view (what's actively shipping)
   ▸ doc/claude/* (depth — this is where contributors live)
```

The four sections each pull from the curation engine:

| Section | Source |
|---|---|
| Where is loft going | `ROADMAP.md § Near-term focus` (parsed) + manual elevator pitch |
| What we finished recently | `plans/finished/*/` mtime-sorted + PROBLEMS.md "(closed)" rows date-sorted |
| Pick your interest | Hand-curated mapping of interest area → key docs (lives in `tools/viewer/src/interests.loft`); refreshed manually as the project evolves |
| Where could I help | Active plans' open phases + future plans marked "approachable" + open P-issues; classified by approachability heuristic (size estimate from plan READMEs, "good first issue"-style markers) |

The "Pick your interest" section is the user-described
**"what interests me the most"** entry point — different
visitors take different paths, all valid.

The "Where could I help" section is the user-described
**"where would I like to help"** — surfaced by approachability,
not just priority.  A new contributor scrolls here, finds an
"easy starter," picks one up.

Both sections together transform the landing from a
"here's our doc tree, good luck" page into a
"here's what loft is, here's how to engage with it" page.

The default dashboard (branch state + changed files + commits)
remains the user's personal review tool; `/welcome` is a
separate landing page friends arrive at.  Both routes coexist;
the navigation lets you flip between them.

File as **phase 08** of this plan when v1 ships.

### Phase 09 — Static dump (`loft-view --static --out=site/`)

The viewer's HTML rendering is already deterministic — given
the same input files, every route produces the same HTML.
Phase 09 adds a `--static` mode that walks **a curated subset
of routes**, dumps the rendered HTML to a directory, and emits
an `index.html` linking everything.

#### Subset selection — what's IN the dump

Not every doc gets dumped.  The dump is the public face of the
project for newcomers; deep-detail docs link OUT to GitHub's
raw view rather than getting bundled.  Three buckets:

| Bucket | What's dumped | Why included |
|---|---|---|
| **Newcomer surface** | `/welcome` landing, root `README.md`, `doc/learn-loft.md`, `STDLIB.md`, `LOFT.md` (or selected sections), `examples/*.loft` (rendered with syntax highlighting + line numbers) | Friend-onboarding is the dump's whole point |
| **Top-level project docs** | `RELEASE.md`, `CHANGELOG.md`, `PLANNING.md` (current section only), the curation engine's status pages | The "what is loft / what just shipped / what's planned" view |
| **Plan READMEs** | Each plan dir's top-level `README.md` (active + future + recently-finished — last 60 days) | Plan READMEs are the public-facing summary; phase docs go below |

Everything else — full PROBLEMS.md catalogue, individual
plan-phase docs, lib/ READMEs, internal architecture docs
like INTERMEDIATE.md / NATIVE.md / WASM.md — does NOT get
dumped.  The curated pages link to those via GitHub's
raw-blob view:

```html
<a href="https://github.com/loft-lang/loft/blob/main/doc/claude/PROBLEMS.md">
   PROBLEMS.md (full catalogue on GitHub)
</a>
```

This keeps the Pages site small (<10 MB target) + focused
on what newcomers actually need, while still letting curious
visitors drill into the deep tree via GitHub's familiar
file viewer.

#### Code-block syntax highlighting in the subset

Pages that DO get dumped have code blocks rendered with
syntax highlighting (rust, loft, toml, bash, json, diff).
Pre-fix v1 of the viewer ships without highlighting (per
phase 02's "forward-looking" note); the static dump phase is
a natural time to add it because:

- The cost of compute is paid once at dump time, not per
  request.
- The newcomer landing's "Try it in 5 minutes" examples
  benefit visibly from highlighted loft snippets.
- Adding a small per-language tokenizer in loft (~150 lines
  per language for rust + loft minimum) doubles as a driver
  for `lib/syntax/` — file as a separate sibling plan if
  the cost is large.

If syntax highlighting isn't ready, dump the code blocks as
plain `<pre>` and add highlighting in a follow-up.  Phase 09
does not block on it.

#### Link rewriting for the dump

The viewer's relative `.md` link rewriter (phase 03) needs a
dump-mode variant:

| Source link | Live viewer | Static dump (this phase) |
|---|---|---|
| `[X](other.md)` where `other.md` IS in the subset | `/file/dir/other.md` | `dir/other.html` |
| `[X](other.md)` where `other.md` is NOT in the subset | `/file/dir/other.md` (lives) | `https://github.com/loft-lang/loft/blob/main/dir/other.md` (GitHub raw) |
| `[X](other.md#section)` not in subset | `/file/dir/other.md#section` | `https://github.com/loft-lang/loft/blob/main/dir/other.md#section` (GitHub respects anchors) |
| `[X](src/parser/foo.rs)` (code file) | `/file/src/parser/foo.rs` | `https://github.com/loft-lang/loft/blob/main/src/parser/foo.rs` (GitHub renders Rust nicely) |

The viewer's static-mode rewriter knows the in-subset set and
applies the right rule per link.  GitHub's URL convention
(`blob/main/path`) is stable; if it changes, the rewriter
adjusts in one place.

#### Build pipeline

```bash
$ loft-view --static --out=public/ --base-url=https://user.github.io/loft/ --github-blob=https://github.com/loft-lang/loft/blob/main/
   ✓ rendered /welcome → public/welcome.html
   ✓ rendered /file/README.md → public/file/README.html
   ✓ rendered /examples/hello.loft → public/examples/hello.html
   ... (rendered 47 files — curated subset, not 247)
   ✓ external links resolved against github-blob
   ✓ wrote public/index.html → /welcome
   ✓ wrote public/static/style.css
```

CI workflow:

```yaml
# .github/workflows/docs.yml (sketch)
on:
  push:
    branches: [main]
jobs:
  build-docs:
    steps:
      - uses: actions/checkout@v4
      - run: make view-build
      - run: ./tools/viewer/bin/loft-view --static \
               --out=public \
               --base-url=https://loft-lang.org/loft/ \
               --github-blob=https://github.com/loft-lang/loft/blob/main/
      - uses: actions/deploy-pages@v3
        with: { artifact_name: public }
```

Acceptance:
- `loft-view --static --out=site/` produces ≲ 10 MB of HTML
  covering the curated subset + working internal navigation.
- Links to non-subset content go to GitHub's `blob/main/`
  view; verified by clicking into the rendered site and
  ensuring "deep detail" links land on GitHub correctly.
- Code blocks in dumped pages have syntax highlighting (or
  plain `<pre>` if highlighting deferred).
- Snapshot timestamp visible on every page; "this is a
  read-only snapshot of <main@sha>" footer.
- Site loads correctly when served from
  `https://user.github.io/loft/` (non-root path).

File as **phase 09** when phase 08 ships.

### Phase 10 — Public-instance hosting

Two deployment options unlocked by phases 08 + 09:

**(a) GitHub Pages** — `.github/workflows/docs.yml` from
phase 09 runs on every push to `main`; static dump goes
straight to `https://loft-lang.org/loft/`.  Lag is
"push-to-main + 1 min" — much better than the original
GitHub Pages worry because the dump is fast and the curation
filters out the dev-internal noise.

**(b) Live VPS** — same binary running on a tiny VPS pointed
at a public mirror of the loft repo.  Live branch state for
visitors who want to see in-flight work.  HTTPS via reverse
proxy.

Pick one or both.  Acceptance varies; key point is friends
visit a URL and land on the curated `/welcome` page (live or
static).

File as **phase 10** when phase 09 ships.

### Why phase 09 rehabilitates GitHub Pages

The original "GitHub Pages will lag and overwhelm" critique
applied to a naive static rendering of the entire doc tree.
Phase 09's static dump is **the curation engine's output** —
not the raw tree.  The newcomer landing, the bucketed status
view, the recently-fixed history are all in the dump, with
the dev-internal noise filtered out.  So the GitHub-Pages
deployment of plan-35's static output is qualitatively
different from "host doc/claude/*.md as raw HTML."

The lag concern remains real — Pages reflects last push to
`main`, not the user's working branch — but for friend-
onboarding, where the value is "what is loft" + "what just
shipped" + "what's planned," last-push-to-main is the right
freshness target.

## Out of scope (deferred / separate plans)

- **Subprocess support in loft** — file as a separate
  `lib_plans/future/<NN>-subprocess/` if the wrapper-script
  approach stops being enough.  Not blocking this plan.
- **Full GFM markdown parser as a `lib/markdown/` library** —
  the viewer ships its own minimal subset.  Promoting the
  parser to `lib/markdown/` is a separate plan with this one
  as a driver.
- **LSP integration / jump-to-definition** — VS Code
  remote-ssh covers this.  The viewer is for review, not
  editing.
- **PDF export per file** — browser print-to-PDF works.
- **Tracker-tag indexer (`@P259` / `@PLAN22-2-ii`)** — the
  broader proposal the user evaluated earlier; separate plan
  if/when promoted.
- **Cross-file full-text search** — browser Ctrl-F covers
  single files.  If cross-file becomes essential, add
  ripgrep-via-wrapper later.
- **Live reload via WebSocket** — phase 08 if F5 stops being
  enough.
- **Multi-branch / PR review** — single-branch (current vs
  `main`) is the user's actual workflow.

## Cross-references

- [`server/src/server.loft`](https://github.com/loft-lang/loft-libs-net/blob/main/server/src/server.loft) in `loft-libs-net`
  — the HTTP / WebSocket primitives the viewer depends on.
- [`plans/finished/22-mutable-closures/`](../22-mutable-closures/README.md)
  — closures are the natural shape for route handlers; viewer
  is the first non-test consumer.
- [`lib_plans/future/08-server/README.md`](../../../lib_plans/future/08-server/README.md)
  — server-lib roadmap; viewer surfaces specific feature gaps.
- [`lib_plans/57-regex/README.md`](../../../lib_plans/57-regex/README.md)
  — regex would simplify markdown parsing; not blocking.
- [`DEBUG.md`](../../../DEBUG.md) — gets the user-facing usage
  section in phase 07.
- [`CLAUDE.md`](../../../../../CLAUDE.md) — gets the
  `make view` entry under § Key commands in phase 07.
- [`lib_plans/66-viewer-lsp-bridge/README.md`](../../../lib_plans/66-viewer-lsp-bridge/README.md)
  — extends the viewer with multi-language LSP code intelligence
  (rust-analyzer / loft-lsp / jdtls).  The viewer is the host;
  plan-14 is the new capability.  Designed 2026-05-13 in response
  to "tooling colleagues will judge by" framing.
