<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN42 — TRACKER_INDEX — `@P-id` / `@PLAN-id` indexer + viewer integration

**Status: FINISHED (closed 2026-09-25; opened 2026-05-13).**  The index is part of the daily
workflow: the scanner, `make index`, `./scripts/idx`, the pre-commit refresh, the broken-tag and
broken-link validators (`tests/index_hygiene.rs`), backlinks, the viewer's `/tag/<bare>`, per-page
sidebar, `/welcome` and `/tags` pages, the loft-native scanner, and phase 10's language harvest.
The rest went to the plans that own it:
- **07 (daemon, CLI client, standalone binary) and 07a (WebSocket push)** → **@PLN68**
  (fs-watch).  Without file events the daemon would have to poll, and the pre-commit hook plus
  the viewer's mtime poll already serve its only consumer, so it starts when fs-watch has a
  second consumer.
- **08 (multi-project deploy)** → **@PLN50** (eagleviewer), which already generalises the
  viewer beyond loft and is where per-project tag families belong.
- **06 closeout:** the migration shipped (1500+ refs).  The planned regression test ("fewer than
  50 legacy refs") is retired, not deferred.  The bare `P<n>` series is the CLOSED archive
  (open work is GitHub issues and `@PLN` plans, [ISSUE_TRACKING.md](../../ISSUE_TRACKING.md)),
  so new work mints no bare ids.  The ~5100 remaining `legacy:` refs are archive names in
  tests, `src/` comments and finished plans, and rewriting them buys nothing.

A small, self-rebuilding index of tracker references (P-issues
+ plan/phase IDs) across the loft repo, plus a CLI for
querying it and a viewer-side surface for browsing.  The index
becomes the canonical "where is this referenced?" answer for
both humans and Claude.

## Drivers

The plan answers four problems, each layered on the
previous:

1. **Grep-based tag lookup is fragile.**  Today `grep -rn
   "@P259" doc/` matches `P2590`, `2P259`, prose like "the
   @P259 fix forward."  Adopting `@P\d+` (and `@PLAN\d+`)
   makes regex unambiguous: `grep -rn '@P259\b'` has zero
   false matches.

2. **Claude per-task token usage is dominated by `grep -rn`
   on docs.**  An indexed lookup is O(1) and pulls only the
   exact lines + few-line context.  Measurable token
   reduction per session.

3. **Plan-35 viewer needs a tag-aware navigation surface.**
   The `/welcome` landing (phase 35-08) needs structured
   buckets (open problems / recently fixed / active plans).
   PROBLEMS.md row-parsing is brittle; an index built from
   `@P-id` mentions is robust.

4. **A few static binaries in `~/bin/` should serve ANY AI
   project.**  This is the user's direction
   (2026-05-13): the tooling stack — scanner, CLI, viewer
   — is loft-native binaries with NO runtime deps (no jq,
   no bash, no Python).  Per-project `.tracker/config.toml`
   selects tag conventions + validators.  Daemon-per-project
   uses the filesystem as the registry.  Loft is the FIRST
   consumer, the test bed; the binaries serve any project
   the user maintains.

Layered like this:

| Layer | Phases | What it gives |
|---|---|---|
| Foundation | 00-03 | Bash scanner + JSON index + CLI + CI gate |
| Integration | 04-06 | Plan-35 viewer reads the index; Claude uses it; legacy refs migrated |
| Loft-native | 07 | Daemon + WebSocket clients in loft |
| **Generic stack** | 08 | mmap-backed; per-project config; install to `~/bin/` |

Filed as a sibling plan to @PLAN35 (not a phase of it)
because the scope is independent — the indexer is useful
without the viewer (Claude queries it directly); the viewer
is useful without the indexer (the existing tree + file
rendering works regardless).

## Architecture

```
.gitignore       /index/                       # output dir, gitignored
.git/hooks/      pre-commit → make index       # auto-refresh
scripts/         idx                           # CLI grep wrapper
tools/indexer/                                 # the scanner
  scan.sh        bash + grep + jq pipeline     # tier-1 implementation
  ARCHITECTURE.md design notes
index/           tags.json                     # {tag: [{file, line, ctx}]}
                 broken.json                   # broken @ref audit
                 stats.json                    # counts per tag prefix
```

Three CLI surfaces:

- **`make index`** — rescan the repo, write `index/*.json`.
  Idempotent, ~1 second on the loft tree.
- **`./scripts/idx <query>`** — query the JSON.  Supported
  query forms:
  - `idx tag:@P259` — exact tag match.
  - `idx prefix:@PLAN22` — all tags starting with prefix.
  - `idx file:doc/claude/PROBLEMS.md` — all tags in a file.
  - `idx broken` — list broken @ref links.
- **`make index-watch`** — optional file-watcher for live
  refresh during heavy editing sessions.  Stretch.

Output is JSON-first; humans and the viewer parse the same
data.

## Tag conventions

Two tag families, both prefixed `@`:

| Tag | Regex | Examples |
|---|---|---|
| **P-issues** | `@P\d+\b` | `@P259`, `@P262`, `@P229b` |
| **Plans + phases** | `@PLAN\d+(?:-[\w]+)*\b` | `@PLAN22`, `@PLAN35-01`, `@PLAN22-2d-iii.a` |

Adoption is INCREMENTAL.  Old bare-name forms (`P259`,
`plan-22 phase 03`) continue to work — the indexer ignores
them.  New documents use `@P259` / `@PLAN35-01`; old
documents get retroactive `@`-tagging via a one-time sed pass
when convenient.

The `@` prefix is the discriminator that makes regex trivial.
Plain words `P259` in prose stay readable; `@P259` is the
machine-grep'able form.

Tag bodies follow these rules:

- **Lowercase + alphanumeric only**: `@P259` not `@p259` not
  `@P_259`.
- **Slash-free**: phase IDs use `-` to separate
  (`@PLAN35-01`), not `/` (would conflict with file paths).
- **Sub-phases via `.`**: `@PLAN22-2d-iii.a` is allowed.
  Mirrors @PLAN22's `02d-iii.a` directory shape.

## Phases

| # | Phase | Effort | What ships | Status |
|---|---|---|---|---|
| 0 | [Tag convention + initial indexer](00-convention-and-scanner.md) | XS | `tools/indexer/scan.sh` + `make index` target + CLAUDE.md docs of the tag convention.  No retroactive tagging yet — indexer scans both old (`P259`) and new (`@P259`) forms with separate prefixes for transition tracking. | **Shipped 2026-05-13** |
| 1 | [CLI query wrapper](01-cli-query.md) | XS | `scripts/idx` bash wrapper around `index/tags.json`.  Supports `tag:` / `prefix:` / `file:` / `all` / `broken` / `help`.  CLAUDE.md updated to recommend it as the canonical reference-lookup. | **Shipped 2026-05-13** |
| 2 | [Auto-refresh on commit](02-auto-refresh.md) | XS | `tools/indexer/install-hook.sh` writes a marker-bracketed snippet to `.git/hooks/pre-commit`; idempotent across re-runs.  Hook re-runs the scanner when an indexed file is staged.  `make index-install-hook` invokes it.  DEBUG.md gains § Tracker-tag indexer with install + usage docs. | **Shipped 2026-05-13** |
| 3 | [Broken-tag validator](03-broken-validator.md) | S | Indexer computes `broken[]` for refs to non-existent P-ids / plans.  `<!--noindex-->` line marker for intentional doc examples.  `tests/index_hygiene.rs` CI gate. | **Shipped 2026-05-13** |
| 4 | [Plan-35 viewer integration](04-viewer-integration.md) | S | Plan-35 viewer reads `index/tags.json` and surfaces tag references.  04a (`/tag/<tag>` route + missing-index banner) shipped 2026-05-13; 04b per-doc sidebar shipped 2026-05-14; 04b welcome landing shipped 2026-05-14 (`/welcome` route consuming the new `problems_open` / `problems_recent` / `plans_*` buckets the indexer produces directly, bypassing @PLAN35 phase 08's never-built curation engine). | **Shipped 2026-05-14** |
| 5 | [Claude integration](05-claude-integration.md) | XS | Update CLAUDE.md "## Key commands" with `./scripts/idx <query>` as the canonical reference-lookup.  Add a § Tag convention section.  Optional MCP wrapper for token-efficient queries. | **Shipped 2026-05-13** (MCP wrapper deferred — optional) |
| 6 | [Retroactive tagging](06-closeout.md) | S | One-shot Python migration (`tools/indexer/migrate.py`): convert `P\d+` → `@P\d+` and `plan-NN` → `@PLANNN` in `doc/claude/**/*.md`.  Backtick-span / fence / `<!--noindex-->` aware; validates against PROBLEMS.md row IDs; skips `P1`-`P9` (overloaded with PERFORMANCE.md design IDs and `Pn-Rm` notation). | **Migration shipped 2026-05-14** (1500+ refs across ~150 files; closeout deferred until after phases 7+8) |
| 7 | [Loft-native scanner + CLI + WebSocket daemon](07-loft-native-scanner.md) | M | Daemon + clients model: long-running loft scanner serves CLI + viewer over local WebSocket.  Drives `lib/fs_watch/` + lib/server binary frames.  Bash artefacts stay as bootstrap fallback. | **MVP shipped 2026-05-15** (single-shot tag scanner: `tools/indexer/src/scan.loft` + `make index-loft` + `tests/index_hygiene.rs` diff gate); **JSON Lines emission shipped 2026-05-15**.  fs_watch, WebSocket daemon, CLI client, standalone binary build still open. |
| 7a | [Indexer → viewer WebSocket push protocol](07a-websocket-protocol.md) | S (design) / M (impl) | Wire format + lifecycle for the indexer daemon to push live tag-table deltas to connected viewers.  Cuts the polling round-trip out of the dev loop; viewer pages stay live as edits happen.  Reuses `lib/server`'s WebSocket plumbing.  Implementation gated on phase 07's `lib/fs_watch/` landing. | **Design shipped 2026-05-15**.  Implementation open. |
| 8 | [Multi-project deployment + mmap-backed index](08-multi-project-deploy.md) | M | Per-project `.tracker/config.toml` (configurable tag families + validators).  Daemon-per-project (filesystem registry — no shared service).  `tags.store` mmap-backed via loft's Store primitive (durability via @PLN43 Tier 1).  Goal: a few static binaries in `~/bin/` that handle ANY AI/coding project, not just loft. | Open |
| 9 | [Backlinks: "who links to me"](09-backlinks.md) | S | Index gains a `links` bucket: every markdown link, resolved against the source file's directory.  CLI `idx incoming:<path>` answers the inverse question; `idx broken-links` flags links to non-existent paths.  Heaviest user: plan READMEs cross-referencing each other. | **Shipped 2026-05-14** |
| 10 | [Language enhancements harvested from the dogfood pass](10-language-harvest.md) | M (cluster of XS/S items) | The bookend of the dogfood cycle.  Bundles the small loft + stdlib enhancements that the phase-07 scanner port surfaced — lexer `\0` escape, compiler warnings → stderr, `args()` builtin, `vector.sort()`, `text.split(text)`, `text.starts_with_at()`, `hash.contains()`, `text::escape_html()`, stdlib `path` module.  Lifts the workarounds from `tools/indexer/src/scan.loft` and the viewer.  Closes 2-3 P-issues; converts STDLIB.md `## Open work` rows into shipped behaviour.  Per [CLAUDE.md § Development cadence](../../../../CLAUDE.md#development-cadence--the-dogfood-loop): real consumer → harvest the lessons → fix the language → ship in the release. | Open — staged for 0.8.5 |

Total estimated effort: **~1 week** of focused work.  Phases
00 + 01 are the minimum viable indexer (~1 day); the rest
compound the value.

## Acceptance — full plan

- `@P\d+` and `@PLAN\d+(?:-[\w]+)*` are the canonical forms;
  ROADMAP / DEVELOPMENT / CLAUDE.md document the convention.
- `make index` rebuilds `index/tags.json` in ≤ 2 seconds on
  the loft tree (~1100 .md/.rs/.loft files).
- `./scripts/idx tag:@P259` returns JSON with all references
  + optional excerpts via `--before` / `--after` / `--para`
  / `--max-bytes` flags.
- Pre-commit hook keeps `index/` fresh on every commit.
- `tests/index_hygiene.rs` catches broken `@P-id` /
  `@PLAN-id` references at CI time.
- Plan-35 viewer `/welcome` landing pulls "open problems /
  recently fixed / active plans / future plans" from the
  index instead of grep'ing files at request time.
- CLAUDE.md instructs Claude to use `./scripts/idx` for
  reference lookups (measurable token reduction per session).
- A loft-native daemon (`bin/loft-index`) + CLI
  (`bin/loft-idx`) replace the bash artefacts as the
  preferred dev path; bash stays as the bootstrap.
- The daemon's index is mmap-backed (`tags.store`); kill +
  restart resumes without re-scanning unchanged files.
- A second AI project on the same machine can install the
  same binaries and run its own daemon with its own
  `.tracker/config.toml` — no shared state, no port
  collisions.
- `./scripts/idx incoming:<path>` returns all docs that
  link to the target via markdown `[text](path)` syntax.
- All 9 phases close → plan moves to `plans/finished/37-…`.

## Why this is a separate plan from @PLAN35

Plan-35 ships a viewer.  Plan-37 ships an indexer.  They
INTERSECT at one phase (35-08 newcomer landing pulls
data from the indexer; 37-04 wires the viewer to read it),
but the indexer is useful **without** the viewer (Claude
queries it directly via `./scripts/idx`) and the viewer is
useful **without** the indexer (the existing tree + file
rendering works regardless).

Splitting also keeps each plan ≤7 phases — @PLAN35 was
already at 7 with two stretches (08 + 09 + 10) waiting.
Adding the indexer to @PLAN35 would have pushed it to ~12
phases, far past a manageable single-plan scope.

## Cross-references

- [`plans/finished/35-branch-review-viewer/README.md`](../finished/35-branch-review-viewer/README.md)
  — the viewer this plan integrates with.
- [`plans/finished/35-branch-review-viewer/README.md § Stretches`](../finished/35-branch-review-viewer/README.md#stretches-post-v1-listed-for-traceability)
  — phase 35-08 newcomer landing depends on this plan's
  data shape.
- [`PROBLEMS.md`](../../PROBLEMS.md) — primary source for
  P-id references; the indexer parses its row format.
- [`ROADMAP.md`](../../ROADMAP.md) — gets a row in
  § Near-term focus once a phase ships.
