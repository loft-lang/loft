<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# EVENT_PROTOCOL — wire format, handshake, encoding

**Status:** spec.  V1 (text-mode) is implemented and validated by
the tic-tac-toe demo; binary-mode is designed but not yet
implemented.  Companion to [EVENT_LOOP.md](README.md), which
covers the application-layer dispatch and handler-registration
concerns.

This document describes the **transport layer**: what bytes flow
on the WebSocket between two endpoints, how the handler-id
namespace is negotiated, how multi-frame messages are
reassembled, and how the protocol evolves across versions.  A
loft program using `el::on(loop, fn(e: T) { ... })` never reads
or writes a frame byte; that surface is documented entirely in
EVENT_LOOP.md.  This document is for:

- Wire-level debuggers (`wscat`, `websocat`, custom tooling).
- Non-loft peers (a JS or Python client talking to a loft
  server, or vice versa).
- v4+ designers extending the handshake or wire format.
- Implementers of the encoder / decoder in `lib/server` and
  `lib/web`.

---

## Why split from EVENT_LOOP

The two concerns evolve at different paces and address different
audiences:

- **EVENT_LOOP.md** changes when the *programmer's API* changes
  — adding `on_at`, refining capture semantics, swap-mode
  rules, priority-lane scheduling, frame-budget tuning.
- **EVENT_PROTOCOL.md** changes when the *wire format* changes
  — moving from text to binary frames, adding encoding modes,
  adding streaming reassembly, evolving the handshake.

A reader debugging a wire trace doesn't need to read about
closure capture; a reader writing a handler doesn't need the
header byte layout.  Splitting the docs lets each evolve without
the other's content getting in the way.

---

## Two transport modes

This spec covers two transport modes that share the same
handshake but differ in frame encoding:

| Mode | Frame format | Status |
|---|---|---|
| **Text v1** | `<id>:<payload>` ASCII text in WebSocket text frames | Implemented; runs on existing `lib/server` + `lib/web` text-only WebSocket.  Validated by tic-tac-toe v1. |
| **Binary v2** | 12-byte header + payload in WebSocket binary frames | Designed; lands when the EventLoop's priority lanes, streaming reassembly, or encoding-mode auto-detection are needed. |

The handshake (a name→id MAP exchange) is the same both ways;
only the subsequent frame encoding differs.  A program upgrades
from v1 to v2 by changing the encoder/decoder; handler
registration, dispatch, and the MAP shape stay identical.

---

## The handshake — server-arbited name→id MAP

Both modes begin with the server sending a **MAP frame** as the
first WebSocket frame after the upgrade.  The MAP carries the
server's authoritative name→id assignments.

### Why the server is the arbiter

The client cannot decide ids unilaterally even if both sides
"agree on a registration order" — that's a fragile coupling
that breaks the moment one side adds, removes, or reorders a
handler.  Centralising id assignment at the server gives a
single source of truth.

The client's role is to conform: parse the MAP, populate its
local registry with the server's assignments, then exchange
frames using those ids.

### Text-mode MAP frame

```
MAP:Click=0,Placement=1,GameOver=2
```

