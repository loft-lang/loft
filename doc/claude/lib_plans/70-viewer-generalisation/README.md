<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `lib/viewer/` — generalisation of the loft branch-review viewer

**Status:** Future — opened 2026-05-15 with two real
customers driving the work: the user's active Java project
and the moros project (loft sibling).  Both want the
viewer's branch-review surface but with their own tag
conventions, file paths, and brand.

## Why

The viewer (`tools/viewer/src/main.loft`) was built for loft.
Two more projects will reuse it.  Without extraction, those
projects either fork the source (drift inevitable) or pull
loft as a build dependency (heavy + tight coupling).  The
right answer is to extract the generic engine into
`lib/viewer/`, leave the loft-specific bits in
`tools/viewer/`, and let the two new consumers ship thin
config + a 30-line `main.loft` shim.

## Convention surface — what the viewer requires from any consumer

The viewer scopes itself to projects that follow these
conventions.  Projects that break them aren't a fit and
should use a different tool.

  - **Tag shape**: `@<UPPERCASE-PREFIX><digits>[<lowercase-letter>]`
    or `@<PREFIX><digits>(-<segment>)*`.  Anchor `@`,
    uppercase prefix, digits, optional letter or sub-segment.
  - **Issue tracker**: ONE markdown file with a table per
    issue.  Columns: `| <id> | <body containing @TAG> |
    <severity> | <fix path> |`.  Severity carries `(open)` /
    `(closed)` / `(partial)` markers.  Closure date in body
    as `Closed (YYYY-MM-DD)`.
  - **Plans**: directories named `<NN>-<slug>/README.md`
    under one or more roots.  Optional sub-buckets
    `future/`, `deferred/`, `finished/`.  README's first
    `# ` heading is the display title.
  - **Branch workflow**: a long-lived base branch (`main`
    / `master` / `develop`); feature branches diff against it.
  - **Source layout**: a small set of top-level directories;
    the viewer walks all and lets the user browse any.

These are NOT configurable — they're the viewer's contract.

## What IS configurable (per-project `.viewer.toml`)

Just enough config to handle naming and paths.  ~25 lines
TOML.  No regex, no plugins, no theme system.

```toml
[brand]
name    = "loft·view"
tagline = "branch-aware code review"

[scan]
source_roots = ["doc", "default", "lib", "src", "tests", "tools"]
skip_dirs    = ["target", "node_modules", ".git", "generated", "pkg"]
extensions   = [".md", ".rs", ".loft", ".toml", ".sh", ".py"]

[git]
base_branch = "main"

[tags]
issue        = "P"       # `@P259` is an issue
plan         = "PLAN"    # `@PLAN37` is a plan
legacy_issue = "P"       # bare `P259` → legacy:P259
legacy_plan  = "plan"    # bare `plan-37` → legacy:plan-37

[issues]
file               = "doc/claude/PROBLEMS.md"
recent_window_days = 30

[plans]
roots                       = ["doc/claude/plans", "doc/claude/lib_plans"]
sub_buckets                 = ["future", "deferred", "finished"]
recent_finished_window_days = 60
```

A different consumer (e.g. the Java project) writes:

```toml
[brand]
name    = "kestrel·view"
tagline = "compiler dev dashboard"

[scan]
source_roots = ["docs", "src/main/java", "src/test/java", "scripts"]
skip_dirs    = ["target", "build", ".git", ".idea"]
extensions   = [".md", ".java", ".kt", ".gradle.kts", ".toml"]

[git]
base_branch = "main"

[tags]
issue = "I"
plan  = "RFC"

[issues]
file = "docs/issues.md"

[plans]
roots = ["docs/rfcs"]
```

Same engine renders Kestrel's branch-review dashboard.

## Phases

| # | Phase | Effort | What ships |
|---|---|---|---|
| 0 | **Audit-and-mark** | XS | Tag every loft-specific block in `tools/viewer/src/main.loft` with a `// CONFIG: <key>` marker noting what config key would replace it.  Pure documentation pass — no behavior change.  Establishes the touch points for phases 1-5. |
| 1 | **Extract pure-generic helpers into `lib/viewer/`** | M | Move `page()`, `breadcrumbs()`, `escape()`, `status_chip()`, `quick_nav()`, `render_activity_card()`, the file-tree walker, the `<details>` accordion helpers, and the BASE_CSS into `lib/viewer/src/viewer.loft`.  No behavior change for the loft viewer; consumer's `tools/viewer/main.loft` becomes thinner. |
| 2 | **Config loader** | S | Add `viewer::load_config(path)` that parses `.viewer.toml` (or returns sensible defaults).  Replace 4 hardcoded paths (`base_branch`, `issues_file`, `plan_roots`, `source_roots`) with config reads.  Loft's existing behavior preserved by default config matching today's hardcoded values. |
| 3 | **Tag-prefix parameterisation** | S | Generalise the two tag prefixes (`P`, `PLAN`) into config-driven names.  scan / regex shapes stay; only the prefix chars come from config.  Loft viewer continues to work unchanged.  Indexer (`tools/indexer/src/scan.loft`) extends with the same config so other consumers can run it on their own trees. |
| 4 | **Brand parameterisation** | XS | Brand string + tagline from config.  Theme stays baked in (Engineering Notebook).  Footer / header text reads from config. |
| 5 | **Consumer shim + docs** | S | Document the convention surface in `lib/viewer/README.md` — what shapes the viewer requires from a consumer, and the `.viewer.toml` schema.  Example minimal `tools/viewer/main.loft` that loads config + invokes the engine.  Loft's `tools/viewer/main.loft` becomes one of these shims. |
| 6 | **Java project consumer** | S | Stand up the viewer in the user's active Java project.  Validates the convention surface against a real non-loft consumer.  Likely surfaces 1-2 small config gaps that get fed back into phases 1-5. |
| 7 | **moros project consumer** | S | Stand up the viewer in the moros project.  Second validation; both consumers should converge on the same `lib/viewer/` API. |
| 8 | **Extract `lib/viewer/` to its own GitHub repo** | M | Move `lib/viewer/` source out of the loft monorepo into a standalone repo (e.g. `loft-viewer` / `branch-viewer`).  Loft consumes it as a package dependency via `loft.toml`; the Java + moros consumers do the same.  Independent release cadence — the viewer can ship without waiting for a loft release.  Includes: history-preserving `git filter-repo` extraction, dedicated CI, README aimed at outside consumers (not loft developers), versioning policy, contribution-guide entry for "I want a feature my project needs."  Pre-requisite: phases 6 + 7 stable so the convention surface is validated by two real consumers BEFORE the repo split. |

