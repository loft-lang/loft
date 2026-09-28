<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 1 — Server state

## Status

Open.  Owns the shared world state and the protocol that connects
audience clients ↔ projector view.  Effort: **M**.

## Goal

A loft program that:

1. Holds the authoritative shared world state (a sparse hex grid
   + per-cell color + per-cell age + active-player registry).
2. Accepts WebSocket connections from audience clients (phase 0)
   and the projector (phase 3).
3. On each event from a client, updates state.
4. On each tick, runs the generation step from phase 2 and
   broadcasts a `world_delta` to all subscribers.
5. Detects "active player" signals (color changes, recent input
   bursts) and broadcasts them so the audience clients can
   surface a **Jump to active** indicator.

Builds on the shipped `lib/server` multi-client WebSocket API
(`srv.run(on_event)`, `ws_clients_*`, `ws_event_*`).

## State model

The world reuses the **`lib/moros_map` chunk pattern**: a sparse
collection of 32×32 chunks at integer chunk coordinates, with
the same `chunk_idx_32` / `hex_idx_32` addressing helpers (which
handle negative-coordinate floor-division correctly).  The
per-hex payload is **slimmer** than the moros editor's `Hex`
struct — the demo only needs colour, height, and age — but
addressing, chunk lifecycle, and the sparse-storage discipline
are reused as-is.

```loft
struct World {
    chunks: vector<Chunk>,    // sparse — only chunks that have at least one
                              // non-empty cell exist; emptied chunks are removed
    players: vector<Player>,  // who is connected, what colour, when last active
    tick: integer not null,   // monotonic
}

// 32x32 grid at chunk coords (cx, cz).  Index into ck_cells: cx * 32 + cz.
// Same shape as lib/moros_map's Chunk minus cy (this demo is single-layer).
struct Chunk {
    ck_cx:    integer not null,
    ck_cz:    integer not null,
    ck_cells: vector<Cell>,
}

// 4-byte cell — colour and height use loft's u8 typedef (integer
// limit(0, 255) size(1)), age uses u16 (size(2)).  Storage packs
// to exactly 4 bytes per cell.
struct Cell {
    c_color:  u8 not null,       // 0 = empty (no crystal), 1-9 = palette
    c_height: u8 not null,       // 0..255, derived from filled-neighbour count
    c_age:    u16 not null,      // 0..65535 ticks since the cell became non-empty;
                                 // projector keys the 5-second growth animation off
                                 // this (small age = still growing, ~50 ticks at
                                 // 10 Hz tick rate = grown)
}

struct Player {
    p_id:               integer not null,  // assigned on connect
    p_color:            integer not null,  // 0 = cleared (no active colour),
                                           // 1-9 = palette index
    p_last_active_q:    integer not null,  // most recent (q, r) painted at
    p_last_active_r:    integer not null,
    p_last_active_tick: integer not null,
}
```

The `Cell` carries no growth-direction or planting-author
metadata.  Both are derived where they are needed:

- **Growth direction** for the projector's 5-second tilt
  animation is computed at render time from the **age
  comparison with neighbours** — older neighbour cluster on one
  side, this cell newer = the new cell extrudes from the older
  side.
- **Plant ownership** (who seeded a cluster) is not tracked
  per-cell; the active-player signal lives on `Player` and is
  enough to drive the audience-client jump-to-active flash.

(Final shape pinned at CI-2 once multi-client behaviour is
observed.)

### Chunk lifecycle

A chunk is **created on first non-empty write**: when a `seed`
event lands at world hex (q, r), the server computes
`(cx, cz) = (chunk_idx_32(q), chunk_idx_32(r))`, ensures a
chunk exists at those coords (build a 32×32 of empty cells if
not), and writes the cell.

A chunk is **deleted** when all 1024 of its cells become empty
(colour = 0).  This matches the user-visible rule: a chunk that
is entirely black does not exist.  Garbage-collection runs at
the end of each tick after generation + age updates.

The world coordinates of any cell are recoverable from
`(ck_cx * 32 + hx, ck_cz * 32 + hz)` where `hx, hz ∈ [0, 31]`
are the in-chunk indices.  No per-cell `(q, r)` storage needed.

## Wire format — JSON events + binary world blobs

The WebSocket carries both JSON text frames and **binary frames**
on the same connection:

| Direction | Format | Used for |
|---|---|---|
| client → server | JSON text | All input events: `seed`, `clear`, `color_select`, `swipe` (single drag → one batched event with the full cell list).  Small, low frequency, human-readable on the wire for debug.  No binary needed on the input side — even drag gestures fit comfortably in JSON |
| server → client | JSON text | Small events: `active_player_signal`, control / status |
| server → client | **Binary blob** | Bulk world data — `world_snapshot` (sent on connect, full chunk dump) and large `world_delta` payloads.  Naturally packs at 4 bytes per cell using the `Cell` payload (1 byte colour + 1 byte height + 2 bytes age) plus a small per-chunk header (cx, cz + cell-count).  Each blob carries a **session id** in its header — primarily useful for the **initial snapshot**, which the server splits across one blob per chunk so a new client's view does not block on serialising the entire world into a single frame |

