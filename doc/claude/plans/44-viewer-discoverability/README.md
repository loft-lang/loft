<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN44 — Viewer discoverability cleanups

**Status: FINISHED (closed 2026-09-25).**  All three phases shipped in one commit.  The site
header reads `dashboard · tree · docs · welcome · tags`, so `/welcome` is reachable from every page
(phase 1).  The dashboard's four hardcoded tag examples are now one pointer to the index (phase 2).
Phase 3 shipped option **(b)**, not (a): `/welcome` turned out to list no tags at all, so it was
not a sensible target for a link named `tags`.  The new `/tags` route lists every canonical `@`-tag in
`index/tags.json`, grouped by family, with its reference count, and links each to `/tag/<bare>`.
It was checked against the index (1060 tags; the counts for `@PLN157` and `@FR-B-Copy` agree), and
the page is byte-identical on `--interpret` and `--native-release`.  Opened 2026-05-15 from a
structure evaluation of the viewer's routes and inbound-link graph.

## Why

A discoverability audit of the viewer (`tools/viewer/src/main.loft`)
turned up three small cleanups that prevent feature drift and
single-path fragility.  All three are XS — single-commit each.
Bundled into one plan because they touch the same area
(`page()` site header + `page_landing` sections) and benefit
from being reviewed together.

Audit summary:

  - All 10 routes ARE reachable from at least one link site
    — no orphans.
  - One route (`/welcome`) is single-path-reachable (the
    dashboard's `W` quick-nav tile).  Fragile if a user
    arrives via a deep-link.
  - The dashboard's "Tracker tags" examples block is 4
    hardcoded links (`/tag/P259`, `/tag/P262`, `/tag/PLAN35`,
    `/tag/PLAN37`) that drift as P-issues close and plans
    finish.  Stale demo content.
  - The site-header `tags` link points at `/tag/PLAN37`
    arbitrarily — the noun is plural but there's no `tags`
    index page.

## Phases

Each phase ships as one commit.  No internal dependencies
between phases — pick in any order.

| # | Phase | Effort | What ships |
|---|---|---|---|
| 1 | **Add `/welcome` to site-header nav** | XS | Insert `<a href="/welcome">welcome</a>` between `dashboard` and `tree` in the `page()` helper.  Closes the single-path fragility — `/welcome` becomes globally reachable from any page. |
| 2 | **Replace dashboard's hardcoded "Tracker tags" examples with one `/welcome` pointer** | XS | Drop the 4 hardcoded `/tag/<bare>` links.  Replace the section with a single one-line pointer: `Browse all tags via [Welcome ▸](/welcome)`.  Removes the drift trap (currently P259 closed, P262 closed, PLAN35 finished — the demo links no longer demonstrate "open" tags). |
| 3 | **Site-header `tags` link → either `/welcome` OR a new `/tags` index** | XS–S | Two options:  (a) **Quick** — change the link to `/welcome` (the closest landing for tracker-tag exploration).  XS.  (b) **Right** — add a new `/tags` route that lists every distinct tag from `index/tags.json` keys, sorted, with ref-count badges.  S.  Recommendation: ship (a) now, file (b) as a follow-up if the index page proves valuable. |

## Acceptance

- All 10 viewer routes remain reachable from at least one
  link site.
- `/welcome` is reachable from EVERY page (via site-header
  nav).
- The dashboard's "Tracker tags" examples no longer
  reference closed/finished tags by hardcoded URL.
- Site-header `tags` link points at a sensible target
  (either `/welcome` or `/tags` index, depending on which
  option ships).

## Out of scope

- Generalising the viewer for other projects — that's its
  own plan ([`lib_plans/70-viewer-generalisation/`](../../lib_plans/70-viewer-generalisation/README.md)).
- Theme / layout changes — the Engineering Notebook
  redesign already shipped in commit `94a4797e`.
- New per-page features — this plan is pure discoverability
  hygiene.

## Cross-references

- [`@PLAN35`](../finished/35-branch-review-viewer/README.md)
  — the original viewer plan (closed); this plan is
  maintenance work on the shipped viewer.
- [`@PLN42`](../42-tracker-index/README.md) — the
  tracker-index plan that drives the `/tag/<bare>` route's
  data source.
- [`lib_plans/70-viewer-generalisation/`](../../lib_plans/70-viewer-generalisation/README.md)
  — the parallel plan that will extract the viewer's
  generic core into a reusable library.  The
  discoverability fixes here land FIRST so the
  generalisation arc inherits a clean structure.
