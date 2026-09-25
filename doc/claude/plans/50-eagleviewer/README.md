<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# eagleviewer — generic branch-aware code + docs review viewer

Successor to [PLAN35](../finished/35-branch-review-viewer/README.md).
PLAN35 shipped the loft-internal viewer (`tools/viewer/` →
`bin/loft-view`, served the loft monorepo via SSH port-forward).
This plan extracts that work to its own GitHub project, generalises
it beyond loft, and positions it as a flagship loft application.

## Status

**PROPOSED.**  Decision 2026-05-24 (after plan-12 chunking
discussion): the viewer should not be loft-specific.  It should
work on any project's git repo — Java, Rust, JS, Python — with
per-project config for tracker-ref conventions.  The fact that
it's written in loft becomes a showcase, not a coupling.

Repository: `<owner>/eagleviewer` *(name chosen 2026-05-24 —
memorable, evokes high-altitude survey; not perfectly descriptive
but punchy)*.

## Strategic positioning

eagleviewer is the loft project's first **outward-facing flagship
consumer** — a tool that:

- Has end-user value outside the loft ecosystem (Java/Rust/JS
  teams who want a lightweight branch-review viewer).
- Is written in loft, distributed as a binary.
- Demonstrates that loft is production-ready for real
  applications, not just toy programs.
- Drives real language stress: HTTP server, markdown rendering,
  syntax-aware code highlighting, git plumbing, file I/O,
  command-line args, structured config parsing.

Per the CLAUDE.md dogfood-loop philosophy: this is the next
"real consumer → harvest language lessons → ship a release"
cycle after moros, audience-demo, and lib/markdown.

## Why not part of the loft repo

PLAN35's viewer lives in `tools/viewer/`, builds against the
in-tree loft binary, serves the loft monorepo.  Extracting it:

1. **De-couples release cadence.**  Viewer improvements don't
   wait for loft minor releases.
2. **Forces a clean dependency boundary.**  Today `tools/viewer/`
   sees the loft monorepo's whole filesystem; an external
   project can only consume loft via the registry like any
   other consumer.  That's the right discipline.
3. **Removes the "loft-ism" lock-in.**  Today the viewer
   knows about `@P###` / `@PLAN##` because those are the loft
   tracker-ref conventions.  Externalising forces the
   convention to become per-project config.
4. **Makes the viewer adoptable.**  Java team can run
   `eagleviewer` against their repo without cloning loft.

## Architecture

### What stays generic

| Concern | How |
|---|---|
| Markdown rendering | Uses `lib/markdown/` (already CommonMark + GFM subset) from the loft registry.  Language-agnostic. |
| Branch / git operations | `git log` / `git diff` / `git ls-files` subprocess calls.  Language-agnostic. |
| HTTP server + WebSocket | `lib/web/` (or `lib/server/` once they split) from the registry.  Language-agnostic. |
| File I/O + path manipulation | Loft stdlib (`02_files.loft` post-Phase-3.6).  Language-agnostic. |
| Search + filter | Loft's text + vector ops.  Language-agnostic. |

### What needs per-project config

Each project the viewer is pointed at carries a config file
(`.eagleviewer/config.toml` at repo root) declaring:

```toml
# Tracker-ref families: how does this project name its
# tickets / issues / plans?  Each family is a name + regex +
# resolution URL template.
[trackers.issue]
regex = "#[0-9]+"        # GitHub-issue style
url   = "https://github.com/<owner>/<repo>/issues/$1"

[trackers.jira]
regex = "[A-Z]+-[0-9]+"  # Jira-style (JIRA-1234, INGEST-42)
url   = "https://jira.example.com/browse/$0"

[trackers.ploft]         # loft project's own convention
regex = "@P[0-9]+[a-z]?"
url   = "/docs/PROBLEMS.md#$0"
file  = "doc/claude/PROBLEMS.md"  # local file for non-URL refs

# Doc roots: directories to scan for markdown.
[docs]
paths = ["doc/", "README.md", "CHANGELOG.md"]

# Code rendering: which directories to include in browse view.
[code]
paths   = ["src/", "lib/", "tests/"]
exclude = ["target/", "node_modules/", "*.lock"]

# Branch policy: which branch is "main" (for diff view).
[branch]
main = "main"
```

The viewer's tracker-ref engine walks every file matching the
markdown + code roots, applies each family's regex, and builds
the tag index for `@click` resolution.  Replaces today's
hard-coded `@P###` / `@PLAN##` logic.

### Source code rendering / syntax highlighting

Three options, in order of preference:

1. **Client-side highlight.js or Prism.js** (Recommended.)
   Bundle one of these in eagleviewer's static assets;
   server emits raw text + language tag, client highlights.
   Supports ~200 languages out of the box, MIT-licensed,
   well-maintained.  No language detection needed in loft.

2. **Server-side tree-sitter** (Future.) More accurate
   semantic highlighting, but requires per-language grammar
   binaries and a tree-sitter loft binding.  Defer until
   client-side hits a wall.

3. **Pure-loft tokeniser** (Reject.) Building a tokeniser per
   language inside eagleviewer is a rabbit hole; loft's own
   gendoc proves how much code that requires for just loft.

Language detection: file extension + first-line shebang.
GitHub's Linguist is the canonical model but is overkill.
A 50-line table of extensions covers >95% of real-world
repos.

