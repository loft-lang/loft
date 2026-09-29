// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debugging Strategy: repo browsing tools

The tracker-tag indexer (`make index`) and the branch review viewer (`make view`).  Part of the debugging guide: [DEBUG.md](DEBUG.md).

---

## Tracker-tag indexer (`make index` + `./scripts/idx`)

The tracker-tag indexer (@PLN42) maintains
`index/tags.json`, a structured map of every `@P-id` /
`@PLAN-id` reference in the tree (plus the `legacy:`
bare-name forms during the migration).  Replaces
`grep -rn '@P259'` with O(1) JSON lookups.

### Usage

```bash
make index                           # rebuild index/tags.json
./scripts/idx tag:@P259              # exact tag → JSON refs
./scripts/idx prefix:@PLAN22         # all PLAN22-* tags
./scripts/idx file:doc/.../X.md      # tags in one file
./scripts/idx all | jq '.[:10]'      # top-N by reference count
./scripts/idx help                   # full usage block
```

### Auto-refresh on commit

After fresh checkout, install the pre-commit hook so
`index/tags.json` stays fresh whenever you commit doc or
code changes:

    make index-install-hook

The hook is idempotent (re-running won't double-install)
and safe with existing pre-commit content (it appends a
marker-bracketed block).  The hook re-runs the scanner
when any `*.md`, `*.rs`, `*.loft`, `*.toml`, `*.py`, or
`*.sh` file is staged; commits that only touch other
paths skip the scan.  Adds ~1 sec to commits that touch
indexed files.

If the scanner fails for any reason, the hook prints a
warning but does NOT block the commit (broken hooks
erode trust faster than stale index data does).

### Where it lives

| Path | Purpose |
|---|---|
| `tools/indexer/scan.sh` | The scanner |
| `tools/indexer/install-hook.sh` | Hook installer (idempotent) |
| `tools/indexer/ARCHITECTURE.md` | Design notes |
| `scripts/idx` | CLI query wrapper |
| `index/tags.json` | Output (gitignored) |
| `doc/claude/plans/42-tracker-index/` | Plan + per-phase docs |

---

## Branch review viewer (`make view`)

A loft-script binary that serves a branch-aware doc + code review
dashboard from a browser.  Useful for reviewing in-flight work
without scrolling through chat snippets.  Built by @PLAN35 (closed
2026-05-14); lives in `tools/viewer/` + `lib/markdown/`.

### Usage

On the machine holding the checkout:

```bash
make view-build          # one-time, when updating the host loft binary
make view                # refreshes git state + starts the server
make view-refresh        # refreshes git state without restarting the server
```

It prints the URL it took.  Port: `LOFT_VIEW_PORT`, else **8765** — set it when two
checkouts (or two people) share a host, because the bind is fatal on a taken port.
A value that is unset, unparseable, or outside 1–65535 falls back to 8765.

From your own machine:

```bash
ssh -N -L 8765:127.0.0.1:8765 user@host        # then http://localhost:8765/
```

⚠⚠ **The viewer binds LOOPBACK only, and that matters more off a VM than on one.**
It serves `/raw/<path>` — the contents of any file under the project root — so a
wildcard bind publishes the working tree to anyone who can reach the port.  This was
written for a VM, where that is invisible and harmless; on a shared or remote host it
is neither, **and your own tunnel works identically either way, so nothing tells you.**
`server::listen_on("127.0.0.1", …)` is what the server library calls the safe default
for exactly this, and it costs a tunnel user nothing: `-L` has sshd connect from the
remote's own loopback.

⚠ It needs `server >= 0.5`; 0.3.x has no `listen_on` and would bind `0.0.0.0`.  The
manifest states that floor so a resolver cannot quietly pick a version where the safe
bind does not exist.

⚠ **Serving a game or data server alongside it:** they are separate listeners, so
forward both — `ssh -N -L 8765:127.0.0.1:8765 -L 9000:127.0.0.1:9000 user@host`.  A
single-channel alternative is `ssh -D 1080` (SOCKS).  Proxying the game through the
viewer is possible but not free: `server`'s response API is text-only, so binary frames
and WebSockets do not pass through it, and it would put the review tool on the game's
critical path.

### Pointing it at another project (moros, dryopea, a consumer repo)

The viewer is `#cwd`: **its project root is the directory it is RUN in**, not the directory
it lives in. So one loft checkout can review any repo on the box — nothing is copied and
nothing of the viewer is installed into the other tree.

Two knobs make that safe, and both matter:

- `LOFT_VIEW_STATE` — where `refresh.loft` dumps the git state. It defaults to
  `tools/viewer/state`, which is *relative to the reviewed repo*, so without this a run
  would create `tools/viewer/state/` **inside someone else's project**. Point it at a
  gitignored path there.
- `LOFT_VIEW_PORT` — 8765 is loft's own. Two viewers on one box must differ, and the bind
  is fatal on a taken port.

Drop this in the other project's `Makefile` (verified against `moros`):

```make
# ── loft-view: browse this repo — docs, code, diffs vs main — in a browser ──
# Needs a loft checkout; point LOFT at it.  Nothing is installed here: the viewer
# runs from the loft tree and serves THIS directory.
LOFT       ?= ../loft
VIEW_PORT  ?= 8766
VIEW_STATE := .loft/view-state
VIEW_PID   := .loft/view.pid
VIEW_LOG   := .loft/view.log

.PHONY: view view-stop view-log

view: view-stop                       ## Start the loft viewer in the background
	@test -x $(LOFT)/target/release/loft || { \
	    echo "no loft binary at $(LOFT)/target/release/loft — run 'cargo build --release' there,"; \
	    echo "or point LOFT at your loft checkout: make view LOFT=/path/to/loft"; exit 1; }
	@mkdir -p $(VIEW_STATE)
	@# git state for the dashboard.  Tolerated on failure: it is a nice-to-have, and
	@# a large repo can trip loft#1061 (a placed library's return crossing) after
	@# writing most of it.  The viewer serves fine without the diff half.
	@LOFT_VIEW_STATE=$(VIEW_STATE) $(LOFT)/target/release/loft --interpret \
	    --lib $(LOFT)/lib $(LOFT)/tools/viewer/refresh.loft >$(VIEW_LOG) 2>&1 || true
	@LOFT_VIEW_PORT=$(VIEW_PORT) LOFT_VIEW_STATE=$(VIEW_STATE) nohup \
	    $(LOFT)/target/release/loft --native-release \
	    --lib $(LOFT)/lib $(LOFT)/tools/viewer/src/main.loft >>$(VIEW_LOG) 2>&1 & \
	    echo $$! > $(VIEW_PID)
	@sleep 2
	@echo "loft-view: http://127.0.0.1:$(VIEW_PORT)/    (make view-stop | make view-log)"
	@echo "  remote:  ssh -N -L $(VIEW_PORT):127.0.0.1:$(VIEW_PORT) <host>"

view-stop:                            ## Stop it (safe to run when it is not running)
	@if [ -f $(VIEW_PID) ] && kill -0 $$(cat $(VIEW_PID)) 2>/dev/null; then \
	    pkill -P $$(cat $(VIEW_PID)) 2>/dev/null; kill $$(cat $(VIEW_PID)) 2>/dev/null; \
	    echo "loft-view stopped"; \
	fi; rm -f $(VIEW_PID)

view-log:                             ## Tail its log
	@tail -40 $(VIEW_LOG) 2>/dev/null || echo "no log yet"
```

⚠ **`view` depends on `view-stop`, deliberately** — so `make view` is a RESTART and is safe
to run repeatedly. An agent re-running it does not accumulate servers or hit "cannot bind
(already in use)", which is fatal.

⚠ **`.loft/` must be gitignored in the target repo** (it is in moros, via `**/.loft/`), so
the state, pid and log never appear in `git status`. Check before adopting.

⚠ The **first** run compiles the viewer (~6 s, cached in the loft tree afterwards), so the
first `make view` is slower than the rest. Nothing is written to the reviewed repo except
the three `.loft/` files above.

### Routes

| Path | Renders |
|---|---|
| `/` | Branch dashboard — branch name + ahead/behind vs `main` + HEAD sha/msg, changed-files list, uncommitted-files list, last 20 commits.  Status badges (M/A/D/R) on every changed file. |
| `/file/<path>` | File view.  `.md` files render via `lib/markdown` (full subset: ATX + setext headings with GH-slug ids, lists with continuation merging, GFM tables with alignment, fenced code, inline formatting, links with relative-path resolution + title attribute, images via `/raw/`, autolinks `<https://…>` / `<email>`, `@P-id` / `@PLAN-id` autolinks, blockquotes, task lists, strikethrough, backslash escapes).  Other files render line-numbered with `<a id="L42">` anchors. |
| `/diff/<path>` | Per-file unified diff vs `main` with hunk colouring (green +, red −, blue hunk header). |
| `/commit/<sha>` | Commit message + per-file diffs via the same hunk-coloured renderer.  Last 20 commits captured. |
| `/tag/<bare>` | Every tracker-tag reference for a P-id or PLAN-id (e.g., `/tag/P259` lists all references to `@P259` and `legacy:P259`).  Reads `index/tags.json` built by `make index` (@PLN42). |
| `/tree/<path>` | Directory listing; sub-dirs are clickable. |
| `/raw/<path>` | Raw file bytes (`text/plain`).  Used by markdown to serve relative image refs. |

### File-page view toggle

Every `/file/<path>` page shows a `[Rendered ¦ Diff vs main]`
toggle in the top-right.  When the file is unchanged on the
current branch (no per-file diff), the "Diff vs main" link
hides — only "Rendered" stays.

### Architecture

| Layer | Where | Purpose |
|---|---|---|
| Server + routes + page templates | `tools/viewer/src/main.loft` | Loft script — HTTP server via `lib/server`, route dispatch, dashboard / tag-page / commit-page / diff-page / file-page rendering |
| Markdown rendering | `lib/markdown/` | Standalone loft library — single-file `src/markdown.loft`, comprehensive `tests/01-render.loft` |
| Git state | `tools/viewer/state/*.json` + `state/diffs/*.diff` + `state/commits/*.diff` | Filled by `tools/viewer/refresh.sh` (uses `git` + `jq`) |
| Tracker-tag index | `index/tags.json` | Filled by `make index` (@PLN42) |
| Static CSS | embedded in `main.loft::BASE_CSS` | Light + dark via `prefers-color-scheme` |

### Dependencies

- **`git`** — used by `refresh.sh` to dump branch state
- **`jq`** — used by `refresh.sh` to safely emit JSON
- **The host loft binary** at `target/release/loft` — built via
  `make view-build`; the viewer is a loft script interpreted
  by it (or `--native`-compiled via the same binary)

No Python, no markdown lib, no syntax-highlighter dep, no
template engine.  All rendering is loft-native through `lib/markdown`
+ string concatenation in `main.loft`.

### Frozen-binary contract

The viewer source (`tools/viewer/src/main.loft`) and the host
loft binary it runs against form a deliberately **frozen pair**.
`make view-build` rebuilds the host binary; `make view` runs
the existing one.  This means the viewer keeps working through
loft refactors — refresh by running `make view-build` against
a known-good loft commit.

### Backends

The viewer runs under **both `--interpret` and `--native`**
(since the seven-bug native arc @P262→@P269 closed 2026-05-13).
`make view` invokes `--interpret` by default for fast iteration;
edit the Makefile target to swap in `--native` for the faster
steady-state runtime.

### Troubleshooting

- **Dashboard shows "No git state. Run `make view-refresh`"** —
  the refresh script hasn't built `tools/viewer/state/*.json`
  yet.  Run `make view-refresh`.
- **`/tag/<bare>` shows "No index found"** — `index/tags.json`
  is missing.  Run `make index`.
- **`/diff/<path>` shows "No diff captured"** — the file isn't
  on the changed-files list (no diff vs `main`), OR refresh.sh
  capped at 100 changed files.  Run `make view-refresh`.
- **`/commit/<sha>` shows "No diff captured"** — refresh.sh
  only keeps the last 20 commits.  For older commits, run
  `git show <sha>` directly.

### See also

- [`plans/finished/35-branch-review-viewer/README.md`](plans/finished/35-branch-review-viewer/README.md) — the full design + per-phase build log
- [`lib/markdown/loft.toml`](https://github.com/loft-lang/loft-libs-docs/blob/main/markdown/loft.toml) — the rendering library
