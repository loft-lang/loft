<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 07 — Closeout: release binary, docs, retrofit

**Status:** Open

## Goal

Ship a tagged release of the viewer binary; wire it into the
project's user-facing docs (DEBUG.md, CLAUDE.md, CHANGELOG);
prove it works end-to-end by **using it to review the next
plan** the user picks up.  Move @PLAN35 to
`plans/finished/35-branch-review-viewer/`.

## What ships

### Release binary

- Build the viewer with the `loft.toml` pinned at the most
  recent loft commit on `main` (not tip-of-feature-branch).
- Tag the binary as `tools/viewer/bin/loft-view-v0.1` (or
  whatever the v1 acceptance criteria call for).
- Commit the binary to the repo (1-2 MB; acceptable for
  long-term reproducibility).
- Optional: attach to a GitHub release tagged
  `loft-view-v0.1` for download convenience.

### Documentation updates

#### `doc/claude/DEBUG.md` § "Branch review viewer (`make view`)"

New section under § Debugging utilities:

```markdown
## Branch review viewer (`make view`)

A frozen loft binary that serves a branch-aware doc + code
review dashboard from a browser.  Useful for reviewing
in-flight work without scrolling through chat snippets.

### Usage

In the VM:

    make view-build          # one-time, when updating
    make view                # starts server on 8765 + refreshes git state

From the host:

    ssh -L 8765:localhost:8765 vm-user@vm-host

Open `http://localhost:8765/`.

### Environment variables

| Var | Default | Purpose |
|---|---|---|
| `LOFT_VIEW_PORT` | `8765` | Listen port |
| `LOFT_VIEW_BIND` | `0.0.0.0` | Bind address (`127.0.0.1` to disable SSH-forward access) |
| `LOFT_VIEW_ROOT` | `.` | Project root |
| `LOFT_VIEW_EDITOR` | (unset) | If set, file pages show "open in editor" link |

### Workflow

The binary is **frozen** — built deliberately with `make
view-build`, not auto-rebuilt.  This means it keeps working
even when loft itself is mid-refactor.  Update the binary by
running `make view-build` against a known-good loft commit.

See [plans/finished/35-branch-review-viewer/](README.md)
for the full design.
```

#### `CLAUDE.md` § Key commands

ADD one row under the existing key-commands list:

```bash
make view                                     # branch-aware doc + code review viewer (port 8765)
```

#### `CHANGELOG.md` (user-facing)

```markdown
### Added (loft-view v0.1)

- New `make view` target launches a branch-aware markdown +
  code review viewer accessible from a browser via SSH
  port-forward.  Renders any file in the repo with line
  numbers + cross-doc links.  Dashboard shows files changed
  vs main, recent commits, and per-file diffs.  See
  [DEBUG_VIEW.md § Branch review viewer](doc/claude/DEBUG_VIEW.md).
```

#### `CHANGELOG_TECHNICAL.md`

```markdown
### Plan-35 (branch review viewer) closed YYYY-MM-DD

Built `tools/viewer/loft-view` — a frozen loft binary that
serves the project tree with markdown rendering, code-file
viewing, git-state-aware dashboard, and diff/commit views.

Per-phase summary:
- 00 Skeleton — package layout, frozen-binary contract.
- 01 HTTP routes — server skeleton, raw file serving, tree.
- 02 Code files — `<pre>` + line numbers + escape.
- 03 Markdown subset — headings, bold/italic, links, code
  blocks, lists, paragraphs.  GitHub-compatible slugs.
- 04 Git state wrapper — `refresh.sh` dumps JSON to
  `state/`; viewer reads it.
- 05 Diff + commit views — unified diffs + per-commit pages
  + Rendered/Diff toggle.
- 06 Proper tables — full GFM support; the marquee feature.
- 07 Closeout — this entry.

Loft drivers — features matured by building this:
- `lib/server` proven beyond test fixtures
- Plan-22 closures used in route handlers
- 0.8.3 coroutines used for streaming responses
- Surfaced gaps: subprocess primitive (workaround:
  refresh.sh), regex (workaround: char-by-char parsing),
  HTML escape lib (workaround: 10-line custom function)