- Reserved prefix `MAP:`.
- Followed by comma-separated `name=id` pairs.
- Names follow the handler-name discipline from
  [EVENT_LOOP.md § Three-tier registration](README.md#three-tier-registration):
  fully-qualified type names (tier 1), `type/instance` (tier 2),
  or explicit overrides (tier 3).
- Ids are non-negative integers in ASCII.
- Whitespace inside the payload is significant — no padding.

The MAP is sent unconditionally as the first frame after upgrade.
The client polls for it (via `web::try_recv` or the
EventLoop pump) and rejects the connection if it doesn't arrive
within a reasonable timeout.

### Binary-mode MAP frame

A single binary frame whose body is a length-prefixed sequence
of (name-length, name-bytes, id) triples.  The exact byte
layout is TBD until binary-mode implementation begins; what
matters for the design is that **the same logical content**
(the name→id map) is conveyed by both modes.

---

## Text-mode wire format (v1)

Every WebSocket text frame after the MAP is `<id>:<payload>`:

- `<id>`: integer in ASCII, the handler id from the MAP (looked
  up by name in the registry).
- `:`: literal single-byte separator.
- `<payload>`: arbitrary text.  V1 carries no JSON wrapping or
  binary framing; the application encodes payload semantics
  itself (e.g. `"X,1,2"` for a placement, `"row,col"` for a
  click).

Example exchange (tic-tac-toe v1):

```
SERVER → CLIENT   MAP:Click=0,Placement=1,GameOver=2
CLIENT → SERVER   0:1,2                  ← Click at row 1, col 2
SERVER → CLIENT   1:X,1,2                ← Placement: X at 1,2
SERVER → CLIENT   1:O,0,0                ← Placement: O at 0,0 (server's move)
CLIENT → SERVER   0:0,0                  ← Click at 0,0
SERVER → CLIENT   1:X,0,0
SERVER → CLIENT   2:X                    ← GameOver: X wins
```

The text encoding is human-readable, debuggable with `wscat` /
`websocat`, and does not require binary WebSocket frames —
ideal for v1, demos, and protocol-level debugging.

### Reserved prefixes

Text mode reserves prefixes that cannot collide with integer
ids (which are always non-negative ASCII digits):

- `MAP:` — the handshake frame, server → client, once per
  connection immediately after upgrade.
- `LOG:` (V4+, planned) — diagnostic / telemetry frames the
  receiver may display or ignore.
- `ERR:` (V4+, planned) — protocol-level error notifications
  (id mismatch, malformed payload, version mismatch).

Disambiguation is by leading character: integer ids always
begin with a digit; reserved prefixes always begin with a
letter and are uppercase.  A receiver examines the first byte
to decide which decoder to use.

---

## Binary-mode wire format (v2)

Every WebSocket binary frame is a 12-byte header followed by a
variable-length payload:

```
┌──────────────────────────┬──────────────────────────────────┐
│ binary header (12 B)     │ payload (variable)                │
├──────────────────────────┼──────────────────────────────────┤
│ handler_id : 4 B  u32    │ JSON  ({"x": 12, ...})            │
│ priority   : 1 B  u8     │  OR                               │
│ seq        : 2 B  u16    │ binary (packed struct)            │
│ flags      : 1 B  u8     │  OR                               │
│ length     : 4 B  u32    │ raw bytes (texture / mesh / chunk)│
└──────────────────────────┴──────────────────────────────────┘
```

| Field | Size | Notes |
|---|---|---|
| `handler_id` | 4 B | u32; the integer id assigned by the handshake. |
| `priority` | 1 B | 0 = highest, 255 = lowest.  HIGH/NORMAL/LOW map to 0/128/255 by default.  Set per-frame by the sender (see § Per-direction priority). |
| `seq` | 2 B | u16 per-(handler_id) sequence number; receiver uses for gap detection and reassembly ordering. |
| `flags` | 1 B | bit 0 BEGIN, bit 1 END (multi-frame reassembly markers); bit 2 COMPRESSED (zlib over payload); bit 3 BINARY (skip JSON decode); bits 4-7 reserved. |
| `length` | 4 B | u32 payload size in bytes. |
| `payload` | length B | encoded per the handler's declared `Encoding`. |

Header total: **12 bytes**.  For a player-input frame
(~30 B JSON), overhead is ~28 %.  For a world-chunk frame
(~4 KiB), overhead is negligible.

### Encoding modes

The library picks the encoding from the handler's recv-type at
registration time; per-frame the BINARY flag indicates whether
the payload should bypass JSON decoding.  Three modes:

- **JSON** (default): structured messages.  Schema-drift
  forgiving (missing fields default; extra fields ignored).
  Round-trip via `OpFormatDatabase` and `database.parse`.
- **Raw** (auto-detected): the recv-type is a wrapper struct
  whose payload is a `bytes` field — typically the pattern for
  world chunks, avatar textures, one-off meshes, audio samples.
  The library transports the bytes unchanged; no JSON
  round-trip.
- **Binary** (typed-struct packing): deferred for v1; revisit
  if benchmarks show JSON encoding dominates frame cost.

Encoding selection is invisible to the loft programmer — the
library derives the mode from the recv-type at registration
time (see [EVENT_LOOP.md](README.md) for how handler
registration consumes the recv-type).

### Per-direction priority

Priority is **per-frame** and **per-direction**.  The byte in
the header is the sender's classification of THIS specific
frame, not a fixed property of the handler.  The receiver's
lane assignment is by what's on the wire, not by lookup.

This matters for asymmetric channels — e.g. a client's
`WorldRequest` flagged HIGH (player wants the chunk now)
returns as a server's `WorldChunk` flagged LOW (delivery
doesn't preempt other traffic).  Same handler-id, different
priorities each direction.

`set_priority(loop, h, p)` semantics: "tag MY outbound frames
for handler h with priority p."  Inbound lane assignment is
read from the wire byte.

### Streaming — multi-frame reassembly

A logical message larger than one WebSocket frame is split
across multiple frames sharing the same `handler_id`, with
BEGIN and END flags delimiting the sequence and `seq` ordering
chunks within a packet:

```
frame N  : flags=BEGIN          handler_id=H seq=0  payload=chunk0
frame N+1: flags=0              handler_id=H seq=1  payload=chunk1
…
frame N+k: flags=END            handler_id=H seq=k  payload=chunkk
```

The receiving library:

1. Buffers chunks per (handler_id, packet_id) until END arrives.
2. Concatenates payloads in `seq` order (gap detection via the
   sequence numbers).
3. Decodes the assembled bytes per the handler's `Encoding`.
4. Dispatches one fully-decoded typed message to the handler.

Chunks are **invisible to the application code**.  The handler's
Recv argument is always a complete typed value.  The "wait" for
all chunks IS the async property — the handler doesn't fire
until the message is whole.

#### Memory and timeout

The library buffers per (handler_id, in-flight packet) until
END.  Configurable per-packet timeout (default 30 s); on timeout
the partial data is freed and a `StreamCancelled(handler_id,
packet_id)` audit event fires.

#### `on_each` (future)

For the **incremental-load** pattern (search results streaming
in row by row, inventory populating, leaderboard filling), a
future opt-in `on_each` form will deliver per-element callbacks
as elements arrive on the wire.  V1 ships only the
all-at-once form; `on_each` lands when concrete use cases
demand incremental display.

---

## Version negotiation (planned)

V1 has no version field — the server unconditionally sends
`MAP:...` and the client unconditionally accepts it.  V4+
extends the handshake with version + capability fields:

```
HELLO:proto=2,encoding=binary,streaming=1,priority=1
MAP:Click=0,Placement=1,GameOver=2,seq_byte_count=2,...
```

The HELLO frame precedes MAP.  Capabilities a peer doesn't
support are silently dropped (graceful degradation).  Version
mismatch ends the connection with `ERR:incompatible_version`.

This is deferred until V4's hot-swap and binary-mode work
land; v1 establishes the handshake-first discipline so the
extension fits cleanly.

---

## Implementation status

| Piece | Status | Where |
|---|---|---|
| Text-mode frame encoding/decoding | Implemented | `lib/server/native/src/lib.rs` (server side); `lib/web/native/src/ws_client.rs` (client side) |
| MAP handshake (text-mode) | Implemented | tic-tac-toe v1 server emits `MAP:`; client parses it |
| Reserved prefix disambiguation | Implemented (programmer-side) | client's `if raw.starts_with("MAP:")` |
| Binary-mode header | Designed, not implemented | EVENT_LOOP.md spec; awaits binary WebSocket frames in `lib/server` / `lib/web` |
| Streaming reassembly | Designed, not implemented | spec only; no consumer yet |
| Encoding mode auto-detection | Designed, not implemented | depends on closure-in-struct-field landing (@P213) for full library-managed dispatch |
| Version negotiation (HELLO) | Designed, not implemented | V4+ |

---

## Cross-references

- [EVENT_LOOP.md](README.md) — application-layer dispatch,
  handler registration, capture semantics, API surface.
- [EVENT_LOOP_DISCUSSION.md](DISCUSSION.md) —
  open issues, alternatives considered, design history.
- [TIC_TAC_TOE.md](../39-tic-tac-toe/README.md) — v1 application using this
  protocol; smallest validating game.
- [lib/server/native/src/lib.rs](https://github.com/loft-lang/loft-libs-net/blob/main/server/native/src/lib.rs)
  — server-side text-mode implementation
  (`n_tcp_*` and `n_ws_*` C-ABI exports).
- [lib/web/native/src/ws_client.rs](https://github.com/loft-lang/loft-libs-net/blob/main/web/native/src/ws_client.rs)
  — client-side text-mode implementation including handshake and
  auto-reconnect with backoff.
- [PROBLEMS.md § 213](../../PROBLEMS.md#213-typefunction-storage-layout-limit--full-design-for-the-proper-fix)
  — closure-in-struct-fields layout limit; gates full
  library-managed dispatch.
