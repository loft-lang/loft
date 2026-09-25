<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 07a — Indexer → viewer push protocol (WebSocket)

**Status:** Design — implementation gated on `lib/fs_watch/`
landing (the indexer daemon needs file events to know when
to push).

## Goal

Cut the polling round-trip out of the dev loop.  Today:

```
edit doc → save → run `make index` → run `make view-refresh`
                ↑                    ↑
                │                    └─ rebuild git state JSON
                └─ rebuild index/tags.json (or git pre-commit hook)
            then                 reload the browser tab
```

After phase 07a:

```
edit doc → save
                │
                ↓
       indexer daemon detects fs event
                │
                ├─ rebuilds the affected tag rows
                │  (delta — not the full repo)
                │
                └─ pushes the delta to every connected viewer
                   over WebSocket (loopback only)
                            │
                            └─ viewer marks affected pages stale;
                               user F5s, or open pages auto-swap
                               their stale sections
```

No polling.  Single source of truth (the daemon's in-memory
tag table).  Disk file `index/tags.json` is still written
atomically on each rebuild for the bash CLI + git-grep
fallback path.

## Why WebSocket (not SSE / HTTP polling / shared mmap)

Considered alternatives:

| Transport | Why not |
|---|---|
| **HTTP polling** | Round-trip cost dominates; viewer would poll every 1-2 s during a typing burst.  Defeats "no polling" goal. |
| **Server-Sent Events** | Unidirectional only.  We want bidirectional later: viewer subscribes to a subset (`subscribe @P259`), reports view state, requests on-demand context excerpts. |
| **Shared mmap** | Same machine works, but doesn't generalize to the multi-project / remote-VM-via-port-forward case (@PLN42 phase 08).  WebSocket forwards through a single SSH `-L` cleanly. |
| **Unix domain socket** | Same generalisation problem.  WebSocket with a binary-frame option (which `lib/server` already supports) gives us local-fast plus remote-friendly without a second transport. |

`lib/server` already ships the WebSocket plumbing (`ws_send`,
`ws_send_binary`, `ws_broadcast`, the `run(on_event)` driver
loop).  Reusing it means:

- One transport across all loft-side tooling (viewer
  multi-client server, indexer daemon, future LSP bridge).
- The library's edge cases (frame fragmentation, idle sleep,
  connection lifecycle) get exercised by a second
  production consumer — pressure that's already shaped
  earlier issues (multiplayer_v2 stack snapshot bug
  `@P229`).
- No new host-bridge code.

## Wire format

Text frames; one event per frame.  Format:

```
<verb>:<noun> <args>
```

Verbs are short (≤ 8 chars) so the prefix-match dispatch in
`lib/server`'s `run` loop stays cheap.  All frames have a
`<msg_id>:` numeric prefix per `lib/server`'s existing
multi-client convention; the verbs below are the payload
that follows.

### Server → client frames

| Verb | Args | Meaning |
|---|---|---|
| `snapshot` | `<json-blob>` | Full `index/tags.json` payload.  Sent once on connect. |
| `delta` | `<file>\n<json-blob>` | One file's tag-row set was rewritten (file edited / created / deleted).  `<json-blob>` is the new array of `{tag, line, context}` for that file (empty array → file deleted). |
| `bucket` | `<name>\n<json-blob>` | Auxiliary bucket replacement (`problems_open`, `plans_active`, `plans_recent`, etc.).  Sent when the source they derive from changes (PROBLEMS.md, plan dirs). |
| `commit-pulse` | `<epoch>` | Marks the end of a debounced batch.  Multiple `delta` / `bucket` frames can fire in close succession (a `git checkout` rewrites hundreds of files); `commit-pulse` says "the batch is done, you can re-render now". |
| `error` | `<text>` | Daemon-side problem (e.g. `index/tags.json` write failed).  Viewer surfaces as a banner; doesn't disconnect. |

### Client → server frames

Phase 07a is push-dominated; the client surface is small.

