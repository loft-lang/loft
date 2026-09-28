<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 04 — Git state via wrapper script

**Status:** Open

## Goal

Make the viewer's landing page **branch-aware**.  Loft has no
subprocess primitive (per the recon), so the viewer cannot
shell out to `git` directly.  Instead, a small bash wrapper
script (`tools/viewer/refresh.sh`) runs git commands and dumps
their output as JSON / plain-text files into
`tools/viewer/state/`.  The viewer reads those files at
request time and renders the dashboard.

Refreshing the dashboard means re-running `make view`
(or just `make view-refresh` to dump state without restarting
the server).

The output of this phase is a **branch dashboard at `/`** that
shows: branch name + ahead/behind, files changed vs `main`,
recent commits, uncommitted changes.

## What ships

### Wrapper script — `tools/viewer/refresh.sh`

```bash
#!/usr/bin/env bash
# tools/viewer/refresh.sh — dump git state for loft-view to consume.
# Loft has no subprocess primitive yet; this script is the bridge.
# Re-run on demand: `make view-refresh` or as part of `make view`.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
STATE="$ROOT/tools/viewer/state"
mkdir -p "$STATE"

cd "$ROOT"

# 1. Branch header
{
  echo "{"
  printf '  "branch": "%s",\n' "$(git rev-parse --abbrev-ref HEAD)"
  printf '  "head_sha": "%s",\n' "$(git rev-parse --short HEAD)"
  printf '  "head_msg": %s,\n' "$(git log -1 --pretty=%s | jq -Rs .)"
  read -r ahead behind <<< "$(git rev-list --left-right --count main...HEAD | tr '\t' ' ')"
  printf '  "ahead": %d,\n' "$ahead"
  printf '  "behind": %d\n' "$behind"
  echo "}"
} > "$STATE/branch.json"

# 2. Files changed vs main (name-status format)
git diff --name-status main...HEAD | jq -Rsn '
  [inputs | split("\t") | {status: .[0], path: .[1]}]
' > "$STATE/changed.json"

# 3. Recent commits (last 20)
git log --oneline -20 --pretty='%h%x09%s' | jq -Rsn '
  [inputs | split("\t") | {sha: .[0], msg: .[1]}]
' > "$STATE/commits.json"

# 4. Uncommitted (porcelain v1)
git status --short | jq -Rsn '
  [inputs | {status: .[0:2] | gsub(" "; ""), path: .[3:]}]
' > "$STATE/uncommitted.json"

# 5. Per-file diffs vs main (capped at 100 changed files for sanity)
DIFFS_DIR="$STATE/diffs"
rm -rf "$DIFFS_DIR" && mkdir -p "$DIFFS_DIR"
git diff --name-only main...HEAD | head -100 | while read -r f; do
  safe="${f//\//__}"
  git diff main...HEAD -- "$f" > "$DIFFS_DIR/$safe.diff"
done

echo "loft-view state refreshed: $(date)"
```

`jq` is a hard dep for the script — already pre-installed in
most VMs; if not, the message at script start says "install
jq".  Acceptable trade-off: a 5-line bash script with no `jq`
would need handcrafted JSON quoting and that's a maintenance
hazard.

### Makefile targets

```make
view-refresh:  ## Dump git state to tools/viewer/state/
	./tools/viewer/refresh.sh

view: view-refresh  ## (UPDATED) Refresh git state, then run binary
	@if [ ! -x tools/viewer/bin/loft-view ]; then \
	   echo "loft-view not built; run: make view-build"; \
	   exit 1; \
	fi
	./tools/viewer/bin/loft-view
```

`make view` now refreshes state every invocation.  The user
can also run `make view-refresh` while the server is running
to update state without restarting.

### Viewer-side: state reader

`tools/viewer/src/state.loft`:

```loft
struct BranchState {
    branch: text,
    head_sha: text,
    head_msg: text,
    ahead: integer,
    behind: integer
}

struct ChangedFile {
    status: text,    // "M", "A", "D", "R..."
    path: text
}

struct Commit {
    sha: text,
    msg: text
}

pub fn read_branch() -> BranchState { ... }
pub fn read_changed() -> vector<ChangedFile> { ... }
pub fn read_commits() -> vector<Commit> { ... }
pub fn read_uncommitted() -> vector<ChangedFile> { ... }
pub fn read_diff(path: text) -> text { ... }   // path → safe filename → file
```

Each reader:
1. Opens the JSON file under `tools/viewer/state/`.
2. Parses with the standard library's JSON parser (per
   loft's `n_struct_from_jsonvalue` infrastructure).
3. Returns the typed result.