Active plans remaining after close: <N>.
Plan moved to `plans/finished/35-branch-review-viewer/`.
```

#### `ROADMAP.md`

Remove @PLAN35 from the "active plans" / current section;
add a one-line entry to the closed section pointing at
`plans/finished/35-branch-review-viewer/`.

### Retrofit — proof of concept

Use the viewer to review the next plan the user picks up
(@PLN28 phase 5 finishing items, or whatever's next).
Specifically:

1. Open the dashboard.
2. Identify the relevant changed files in the sidebar.
3. Use rendered + diff views to review the work.
4. Document the experience in this phase doc as a
   "post-mortem" — what worked, what didn't, what would v2
   prioritise.

This is **dogfooding evidence** that the viewer is useful.

### Move to finished/

`git mv doc/claude/plans/35-branch-review-viewer
doc/claude/plans/finished/35-branch-review-viewer`.  Update
intra-plan link paths (per the @PLAN22 closeout precedent
2026-05-13: `../X` → `../../future/X` for siblings under
future/, `../finished/X` → `../X` for siblings under
finished/).

## Critical files

| Path | Action |
|---|---|
| `tools/viewer/bin/loft-view` | NEW: committed binary artifact |
| `doc/claude/DEBUG.md` | ADD § "Branch review viewer (`make view`)" |
| `CLAUDE.md` | ADD `make view` row in § Key commands |
| `CHANGELOG.md` | ADD user-facing entry |
| `doc/claude/CHANGELOG_TECHNICAL.md` | ADD @PLAN35 retrospective |
| `doc/claude/ROADMAP.md` | REMOVE @PLAN35 from active; ADD to closed |
| `doc/claude/plans/35-branch-review-viewer/` | `git mv` to `finished/` |
| `doc/claude/plans/finished/35-branch-review-viewer/*.md` | UPDATE intra-plan link paths |

## Test surface

- `make view-build && make view` works on a fresh checkout
  with `markdown` / `pygments` not installed (the binary
  doesn't depend on them — it depends on `git` + `jq` + the
  viewer's own pinned loft).
- All 10 verification steps from the README's § Acceptance
  pass on the current branch.
- DEBUG.md / CLAUDE.md / CHANGELOG.md changes render
  correctly inside the viewer itself (meta-test).
- `bash scripts/check_doc_drift.sh` reports clean.
- `cargo test --release --test doc_hygiene` passes.

## Verification

```bash
# Fresh-checkout simulation:
$ git clean -dfx tools/viewer/bin
$ make view-build         # rebuilds from source
$ make view &
$ sleep 1
$ curl -sI http://localhost:8765/ | head -1
HTTP/1.1 200 OK

# Doc + cross-link audit:
$ bash scripts/check_doc_drift.sh
clean

$ cargo test --release --test doc_hygiene 2>&1 | tail -2
test result: ok. <N> passed; 0 failed; ...
```

## Risks

| Risk | Mitigation |
|---|---|
| Binary doesn't run on a different VM (libc / arch mismatch) | Build statically if loft's native backend supports it; document target arch in DEBUG.md |
| User skips `make view-build` after pulling new viewer source | DEBUG.md prominently mentions the rebuild step; `make view` prints the binary's git-commit timestamp at startup so the user notices stale binaries |
| Phase 06 (tables) takes longer than expected and blocks closeout | Phase 07 can ship with phase 06 still open if v1 is "viewer minus great tables" — but the user has named tables as the marquee feature, so we should not split |
| Retrofit step (use the viewer to review the next plan) surfaces a critical bug | Fix it in this phase or split into `35a closeout` + `35b followup` |

## Cross-references

- [README § Acceptance](README.md#acceptance--full-plan)
- [Phase 22 closeout pattern](../22-mutable-closures/06-closeout.md) — the template this phase follows
- [`doc/claude/DEBUG.md`](../../../DEBUG.md) — destination for the user-facing usage section
- [`CLAUDE.md`](../../../../../CLAUDE.md) — destination for the key-command entry