## Acceptance

- `lib/viewer/` builds + tests pass.
- Loft's `tools/viewer/` continues to ship the same UI/UX
  (zero regression for the existing dashboard / welcome /
  tag / file / tree / commit / diff routes).
- Java project consumer renders its issue tracker + RFCs +
  branch dashboard with `~30 lines` of project-specific
  loft + a `.viewer.toml`.
- moros project consumer ditto.
- All three consumers exercise the same viewer binary
  surface — no per-consumer forks of viewer code.
- Documented convention surface in viewer README matches
  what all three consumers depend on.
- Phase 8 acceptance: viewer lives in its own GitHub repo
  with history preserved, independent CI, and consumers
  pull it via `loft.toml` dependency.  Outside contributors
  can land features without commit access to the loft repo.

## Risks

| Risk | Mitigation |
|---|---|
| First non-loft consumer surfaces a hard-coded assumption we missed | Phase 6 is explicitly the validator.  Treat its bug-yield as part of the plan budget, not as "scope creep." |
| Tag-prefix parameterisation breaks scan.loft's @PLAN37 self-references | The indexer is loft-internal and reads loft's config; the prefix char is a substitution. Bash scanner stays as the canonical fallback during the migration. |
| Java project's issue tracker shape diverges from the convention | If divergence is small (e.g., 5 columns instead of 4), absorb via config.  If divergence is large (GitHub Issues integration), that's outside scope — Java project uses a different tool. |
| Theme assumption (Engineering Notebook) doesn't fit Java project's brand | Phase 4 keeps theme baked.  If a consumer needs a different theme, follow-up phase ships theme presets (out of scope here). |
| Two consumers in parallel surface contradictory config needs | Sequence: ship phase 6 (Java) first, refine config based on findings, THEN ship phase 7 (moros).  One-at-a-time validation. |
| Java consumer (phase 6) lives on a different laptop | The user can drive Claude Code on that laptop directly — no remote debugging or cross-machine state shipping needed.  Phase 6 work happens IN the Java project's checkout via the same workflow loft uses. |
| Phase 8 (own repo) breaks loft's dev loop if extracted too early | Sequence: phases 6 + 7 must ship and stabilise before phase 8 starts.  Once two consumers have validated the convention surface AND `lib/viewer/` is the only place the engine lives, the repo split is mechanical (`git filter-repo` preserving history). |

## Out of scope

- Plugin systems / hooks / extension points.  Convention-
  over-configuration.
- Theme presets beyond Engineering Notebook.  Per-project
  `theme.css` override is the escape hatch.
- GitHub Issues / Linear / Jira / external-tracker integration.
  File-based only.
- Multi-project deployment of one viewer instance.  One viewer
  per project, port-forwarded separately.
- Viewer-as-package distribution.  Each consumer builds its
  own viewer binary against this lib.

## Sequencing

Phases 0-5 land in order on loft (each phase preserves
loft's current behavior).  Phase 6 then ships the Java
consumer; observed issues feed back into phases 2-5 for
fixes.  Phase 7 ships the moros consumer once the Java
consumer is stable.

The discoverability cleanups in
[`plans/44-viewer-discoverability/`](../../plans/44-viewer-discoverability/README.md)
should land BEFORE this plan starts — the cleaner the
existing viewer, the cleaner the extraction.

## Cross-references

- [`@PLAN35`](../../plans/finished/35-branch-review-viewer/README.md)
  — the original viewer plan (shipped).  This plan is the
  generalisation arc that extracts the engine.
- [`plans/44-viewer-discoverability/`](../../plans/44-viewer-discoverability/README.md)
  — the small cleanup plan that lands first.
- [`@PLAN37`](../../plans/42-tracker-index/README.md) — the
  tracker indexer the viewer reads.  Indexer also gets
  config-parameterised in phase 3 so other consumers can
  run their own.
- [`lib/markdown/`](https://github.com/loft-lang/loft-libs-docs/tree/main/markdown) — already
  generic; viewer pulls it as-is.
- [`lib/server/`](https://github.com/loft-lang/loft-libs-net/blob/main/server/src/server.loft) —
  already generic; viewer pulls it as-is.
- [`STDLIB.md § Open work`](../../STDLIB.md#open-work) —
  several stdlib gaps surfaced during viewer development;
  closing them simplifies phase 1 (helper extraction).