If a state file is missing (refresh.sh hasn't run): show a
"refresh state with `make view-refresh`" placeholder on the
dashboard rather than crashing.

### Dashboard route

`/` becomes the branch dashboard:

```html
<header>
  <h1>loft-view — branch <code>{branch}</code></h1>
  <p>{ahead} ahead, {behind} behind <code>main</code></p>
  <p>HEAD: <code>{head_sha}</code> {head_msg}</p>
</header>

<section>
  <h2>Changed files (vs main)</h2>
  <ul>
    {for each ChangedFile:
       <li><span class="status-{status}">{status}</span>
           <a href="/file/{path}">{path}</a></li>}
  </ul>
</section>

<section>
  <h2>Uncommitted</h2>
  ... same shape ...
</section>

<section>
  <h2>Recent commits</h2>
  <ol>
    {for each Commit:
       <li><a href="/commit/{sha}"><code>{sha}</code></a> {msg}</li>}
  </ol>
</section>
```

`/commit/<sha>` is a stub for phase 05 — for now it shows
"phase 05 will land this; here's the message: …".

### Status badges

CSS color-codes the status badges:
- `M` (modified): yellow background
- `A` (added): green
- `D` (deleted): red
- `R` (renamed): blue
- `?` (untracked): gray

## Critical files

| Path | Action |
|---|---|
| `tools/viewer/refresh.sh` | NEW (the wrapper script) |
| `tools/viewer/src/state.loft` | NEW (state readers) |
| `tools/viewer/src/route.loft` | UPDATED: `/` becomes dashboard, `/commit/<sha>` becomes stub |
| `tools/viewer/src/style.loft` | UPDATED: status-badge CSS |
| `Makefile` | ADD `view-refresh:` target; UPDATE `view:` to depend on it |

## Existing functions / tooling to reuse

- **`File` primitive** for reading state files.
- **JSON parsing** — loft's standard JSON parser
  (`n_struct_from_jsonvalue` infra per
  `doc/claude/QUALITY.md`).  Verify against the JSON spec
  used by P54 (JsonValue enum); fall back to a tiny
  hand-written JSON parser if loft's JSON isn't yet
  ergonomic for parsing into `vector<struct>`.
- **`git` CLI** in the wrapper script — standard.
- **`jq`** for safe JSON quoting — flag as a dependency in
  the script's first line.

## Test surface

- `./tools/viewer/refresh.sh` runs without error on a clean
  branch.
- `tools/viewer/state/branch.json` parses as valid JSON
  (`jq . tools/viewer/state/branch.json` exits 0).
- `cat tools/viewer/state/changed.json | jq 'length'` matches
  `git diff --name-status main...HEAD | wc -l`.
- `make view` brings up the dashboard at `/`; sidebar shows
  status badges next to changed files.
- Edit a file (uncommitted change), `make view-refresh`,
  refresh browser → file appears in "Uncommitted" section.

## Verification

End-to-end on `demo_dev` (currently 18 ahead of main):

```bash
$ make view-refresh
loft-view state refreshed: ...

$ jq . tools/viewer/state/branch.json
{
  "branch": "demo_dev",
  "head_sha": "a2eef643",
  "head_msg": "docs(plan-22): closeout — move to finished/, update cross-refs",
  "ahead": 18,
  "behind": 0
}

$ jq 'length' tools/viewer/state/changed.json
12   # or whatever the actual count is

$ make view &
$ curl -s http://localhost:8765/ | grep -o 'demo_dev'
demo_dev

$ curl -s http://localhost:8765/ | grep -c 'class="status-M"'
# matches the M-status count from changed.json
```

## Risks

| Risk | Mitigation |
|---|---|
| `jq` not installed in user's VM | `refresh.sh` first line: `command -v jq >/dev/null || { echo "needs jq: apt install jq"; exit 1; }` |
| Loft's JSON parser ergonomically painful for `vector<ChangedFile>` | If the standard parser path is rough, fall back to per-line parsing of a custom format (one path-per-line, status as first byte).  File the parsing pain as a P-issue against the JSON ecosystem (see `doc/claude/QUALITY.md` § JSON ecosystem) |
| `state/diffs/` directory grows unbounded across `git checkout`s of different branches | `refresh.sh` `rm -rf "$DIFFS_DIR"` before re-creating; cap at 100 files per refresh |
| Dashboard is stale if user `git commit`s but doesn't `make view-refresh` | Document the workflow; `make view` refreshes; "stale state" timestamp on the dashboard makes it obvious |
| Path with weird chars (`spaces`, `unicode`) in `safe` filename mapping | Use a base64 encoding of the path as the safe filename; or document the path-safety constraints |

## Cross-references

- [Phase 05 — diff and commit views](05-diff-and-commit.md) — consumer of `state/diffs/` + `/commit/<sha>` route
- [`doc/claude/QUALITY-history.md` § JSON ecosystem](../../../QUALITY-history.md) — JSON parser story
- [Plan-04 — JsonValue (P54)](../../../QUALITY.md) — what the dashboard's JSON parsing depends on
- [Plan-22 README § Loft has no subprocess primitive](README.md#out-of-scope-deferred--separate-plans) — the architectural reason for the wrapper-script approach
