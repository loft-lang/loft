# Phase 05a — the quick UDP channel (state-sync datagrams beside the websockets)

**Status: v1 shipped (2026-06-10).**  The minimal cut the traffic-class table
makes possible: a datagram path for the **state-sync class only** — events and
bulk stay on WS, so there is **no** reliable-UDP layer, no fragmentation, no
congestion control.  Native peers gain the fast path; phones stay `wss`; same
world, same `sync_send` call.

## What shipped

Kernel mechanics (`src/engine_host.rs`):

- **One UDP socket on the SAME port number** as the WS listener (zero config;
  a UDP bind failure is a warning — everything rides WS until restart).
- **A session key ties the UDP path to the WS session**: at the upgrade the
  kernel draws a random 32-byte key per connection from the OS and hands it
  over with the cid.  **Every datagram, both ways, is sealed under it**:
  `kind | cid | seq | payload | tag`, the tag 16 bytes of HMAC-SHA256 over a
  direction byte and everything before it (the format: `src/engine_host.rs`
  § the seal; the C runtime's `loft_engine.h` speaks the same and the two
  interoperate).  A datagram whose tag does not hold, or whose seq was taken
  already (a 64-wide replay window), changes nothing — no slot, no path, no
  keepalive.  The hello carries no secret.  The source address is a routing
  hint: the newest authenticated datagram moves the path (NAT rebinding).
  The seal protects the seat and the integrity of the samples, not their
  confidentiality, and it is only as strong as the channel that carried the
  key: under `ws://` the key crosses the network in plain text, so such a
  session is LAN-grade.
  **The negotiation is fully kernel-internal** (user directive 2026-06-10 — a
  core lavition/loft value: never bother the developer with details that
  don't aid them): the offer rides the WS **101 upgrade response** as an
  `X-Loft-UDP: <cid>:<key>` header — browsers can't read upgrade-response
  headers from JS and ignore it; a native client's kernel reads it and sends
  a sealed hello, which binds the datagram source addr to that cid and is
  acked with a sealed `A`.  No loft code on either side ever touches the key
  (the original `udp_cookie()` surface was removed the same day it shipped —
  GOALS § Goal F records this as the engine-surface instance of the friction
  test).
- **Inbound conflation slots** (their first consumer, as deferred from phase
  01): sync datagrams (kind `S`) conflate to newest per sender — a higher
  seq overwrites the slot, a stale/reordered seq is **discarded** (never apply
  an old pose; minimal-latency goal).  The loft side drains dirty slots with
  `sync_next()` inside `on_tick` — **late-latch by construction** (the
  freshest datagram before the tick is the one read).
- **Outbound `sync_send(cid, msg)`**: stamps a per-cid seq and sends a
  datagram when the path is bound, else falls back to a plain WS frame — the
  transport split is invisible to meaning.
- **Keepalive/timeout**: any authenticated datagram refreshes the path; silence past 3 s
  unbinds it and sends revert to WS transparently.  (The client's steady
  keepalive cadence doubles as the phone radio wake.)
- **≤1200 B datagram cap**, the 29 bytes of seal included (overlay-shaved
  MTUs fragment silently above that): oversized sync sends drop with a once-per-kernel warning — never a
  halt; bulk belongs on the WS channel.

Loft surface (`lib/engine_host`): `sync_class(msg_id)` (the wire-schema
table, minimal form — declare a message KIND as latest-value state once;
user-directed 2026-06-10) + `sync_class_keyed(msg_id)` (the payload's first
field is an entity id — conflation keeps the newest per (peer, kind,
entity), so one kind carries N entities: poses keyed by plane; added for the
phase-04 measurements when per-kind conflation collapsed 30 planes to one
slot), `udp_bound(cid)` (read-only introspection),
`sync_next()` / `sync_cid()` / `sync_seq()` / `sync_payload()`.  There is no
per-call transport API at all: ordinary `send`/`broadcast` route by the
declared class — `sync_send` existed for a few hours and was absorbed.
Inbound conflation is per (sender, msg_id) — two sync kinds from one client
never collapse each other (kind count bounded at 64/client; over-cap sync
datagrams drop, which latest-value semantics permits).

The class is data, not code, which is what the service surface (phase
**05d**, design verified — ENGINE_HOST.md § Services) hangs lanes on: a
registered service = handler + kind + (late, separately) its lane;
`sync_class` is that table's lane column.  `run()` is unchanged — sync drains inside the user's
`on_tick`, which is what makes late-latch automatic.

**The transport-selection contract (user-confirmed 2026-06-10): automatic per
client.**  A native client hellos (it can speak UDP) and `sync_send` rides
datagrams; a web page cannot send UDP, never binds, and the very same call
stays WS — meaning-code never branches on transport.  The phase-04 connector
kernel auto-performs the hello, so for native seats the selection is automatic
end-to-end with zero client code either.

**Priority keyframes** (the class table's discontinuity rule, landed
2026-06-10 with the phase-04 work): `keyframe(cid, msg)` promotes ONE sample
of a sync kind to must-deliver — it rides the reliable channel `S:`-framed in
the SAME seq space as the datagrams, so a bound connector's slots keep total
order across both carriers (an in-flight older datagram is discarded as
stale) and an unbound web client just gets the plain message it always gets.
The promotion DECISION (what counts as a bounce/teleport) is meaning;
delivery is mechanics.  Proven under a TOTAL datagram blackout
(`engine_host_connector::keyframes_survive_total_datagram_loss`): with 100 %
injected drop, only the promoted samples arrive.

## Acceptance

`tests/engine_host_udp.rs` (end-to-end, real sockets, the datagram written
from its description in `tests/common/udp_wire.rs`): cid and key read from
the 101 `X-Loft-UDP` header (the fixture program never references transport)
→ WS-fallback beacons pre-hello → sealed hello / ack binding → the same
`sync_send` arriving as sealed datagrams → a same-tick burst (seq 110, 112,
then reordered 111) yields **exactly one** echo (the newest, 112) → a stale
seq 105 is never echoed → 3 s of silence reverts beacons to WS.  Beside it,
datagrams the session's holder did not send: a forged sample claiming the
highest seq neither applies nor makes the next genuine one stale; a hello
seen on the path and replayed from another address takes no seat; forged and
replayed keepalives do not hold a quiet path open; a genuine datagram from a
new address moves the path.  `tests/c_emit.rs` runs a C listener with an
interpreted seat and the other way round over the same datagrams.  Green (~6.5 s, includes the
timeout wait).

**The auto-path proof on the real consumer** (user-directed 2026-06-10):
`probe_server_kernel.loft`'s poses + echoes ride `sync_send` (EXIT stays on
`send` — the class table in action: state syncs, events deliver).  Proven
both ways: (a) the unchanged WS probe client at 12 clients behaves
identically (hold p50 14.7 ms ≈ the half-tick floor — fallback is
byte-identical WS); (b) `engine_host_udp::probe_server_poses_ride_the_
fastest_path_per_client` runs ONE server with client A (web-page tier, no
hello → WS pose frames) and client B (native tier, hellos → the same world's
poses as seq-stamped datagrams) — one call site, zero transport logic in the
server program.