### Tracker-ref federation (cross-repo)

A Java repo's docs might say "see `[loft#P259](...)`" referring
to a P-issue in the loft monorepo.  eagleviewer's config
declares external references:

```toml
[trackers.loft]
regex = "loft#P[0-9]+"
url   = "https://github.com/loft-lang/loft/blob/main/doc/claude/PROBLEMS.md#$0"
external = true        # don't try to resolve locally
```

For local browsing where the cross-repo target file lives on
the same machine, an optional `external_path` points at the
filesystem copy:

```toml
external_path = "/home/jurjen/Documents/loft/doc/claude/PROBLEMS.md"
```

This is the "federation" answer — loose coupling, no central
registry of tracker refs.  Each project declares what it
points at.

### Distribution

| Channel | Form |
|---|---|
| **Source** | GitHub repo `<owner>/eagleviewer` |
| **Prebuilt binaries** | GitHub releases — per-platform (`eagleviewer-<version>-<target>.tar.gz`).  Target list: x86_64-linux-gnu, x86_64-darwin, aarch64-darwin, x86_64-windows. |
| **Loft registry** | `eagleviewer` package, published alongside binaries (registry-side binary distribution support is part of the [PKG_REGISTRY.md schema](../../PKG_REGISTRY.md) `binaries` field — currently reserved, implemented as part of this plan). |
| **Install command** | `loft install eagleviewer` (once registry-binary support lands), OR direct `curl … | tar -xz` from GitHub releases. |

The recommended invocation is `eagleviewer` (binary on PATH)
rather than `loft view` — these are decoupled tools.

## Migration from `tools/viewer/`

`tools/viewer/` in the loft monorepo is the prototype that
PLAN35 produced.  Migration path:

1. **Fork to standalone repo.**  Copy `tools/viewer/src/main.loft`
   (plus assets and `BUILD_NOTES.md`) to the new
   `<owner>/eagleviewer` repo.  Initial commit credits PLAN35
   as the origin.
2. **Generalize incrementally.**  First release of eagleviewer
   is essentially the existing viewer with `@P###` /
   `@PLAN##` hardcoded but a `.eagleviewer/config.toml`
   parser scaffolded.  Subsequent releases peel back the
   loft-isms.
3. **Loft monorepo cutover.**  Once eagleviewer is stable
   enough to serve the loft monorepo (probably v0.2.0 with
   the config system working), the loft repo's `make view`
   target switches from "build the in-tree viewer" to
   "ensure `eagleviewer` is installed, point it at this
   repo with the local config".
4. **`tools/viewer/` deletion.**  After cutover, delete
   `tools/viewer/` from the loft monorepo.  The compiled
   binary is no longer maintained in-tree.

## Phases (rough)

| Phase | Scope | Effort |
|---|---|---|
| **E1** | Bootstrap `<owner>/eagleviewer` repo from `tools/viewer/`.  Keep loft-ism hardcodes but add the config-file scaffold. | S |
| **E2** | Per-project tracker-ref config schema parsed + applied; `@P###` / `@PLAN##` removed as hardcodes. | M |
| **E3** | Generic source-language detection + client-side syntax highlighting (highlight.js bundle). | M |
| **E4** | First external user — point eagleviewer at a non-loft repo (Java or Rust), document the setup. | S |
| **E5** | Pre-built binary releases per platform; GitHub Actions workflow. | S |
| **E6** | Registry-binary distribution support (`loft install eagleviewer`).  Wires registry binary semantics from [PKG_REGISTRY.md schema](../../PKG_REGISTRY.md). | M |
| **E7** | Cutover the loft monorepo's `make view` to use external eagleviewer. | XS |
| **E8** | Delete `tools/viewer/` from loft monorepo. | XS |

## Open questions

1. **`<owner>` in the repo name** — `loft-lang/eagleviewer`,
   `jjstwerff/eagleviewer`, or its own GitHub org?  Tied to
   the broader question of how independent the project is
   from loft governance.
2. **Self-hosting auth.**  Today `make view` uses SSH
   port-forward (no auth needed — the SSH tunnel IS the
   auth).  For external adopters who run eagleviewer on a
   shared host, some auth model is needed.  Defer to E5+.
3. **Editing.**  Today the viewer is read-only.  Inline
   markdown editing (and commit-from-the-viewer for trivial
   doc fixes) is a natural extension but expands scope a lot.
   Park as a future phase.
4. **Multi-repo browsing.**  Single eagleviewer instance
   serving multiple repos via a switcher in the UI.  Useful
   when you maintain several loft chunks.  Park.
5. **Search.**  Cross-repo grep + tracker-ref-aware search.
   Park.

## Why now

The plan-12 chunking work + the strategic pivot to
loft-as-application-platform (moros JS-to-loft migration,
dryopea greenfield) make eagleviewer's positioning crisp:
it's the first **non-game** flagship loft application.  Game
projects exercise graphics, physics, real-time loops; a dev
tool exercises HTTP, parsing, markdown, file I/O,
command-line args — a complementary surface that catches
language gaps games don't.

The work is non-blocking on plan-12 — it can interleave with
the chunk extractions or wait until after.  Phases E1–E4 can
start any time; E5+ benefit from PKG.REG R10 (registry-binary
support) being live.
