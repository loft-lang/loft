<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN18 — the engine host: tiered execution + the main-loop IO contract (design)

> **Identity:** the design doc of [`@PLN18`](README.md) (engine host), slug `engine-host`.
> Grown from the @PLN16 (debugger) exploration and **graduated to this plan**.
> **Status: recorded evaluations — the build is phased in [README.md](README.md).**
> This is the C71/N9 execution model made buildable (the per-fn execution model
> [DESIGN_DECISIONS § C71](../../DESIGN_DECISIONS_PLATFORM.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model)
> names as the steady state). It gates [IDE.md](../16-debugger/IDE.md) slices **6b** (hot-swap `reload`)
> and **6c** (breakpoint-in-game). Canonical context: [LAVITION.md](../../LAVITION.md)
> (the engine), [GOALS.md](../../GOALS.md) § Purpose (live prototyping, AS/400 reliability).

The question this answers: **what does the rustc-built main loop look like** such that
(a) an edited function reaches the *running* game immediately and safely, and (b) the
loop mixes a server's **long data loads** with **short events** without either stalling
the other. Two evaluations, recorded 2026-06-10.

---

## The host-boundary principle — rich without recompiling (the keystone)

The governing question for the whole note: **can a Rust main loop carry this richness
(tick simulation, interest management, traffic classes, interpolation, forecasting)
without recompiling the host every time a feature is added?** Yes — and the existence
proof is already in-house: **@PLN51 built ALL of its richness without touching a line
of Rust.** Sight-range filtering, edge-triggered EXIT, seq numbers, the 30 Hz tick
body, Hermite interpolation, bounce forecasting — every one landed as loft code over
`lib/server`'s `poll_event()` pump; the pump's native half never changed.

**The principle: the Rust kernel owns MECHANICS; loft owns MEANING.** The kernel is
semantics-free:

- the frame cycle (timing, drift-free ticks, idle backoff),
- the socket pumps (accept / read / write, framing, reconnect),
- the queue machinery (event queues, conflation slots, budget-bounded bulk
  accumulation into store regions),
- the store + the N9 dispatch table (calling loft fns — interpreted, wasm, or native),
- the window / GL context.

Its entire contract: *"each tick I hand you drained, classified inputs and call your
loft tick/handler functions over the shared store."* Every **feature** — what an event
means, which records a pose updates, who sees whom, how a chunk publishes, what the
tick simulates — is a loft function: tier 0 instantly on edit, tier 1 wasm when the
background build lands. **rustc never enters the iteration loop.**

**The one design move that makes it stick: wire-schema-as-data.** The trap that would
force host recompiles is classification in Rust (`match msg_id { 4 => pose… }`).
Instead the loft program **registers** its traffic at startup — "msg 4 = state-sync
keyed by cid, conflate; msg 9 = bulk, accumulate to store; msg 1–3 = events" — and
the kernel's drain rules are driven by that table. This is loft's own "schema is data"
principle (`Stores.types`) extended to the wire: a new message kind is a registration
line in loft, not a host change.  *Minimal form LANDED 2026-06-10:*
`sync_class(msg_id)` is the table's first column — both roles' `send`s route
by it (datagram vs WS, per message per client) and inbound datagrams conflate
per (sender, msg_id); undeclared kinds default to must-deliver events.  The
kernel reads only the `<msg_id>:` framing — classification, never meaning.

**The honest residual — what still recompiles, and why that's fine.** Three things
genuinely need Rust: a new **traffic-class kind** (a fourth drain rule — rare,
kernel-level), a new **transport** (QUIC/UDP), a new **platform capability** (GL
feature, audio). Those are exactly C71's *library* tier — they arrive as dlopen'd
native libraries from the registry, not as edits to the host binary. The kernel's
recompile cadence becomes engine-release cadence, not feature cadence. The discipline
that keeps it true: when a feature seems to need kernel code, ask *"mechanics or
meaning?"* — meaning goes to loft, new mechanics go to a library, and only a genuinely
new queue/loop primitive touches the kernel. (@PLN51's open ~12 Hz pump finding is
the boundary working as intended: a **mechanics** fix, made once in `lib/server`'s
native half, inherited by every loft program on top.)

---

## Part 1 — tiered execution: interpret now, WASM-swap soon, native baseline