Transport-latency delta is **not** asserted: on loopback it is µs-noise; the
win (no retransmit stalls, no head-of-line blocking, discard-stale) is a
wifi/LAN property — measured when a native client exists (phase 04 extends
the @PLN51 probe targets with a loss% axis).

## Build findings

- **Dest-passing arg order**: the dest `DbRef` is pushed LAST by the emitter,
  so a `_dest` native pops it FIRST, then the declared args in reverse
  (`n_ymd_days_ago_dest` is the reference).  Popping `cid` first read the
  dest ref as an integer and the next stack bytes as a "dest" → write into a
  read-only store (store.rs:1537 assert).  The assert caught it immediately;
  `LOFT_STUB_DEBUG=1` (new, compile.rs) lists which `#native` symbols got
  panic-stubs at registration — both diagnostics earned their keep here.
- **Text-dest naming**: `is_text_dest_native` keys on the *def name*, so a
  text-returning kernel native must keep the `kernel_*` loft decl
  (`n_kernel_udp_cookie`) with a pub wrapper for the surface name — the
  `kernel_event_payload` pattern, now stated in the lib's comments.
- **`[native] in_binary = true`** (lib/engine_host/loft.toml) — the manifest
  mark phase 01 predicted, landed here when the extraction-hygiene gate
  flagged the kernel symbols in `src/**`.  One mark, two readers: the
  auto-native driver skips the doomed cdylib compile (the 36-line P269
  warning block on every kernel-program start is gone), and the hygiene gate
  sanctions the `n_kernel_*` symbols inside the compiler crate.

