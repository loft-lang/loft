<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN41 — `lib/server` hardening (post-v5 polish + @PLN6 prereqs)

**Status — CLOSED 2026-07-31, nothing delivered.**  The six items below are kept
because they are concrete and correct; what was wrong was tracking them here.

Two facts closed it, both checkable rather than argued:

* **The work is not in this repository.**  Every item edits `lib/server` or
  `lib/web`, which live in
  [loft-libs-net](https://github.com/loft-lang/loft-libs-net) — this repo's `lib/`
  holds none of them.  A plan in the language repo cannot carry library work: it
  is invisible from the repo that would do it, and nothing there fails when it
  rots.
* **The "prereq" framing did not survive contact.**  @PLN41 existed so
  [@PLN6](../6-audience-generative-art/README.md) would not hit these gaps "on day
  one".  @PLN6 and [@PLN39](../39-tic-tac-toe/README.md) have both since SHIPPED
  and closed — without it.  Item (a) is still absent from the published API
  (`server` v0.5.0 on `origin/main` has `broadcast`, `send_to` and `send_binary`,
  but no `broadcast_binary` / `send_binary_to`), so this is not a plan whose work
  quietly landed elsewhere: it is a prerequisite that turned out not to be one.

So the items are not refused, and none is blocked on design.  If a consumer wants
binary broadcast, the place to ask is an issue on `loft-libs-net`, where the code
and its CI are — build the catalogue with `make libcatalogue` and read `LIBRARIES.md` for the
current API before re-deriving any of this.

---

## Items

### (a) `srv.broadcast_binary(msg) -> integer` and `srv.send_binary_to(cid, msg) -> boolean`

**Cost: XS.**  TTT v5 t4 had to declare `n_ws_send_binary` inline as
`ws_send_binary_to_native(handle, msg)` to send a binary frame to a
specific client slot, because `lib/server::send_to` is text-only.
Plan-36's projector broadcast (world snapshots + deltas) needs both
the broadcast and the per-slot binary path.

**Implementation:**
- New `n_ws_broadcast_binary(handle, msg_ptr, msg_len) -> i32`
  in `lib/server/native/src/lib.rs` — mirrors `n_ws_broadcast` but
  with `OP_BINARY` instead of `OP_TEXT`.
- Loft binding `pub fn broadcast_binary(self: Server, msg: text) -> integer`.
- Loft binding `pub fn send_binary_to(self: Server, client_id: integer, msg: text) -> boolean`
  routed through the existing `n_ws_send_binary` (which the
  multi-client slot table already shares with the single-client
  one — slot ids are interchangeable).
- Drop the inline `ws_send_binary_to_native` declaration in
  `lib/game_protocol/examples/v5_t4_catch_up_server.loft`; use the
  new public method.

**Test:** extend `tests/multiplayer_v5.rs::v5_t3_n_client_broadcast`
with a binary-frame variant — server `broadcast_binary` once,
each of N clients receives it via `try_recv` + `last_opcode == 2`.

---

### (b) Server-side binary recv: switch from `from_utf8_lossy` to `from_utf8_unchecked`

**Cost: XS.**  Asymmetry between `lib/web/native/src/ws_client.rs::recv`
(uses `from_utf8_unchecked` for `OP_BINARY`, preserving arbitrary
bytes) and `lib/server/native/src/lib.rs::n_ws_recv` (uses
`from_utf8_lossy` regardless of opcode, replacing non-UTF8 bytes
with U+FFFD = 3 corrupted bytes per invalid sequence).  Today the
server SILENTLY corrupts client→server binary payloads with any
high-bit byte.

```rust
// Today (lib/server/native/src/lib.rs:262)
WS_LAST_MSG.with(|m| {
    *m.borrow_mut() = String::from_utf8_lossy(&frame.payload).to_string();
});

// After (mirror lib/web's client)
let s = if frame.opcode == OP_TEXT {
    String::from_utf8_lossy(&frame.payload).into_owned()
} else {
    unsafe { String::from_utf8_unchecked(frame.payload) }
};
WS_LAST_MSG.with(|m| *m.borrow_mut() = s);
```

**Same trust boundary as the client side** — loft `text` is a
byte buffer, the receiver decodes via `byte_at` (@P246-era
addition), not via UTF-8 reading.  The lossy path was a
defensive choice from the text-only era; binary opcodes need
the bytes verbatim.

**Test:** new `v5_t6_arbitrary_byte_round_trip` — client sends a
9-byte payload that includes 0x80, 0xFF, 0x00 (definitely not
valid UTF-8); server echoes; client verifies bytes-equal via
`byte_at` per offset.  Today this would fail because the server
would read corrupted bytes.

---

### (c) Full 30-client × 30-second soak (the v5 plan's original t3 spec)

**Cost: M.**  Compressed to 5 clients × ~1 s for CI; the v5 plan
called for "30 clients × 5 minutes without dropping any".  Run as
an `#[ignore]`-by-default `#[test]` in `tests/multiplayer_v5.rs`,
opt-in via `cargo test --release v5_t3_30client_soak --ignored`.

**Spec:**
- 30 client subprocesses connect; READY broadcast fires when all
  30 are seated.
- Each client sends one Hello, receives 30 (broadcast loop).
- Server emits a 1 Hz "active_player_signal" (new — see (e) below
  but a degenerate timer-driven broadcast works for the soak too)
  for 30 seconds.
- Each client confirms it saw 30 + ~30 signal frames + the
  server logs no slot-leak diagnostics.
- Server stays up for the entire window; no `panic!`,
  no zombie slots after the run completes.

Validates the routing pattern under real load and reveals any
per-connection state leaks the smaller test missed.

---

### (d) `srv.run()` panic recovery — what happens when on_event panics?

**Cost: S.**  Today: a panicking `on_event` callback unwinds
through `srv.run()` and kills the entire server process.  For a
multi-client server one bad message from one client takes down
all 30.  Two reasonable policies:

1. **Catch + log + drop the message + continue** (Erlang-style:
   the server keeps serving the other clients).
2. **Catch + disconnect the offending client + continue**
   (defensive: a panic suggests bad input from THAT peer).

Loft has no `try { } catch { }` at the language level, so this
has to live in the Rust runtime.  `std::panic::catch_unwind`
around the `on_event` invocation in `lib/server/src/server.loft`'s
`run()` body — but `run()` is loft code, not Rust, so the right
place is actually inside the dispatcher that invokes the loft
function pointer.

**Decision required (open question):** policy 1 or 2?  Spec says
1 by default with a `srv.set_disconnect_on_panic(true)` opt-in
for policy 2.  Either way, document the chosen behaviour in the
`srv.run()` doc comment.

**Test:** `v5_t7_panic_isolation` — server has 3 clients; client
B sends a frame that triggers `assert(false, ...)` in the loft
handler; clients A and C continue receiving subsequent
broadcasts.

---

### (e) Built-in tick-driven broadcast scaffold (1 Hz heartbeat)

**Cost: S-M.**  Plan-36 needs an "active_player_signal" emitted
once per second; t3's broadcast triggers off a connect event,
not a wall-clock tick.  Today there's no clean way for a loft
server to inject a periodic broadcast — the `srv.run()` loop
sleeps 2 ms when idle but doesn't surface "now is a tick" to
loft.

**Design sketch:**
- Add `srv.run_with_tick(on_event, on_tick, tick_ms)` —
  identical to `srv.run` but also fires `on_tick()` every
  `tick_ms` milliseconds.
- Implementation: extend the `run()` loop with a `now()`-based
  check against last_tick; call `on_tick` when `now - last_tick
  >= tick_ms`.

This is a strict superset of `run()`, so the existing surface
stays untouched.  Plan-36's projector + the v5 t3 30-client soak
both want this.

**Test:** `v5_t8_tick_cadence` — server uses
`run_with_tick(noop_msg, broadcast_tick, 100)`; one client
receives ≥ 9 tick frames over a 1-second window.

---

### (f) Connection-level metrics

**Cost: S.**  Today's `srv.run()` log lines are
"connected seat=N" / "broadcast hello from L" etc. — chatty
text.  For a 30-client × 30-second soak we want STRUCTURED
counters:

- per-server: `total_connects`, `total_disconnects`,
  `active_clients`, `bytes_in`, `bytes_out`, `frames_in`,
  `frames_out`.
- per-client: `connected_at_ms`, `last_recv_ms`, `frames_in`,
  `frames_out`.

Expose as native fns:
- `srv.metrics() -> ServerMetrics` (struct of counters).
- `srv.client_metrics(cid: integer) -> ClientMetrics`.

Lets the soak test ASSERT health invariants instead of
inferring from stdout.  Also gives @PLN6's audience-server a
free `/metrics` HTTP endpoint by handing the counters through.

---

## Sequencing

```
(a) broadcast_binary  ┐
(b) recv unchecked    ┴── XS pair, land together (one commit)
                            │
(c) 30-client soak    ───── needs (a)+(b); +M
                            │
(d) panic recovery    ───── independent; +S, after (c) so the soak surfaces
                            real-world panic patterns first
                            │
(e) tick scaffold     ───── independent; +S/M, plan-36 phase 1 starts here
                            │
(f) metrics           ───── independent; +S, makes (c)'s assertions trustable
```

**Recommended landing order:** (a)+(b) first as a single small
commit; then (e) before @PLN6 phase 1 starts (it unblocks the
real demo); then (c) and (f) together (the metrics make the
soak meaningful); then (d) once we've SEEN a real panic in the
wild.

## Out of scope (deferred to a later plan)

- TLS / `wss://` support — @PLN6 is HTTP-only over a trusted
  LAN.  Add to a future hardening pass once a real deployment
  needs it.
- HTTP/2 or WebSocket multiplexing — single-stream WS is fine
  for the audience demo; multiplexing earns its keep when
  per-client message rates reach kHz.
- Persistent replay cache — @PLN6's catch-up can reconstruct
  from the live world state; on-disk session log not needed.
- Connection-rejection-at-cap — v5 explicitly deferred this;
  re-deferred here.
- HTTP route decorator syntax (C57 from
  [@PLN37](../37-server-features/README.md)) — orthogonal
  language feature; not on @PLN6's critical path.

## Dependencies

- **Builds on:** [@PLN39 v5](../39-tic-tac-toe/README.md#tic-tac-toe-v5--binary-world-stream--many-clients--reconnect-catch-up--sluggish-tempo)
  (the binary WS surface, blob format, multi-client routing).
- **Unblocks:** [@PLN6](../6-audience-generative-art/README.md)
  phase 1 (server-state) — items (a), (b), (e) are prereqs;
  (c), (d), (f) are post-launch hardening.

## See also

- [PROBLEMS.md @P244](../../PROBLEMS.md) — text-returning native
  bindings under `--native` (closed; resolved during v5 work).
- [PROBLEMS.md @P245](../../PROBLEMS.md) — `parallel{}` + I/O
  composition (closed; the snapshot fix unblocks any single-process
  server + client variant a future @PLN6 demo wants).
- [PROBLEMS.md @P246](../../PROBLEMS.md) — file-scope `const`
  (closed; relevant because the server's wire-format constants like
  `H_HELLO`, `TYPE_DELTA` now declare cleanly).
- [INCONSISTENCIES.md § 33](../../INCONSISTENCIES.md#33-const-applies-to-locals-and-parameters-but-not-fields)
  — the const-fields gap.  Tangentially relevant: tightening
  `Cell` / `Player` field immutability would catch tick-loop
  mistakes early in @PLN6.
- [lib/game_protocol/examples/v5_t3_n_clients_server.loft](https://github.com/loft-lang/loft-libs-net/blob/main/game_protocol/examples/v5_t3_n_clients_server.loft) —
  current multi-client routing pattern; (a)+(b)+(e) extend it.
- [lib/server/native/src/lib.rs](https://github.com/loft-lang/loft-libs-net/blob/main/server/native/src/lib.rs) —
  fix sites for items (a), (b), (e).