| Verb | Args | Meaning |
|---|---|---|
| `subscribe` | `*` or `tag:@P259` or `file:doc/claude/PROBLEMS.md` | Filter spec.  `*` (default on connect) gets all pushes; `tag:` / `file:` filters limit fan-out for narrow consumers (a single `/tag/<tag>` page needn't receive every unrelated delta).  Multiple `subscribe` calls add filters; client receives the union. |
| `unsubscribe` | same arg | Removes a previously-added filter.  After all filters are removed, the client receives nothing until it `subscribe`s again. |
| `ping` | (none) | Daemon replies with `pong`.  Idle-keepalive for VM-hosted viewers behind aggressive NATs. |

### Why text frames (not binary)

The full snapshot is on the order of 100-500 KB JSON.  A
text frame:

- Keeps the CLI debug story trivial (`websocat ws://localhost:NNNN`,
  paste `*\nsubscribe`, watch frames stream in).
- Makes the test gate easy — assert exact frame text.
- Costs one extra UTF-8 validation per frame, dwarfed by
  the JSON parse cost on either end.

Binary frames are reserved for a stretch goal: per-file
diff blobs that the viewer's `/diff/<path>` page would
fetch on demand without an HTTP round-trip.  Out of scope
for phase 07a.

## Daemon lifecycle

```
loft-index --daemon [--port NNNN] [--filter <glob>...]
```

1. **Startup** — full scan, write `index/tags.json` atomically,
   bind `127.0.0.1:NNNN` (default 8766; viewer's HTTP server
   uses 8765, so `view-port + 1`), accept WebSocket
   connections.

2. **Connect** — client gets `<msg_id>:snapshot:<json-blob>`
   immediately.  Default filter is `*` so every later push
   reaches them until they `subscribe` to something narrower.

3. **File event** — `lib/fs_watch/` (separate phase 07
   dependency) calls back with `{path, kind}` (kind ∈
   {Created, Modified, Deleted, Renamed}).

4. **Debounce** — collect events for 200 ms (editor save
   bursts, `git checkout` cascades) into a per-path set.

5. **Rescan** — for each unique changed path, re-scan that
   ONE file (not the whole tree).  Diff against the stored
   row set for that path.

6. **Push** — broadcast `delta` frame per affected file;
   broadcast `bucket` frame per affected bucket (only when
   the changed file was `PROBLEMS.md`, a plan README, etc.);
   broadcast `commit-pulse <epoch>` to mark batch end.

7. **Disk write** — apply the same diffs to the in-memory
   tag table, serialise to a temp file, atomically rename to
   `index/tags.json` (the viewer's disk-fallback path keeps
   working).

8. **Shutdown** — SIGINT / SIGTERM closes the WebSocket
   listener cleanly, flushes pending writes, exits.  Viewer
   detects the disconnect, falls back to disk reads.

## Viewer integration

`tools/viewer/src/main.loft` gains:

1. **WebSocket client** — connects to the daemon on startup;
   single connection; auto-reconnect on disconnect (1 s
   backoff, capped at 10 s).  Default filter `*`.

2. **In-memory cache** — replaces / augments the per-request
   `file("index/tags.json")` reads with a closure-captured
   snapshot that the WebSocket handler keeps current.  Disk
   reads remain as the fallback when the daemon is down.

3. **Push UI surface** — a small "live" indicator in the
   page header (green dot when connected, grey when
   disconnected with disk-fallback active).  No automatic
   page reload — pressing F5 reads the closure-captured
   snapshot, which is always at most one `commit-pulse`
   stale.

   Stretch (phase 07b): tag-affected page sections add a
   `data-stale` attribute when their underlying tag's
   `delta` arrives; a tiny inline JS adds an "[update available]"
   banner that swaps the section without a full reload.
   Optional — the no-JS baseline ships first.

4. **Welcome page** (`/welcome`) and **dashboard** (`/`) —
   bucket reads (`problems_open`, `plans_*`) come from the
   in-memory cache, no JSON re-parse per request.

## Failure modes

| Failure | Behaviour |
|---|---|
| Daemon not running | Viewer shows "live: off" indicator; reads `index/tags.json` from disk on every request (the current behaviour).  The bash scanner + git pre-commit hook keep the disk file fresh enough for non-live use. |
| Daemon crashes mid-session | Viewer detects WebSocket close, surfaces "live: off"; auto-reconnect attempts with backoff; falls back to disk reads in the meantime. |
| Multiple viewers on one daemon | Daemon broadcasts to all; per-client filter state is per-connection.  Reading is read-only so no consistency risk. |
| Daemon disk-write fails | Daemon logs `error:<text>` to clients and to its own log; in-memory state stays correct; clients can still query.  Disk file may become stale until the next successful write. |
| Network partition (impossible on loopback, but for SSH-forwarded VMs) | Same as "daemon crashes" — connection close drives the fallback. |
| Slow client | `lib/server`'s WebSocket implementation already handles back-pressure (drops or closes overloaded clients).  Phase 07a doesn't add new policy. |

## Bootstrap & dev workflow

```
make index-watch    # starts indexer daemon, blocks
make view           # starts viewer (separate terminal), connects
                    # automatically; falls back to disk if daemon
                    # not running
```

`make view` works standalone (no daemon needed) — backward
compatible with the current workflow.  `make index-watch`
adds the live-update surface without changing anything else.

## Test gate (`tests/index_hygiene.rs::live_push_smoke`)

Smoke test scaffold (gated on `lib/fs_watch/`):

```rust
#[test]
fn live_push_smoke() {
    // Start daemon on a free port.
    let port = pick_free_port();
    let daemon = spawn_daemon(port);

    // Connect a WebSocket client.
    let mut ws = connect(format!("ws://127.0.0.1:{port}"));
    let snap = ws.read_frame();
    assert!(snap.starts_with("snapshot:"));

    // Touch a tracked file.
    let path = scratch_file("@P259 example\n");

    // Expect a delta frame within 1 s.
    let delta = ws.read_frame_within(Duration::from_secs(1));
    assert!(delta.starts_with("delta:"));
    assert!(delta.contains("@P259"));

    // Expect commit-pulse closing the batch.
    let pulse = ws.read_frame_within(Duration::from_secs(1));
    assert!(pulse.starts_with("commit-pulse:"));

    daemon.shutdown();
}
```

The smoke test is the minimum gate: it doesn't validate the
delta's exact JSON content (handled by phase 07's
`index_hygiene_clean`), just that the wire shape is what
the viewer expects.