> **Canonical home:** this tier model is already specified in
> [LAVITION.md § Execution granularity](../../LAVITION.md#execution-granularity--per-function-interpret-over-a-compiled-baseline)
> — edit → the fn drops to the interpreter instantly; the server recompiles that one
> fn to optimized wasm and hot-swaps it back; graceful degradation when the server is
> unreachable. The evaluation below **re-derived that design independently** (a good
> convergence signal) — what this note *adds* is the entry-gate probes, the risk
> register, and the sequencing; LAVITION stays the canonical statement of the model.

C71's model is "interpret the edited fn on a compiled baseline." Its honest weakness:
a function in the **frame path** may blow the 16 ms budget under interpretation — and
frame-path gameplay logic is exactly what live editing is *for*. The evaluation's
synthesis is a **middle tier**, mirroring how JS engines resolved the same tension:

| Tier | What runs | Latency to take effect | Speed |
|---|---|---|---|
| **0 — interpret** | the edited fn, interpreted over the shared store (C71 minimal) | **instant** (the save) | interpreter speed |
| **1 — WASM swap** | the same fn, compiled in the background (loft → Rust → `wasm32` via the existing `--html`/wasip2 pipeline) and swapped in at a frame boundary | seconds (background; tier 0 covers the gap) | ~compiled, minus the bridge tax |
| **baseline — native** | everything untouched (rlib/cdylib) | — | full native |

The user-visible contract: **a save takes effect this frame (tier 0); the function
quietly returns to compiled speed when the background build lands (tier 1); nothing
else slows down (baseline).**

### Why WASM is the swap artifact (and not a native cdylib)

1. **Unload safety.** Unloading a native cdylib is UB-laced (dangling fn pointers, TLS,
   unwind tables); hot-swap *requires* unload. A wasm instance is a value you drop.
2. **Sandboxing = the AS/400 goal applied to hot code.** A swapped-in wasm fn cannot
   segfault the engine; a bad edit traps, is reported, and **falls back to tier 0** —
   the live loop degrades, never dies.
3. **"From the server" generalizes to ops.** A wasm module is the only sane artifact to
   ship over a wire (platform-independent, sandboxed, verifiable). The same swap
   mechanism live-patches a **remote game server** (the `server`/`game_protocol`/
   multiplayer stack) — one mechanism, dev-to-ops. This is the strongest argument that
   wasm is the *right* tier-1 artifact rather than a nice-to-have.
4. **The store is already the shared ABI across the boundary** — the wasip2 rlib + the
   WASM.md host bridges mean loft-wasm already reads/writes `Stores` through bridge
   calls. The classically-hard half of FFI hot-swap (data marshalling) is free here.

### Load-bearing risks — the probes that gate the tier (falsify before building)

- **The bridge tax.** A swapped fn is *call-bridged*, not *memory-shared* (wasm linear
  memory cannot alias the host store). Entry-gate probe: one frame-path fn measured
  interpreted vs wasm-bridged vs native — tier 1 earns its place only if it lands
  meaningfully closer to native.
- **rustc latency defines tier-1 lag** (seconds → tens of seconds). Acceptable *only
  because tier 0 exists*; wasm-swap **without** the interpret tier is a compile loop —
  exactly what C71 rejects. The tiers are a package, not options.
- **Bulk-data inner loops stay native.** Call-bridging per element is the wrong shape;
  the tier model applies to logic fns, not to hot loops over large store regions.
- **The dispatch table is the real build.** Per-fn indirection with three targets
  (native symbol / wasm export / interpreter) — N9. The tiers are just its values; its
  design (cost per call, swap atomicity at frame boundaries, identity across reloads)
  is where the engineering lives.

---

## Part 2 — the main-loop IO contract: budgeted drain, completion-as-event

**Not a threading question.** The problem is one stream from a server carrying both a
50-byte input event and a 20 MB world chunk: the failure mode is **head-of-line
blocking** (every short event queued behind the big read) and its dual, **handling a
load whenever its bytes happen to finish** (blowing the frame budget mid-frame). The
contract has two halves: *interleave on the wire, accumulate-then-publish at the loop*.

**Three traffic classes, three drain rules** (the third surfaced by @PLN51's pose
sync — see Prior art):

| Class | Examples | Delivery | Drain rule at the tick |
|---|---|---|---|
| **Short events** | input, control, world deltas | every one matters | queue → drain **to empty** |
| **Fixed-rate state sync** | 30 Hz poses, health bars | latest-value; loss fine, `seq`-numbered | **conflate to newest per sender**; a discontinuity (bounce) promotes a sample to a must-deliver event (priority keyframe — `keyframe(cid, msg)`, LANDED 2026-06-10: same seq space, reliable carrier, blackout-proven) |
| **Long loads** | assets, snapshots, wasm modules | complete-or-nothing | **byte/time-budgeted** ingest, accumulate invisibly, publish on completion as an event |

### The loop side

- **No async runtime in the engine.** GL/window (winit) demands the main thread anyway;
  a scheduler would fight the frame budget for its own thread and end up relegated to
  side threads — the drain pattern with extra steps. IO sits on plain blocking threads
  (today's serve/WS loops) feeding queues; for scale, mio/zero-timeout polling is the
  same semantics without threads.
- **Two budgets per frame tick:** short events drained **to empty** (small, bounded);
  bulk chunks ingested up to a **byte/time budget** (e.g. 256 KB or 500 µs — tunable).
  A long load trickles across frames *by design*.
- **A partial load is never visible to game logic.** Chunks accumulate silently; on the
  final chunk the load becomes an ordinary event in the same ordered queue ("asset X
  ready", "snapshot ready", "**wasm module Y ready**" — tier 1 needs no special
  transport, a module is just a long load with a verify step). The sim's world is:
  events, some of which announce completed loads. A paused debug frame therefore always
  sees a consistent event log — the IDE story stays clean.
- **Backpressure for free:** when the budget stops reading, the kernel TCP buffer fills
  and the server stalls. The ingestion budget *is* the flow control; no window protocol
  needed at first.

### Accumulate WHERE: into the store, once

The load's OPEN frame announces its size → **claim a store region up front, write
chunks directly at offset, publish the `DbRef` on completion** (abort → free). No
`Vec`-then-copy double buffering; the reassembly buffer *is* the final resting place
and "publish" is a pointer-sized commit. The store-as-shared-ABI doing the same job it
does everywhere else.

### The wire — two viable shapes

| | **A. Two channels** (control + bulk) | **B. One channel, chunked frames** (HTTP/2-ish) |
|---|---|---|
| Mechanism | events on one socket, loads on a second, reassembly per transfer | frame header `{stream_id, kind, len}`; events = one frame; loads = OPEN / CHUNK\* / CLOSE, sender-interleaved |
| HOL blocking | solved by construction | solved by interleaving |
| New protocol code | nearly none | a small mux/demux layer |
| Several concurrent loads | crude (one bulk pipe) | per-stream fairness, natural |
| Connections | two | one |

A is the classic game pattern and the cheapest first step; B is what asset streaming
grows into. Prior art in-house: the serve WS already frames; `--html` shipped a chunked
asset topology (@PLN80's asset chunk).

### The one semantic decision to settle before the protocol freezes

Events that depend on an in-flight load ("spawn entity with asset X"). Clean contracts:
**the server sequences sends** (dependent event after the load), or **game logic treats
assets as by-id references that may be not-ready** (placeholder until the completion
event). The *wrong* answer is the engine holding back dependent events — that
reintroduces HOL blocking one layer up. The choice determines whether completion events
need sequence numbers.

---

## Sequencing read (analysis, not a commitment)

1. **The in-process engine host + frame-boundary drain** — restructures `--serve`
   (engine thread + control channel; the request loop becomes one producer). Needed by
   every variant, wasm or not. Replaces the IDE's `gameStatus` polling with the channel.
2. **The N9 dispatch table with the interpreter as the only alternate target** — C71
   minimal: hot-swap-by-interpret + breakpoint-in-game (= IDE slices 6b/6c).
3. **The WASM promotion tier** on top — background compile + frame-boundary swap,
   gated on the bridge-tax probe.

Each stage independently shippable; tier 0 means the wasm tier never blocks the live
loop. Open questions worth settling early: the bridge-tax measurement, the
dependent-event contract, and whether remote-server patching is a near-term requirement
(it decides how much weight the wasm tier carries).

---

## Prior art in-house (evaluated 2026-06-10): @PLN6 runs the drain; @PLN51 runs the WHOLE loop

Two dogfood projects already prototype this design in pure loft — the second more
completely than the first.

### @PLN51 bumper-airplanes — the complete main-loop shape + measured findings

[`plans/51-bumper-airplanes/`](../51-bumper-airplanes/00a-network-probe.md)
+ `tools/audience-demo-50/` (working MVP tests). `probe_server.loft`'s main loop **is**
this note's loop contract, verbatim:

```
while true {
  1. drain pump events to empty          (srv.poll_event() until null)
  2. fixed-rate tick when due            (30 Hz broadcast_tick; drift-free:
                                          last_tick_us += TICK_INTERVAL_US, never = now)
  3. idle backoff                        (sleep 2 ms only when zero work)
}
```

Beyond the loop, it contributes pieces this note had not named:

- **A THIRD traffic class: fixed-rate state sync.** 30 Hz pose frames are neither
  short *events* (must deliver every one) nor long *loads* (accumulate then publish) —
  they carry **latest-value semantics**: loss is fine, stale frames are superseded,
  each plane carries a `seq` that ticks per broadcast. The drain rule differs: events
  queue-to-empty; poses **conflate to newest per sender**.
- **Interest management** — `peer_sight_range` filtering shapes *outbound* per
  recipient (a phone sees ~5–10 peers, not N−1; the projector is unfiltered), with
  **edge-triggered EXIT** signaling (per-tick visibility diff fires exactly once on an
  outward crossing). Rate-LOD bands (30/15/7.5 Hz by distance) designed, deferred.
- **Reconstruction findings, measured** (`interp_test.loft`, ground-truth trajectories):
  Hermite cubic is the default interp (linear shows 67 mm error on circular motion at
  7.5 Hz; Hermite ~0); **a discontinuity (bounce) must emit a priority keyframe** —
  no interpolation hides a missed bounce (0.3–1.0 m peak error). I.e. *discontinuities
  promote a state-sync sample into a must-deliver event* — the classes interconvert.
- **Server-side forecasting probed** (`forecast_test.loft`): can the server predict
  bounces ahead, and how much does an input change invalidate the forecast —
  speculative sync as a measured question, not a hope.
- **The probe discipline this plan should copy**: explicit targets + failure thresholds
  (30 clients × 30 Hz both ways, p99 < 100 ms, zero drops, CPU < 75%) with graduated
  acceptance (20-cap fallback / architecture-rethink trigger). **Known open finding:**
  the MVP observed ~12 Hz per peer vs the 30 Hz target — the pump throughput question
  is inherited by this design and is exactly what the probe exists to settle.

### @PLN6 audience demo — the event-world shape

The @PLN6 audience demo (`tools/audience-demo/` + the `server`/`web`/`graphics` registry
libs) prototypes the event-driven half:

- **The client side IS the frame-boundary drain.** `projector.loft`'s main loop:
  `while gl_poll_events() { while (msg = ws.try_recv()) != null { apply_frame(...) } …
  cam_step … lazy VBO rebuild on world.version change … render }` — a non-blocking
  drain-to-empty, mutations into world state, version-keyed incremental rebuild
  (dirty render-groups only), then draw. Exactly Part 2's loop contract, minus the
  budgets.
- **The server side is the event-callback shape.** `server.loft` holds the world and
  runs `srv.run(fn(ev) { … broadcast(delta) … })` over `lib/server`'s
  `<msg_id>:<payload>` text framing — purely event-driven (no frame loop), broadcasts
  every change, replays state to new connections; `lib/server` absorbs disconnects and
  malformed frames (the no-runtime-halt preference, in practice).
- **The long-load problem is solved by EVENT-DECOMPOSITION instead of chunking:** the
  "snapshot" (the demo's only bulk transfer) is replayed as many small idempotent
  delta events, with a capped re-request watchdog (retry while the world stays empty).
  That is a third wire shape Part 2 should name: when a load *can* be decomposed into
  idempotent events, no bulk path is needed at all — reassembly, budgets, and
  completion-events collapse into ordinary event handling + a re-request for loss.
- **Resilience patterns worth keeping:** `lib/web`'s auto-reconnecting `ws_handler`;
  the snapshot watchdog (re-request, capped attempts).

**The deltas this design still adds over the demo:** (1) the demo's drain is
**count-unbounded** — a many-thousand-delta replay lands in one frame (a visible
hitch); Part 2's byte/time budget is the fix. (2) No true bulk path exists — fine for
hex deltas, not for assets/wasm modules that cannot be event-decomposed; that's where
the chunked/store-accumulation shape earns its place. (3) No store-resident
accumulation (nothing needed it yet). The demo therefore validates the loop contract
and the event-server shape, and sharpens Part 2's wire options to **three**:
two-channel, chunked-frames, or **decompose-into-idempotent-events** (preferred
whenever the data model allows it — it is what the demo proves).

---

## One kernel, two roles — server and client share the implementation (evaluated 2026-06-10)

Server and client are **the same kernel in two configurations**, not two implementations:

| Kernel piece | Server | Client | Verdict |
|---|---|---|---|
| Frame cycle | timer-paced sim tick (30 Hz) + idle backoff | vsync-paced render + the same fixed sim tick | **shared** — cadence source is a parameter |
| Socket pump | *listener* (accept N, broadcast) | *connector* (1 conn, auto-reconnect) | **shared core, two thin frontends** (today's `lib/server` + `lib/web`, unified) |
| Queue machinery (3 classes, conflation, budgeted bulk → store) | identical | identical | **fully shared — must never fork** or the wire gains two interpretations |
| Store + N9 dispatch (tiers) | live-patching the server needs it | live-editing the game needs it | **fully shared** |
| Window / GL | — (headless) | the one client-only module | **feature-gated** (`--features window`), never forked; server builds stay GL-free |

The asymmetries people expect to force a fork — authority, validation, prediction,
interpolation — are all **meaning, not mechanics** (loft fns per the boundary
principle), so they never touch the kernel. In-house evidence: `probe_server.loft`
and `projector.loft` already run the same loop body, differing only in cadence
source and the GL tail.

**Status (2026-06-10): both roles are LANDED** — `run` (listener) and
`run_client` (connector) in `lib/engine_host`, sharing the frame I/O, the
drift-free tick, the `sync_class` table and the `conflate_slot` machinery
(factored shared in `src/engine_host.rs`, so the queue semantics *cannot*
fork).  The connector auto-hellos from the `X-Loft-UDP` 101 header and
keepalives at 500 ms; `tests/engine_host_connector.rs` is the loopback proof
(a loft client against a loft server, both transport-free).  The one
deliberate asymmetry: `run_client` returns when the server dies; `run`
serves forever.  Window/GL remains the future client-only module.

**Update (2026-06-12): the standalone windowed host landed as `run_local`** —
the third kernel configuration, proving "cadence source is a parameter": the
connector loop with NO transport (`ClientKernel.conn: Option`, a `kernel_local`
native on all three calling conventions).  A windowed program with no server
gets the same drift-free tick (one tick = one frame), frame yield, swap
machinery and debug control endpoint; `send` reports false (no peer), and
moving the program online later means swapping `run_local` for `run_client` —
the handlers never change.  Regression:
`tests/engine_host_kernel.rs::run_local_ticks_and_stops_without_a_server`
(both backends).

**Update (2026-06-12, later): the crawler K2 trio.**  `post(msg)` enqueues a
local event on whichever role runs — window input becomes an ordinary
events-class message (`cid: -1` marks local origin; the connector loop now
reads the real cid via `kernel_client_event_cid` instead of hardcoding the
server's 0), so handlers treat keys and remote messages identically and
intent-shipping (K4) serializes the stream that already exists.  The listener
gained its exit (`stop()`; `run` loops on `kernel_alive()`) and the per-turn
`kernel_frame()` yield — a windowed LISTENER (draw in `on_tick`, observers
connected) now works on both native and browser contracts.  Regression:
`tests/engine_host_kernel.rs::post_and_stop_in_both_roles`.

Compounding payoffs: **loopback testing** (server + client kernels in one process
over an in-memory channel — the whole protocol tested without sockets; also
single-player for free), **the IDE host converges** (`--serve`'s hand-rolled WS loop
becomes the listener-role kernel), and **no protocol drift** (one traffic-class
implementation). Honest residual: the **browser** client (`--html`/wasm) is a
genuinely different host — the frame-yield contract instead of owning the loop — so
it shares the loft-side contract but not the kernel binary.

---

## Services — register meaning, assign speed later (design verified 2026-06-10)

The developer surface over the class table, in the user's own framing: *"all a
developer has to do is register services (listener & writer combined); that
service can send/receive messages; after building everything he can decide
that some services need a faster bus than others and can assign some to a
fast lane and some to a slow one (some stay normal)."*

**The invariant: a service is the ONE home for a message kind** — its handler
(listener), its writer, and (late, separately) its **lane**:

| Lane | Class underneath | Delivery contract (both directions) |
|---|---|---|
| **fast** | state-sync | newest wins: inbound conflates, outbound rides datagrams when the peer can |
| **normal** | events | must-deliver, in order, on WS — the default |
| **slow** | bulk | budget-ingested into store regions, publishes a completion event (05c machinery) |

Why this shape wins:

- **Late binding of performance.** Lane assignment comes AFTER the build —
  one reversible line per service, no restructuring.  Goal F applied to
  performance: meaning first, speed as a tweak.  (Live re-assignment from the
  debugger/REPL is the natural extension — the lane is just table data.)
- **One home per kind.** The `sync_class` step already collapsed transport
  choice into data; services collapse the remaining duplication — the msg_id
  literals in send strings and the `handle_message` if-chain every consumer
  hand-rolls (audience + probe servers both have one) become registration.
- **The lane never leaks into service code.** A service reads and writes
  messages; the kernel owns delivery.  `slow` can exist in the API before
  05c's machinery lands (behaves as normal until then) — declaring it is
  data, not behavior the developer observes.

Settled while verifying (2026-06-10):

- **Wire identity stays the explicit `msg_id`** — browser pages hand-roll
  `"2:..."` today; auto-numbered services would need a schema handshake with
  JS clients.  The end state (service NAMES on a shareable schema the client
  fetches) layers on later without breaking this.
- **`sync_class` is the lane column of this table**, already landed; the
  service registry adds the handler + writer columns.
- **`on_event` stays as the default service** for unregistered kinds —
  consumers migrate one service at a time.

**The gate probe ran (2026-06-10, post-#325 rebase) and answered:** the
registry column is language-gated.  #318 rejects collections of
closure-holder structs at compile time (element copies would dangle), and a
bare `vector<fn>` of CAPTURING closures is @P213/@P214-deferred — both with
graceful, prescriptive errors.  **v1 therefore shipped the other columns:**
the lane vocabulary (`fast_lane`/`fast_lane_keyed`/`slow_lane`) as late
data, and the ONE receive surface — `run`/`run_client` drain the conflation
slots into `on_event` at tick time, so the lane never leaks into handlers
and consumers' manual drain loops dissolved.  The per-kind split is a match
inside `on_event` (the shape #318's guidance prescribes); when @P213/@P214
lands, that match becomes the registered-handler column with zero wire or
lane changes.

---

## Coverage — is this rich enough for most multiplayer games? (evaluated 2026-06-10)

**Yes for most; the residue reduces to one primitive + one transport.** Covered by
construction: party/audience, turn-based (events + @PLN43 durable stores), co-op
(one-kernel → host-as-server falls out), MMO-lite/persistent (interest mgmt +
rate-LOD + durable stores + lenient migration), spectator/replay (the projector IS an
unfiltered spectator; deterministic tick + event log ≈ replay).

**The gap tier — competitive twitch (FPS/fighting):** rollback, lag-compensation
rewind, client prediction + reconciliation, delta-compressed snapshots. Under the
host-boundary principle these all decompose into loft *meaning* over **one new
mechanics primitive: cheap store snapshot / restore / diff with a short tick-history
ring** — rollback = snapshot + re-tick; lag comp = read an old ring entry; delta
encoding = store-diff-as-protocol; reconciliation = client-side rollback. loft already
holds three embryos of this machinery (the @PLN16 M2 undo journal = store
save/restore; the store journal work = change capture; record-references #15 = stable
identity across states). Four classically-hard features, one primitive — the
store-centric design is what makes that collapse possible.

**The transport gap:** WebSocket/TCP retransmit stalls hurt twitch play; a
**UDP/QUIC pump frontend** is C71-library-tier mechanics, and the traffic classes are
transport-agnostic by construction (conflation maps *more* naturally onto unreliable
delivery than onto TCP).

**Acceptable residuals:** lockstep RTS (cross-machine determinism — Goal D is already
a determinism discipline, made load-bearing by the tier swap, but lockstep stays
non-first-class), host migration, NAT traversal/relays (a relay is a forwarding
listener-role kernel), TLS, matchmaking (meaning-level, nothing new). **The test that
matters: nothing in the design must be *undone* to add any of these.**

---

## The UDP pump frontend — a custom layer the class table keeps small (evaluated 2026-06-10)

A custom UDP layer slots in as a **pump frontend** (mechanics, library tier), and the
wire-schema-as-data table does the classically hard part — per-message guarantees are
already declared per `msg_id`, so the UDP layer reads the SAME table the drain rules
read:

| Class | Delivery over UDP | Custom work |
|---|---|---|
| Events | reliable + ordered | the one real piece: seq + ack-bitfield + retransmit (Gaffer-style channel, ~hundreds of lines) |
| State sync | raw datagrams, `seq`-stamped | **nothing** — conflation already tolerates loss/reorder; never retransmit a stale pose |
| Long loads | stay on the TCP/WS channel | nothing (the two-channel shape; bulk *wants* TCP behaviour) |

Remaining mechanics: stateless-cookie handshake (spoofing), keepalive/timeout, MTU
fragmentation for oversized events, sender pacing (token bucket — the receive budget
is the other half), DTLS / `crypto`-lib encryption.

Consequences: **(1) heterogeneous transports per client, one server** — the pump core
feeds the same class queues from any frontend, so browser phones stay on `wss` while
native clients ride UDP in the same world (WebTransport / WebRTC datachannels are the
eventual browser-side unreliable frontend); **(2) no async runtime** — a nonblocking
`UdpSocket` polled by the pump thread fits the no-tokio stance; hand-rolled stays
small and auditable, `quinn`/QUIC is the heavier fallback if congestion control
outgrows a token bucket; **(3) deterministic netcode tests** — the in-process
loopback channel gains loss/reorder/duplicate injection, so the reliability layer is
tested without a network, and @PLN51's probe targets extend with a loss% axis.

### Broadcast bulk: measured, never assumed (user-directed 2026-06-10)

Whether LAN broadcast actually helps is **environment-dependent in both
directions** — some APs forward broadcast at base rate, some filter it
entirely, some convert multicast to unicast; wired switches flood it for
free.  A static rule ("wired = broadcast, wifi = unicast") misclassifies
real venues both ways.  So 05c selects by **measurement, not classification**
— and the NACK-chunk protocol is its own measuring instrument:

1. **Probe burst**: a transfer opens with K chunks sent via broadcast; each
   seat's first bitmap ack reports how many arrived.  That per-seat delivery
   rate IS the decision variable — no separate benchmarking machinery.
2. **Per-seat join/demote**: seats above the threshold (where
   `repair traffic < full unicast send`, i.e. broadcast loss below ~30–50%)
   ride the broadcast group; seats below get unicast chunk streams.  The
   bitmap keeps measuring DURING the transfer, so a seat whose broadcast
   loss spikes is demoted mid-flight.
3. **Transport-fungible chunks**: a chunk is idempotent and the bitmap does
   not care how it arrived — broadcast, unicast UDP repair, or WS.  So the
   fallback ladder (broadcast → unicast UDP → WS) is not a mode switch;
   it is just where a seat's next chunks come from, converging on the same
   completion event.

This is the 05a transport contract extended one level: the kernel measures
and picks per seat (and per chunk); meaning never branches on transport.

### UDP on a normal LAN — what's actually needed (evaluated 2026-06-10)

Engine-side (small): one well-known UDP port (client identity = the datagram's source
4-tuple + the handshake cookie — clients need no config); discovery reuses the demo's
IP-harvest + QR, optionally upgraded with a **broadcast discovery beacon** (~30 lines;
broadcast for discovery ONLY — wifi sends broadcast at base rate and APs filter it,
so gameplay stays unicast); keepalive at a steady cadence (doubles as the phone
wifi-power-save radio wake — idle radios add 100 ms+ bursts); datagrams ≤ ~1200 B
(VPN/overlay-shaved MTUs fragment silently above that).

Environment-side (the real blockers, in order): **(1) the server's host firewall**
(inbound UDP allow — the #1 cause of "UDP doesn't work"; the TCP port has the same
requirement so the runbook already crosses it); **(2) AP client isolation** on
venue/guest wifi (kills TCP too — THE classic venue killer; needs an SSID without
isolation or the server wired); **(3)** same subnet = **no NAT machinery at all**
(the relay/port-forward residual starts only when a peer leaves the LAN);
**(4)** browser phones still can't UDP on LAN — and WebTransport/WebRTC want TLS,
which is painful for bare LAN IPs (self-signed ceremony) — so the transport split
(phones `wss`, native peers UDP) is the LAN answer too.
