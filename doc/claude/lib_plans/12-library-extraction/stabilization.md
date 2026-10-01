<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library stabilization worklist — pass the library rules before extraction

A library is one of the two prime surfaces a programmer touches (the
other is the language/stdlib).  Before a library is extracted and
published (the rest of this plan), it must pass the
[LIBRARY_CHECKLIST](../../LIBRARY_CHECKLIST.md) — the per-library
application of Goals A–F plus doc quality.  The registry `verified`
mark is how that gate is administered; this file is the worklist that
gets each library there.

This is a **2026-07-cycle stabilization effort** (see
[ROADMAP.md § Scope during a cycle](../../ROADMAP.md#scope-during-a-cycle--the-warm-feature-freeze)):
making libraries work on loft is the only new-feature track during the
freeze, and "work" means *passing the rules*, not just compiling.

## How each library is checked

The tooling from the 0.8.5 work IS the checklist — a finding is "done"
when it stops appearing:

```bash
scripts/api_lint.py lib/<name>      # S1 exact-dup · S3 missing-doc · S3q doc-quality
scripts/api_lint.py -c lib/<name>   # counts only (the thermometer)
scripts/doc_review.py               # per-section staleness over user-visible surfaces
```

Then the `[review]` half of [LIBRARY_CHECKLIST](../../LIBRARY_CHECKLIST.md)
by hand (API shape, footguns, brittle setup/hidden state, the
oversized-section red flag), and finally the `verified` mark.

## Scope — the libraries still in the monorepo

The extracted families (`core`, `net`, `graphics`) already live in
external repos and pass via their own `library-ci.yml`; they are out
of scope here.  What remains in `lib/`:

### Tier 0 — the stdlib (the library every program imports)

| Surface | api_lint | doc_review | LIBRARY_CHECKLIST | Notes |
|---|---|---|---|---|
| `default/` (6 files) | ☐ | ☐ | ☐ | swept in 0.8.5; re-verify against the finalized rules |

### Tier 1 — packaged libraries (`lib/*` with `loft.toml`)

| Library | `.loft` | description? | api_lint | doc_review | `[review]` | verified |
|---|---|---|---|---|---|---|
| `audience_crystal` | 4  | ✗ missing | ☐ | ☐ | ☐ | ☐ |
| `html`             | 3  | ✗ missing | ☐ | ☐ | ☐ | ☐ |
| `input`            | 2  | ✗ missing | ☐ | ☐ | ☐ | ☐ |
| `markdown`         | 2  | ✗ missing | ☐ | ☐ | ☐ | ☐ |
| ~~`moros_editor`/`moros_map`/`moros_render`/`moros_sim`/`moros_ui`~~ | — | **MOVED 2026-06-14** | — | — | — | — |

**Cross-cutting finding:** none of the remaining four (`audience_crystal`,
`html`, `input`, `markdown`) declares a `description` in its `loft.toml` — a
LIBRARY_CHECKLIST metadata gap to fix across the set.  (The five `moros_*`
packages — the moros RPG family — left this table on 2026-06-14, moved to the
moros project's own repo; see [moros-split.md](moros-split.md).)

### Loose single-file libraries (`lib/*.loft`, no manifest) — triage

These predate the package format and need a decision: package them
(`loft.toml` + `src/`) or retire them.

| File | lines | pub items | decision |
|---|---|---|---|
| `code.loft`    | 263 | 26 | ☐ package / retire |
| `lexer.loft`   | 477 | 26 | ☐ package / retire |
| `parser.loft`  | 674 | 1  | ☐ package / retire |
| `testlib.loft` | 57  | 7  | ☐ package / retire |
| `docs.loft`    | 21  | 0  | ☐ likely retire (no public surface) |
| `logger.loft`  | 34  | 0  | ☐ likely retire (no public surface) |

## Explicitly out of scope (not shipped libraries)

- `tests/fixtures/libs/*` and `tests/lib/*` — test fixtures.
- `tools/*` (audience-demo, brick-buster, viewer) — demo apps; own
  lifecycle per [RELEASE.md](../../RELEASE.md).
- `lib/imaging`, `lib/server`, `lib/web`, `lib/world` — **untracked
  local build litter** left after those libraries were extracted
  (only `native/target/` artifacts, zero tracked files).  Safe to
  delete locally; not part of the repo.