## Open questions

1. **Per-tag fan-out** — should the daemon split a single
   file's `delta` into per-tag `delta` frames so a viewer
   subscribed only to `tag:@P259` doesn't receive
   irrelevant tags?  Trade-off: more frames, less per-frame
   parse cost on the client.  Default: keep file-grained,
   let clients filter on `tag` field.

2. **Snapshot compression** — `index/tags.json` is ~500 KB
   uncompressed and most of it is repetitive (file paths,
   tag names).  WebSocket's `permessage-deflate` would cut
   that ~5×.  Defer until measured (snapshot is sent once
   per connect; not the hot path).

3. **Schema versioning** — first frame after `snapshot`
   could be `version:<schema>`; clients refusing the schema
   close the connection.  Defer until a breaking change
   actually lands.

4. **Plan-08 integration** — multi-project mode means N
   daemons, one per project root.  Viewer needs to know
   which daemon to talk to per page.  Resolved by the
   per-project config (`.tracker/config.toml` from phase
   08) listing the daemon's port.

## Cross-references

- [Phase 07 — loft-native scanner](07-loft-native-scanner.md)
  — the daemon this protocol rides on.  Phase 07a depends
  on phase 07's JSON-emit slice + `lib/fs_watch/` landing.
- [Phase 04 — viewer integration](04-viewer-integration.md)
  — the viewer-side surface this push protocol upgrades.
  Phase 04b shipped a `/welcome` route that reads
  `index/tags.json` from disk per request; phase 07a moves
  that read to a closure-captured snapshot kept current
  by the WebSocket subscription.
- [Phase 08 — multi-project + mmap index](08-multi-project-deploy.md)
  — generalises the daemon to one-per-project; per-project
  config lists the daemon port the viewer connects to.
- [`lib/server/src/server.loft`](https://github.com/loft-lang/loft-libs-net/blob/main/server/src/server.loft)
  — the WebSocket plumbing this protocol reuses (`run`,
  `send_to`, `broadcast`, `WsEvent`).
- [`tools/viewer/src/main.loft`](../../../../tools/viewer/src/main.loft)
  — the viewer that becomes a WebSocket client in this
  phase.