## Deferred (with triggers)

- **Bulk over UDP** — promoted to its own parked phase row, **05c-udp-bulk**
  (README § Phases; user-directed 2026-06-10): the one-to-many seat push
  (broadcast/multicast + NACK chunks) is the case TCP structurally can't
  match; single-receiver bulk stays on WS by design.  Trigger: a consumer
  that pushes big payloads to many seats.
  **Evaluated for game-code distribution (2026-06-11) — stays parked.**
  (1) The lean walk-up tier is BROWSERS, which have no UDP receive path at
  all (no raw sockets; WebRTC channels are DTLS unicast) — the S6 wasm
  bundle can only arrive over HTTP/WS, so broadcast's audience shrinks to
  native seats.  (2) On Wi-Fi, broadcast frames go out at the base rate
  (~6 Mbps, no L2 acks) while unicast runs 100–400+ Mbps — broadcast loses
  until seat counts far beyond a cabinet; only WIRED seats remain.
  (3) Artifacts are small (kernel binary ~1.0 MB, wasm bundle ~2.5 MB):
  10 wired seats × 1 MB ≈ 80 ms of WS unicast in a path that is not
  latency-critical (S4/S5: the old build serves throughout).  (4) The one
  real broadcast advantage is INTERFERENCE (push bytes contending with
  live sync datagrams), and a token-bucket pace on the WS bulk sends buys
  the same protection without re-building reliability/congestion control
  — the machinery 05a deliberately refused.  Sharpened trigger: a measured
  cabinet profile (many WIRED seats, frequent swaps) where bulk pacing
  demonstrably cannot hold sync latency — the stamp chain shows it.
- ~~Client-side kernel (connector role)~~ — **LANDED 2026-06-10** (pulled
  forward from phase 04; see § The connector role below).
- **Broadcast discovery beacon** (~30 lines, discovery only) — with the first
  LAN-party consumer.
- **DTLS / crypto-lib encryption** — hostile-network deployments, post-LAN.

## The connector role (landed 2026-06-10 — pulled forward from phase 04)

One core, two roles: `run_client(host, port, tick_us, on_event, on_tick)` is
the native client's half of the auto-path, with the same zero-transport
surface as the listener.  The connector connects + upgrades (masked client
frames per RFC 6455), reads the kernel-negotiated `X-Loft-UDP` cookie from
the 101 head, **auto-hellos until acked and keepalives after** (500 ms — the
phone-radio-wake cadence, inside the listener's 3 s timeout); `client_send`
routes by the SAME `sync_class` table; inbound server sync conflates through
the SAME `conflate_slot` machinery (per msg_id) — the queue semantics never
fork between roles.  Unlike `run`, `run_client` RETURNS when the server
connection dies (the kind-2 disconnect event reaches `on_event` first).

Acceptance: `tests/engine_host_connector.rs` — a loft client against a loft
server, BOTH programs transport-free: WS event round-trip, a sync-class
broadcast arriving as datagrams into the client's slots (`udp=true`), and the
return-on-disconnect lifecycle.  Green in ~1 s.

Native seats now need zero client transport code, which unblocks the phase-04
probe extensions (loss% axis, native-client stamp chain).