The binary frames use loft's existing typed-binary read/write
pattern (see [`STDLIB.md` § File I/O](../../STDLIB.md) — the same
`f#read(n) as u8` / `as u16` shape works against any byte source,
not just files).  Server-side serialisation is straight
`vec<u8>` packing in chunk-major order; client-side deserialisation
is the inverse.  No JSON parsing on the hot path for world data.

**Per-blob session id** (`u32` in the 5-byte blob header
`[type:u8] [session:u32] [...payload...]` — see [TTT v5 wire
spec](../39-tic-tac-toe/README.md#tic-tac-toe-v5--binary-world-stream--many-clients--reconnect-catch-up--sluggish-tempo)
for the validated format).  Mostly useful when starting a new
client:
the server splits the initial `world_snapshot` into one blob per
chunk, all tagged with the same session id.  The client buffers
all blobs sharing that session id and renders them as one
coherent "world appears" event — rather than each chunk popping
into existence in arrival order.  Visually, a new joiner sees
the current world fade in as a single picture, not a chunk-by-
chunk build-up.

Steady-state per-tick `world_delta` blobs typically fit in a
single frame and don't need session grouping.  The session id
is included in their header anyway (cheap; one word) so the
client logic stays uniform across snapshot + delta paths.

### Catch-up mode

A client with connection problems (dropped packets, network
blip, reconnect after a brief outage) can fall behind the
session id stream.  The client detects this when it sees a gap
in incoming session ids or no delta arrives within a watchdog
window.  Recovery:

1. Client sends a JSON `catch_up` request with its last-known
   session id (the most recent one it fully applied).
2. Server picks the cheaper response:
   - **If the missed range is small + still in the server's
     short delta-cache**: replay the missed `world_delta`
     blobs in order, all sharing the catch-up session id so
     the client buffers them as one.
   - **Otherwise**: send a fresh `world_snapshot` (one blob
     per chunk under a new session id) — the same path used
     for a brand-new client.

The client treats the catch-up payload exactly like a normal
session: buffer all blobs sharing the session id, render as one
coherent update.  No special render path; recovery is a
configuration of the same primitives.

```json
{ "type": "catch_up", "last_session": <session_id> }
```

Event-side JSON stays small enough that text encoding cost is
negligible and the wire stays human-readable for the talk
("here is a `seed` event going to the server, this is exactly
what your tap sent").

## Event handling (client → server)

| Event type | Server action |
|---|---|
| `seed` | Set the cell's `c_color` if empty; if hex was own-color, treat as removal (set to 0); if hex was other-color, ignore.  Reset `c_age` to 0.  Update `player.last_active_*`.  Add active-player signal entry |
| `swipe` | Apply the same logic as `seed` for each cell in the batch list (with the same semantics for own / other / empty).  Single delta tick covers the whole gesture |
| `clear` | Set the cell's `c_color` to 0 (empty) following the same own / other / empty rules as `seed` |
| `color_select` | Update `player.color` and broadcast active-player signal |
| `connect` (implicit on WS open) | Assign player_id, send initial world snapshot (binary, session-tagged across one blob per chunk) |
| `disconnect` (implicit on WS close) | Mark player inactive but keep their cells |

## Tick loop

Server runs a fixed-rate tick at **10 Hz** (100 ms per tick;
resolved 2026-05-10).
The tick has **no autonomous growth step** — placement is pure
direct painting from audience events.  But it does have an
**automatic decay step**: older cells expire and are removed,
producing an "inverse growth" that erodes the canvas back toward
empty without audience effort.  Filled neighbours extend a
cell's effective lease, so decay starts at the **edges** of a
cluster (where neighbour counts are lowest) and works inward,
rather than punching holes through the middle.

**Decay timing (resolved 2026-05-10):**

- **Base lifetime: 5 minutes** (3000 ticks at 10 Hz).  No cell
  decays before that — the canvas accumulates as the audience
  paints.
- **Lease per filled neighbour**: a default extension on top
  of base lifetime (suggested ~1 minute = 600 ticks; tunable
  at first prototype).  A fully-surrounded cell with 6 neighbours
  lives base + 6 × lease ≈ 11 minutes before becoming eligible
  for removal; an isolated cell with no filled neighbours
  becomes eligible exactly at 5 minutes.
- **Decay window** (eligible → removed): even after a cell
  becomes eligible, removal is **slow**.  Suggested window:
  ~30 seconds (300 ticks).  Server keeps the cell in the world
  during the decay window with `c_age` continuing to increment
  past `effective_lifetime`; cell is fully removed (set to
  `c_color = 0`) at `c_age >= effective_lifetime + decay_window`.

  > **Shrink animation dropped (decision 2026-05-20).**  The
  > original spec had the renderer animate the crystal mesh
  > shrinking down across the decay window, which would have
  > needed per-cell age on the wire.  Judged not worth it: a
  > removed cell just disappears, with a client-side alpha
  > fade-out on the removal delta (shipped — `index.html`,
  > ~600 ms) as the cheap cosmetic substitute (no wire change).
  > The window therefore now just
  > adds a flat tail to total lifetime rather than driving an
  > animation.

The slow decay is part of the **sluggish-by-design** philosophy
— see [`README.md` § Tempo philosophy](README.md#tempo-philosophy--sluggish-by-design).

Each tick:

1. Apply the queue of audience events received since last tick
   (`seed`, `clear`, `color_select`).  Each event mutates one
   cell's `c_color` (and resets that cell's `c_age` to 0 on a
   new placement).
2. Update `c_height` on each cell whose filled-neighbour count
   changed (cell newly filled, or a neighbour newly filled /
   emptied).  Height is derived from the local neighbour count.
3. Increment `c_age` on every non-empty cell; clamp to 65535
   (the 2-byte ceiling).
4. **Decay step**: for each non-empty cell, compute
   `effective_lifetime = base_lifetime + lease_per_neighbour
   * filled_neighbour_count`.  If `c_age >= effective_lifetime
   + decay_window`, set `c_color = 0` (cell fully removed).
   For cells in the eligible-but-not-yet-removed range
   (`effective_lifetime <= c_age < effective_lifetime +
   decay_window`), leave `c_color` set; the renderer reads
   `c_age` and animates the crystal shrinking over the decay
   window.  Edges of a cluster cross the eligible threshold
   first because their `filled_neighbour_count` is low; deep-
   interior cells get many leases and survive longest.  Decay
   timing values: see [`README.md` § Tempo philosophy](README.md#tempo-philosophy--sluggish-by-design)
   and the Decay timing block above (base 5 min, ~30 s decay
   window).
5. Compute `world_delta`: per-chunk lists of `(hx, hz, color,
   height, age)` for cells whose payload changed since last
   tick (placements, height-recomputes, decays).
6. Garbage-collect chunks: any chunk whose 1024 cells are all
   empty (colour = 0) is removed from `chunks`.
7. Broadcast delta to all subscribers (including chunk
   creations + chunk deletions).

~~The renderer treats a cell-decay event symmetrically to a cell-
placement event: a decaying cell shrinks down over the same
~5-second window the growth animation uses.  Direction of the
"reverse extrusion" derives from the same age-comparison-with-
neighbours rule (the cell collapses toward where the youngest
neighbour is — i.e., away from the most-recent painting
direction).~~  **Superseded by the 2026-05-20 decision above** —
no reverse-extrusion shrink; a removed cell just disappears (with
a client-side fade-out, shipped in `index.html`).

Note: the **3D height field** the projector renders is computed
**from the per-cell `c_height` byte the server ships** (so all
clients agree on the surface shape).  The growth-direction tilt
on cells whose age < the 5-second window is derived
**client-side** from comparing this cell's age with its
neighbours' ages — older neighbours contribute the "from"
direction.  Server state stays minimal: 4 bytes per cell, no
per-cell direction or origin metadata.

## Active-player signal

Each player change is timestamped.  **Once per second
(resolved 2026-05-10)**, the server picks the most-recently-
active player (excluding the recipient) and broadcasts an
`active_player_signal { x, y }` to each audience client.  This
drives the **Jump to active** box's flash.

The 1-second steady heartbeat is the validated cadence in
[TTT v5](../39-tic-tac-toe/README.md#tic-tac-toe-v5--binary-world-stream--many-clients--reconnect-catch-up--sluggish-tempo).
Per-change signalling was rejected as overwhelming during busy
periods; tick-rate signalling (10 Hz) was rejected as wasting
bandwidth in quiet moments.  One every second feels like a
heartbeat and matches the sluggish-by-design tempo.

## Sub-tasks

| # | Task | Effort |
|---|---|---|
| 1.1 | World struct + sparse-cell access (`get_cell(q, r)` / `set_cell(q, r, color)`) | S |
| 1.2 | WebSocket event dispatch (route by `type` field) | XS |
| 1.3 | Per-player state + connect/disconnect lifecycle | S |
| 1.4 | Tick loop (timer-driven, fixed rate) | S |
| 1.5 | World-delta computation (diff against previous tick) | S |
| 1.6 | Broadcast helpers (single-client send + all-subscribers send) | XS |
| 1.7 | Active-player signal aggregation + emit | XS |
| 1.8 | Multi-client load test (2-3 → 10-30 → upper bound) — **done** (see § Load-test findings) | S |
| 1.9 | Crash-resistance — does a misbehaving client (malformed frames, oversized frame, dropped mid-handshake) bring the server down? — **done** (see § Crash-resistance findings) | S |

## Crash-resistance findings (1.9)

Audited the event-pump path (which the server now runs on) against
hostile clients.  One real crash, now fixed; the rest already safe.

- **FIXED — unbounded WS frame allocation (process abort).**
  `websocket::ws_read_frame_detailed` did `vec![0u8; payload_len]` with
  no cap, trusting the frame's 64-bit length field.  A client sending a
  127-length header claiming ~u64::MAX bytes triggered a multi-exabyte
  allocation → abort, taking *every* connected client down.  Capped at
  16 MiB (real client→server frames are tiny control messages); an
  oversized frame is now dropped as `Closed`.  Regression test:
  `lib/server/native` `p_oversized_ws_frame_rejected_without_alloc`.
  This hardens **all** `lib/server` consumers, not just the demo.
- **Malformed text frames — already safe.**  Probed with a battery of
  bad frames ([`tools/audience-demo/crash_probe.loft`](../../../../tools/audience-demo/crash_probe.loft):
  non-numeric coords, unknown msg ids, no-colon frames, empty frames,
  out-of-range / negative / huge values).  The pump logs+drops
  unparseable `<id>:<payload>` frames; `handle_message` ignores unknown
  ids and rejects out-of-range colours.  Server stayed up and a fresh
  good client still painted + received its echo.
- **Dropped mid-handshake / garbage TCP — already safe.**  `parse_request`
  reads the head byte-by-byte (16 KiB cap) and returns `Error` on
  EOF/timeout/garbage; the pump treats that as "nothing to deliver."
- **Known non-crash DoS (out of 1.9 scope):** a slow-loris that connects
  but sends no request stalls the accept in `parse_request` for its
  500 ms read timeout — degrades latency under a flood but does not
  crash.  A true fix is the Tier B reactor (non-blocking accept).

## Load-test findings (1.8)

Driver: [`tools/audience-demo/load_test.loft`](../../../../tools/audience-demo/load_test.loft)
— a single process opens N real WebSocket connections, has each paint a
disjoint column of cells, drains the broadcast fan-out, and reports
per-tier metrics.  Run against a live server; ramps 3 → 12 → 30 clients
by default (`LOFT_AUD_CLIENTS=<n>` for a single tier).

Measured (3 / 12 / 30 clients × 10 paints, fresh world, interpreter):

| Clients | Connect time | Fan-out | send_fail / drops |
|---|---|---|---|
| 3  | 1.9 s (~0.6 s/client) | 100 % | 0 |
| 12 | 33.8 s (~2.8 s/client) | 100 % | 0 |
| 30 | ~220 s (~7.3 s/client) | 100 % | 0 |

**Broadcast correctness is solid** — every paint reached every client at
all tiers, no dropped clients, no failed sends.

**The bottleneck is connection establishment, not steady-state
throughput.**  Per-client connect latency degrades super-linearly, so all
30 clients took ~3.7 minutes to connect — a real concern for a meetup
where ~30 phones connect at once.  Root cause is the
[Polled-only single-threaded loop](../39-tic-tac-toe/README.md):
the loop is too busy to promptly `accept()` new sockets because each
iteration does O(N) per-client polling + a **synchronous O(cells)
`replay_world_to` on every connect** + a periodic O(cells) `world_save`,
compounded by the web client's connect-time backoff.

### Optimization follow-up (post-1.8, not a phase-1 blocker)

Correctness is met, so neither tier blocks the phase.  Two tiers: **A**
is a cheap app-level fix that stays inside the current single-threaded
polled model and should be enough for the meetup; **B** is the durable,
scalable fix and lives in `lib/server` (benefits every loft server
consumer, not just this demo).  Do A first — the load harness
(`load_test.loft`) proves the win immediately.

#### Tier A — prototyped + measured 2026-05-20: loft hygiene helps a real burst, but does NOT fix the connect metric

The original hypothesis was that the single loop is slow to `accept()`
because it spends each iteration replaying the whole world to a new
client + polling every existing client.  Three loft-level changes were
prototyped in
[`single_port_server.loft`](../../../../tools/audience-demo/single_port_server.loft)
and **they work as improvements** (server runs, correctness 100 %, no
crash) — they are the right concurrency hygiene and *do* help a genuine
concurrent burst:

- **A1 — chunked, non-blocking replay-on-connect (shipped).**
  `replay_world_to`'s synchronous full-world dump in the accept branch is
  replaced by a per-client *replay cursor* (`replay_keys` + `replay_pos`)
  that the loop streams `REPLAY_BUDGET` (64) cells per iteration via
  `advance_replays`, reading each cell's current value at send time.  The
  accept path no longer blocks on an O(cells) dump.
- **A2 — batched accept (shipped).**  The loop now drains up to
  `ACCEPT_BUDGET` (8) connections per iteration instead of one.
- **A4 — snapshot deferral (shipped).**  `world_save` is skipped on any
  iteration that accepted a connection, so a synchronous O(cells) write
  never delays a connect.
- **A3 — OS listen backlog: no change needed.**  The native listener is
  Rust std `TcpListener::bind`, which already uses a 128-deep backlog.

**Measured result (load_test.loft, 3/12/30 × 10 paints):** connect time
was **unchanged** — 1.9 s / 33.8 s / ~210 s, vs the ~1.9 / 33.8 / 220 s
baseline.  So the loft changes are *necessary-but-insufficient*: they
remove real per-iteration work and protect against a true simultaneous
burst, but they do not move the **sequential-connect** latency the load
test measures.  (Measurement caveat: with incremental replay, replay
frames now overlap the paint phase, so the load test's `fanout%` can read
>100 % — replay deltas counted alongside paint echoes.  Correctness is
unaffected.)

**Corrected root cause (confirmed by reading the native layer): the
bottleneck is socket configuration, below the loft loop —**

1. **No `TCP_NODELAY`** on the client connect stream
   (`lib/web/native/src/ws_client.rs`) or the server's accepted streams
   (`lib/server/native/src/lib.rs`) → Nagle + delayed-ACK latency on the
   multi-round-trip WS handshake (the per-connect floor).
2. **The per-client poll read blocks on a 20 ms timeout**
   (`lib/server/native/src/lib.rs:482`, `set_read_timeout(20 ms)`).  Each
   idle client costs ~20 ms per loop sweep, so a sweep is O(N×20 ms) and
   each sequential connect waits behind an ever-slower sweep → the O(N²)
   curve.  (`parse_request` stops at the header `\r\n\r\n`, so the 500 ms
   accept timeout is *not* the floor.)

No loft-level change can reach either — which is exactly why A1/A2/A4
didn't move the metric.

**TCP-layer experiment (2026-05-20) — root cause pinned exactly, and a
"surgical" native fix proved NOT safe.**  Per-connect timing
(`load_test.loft` instrumented) showed `connect[k] ≈ k × 505 ms` — each
connect costs ~500 ms *per already-connected client*, i.e. O(N²) at
500 ms/client.  Tracing the native layer:

- **`set_nodelay(true)`** on both the server's accepted streams and the
  client connect stream made **zero difference** — Nagle is not the
  cause.
- **Exact cause:** WS streams accepted via the HTTP-upgrade path
  (`n_tcp_accept_nonblocking` → `n_ws_upgrade`) keep the **500 ms read
  timeout** set for reading the request head (`lib.rs:153`) — `n_ws_upgrade`
  pushes the stream into `WS_CONNS` without resetting it (the *other*
  accept path, `n_ws_accept_nonblocking`, does reset to 20 ms at
  `lib.rs:488`).  So every idle-client poll (`n_ws_recv` → `ws_read_frame`
  → `read_exact`) blocks the full 500 ms → poll sweep is
  O(clients × 500 ms) → connect is O(N²).

Two "surgical" fixes were tried and **both gave a ~23× win for the
audience server (30-client connect 220 s → ~9.5 s, fan-out 100 %) but
broke `multiplayer_v3`:**

1. **Lower the timeout to 20 ms** — `ws_read_frame` uses `read_exact`, so
   a timeout that fires *mid-frame* consumes and discards the partial
   bytes → frame desync.  v3's larger/slower frames truncate.
2. **Non-blocking `peek`-gate before the blocking read** — corrupts the
   read for the case where the byte-by-byte handshake left the client's
   first WS frame in the kernel buffer; v3 dies on its **first move**
   (server never reads it → client `recv-timeout`).

…but then the real answer surfaced: **the problem was already solved on
the *other* server path.**  `lib/server` has two paths — the manual one
the audience server hand-rolled (`next_nonblocking` + `ws_upgrade` +
per-client `next()`, the 500 ms streams above) and the **event pump**
(`srv.run` / `n_ws_next_event`), which the code itself calls *"the single
supported path for multi-client servers."*  The pump uses 20 ms streams
**and** `ws_read_frame_detailed` — a `NoData`/`Frame`/`Closed` reader that
returns cleanly on a no-data poll instead of the manual path's lossy
`read_exact`→`None`.  It is validated by the TTT v5 many-clients tests.

#### Tier A′ (SHIPPED 2026-05-20) — put the single-port server on the event pump

The audience server only stayed on the manual path because it serves the
HTML page *and* WebSockets on one port, and the pump's accept only handled
WS upgrades.  A′ closes that gap:

- **native:** `try_accept_inner` now surfaces a non-WS request as
  `AcceptOutcome::Http` (stream parked in `CURRENT_CONN`, path in
  `LAST_PATH`) instead of dropping it; `n_ws_next_event` delivers it as
  event kind 3.  WS-upgrade behaviour unchanged.
- **loft:** `WsEvent` gains `http` / `path`; new `Server::poll_event` (a
  non-blocking single-event drain, so a server with its **own tick loop**
  can still use the pump) plus `respond_html` / `respond_404` /
  `respond_typed`.  `run()` is now built on `poll_event`.
- **audience server** rewritten onto `poll_event`: clients are keyed by
  `cid` in a `players` hash (replay cursor + last-paint position for the
  heartbeat); fan-out via `srv.broadcast` / `send_to`; the manual client
  vector + `poll_clients` + `ws_upgrade` are gone.  Tick / decay /
  heartbeat / persistence unchanged.

**Result (`load_test.loft`):** 30-client connect **220 s → ~10 s
(21.6×)**, fan-out 100 %, 0 send failures, HTTP page still served — and
`multiplayer_v2/v3/v5` stay green.  The same win as the unsafe experiment
above, but on the supported, validated framing.

`set_nodelay` was *not* part of the fix (it made no measurable difference)
and was reverted; it remains a reasonable hygiene addition for real WAN /
browser clients but is out of scope here.

What A′ does **not** do: the pump is still O(clients × 20 ms) per drain
(idle clients each cost a 20 ms poll), so connect stays O(N²) — just 25×
cheaper.  ~10 s for 30 phones is fine for the meetup; true O(ready)
scaling (idle clients cost *zero*) is still **Tier B** below.

#### WsGroup — multiplexed non-blocking client receiver (SHIPPED 2026-05-20)

The load test's **client-side** drain was as slow as the old server-side
connect bottleneck: `try_recv()` on each of N sockets blocks 7 ms per
empty socket, so draining 30 clients costs 30 × 7 ms = 210 ms per pass.
With 25 rounds of 30 paints, the original load test took ~295 s total.

**Fix:** `WsGroup` in `lib/web` — a multiplexed receiver that scans N
WsHandler connections with `set_nonblocking(true)` during the scan,
returning the first socket with data ready.  Empty sockets return in
microseconds instead of 7 ms.  Timeout is restored after each scan.

API (`lib/web/src/web.loft`):
```loft
group = web::ws_group()
group.add(h1)
group.add(h2)
while true {
  ev = group.poll()        // one non-blocking scan of all handles
  if ev.handle < 0 { break }
  process(ev.message)
}
```

Round-robin fairness: each `poll()` starts scanning from the handle after
the one that last returned data.

**Measured result (load_test.loft, 30 clients × 25 paints):**

| Metric | Before (per-client try_recv) | After (WsGroup) |
|---|---|---|
| Send phase | ~295 s | **9–19 ms** (~15,000×) |
| Total (incl. server drain) | ~295 s | ~224 s |
| Fan-out | 100 % | 99 % |

The send phase is now instant — the remaining 224 s is the server's
single-threaded O(N²) broadcast delivering 22,500 frames.  At real
human painting speed (~1 paint/s/person), the server handles 30 phones
comfortably: each paint triggers 30 broadcasts in one pump cycle (~2 ms),
using ~6 % of loop capacity.

**Live validation (2026-05-20):** the server ran unattended for ~4 hours
(18:04–22:00 Amsterdam) with zero crashes, zero restarts, same PID
throughout.  Keepalive script + caffeinate prevented sleep; automatic
shutdown at 22:00.

#### Tier B — background I/O reactor in `lib/server` (effort M)

The durable fix: separate socket I/O from the simulation loop using OS
readiness multiplexing.  This is the model `lib/server` already designs
toward in [§ Multi-threading model](../../lib_plans/future/08-server/README.md)
(tokio runtime + thread pool); the shipped server is the "polled-only"
subset noted in [TTT v5](../39-tic-tac-toe/README.md).  What's
needed:

- **B1 — a dedicated reactor thread** in the native layer owns the listen
  socket + every client socket and waits on **`epoll` (Linux) / `kqueue`
  (macOS) / IOCP (Windows)** via the `mio` crate (one cross-platform
  abstraction).  Readiness-driven ⇒ O(ready), not O(N): `accept()` fires
  the instant a SYN lands; idle connections cost nothing.
- **B1a — buffered, partial-frame-tolerant WS framing (the specific
  blocker the Tier-A experiment hit).**  Today `ws_read_frame` does
  blocking `read_exact` per frame; under non-blocking readiness it must
  instead accumulate bytes into a per-connection buffer and only surface a
  frame once fully arrived — never consuming-then-discarding on a short
  read.  This is what makes "cheap idle poll" safe (no v3-style
  first-frame loss), and it is unavoidable for the reactor.
- **B2 — two mpsc channels** between reactor and loft loop: inbound
  frames → loft drains each tick; loft deltas → reactor fans out.  The
  loft-facing API (`next_nonblocking` / `broadcast` / `try_recv`) is
  unchanged, just backed by channels — "single-threaded from loft's
  view."
- **B3 — reactor serves the connect snapshot itself.**  The loft loop
  publishes the world as an atomically-swapped `Arc<blob>` (or an mmap'd
  buffer — composes with [@PLN43 durable loft-store](../43-loft-store-durable/README.md)).
  On accept the reactor completes the WS handshake *and* ships the
  snapshot with one buffered `writev` / large `SO_SNDBUF`, never touching
  the sim loop.  Replay latency leaves the hot path entirely.
- **B4 — per-socket backpressure in the reactor** (`EPOLLOUT` /
  write-readiness): a slow client can't stall the sim loop or its peers.
- **B5 (scaling knob, optional) — `SO_REUSEPORT` + multiple acceptor
  threads** for hundreds+ of clients.  Overkill for a ~30-phone meetup.

*Cross-platform note:* the reactor is the one piece that differs per OS;
`mio` covers epoll/kqueue/IOCP, but the kqueue (macOS) path wants a real
macOS run to validate — see [§ Validation](#validation) below.
*Canonical home:* this is `lib/server` work — track the build under
[lib_plans/future/08-server](../../lib_plans/future/08-server/README.md);
this section is the **driver** (the concrete load-test finding that
motivates it), not a second copy of the design.

- **Also pending (independent of A/B): binary wire** (phase 1 step 2) —
  cuts per-message text parse/format; reduces fan-out CPU but does not by
  itself fix connect latency.

#### Validation

Both tiers are measured the same way: start the server, run
`load_test.loft` at the 3 → 12 → 30 ramp, and compare connect time +
fan-out against the [§ Load-test findings](#load-test-findings-18) table.
Tier B additionally needs a **macOS run** (kqueue path) and ideally a
Windows run (IOCP) since the reactor is the OS-specific component.

## Open design questions

- ~~**Tick rate**~~ — RESOLVED 2026-05-10 at **10 Hz** (100 ms
  per tick).  Decay step + delta broadcast run at this cadence;
  the renderer interpolates between ticks at 60 FPS to keep
  crystal growth/decay animations smooth.  Revisit at CI-2 if
  multi-client testing shows the projector visibly stepping or
  the bandwidth saturating venue WiFi.
- ~~**Delta vs full-snapshot**~~ — RESOLVED 2026-05-10:
  delta-always + snapshot-on-connect.  Each tick the server
  broadcasts a `world_delta` with only the cells that changed.
  When a new client connects, the server sends a one-shot full
  snapshot of all chunks first, then begins streaming deltas
  starting from that point.  Simplest invariant; new clients
  start consistent without bloating the steady-state stream.
- **Persistence** — REOPENED 2026-05-21.  Original 2026-05-10
  resolution (no persistence; decay self-cleans) holds for
  *crash* recovery, but development iteration and clean restart
  of the server should NOT lose the audience's painting.  Full
  design captured in [§ Persistence — snapshot + change log](#persistence--snapshot--change-log)
  below; not yet implemented.
- ~~**Bandwidth ceiling**~~ — RESOLVED 2026-05-10 as a
  measurement target, not a hard cap: phase 1's load-test
  deliverable (`--load-test 30` for 5 minutes) measures actual
  wire volume.  Reasonable for the meetup audience size; if
  venue WiFi cannot sustain it we know early.  No server-side
  per-tick cap unless the load test surfaces pathological busy
  moments — optimisation is its own follow-up, not a phase 1
  blocker.
- ~~**Authority**~~ — RESOLVED 2026-05-10: server-authoritative,
  no client-side prediction.  Clients are pure renderers of
  server state.  Local taps show nothing on the local hex until
  the next world_delta confirms (up to ~100 ms tick lag plus
  network RTT).  Simplest invariant; impossible to desync; one
  truth.  Re-evaluate only if rehearsal shows the input lag
  feels disconnected.

## Persistence — snapshot + change log

*(Designed 2026-05-21; not yet implemented.  Reopens the
2026-05-10 "no persistence" decision for the clean-restart case
— crash recovery is still considered acceptable to lose, per
the original reasoning.)*

### Goals

- **Survive clean server restart** without losing the audience's
  painting.  The dev iteration loop (edit the server, restart,
  pick up where we left off) should be transparent — the
  projector reconnects, the world is still there.
- **Per-event hot path stays fast.**  Every paint / clear /
  colour-select runs in the WebSocket-event loop; any
  per-event disk I/O cost is paid for every audience tap.  Cap
  it at "fixed-size append, < 50 µs" so the round-trip latency
  stays below the user-perceptible threshold (~100 ms).
- **Bounded restart-replay time.**  However big the log grows,
  startup should take O(1) wrt total demo runtime — i.e., load
  a snapshot in constant time + replay the small tail since the
  snapshot.

### Two-file architecture: snapshot + log

```
tools/audience-demo/world.snapshot     # full world dump
tools/audience-demo/world.log          # change log since snapshot
```

The **snapshot file** is a full dump of `world.cells` at a point
in time.  Layout (little-endian):

```
[8B sig "AUD-SNAP"] [4B u32 epoch_id] [4B u32 cell_count]
[cell_count × { [4B i32 x] [4B i32 y] [1B u8 colour] }]
[16B trailer "AUD-SNAP-END\0\0\0\0"]
```

- `epoch_id` increments every time a new snapshot is written.
  Used to match the snapshot with the change log: the log
  records its own `epoch_id` in its header, and a mismatch on
  startup means the log is stale (refers to an older snapshot
  that's been replaced) and should be discarded.
- Trailer marker tells the loader that the write completed
  cleanly.  If absent, the file was killed mid-write —
  fall back to the previous snapshot file (kept as
  `world.snapshot.prev` for one generation).

The **change log file** is an append-only stream of paint /
erase events since the latest snapshot.  Per-event record (9
bytes):

```
[4B i32 x] [4B i32 y] [1B u8 colour]    # colour=0 = erase, 1-9 = paint
```

Log header (16 bytes, written once when the log is created):

```
[8B sig "AUD-LOG\0"] [4B u32 epoch_id] [4B u32 reserved]
```

The log file APPENDS for each event — no full rewrite, just
one 9-byte tail-write.  Per-event cost is one syscall + 9 byte
copy, well under the latency budget.

### Background snapshot writer

A separate worker (designed as a loft `parallel { … }` worker
once loft exposes long-lived background workers — currently
the practical fallback is "do it in the main loop on a timer")
periodically:

1. Increments the in-process `epoch_id`.
2. Atomically renames `world.snapshot` → `world.snapshot.prev`
   (if it exists).
3. Writes a fresh `world.snapshot.tmp` from a consistent read
   of `world.cells` at the start of the snapshot.
4. fsync + rename `world.snapshot.tmp` → `world.snapshot`.
5. Truncates `world.log` to the header only (epoch_id updated
   to the new value).
6. Logs a single line to stdout:
   `single-port: [snapshot saved] epoch={N} cells={C} bytes={B}`
   — outside watchers (and the dev iteration loop) can grep
   for this to know the on-disk state is current.

The snapshot interval is configurable (default ~5 s); each
snapshot bounds the log size and bounds the replay time on
restart.

### Startup load sequence

1. Open `world.snapshot`.  If trailer is missing OR file
   doesn't exist, try `world.snapshot.prev`.  If neither
   loads cleanly, start with an empty world.
2. Read all cells from the snapshot into `world.cells`.
3. Open `world.log`.  If its header `epoch_id` matches the
   snapshot's, replay every 9-byte record in order.  Apply:
   - colour ∈ [1..9]: insert or recolour the cell at (x, y).
   - colour == 0: erase the cell at (x, y).
4. If the log's `epoch_id` does NOT match (i.e., the log is
   for an old snapshot that's been replaced), discard the
   log and start with just the snapshot's contents.
5. Print summary:
   `single-port: loaded snapshot epoch={N} cells={C}, replayed {R} log records → {T} live cells`

### Crash safety (not in tonight's scope)

The current design tolerates "clean shutdown" (SIGTERM /
Ctrl+C) and "OS-kill mid-snapshot" (the trailer + prev-file
fallback covers it).  It does NOT yet tolerate "OS-kill
mid-log-append" — a partial 9-byte record at the tail of
`world.log` is detected (file size not a multiple of 9 + log
header) and the partial record is discarded, but the previous
records replay cleanly.

What this design does NOT yet do, and that @PLN43 picks up:

- **Per-record CRC** — a torn write inside a 9-byte record
  could write garbage that passes the size check.  CRC at the
  byte level catches this.
- **WAL fsync per record** — currently fsync is per-snapshot
  only; a paint that arrived 1 second before an OS-kill may
  not yet be on disk.  For the audience demo this is the
  bounded-loss the user explicitly accepted ("not yet for
  emergencies").
- **mmap-backed storage** — true mmap would let the kernel
  handle the snapshot writes in the background entirely; loft
  doesn't expose this from the language side yet
  (`src/store.rs::open` exists as a Rust API but no `n_store_*`
  native binding maps it to loft programs).

### Loft-side blockers

**Blocker 1 — `file()` always truncates existing files.**  Loft's
binary write path opens via `File::create(&file_name)`
(`src/state/io.rs:200`), which truncates on every open.  The
append-only `world.log` design cannot be implemented without a
file-API change: every `file(path)` call destroys the file's
existing content.

**Designed fix — semantic change to `file()`:**

- **New behaviour:** `file(path)` opens an existing file for
  read+write (preserves content; cursor at start).  Creates an
  empty file if missing.  Writes via `f += value` go at the
  current cursor position; subsequent writes advance the cursor.
  Use `f#next = f.size` to seek to the end before writing —
  that gives true append-mode.
- **Current behaviour to retire:** truncate-on-open.  Programs
  that genuinely want to overwrite a file from scratch will
  explicitly opt in with a new `f.truncate()` method called
  before the first write, OR a separate `file_create(path)`
  function (truncate semantics; current `file()` behaviour
  preserved under a new name).
- **Rust implementation:** swap `File::create(&file_name)` for
  `OpenOptions::new().read(true).write(true).create(true).open(&file_name)`
  in `src/state/io.rs`.  The `open()` variant preserves existing
  content; `create(true)` ensures missing files are created as
  empty.  `read(true)` lets the same handle service `f#read(n)`
  calls.
- **Migration cost:** every existing loft program that does
  `f = file("out.txt"); f.write("hello")` must add an explicit
  truncate or move to `file_create()`.  Affected sites grep'd
  with `grep -rn "file(.*)\.write\|f += " --include='*.loft'`.
  Audit before landing the change.
- **Why the breaking change is worth it:** the current default
  is a silent data-loss footgun — any program that opens a
  file to read its size or seek inside it destroys the file
  the moment `file()` returns.  A "preserve unless explicitly
  truncated" default matches Rust's `OpenOptions` default and
  every other language's file-open convention.  File this as a
  P-issue under PROBLEMS.md (next free P#); the persistence
  design here depends on its resolution.

**Blocker 2 — long-lived `parallel { … }` worker.**  Loft's
`parallel` blocks until all workers return, so a forever-running
snapshot worker would never let the main thread exit.  Designs
that need a background worker today have to either:

- Run the snapshot synchronously on a timer in the main loop
  (the periodic-snapshot fallback below), OR
- Spawn a separate loft process and communicate via the
  filesystem (heavy for a single demo binary).

Long-running background workers are a language gap; not
in @PLN6's scope to fix.  Capture it as an open ask under
THREADING.md when next prioritised.

### Fallback if blockers don't unblock

The practical fallback for "just persist the data" without the
loft-side changes above is to skip the change-log entirely and
write the full snapshot synchronously on a timer (every ~5 s,
bounded loss = 5 s of paints).  Costs O(N) per snapshot but is
non-blocking from the per-event perspective (only fires when the
timer elapses, not on every paint).  This is the
"minimum-viable persistence" subset of the full design — pick
it up explicitly as a follow-up sub-task rather than freelance
it, and once the file-API blocker clears, evolve it into the
snapshot + log design above.

## Deliverable

A loft program (`lib_examples/audience_demo/server.loft` or
similar) that:

- Listens on a configurable port.
- Accepts WebSocket connections from any number of clients.
- Drives the world forward at the configured tick rate.
- Survives `--load-test 30` (30 simulated clients each tapping
  randomly for 5 minutes) without leaking memory or losing
  events.

## See also

- [`README.md`](README.md) — parent plan
- [`00-audience-browser-page.md`](00-audience-browser-page.md) —
  what sends events into this server
- `02-generation-script.md` (not yet written) — the
  growth algorithm this server runs each tick
- [`03-projector-view.md`](03-projector-view.md) — the second
  subscriber type
- `lib/server/src/server.loft` — shipped multi-client WS API
- [`../../../lib_plans/future/08-server/`](../../lib_plans/future/08-server) —
  upstream library work this phase sharpens
