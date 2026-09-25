<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `lib/fs_watch/` — file-event watcher

**Status:** Future — opened 2026-05-15 from @PLAN37 phase 07a
(WebSocket push design) which is gated on this lib landing.

## Why

@PLAN37 phase 07a (indexer → viewer WebSocket push) needs
file events to know when to broadcast deltas — without them
the daemon would have to poll the filesystem, defeating the
"no polling" goal.  Same pattern any future "live reload" /
"continuous test" / "auto-rebuild" tool would need.

## Surface

```loft
enum FsEventKind { Created, Modified, Deleted, Renamed }

struct FsEvent {
  kind: FsEventKind,
  path: text,
  // For Renamed only:
  old_path: text
}

pub fn watch(root: text) -> iterator<FsEvent>;
```

The iterator is infinite — yields events as the OS reports
them, blocking when none are pending.  Subscribers run in a
loop:

```loft
for ev in fs_watch::watch("doc") {
  println("changed: {ev.path}");
}
```

Bounded variant for "drain pending then exit":

```loft
pub fn drain(root: text, timeout_ms: integer) -> vector<FsEvent>;
```

## What ships

- `lib/fs_watch/src/fs_watch.loft` — the API above as
  `#native` fns.
- Host bridge (Rust): `notify` crate (cross-platform — uses
  inotify on Linux, kqueue on macOS, ReadDirectoryChangesW
  on Windows).  Wraps the `RecommendedWatcher` + a `Receiver`
  in a `Mutex` so the iterator's `next()` can `recv()` cleanly.
- Tests: create + modify + delete a file in a scratch dir,
  assert the matching events arrive within 200 ms.

## Consumer changes once shipped

- `tools/indexer/src/scan.loft --watch` becomes possible:
  full scan on startup, then `for ev in fs_watch::watch(".")
  { rescan_one(ev.path); broadcast_delta(...); }`.
- @PLAN37 phase 07a's WebSocket daemon ships.
- Future: `make test --watch` (re-run tests on every save),
  `make view --live` (auto-reload viewer pages).

## Effort

L (week+).  The Rust `notify` crate gives most of the
implementation; the work is the loft-side iterator-as-blocking-
recv shim, the cross-platform path-normalization (Windows
backslash vs forward slash; macOS FSEvents debounce
behavior), and the test harness for events that fire
asynchronously.

## Risks

| Risk | Mitigation |
|---|---|
| Event coalescing differs per OS | Document the per-OS semantics; expose `drain` with `timeout_ms` for "give me a coherent batch" usage |
| Editor swap-file noise (vim's `.swp`, emacs `#~`) | Skip filenames matching common editor patterns by default; expose a filter arg |
| Watching huge trees burns inotify quota on Linux | Document the 8192-watcher default; recommend per-subdir watchers for >1000-file trees |
| Iterator never returns means consumer can't shut down | Add `shutdown()` method on the iterator handle; propagate as iterator-end |

## Cross-references

- [@PLAN37 phase 07a](../../plans/42-tracker-index/07a-websocket-protocol.md)
  — the design that depends on this lib.
- [`lib/server/`](https://github.com/loft-lang/loft-libs-net/blob/main/server/src/server.loft)
  — pattern for a long-running loft program with a Rust host
  bridge.
- [STDLIB.md § Open work](../../STDLIB.md#open-work)
  — sibling stdlib gaps surfaced by the same dogfood pass.
