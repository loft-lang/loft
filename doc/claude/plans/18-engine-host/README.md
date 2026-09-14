<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# @PLN18 — Engine host: the C71/N9 execution-model build (one kernel, tiers, main-loop IO contract)

> **Subject:** loft   ·   **Type:** plan   ·   **Area:** native · runtime · server
> **Effort:** H   ·   **Value:** Enabling
> **Driven-by:** the @PLN6 audience demo + @PLN51 bumper-airplanes (the dogfood
> consumers), and @PLN16's 6b/6c (hot-swap `reload` + breakpoint-in-game) which are
> gated on this plan
> **Depends-on:** —  (consumes existing substrate: `lib/server`'s pump, the wasm
> pipeline, the @PLN16 debug engine)
> **Live status · milestone · order:** [loft-lang/plans ▸ @PLN18](https://github.com/loft-lang/plans/issues/18)  ← single source of truth for lifecycle

## Status

**DONE / SHIPPED — closed 2026-07-08.** The engine-host execution model is built,
tested, and on `main`; the plan closes on merge of this close-out (`Closes @PLN18`).
Verified: all engine_host tests green
(`tests/engine_host_{audience,kernel,reload,udp,connector,probe,http}.rs` — 27 tests,
0 failed, 0 ignored).

**Shipped:** 01 kernel · 02 live dispatch/reload (`LOFT_LIVE_RELOAD`) · 04 state-sync
at 30×30 Hz · 05a UDP quick channel · 05d services v1 (lanes-as-late-data + one
receive surface) · 07 browser kernel (native ↔ headless-chromium differential green) ·
**08 live-build-swap — all eight scenarios**. 03 (per-fn wasm tier) superseded by 08.

**Deferred — each behind an explicit trigger, design retained in the phase table below
+ [ENGINE_HOST.md](ENGINE_HOST.md):** 05b full-UDP frontend · 05c UDP bulk push · 06
snapshot/restore ring · 08's five residuals (line-keyed breakpoints, compiled-`--html`
swap, full-expression eval at a pause, snapshot coverage growth, mid-freeze event
loss). 05d's service *registry column* waits on the closure-in-collection language
feature (P213/P214); 07's real-phone probe is a manual hardware check. None is
open/unfinished work — all are future-triggered, language-blocked, or hardware.

## Thesis

Build the **lavition engine host**: a semantics-free Rust kernel (frame cycle, socket
pumps, queue machinery for three traffic classes, wire-schema-as-data, store-resident
bulk accumulation) plus the **N9 per-function dispatch table** over the shared store —
[C71](../../DESIGN_DECISIONS.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model)
made buildable, with the tier model
([LAVITION § Execution granularity](../../LAVITION.md#execution-granularity--per-function-interpret-over-a-compiled-baseline)
is the canonical statement: edit → interpret instantly → background wasm swap).
Until this ships, no loft game can be **live-edited while it runs**: the IDE
(@PLN16) can launch a game only as an opaque child process, the audience demo cannot
be patched mid-show, and @PLN51's 30 Hz pose sync has no kernel to run on. The
governing property is the **host-boundary principle**: the kernel owns *mechanics*,
loft owns *meaning* — new features land as loft functions (tier 0 instantly), never
as host recompiles.

**Current focus (the user's call, 2026-06-10): this is the main goal.** Get the
**Rust version of the main loop running** (phase 01, entered through phase 00's
measurements) and make a **quick UDP channel possible beside the websockets**
(phase 05a — the minimal cut: datagrams for the state-sync class only, everything
reliable stays on WS). The arcade-machine model (§ below) is what these two serve.

## Phases

| Phase | Goal | Status | Outcome / ref |
|---|---|---|---|
| 00-probes | The entry-gate measurements: (a) the wasm **bridge-tax** probe (one frame-path fn: interpreted vs wasm-bridged vs native — tier 1 only earns its place if meaningfully closer to native); (b) adopt @PLN51's [`00a` network probe](../51-bumper-airplanes/00a-network-probe.md) as the pump harness and settle the **~12 Hz vs 30 Hz** finding (the mechanics fix lands once in `lib/server`'s native half); (c) **the turn-around stamp chain** — extend the probe's `t_us` seed into a per-phase breakdown of the full round trip (`t0` input → `t1` pump recv → `t2` drained → `t3` tick consumed → `t4` broadcast enqueued → `t5` sent → `t6` client recv → `t7` rendered): server-local deltas need no clock sync (one clock — exposes drain lag, **tick wait** the dominant hidden phase, send-queue time exactly), the wire legs come from the echo (RTT = (t6−t0) − (t5−t1), split /2 on a symmetric LAN). Baseline the LAN first (`ping` / `iperf3 -u` / `ss -ti`, check `TCP_NODELAY`) so the loop is never debugged against a wifi problem. The stamp chain graduates into a **debug-mode kernel primitive** in phase 01 (stamp at each queue boundary when enabled) — which later feeds the IDE a live pipeline-breakdown panel | ✅ v1 complete (2026-06-10) | (a) exec ratios: interp 6×, **wasm ≈ 1.0× native** (bridge cost → 03); (b) root-caused (20 ms blocking poll × in-order scan; sweep 252 ms→17 µs @12, 6 µs @30; fix on `loft-libs-net:pump-fast-idle-poll`, lands with the arc) + 30 Hz held at 30 clients; (c) stamp chain live — hold p50 ≈ half-tick (the tick-wait, measured), ~40 ms residual = interp harness (the kernel's acceptance metric). [probes/00b-baseline](probes/00b-baseline-2026-06-10.md). Baseline reproduced 2026-06-10 — [probes/00b-baseline](probes/00b-baseline-2026-06-10.md): 12 clients × 10 s → ~27–45% of target pose rate (the ~12 Hz band) + **a new finding: EXIT events drop under pose load** (4/11 clients missed a must-deliver event on reliable transport → the pump starves the event class; the class-separation argument, measured) |
| 01-kernel | The one-kernel loop (drain → tick → output; drift-free timing; idle backoff) + queue machinery (events queue-to-empty · state-sync conflates per sender · bulk budget-ingests into claimed store regions, publishes as a completion event) + **wire-schema-as-data** registration. Listener/connector pump roles over one shared core; window/GL feature-gated. **Accepted by migrating the @PLN6 audience server onto it** — identical observable behaviour (the existing `load_test.loft` + probe suites are the harness) | ✅ complete (2026-06-10) | [01-kernel.md](01-kernel.md) — natives (src/engine_host.rs) + `lib/engine_host` loop skeleton; closures-as-arguments (#313), struct-held world (#314); **audience server ported + differential acceptance green** (`tests/engine_host_audience.rs`: kernel ≡ original transcripts); **00(c) stamp chain re-run on the kernel**: server hold p50 18.6 ms @12 ≈ the half-tick floor (16.7 ms by-design tick wait + ~1.9 ms drain; baseline 253.6 ms — the interp-harness pump term collapsed), N-stable @30 (22.8 ms, tick-bounded) with full 30×30 Hz pose fan-out (~29.8 poses/tick at center, ~21 k sends/s) — the phase-04 checkpoint at probe level. En-route fix: package-file-shadows-declared-dep parser bug (`Parser::lib_path` guard + `tests/fixtures/dep_shadow/`). Deferred: bulk class (05c). Conflation slots + the wire-schema table (minimal form) + the **connector role** (`run_client`) all landed in/after 05a; the in-binary-native cdylib warning fixed by `[native] in_binary` (05a). **The third role landed 2026-06-12: `run_local`** (standalone windowed host — the connector loop with no transport; #343, ENGINE_HOST.md § Update 2026-06-12), adopted same-day by the crawler consumer (its PLAN-KERNEL K1). The crawler K2/K4 flow-back asks **landed 2026-06-12**: `post(msg)` (local-event enqueue, any role — window input as the events class, `cid: -1` = local origin, carried by the new `kernel_client_event_cid`), the listener's `stop()` (mirror of `client_stop`; `run` loops on `kernel_alive()`), and `kernel_frame()` riding every `run` turn (the windowed *listener* frames in a browser too). Regression: `tests/engine_host_kernel.rs::post_and_stop_in_both_roles` (both roles × both backends) |
| 02-dispatch | The N9 per-function dispatch table, **interpreter target first** (= C71 minimal): swap one fn of the *running* demo to interpret on edit, over the shared store. This phase lights up @PLN16 **6b** (hot-swap `reload`) and **6c** (breakpoint-in-game → the IDE's variables panel against a live frame) | ✅ SHIPPED (2026-06-11) | [02-dispatch.md](02-dispatch.md) — `LOFT_LIVE_RELOAD=1`: edit a named fn of a RUNNING kernel server and it swaps live with world-state continuity (`tests/engine_host_reload.rs`: clean swap / broken-edit-keeps-old-body / fix-up / signature-rejection). The table was already there in two halves (`fn_positions` per-call fn-ref dispatch + the `calls` patch list); reload = shadow-session temp-name parse + append-only `def_code` + patch (src/live_reload.rs). Slice 1 closed: `State::reenter` (320 ns/call) + the frame contract (stack_high; the standing `dispatch_reentry` test). 6b wired: IDE-launched games default to live reload. The native-baseline build = 08-S2 (one home; the ABI finding — generated args ARE reenter's push contract — recorded there) |
| 03-wasm-tier | The background promotion tier: server compiles the edited fn (loft → Rust → `wasm32`) and the kernel swaps it in at a frame boundary; a trap falls back to tier 0. Gated on the phase-00 bridge-tax probe; unlocks remote-server patching (the same swap artifact over the wire) | ⏸ superseded as the promotion tier by [08](08-live-build-swap.md) (2026-06-10): native hosts swap WHOLE builds (no wasmtime, no bridge tax), browsers swap whole modules in the living page. 03's exec measurements stand; per-fn wasm modules remain only a possible future for sandboxed third-party patches | [03-wasm-tier.md](03-wasm-tier.md) — invariant: tier 1 changes a dispatch target, nothing else. Exec ≈ native is measured; the OPEN gate number = the per-call bridge tax on a store-heavy frame-path fn (slice-1 probe). wasmtime embeds behind `--features wasm-tier` (weight measured before committed); per-fn modules so swap = drop-instance; trap → tier 0 + warning; remote patch = same artifact + source-hash gate over the bulk path |
| 04-state-sync-at-rate | @PLN51's probe at **30 clients × 30 Hz** on the kernel: the fixed-rate traffic class proven at target (sight-range interest management, edge-triggered EXIT, priority keyframes), with the probe's published targets + failure thresholds. Unblocks bumper-airplanes phase 0 | ✅ measured at target (2026-06-10) | [probes/04-at-rate](probes/04-at-rate-2026-06-10.md) — tier ladder 3/12/20/30 all ≥ 30.3 Hz/peer (target ≥28); EXIT delivery symmetric/deterministic (the phase-00 starvation pathology absent — separate queues by construction); native seat via the connector: **e2e = one tick (33.34 ms p50, 33.4 p95)** — 3× inside the 100 ms target; two findings: tick-grid quantization (e2e = k×T constant, the wire is µs-invisible → T and phase are the only levers) and cadence phase-lock (input cadence dividing T pins one fixed phase — de-alias or nudge); loss axis @25% injected: **linear degradation, survivor latency unchanged, no stall** (the sync-class claim measured; `LOFT_UDP_DROP_NTH` instrument + `sync_class_keyed` for entity-keyed kinds landed en route). **Priority keyframes LANDED** (`keyframe(cid, msg)`): one sync sample promoted to must-deliver — S:-framed on the reliable channel in the SAME seq space as the datagrams, so the receiver's slots keep total order (an in-flight older sample goes stale); proven by `keyframes_survive_total_datagram_loss` (100% injected drop — only the keyframe arrives). WHAT counts as a discontinuity stays consumer meaning |
| 05a-udp-quick-channel | **The quick UDP channel beside the websockets** (in scope per the user's call, 2026-06-10): a datagram channel for the **state-sync class only** — events + bulk **stay on WS**, so no reliable-UDP layer, no fragmentation (poses are small), no congestion control is needed; just the stateless-cookie handshake (identity = source 4-tuple + cookie tied to the WS session), `seq`-stamped ≤1200 B datagrams into the conflation slots, keepalive-as-radio-wake, host-firewall note in the runbook. The minimal cut the class table makes possible — native peers gain the UDP fast path while phones stay `wss`, same world | ✅ v1 shipped (2026-06-10) | [05a-udp.md](05a-udp.md) — same-port UDP socket, `H:`/`A:` cookie handshake, negotiated fully kernel-internally (`X-Loft-UDP` header on the WS 101 — no loft code touches it; GOALS § Goal F engine-surface instance), conflation slots live (first consumer), `sync_send` transport-transparent (UDP when bound, WS fallback), 3 s keepalive timeout, ≤1200 B cap. E2E green (`tests/engine_host_udp.rs`: conflate-to-newest, stale-discard, timeout-revert; + the real-consumer proof — probe_server_kernel declares `sync_class(2/5)` and uses ordinary `send` everywhere; web client gets WS / native client gets datagrams from one call site). **Classes are declarative** (wire-schema table, minimal form): `sync_class(msg_id)` once at startup; no per-call transport API (`sync_send` absorbed); inbound conflation per (sender, msg_id). **Transport selection is automatic per client** (user-confirmed contract): a native client hellos (it CAN speak UDP) and `sync_send` rides datagrams; a web page cannot send UDP, never binds, and the same call stays WS — meaning-code never branches on transport. The connector kernel (`run_client`, landed) auto-performs the hello — automation is end-to-end for native seats with zero client transport code (`tests/engine_host_connector.rs`) |
| 05b-udp-full-frontend | The full custom UDP frontend ([design](ENGINE_HOST.md#the-udp-pump-frontend--a-custom-layer-the-class-table-keeps-small-evaluated-2026-06-10) + [LAN notes](ENGINE_HOST.md#udp-on-a-normal-lan--whats-actually-needed-evaluated-2026-06-10)): the Gaffer-style reliable event channel, MTU fragmentation, pacing, DTLS, LAN discovery beacon | ⏸ parked — **trigger:** a consumer needs reliable traffic off TCP (measured by the phase-00 harness + a loss% axis), or the discovery beacon gets demanded by a LAN consumer | |
| 05c-udp-bulk | **Bulk over UDP — the one-to-many push** (evaluated 2026-06-10, user-directed): the arcade-cabinet shape is N seats receiving the SAME level/world/asset pack — TCP must send N copies (30 seats × 100 MB = 3 GB through the cabinet NIC); UDP **broadcast/multicast sends it once** to every wired seat, NACK rounds mop up each seat's losses individually. The custom piece is a NACK-based chunk protocol (~200–300 lines kernel mechanics): ≤1200 B chunks, receiver bitmap, NACK rounds at tick cadence, sender pacing (token bucket — unpaced floods silently drop 30–50% in the receiver's socket buffer), integrity check; simpler than a general reliable channel (no ordering — complete-or-nothing). Slots in as a third frame type (`B:`) over 05a's socket + cookie handshake, budget-ingests into claimed store regions, publishes a completion event (the bulk-class design in ENGINE_HOST.md). Wifi/phone seats fall back to unicast chunks or plain WS (broadcast is base-rate + AP-filtered on wifi — wired-seats-only win). For a SINGLE receiver TCP is already ≈line-rate on a LAN — that case stays on WS by design. **Broadcast is measured, never assumed** ([design](ENGINE_HOST.md#broadcast-bulk-measured-never-assumed-user-directed-2026-06-10)): the probe burst + per-seat bitmap acks decide who rides broadcast; seats demote to unicast/WS mid-transfer when their measured loss flips the math — chunks are transport-fungible, so fallback is not a mode switch | ⏸ parked — **trigger:** a consumer that pushes big payloads to many seats (level/world push; phase-04 / bumper-planes era or the asset pipeline) | |
| 05d-services | **The service surface** (user-designed 2026-06-10, [design](ENGINE_HOST.md#services--register-meaning-assign-speed-later-design-verified-2026-06-10)): register services (listener & writer combined — the ONE home for a message kind), then assign lanes LATE (fast = state-sync / normal = events / slow = bulk) — one reversible line per service, after meaning works; the lane never leaks into service code. Dissolves the hand-rolled `handle_message` if-chains; `sync_class` (landed) is this table's lane column; `on_event` stays as the default service so consumers migrate one service at a time | ◐ v1 SHIPPED (2026-06-10) | The gate probe ran on the rebased tree and CLOSED the registry column for now: capturing closures cannot live in ANY collection (#318 rejects holder-struct collections; bare `vector<fn>` of captures = @P213/@P214 deferred — the language names the boundary itself). What shipped is everything else the model promised: **lanes as late data** (`fast_lane`/`fast_lane_keyed`/`slow_lane` — the user's vocabulary over the class table; slow declared now, lands with 05c) and **ONE receive surface** — `run`/`run_client` drain the conflation slots into `on_event` at tick time (late-latched), so handlers never know the lane and every consumer's manual drain loop dissolved (probe client/server, fixtures — measurably smaller). The per-kind handler split stays a match inside `on_event` — the shape #318's own guidance prescribes — and becomes the registry when @P213/@P214 lands |
| 07-browser-kernel | **The phone as the third host of one program** (user-directed 2026-06-10): the connector role rebuilt on what the browser already provides — WebSocket as the pump, the event loop as the idle, `performance.now()` as the tick grid — with the pure machinery (conflation, wire-schema table, keyframe parse, queues) compiled UNCHANGED from the native kernel (one implementation, two targets). **The invariant: the script is the contract; the kernel is swappable** — the same client `.loft` runs on a native seat and a phone with zero edits. Five seams close it kernel-side: same-symbol wasm registration, the yield hidden in the shared lib (per-turn `kernel_client_frame`), time (bridge verified — `host_time_ticks`), one pointer-event input surface, `default_host()`. Acceptance = the one-script differential (native vs headless-chromium, equal transcripts) | ◐ core + acceptance SHIPPED (differential green 2026-07-07; only the manual phone probe remains) | [07-browser-kernel.md](07-browser-kernel.md) — engine_host compiles on every target (pure half shared, sockets cfg-native, browser kernel = the `browser` submodule over the existing host_ws_* bridge, same symbols via KERNEL_FUNCTIONS_WASM); per-turn yield in the shared loop; default_host(); inbound WS class-routing completes carrier symmetry (conflate_ws). Two real time bugs fixed (loft-rt 0-stub default; loft-gl ms-for-µs 1000×). **Acceptance GREEN:** the one-script differential (`engine_host_connector::browser_kernel_one_script_differential`) and touch both shipped; the differential is a live `#[test]` gate (native leg + headless-chromium browser leg produce the identical transcript, self-skips without chromium/node) — verified green 2026-07-07. The `Instant::now`/parked-frame findings it caught are both fixed (`compile_and_start` + `resume_frame`). Sole residual: the manual real-phone probe (hardware day) |
| 08-live-build-swap | **The full loop, one invariant per host** (user-designed 2026-06-10): compiled baseline → debugger flips fns to the interpreter → live body-only edits → mixed run → background FULL rebuild → **swap the whole build under the running world** — native = process cutover over a store snapshot; browser = module swap inside the living page (the page keeps the WebSocket + canvas; the new wasm arrives over the WS bulk channel — 05c's first consumer). *The build is replaceable; the state (store) and the connections (host layer) persist.* Supersedes 03's per-fn wasm modules as the promotion tier (03's measurements stand; its browser variant folds in here). Eight scenarios each paired with tests + six probe gates | ✅ **ALL EIGHT SHIPPED** (2026-06-11) | [08-live-build-swap.md](08-live-build-swap.md) — S1 typed twins · S2 live flip (parse-seeded shared world, entry checks inside the callee) · S3 live edit (fn_positions = the one dispatch home) · S4 background rebuild (`--check --native` orchestrated; snapshot-vs-settled staleness) · S5 native swap (swap_world + SO_REUSEPORT overlap handover, ~35 ms gap, rollback-by-default, SOCK_CLOEXEC) · S6 browser swap (living page; compile_and_start pairing; ws adoption) · S7 debugger loop e2e (D!: control channel, pause with mechanics alive, name-keyed bp re-resolution) · S8 the standing four-state differential (self-swap mid-sequence, byte-equal).  Residuals listed in § Scenario scoreboard |
| 06-snapshot-ring | The store **snapshot / restore / diff** primitive with a short tick-history ring — the ONE mechanics piece behind prediction, lag-compensation rewind, rollback, and delta-compressed snapshots (all loft *meaning* over it; embryos exist: the @PLN16 M2 undo journal, the store journal, record-refs #15) | ⏸ parked — **trigger:** an arcade consumer needs client prediction / hit-rewind (the bumper-planes forecast tests are the likely first caller), or replay/delta encoding gets demanded by a shipped game | |

## The goal this plan serves — arcade-style multiplayer (one machine, distributed)

This goal is an instance of the project's purpose
([GOALS § Purpose](../../GOALS.md) — **bringing fun to game developers and
players**): for *players*, the arcade machine is the most fun-dense multiplayer form
ever built — walk up, grab the stick, compete with the people in the room; for
*developers*, the same-machine model is the most fun to BUILD on — one world, one
tick, no netcode illusions to maintain, and the whole thing live-editable while
friends are playing it (the @PLN16 IDE + this kernel). Every technical choice below
serves that, not a latency spec.

**Arcade-style multiplayer is a named goal of the engine** (the user's call,
2026-06-10), and the model is precise: **a traditional LAN setup where people compete
as if they were on the same arcade machine.** The server *is* the cabinet; the LAN
only spreads the joysticks and screens around the room:

- **One authoritative world, one tick** — the server simulates THE game; every player
  acts in the *same frame of the same world*. No client-side divergence: clients send
  inputs and render authoritative state — extra joysticks + extra screens on one
  machine (linked-cabinet style).
- **Same-frame fairness for free at LAN latency.** Sub-millisecond RTT means
  input → server tick → broadcast → render fits the arcade responsiveness budget
  *without* prediction, rollback, reconciliation, or lag compensation — the
  single-machine illusion holds because the network is effectively a long controller
  cable. The thin-client model is not a limitation here; it **is** the goal.
- **Minimal latency is a goal in its own right** (the user's call, 2026-06-10) —
  short-term accuracy is appreciated, **latency wins when they conflict**. Concretely:
  - The **latency budget is dominated by self-inflicted waits, not the wire**: tick
    wait (an input arriving just after a tick waits a full interval — up to 33 ms at
    30 Hz, dwarfing ~1 ms LAN RTT), broadcast batching, client frame pacing. The
    phase-00 stamp chain is this goal's instrument — every wait phase measured, then
    minimized (send immediately after the tick; apply inputs at drain time; late-latch
    the freshest state into the render; `TCP_NODELAY` on the WS path).
  - **The interpolation trade flips**: smoothing renders the *past* (@PLN51's Hermite
    work buys accuracy at 1–2 ticks of display lag). Under this goal the client
    renders the **newest state** (or forward-extrapolates) by default; interpolation
    delay becomes a tunable that defaults toward zero, not toward smooth.
  - The quick UDP channel (05a) serves this goal directly: no retransmit stalls, no
    in-order head-of-line — a stale-but-late pose is *discarded*, never waited for.
  - **The keystone: the advance-collision-forecast protocol** (already prototyped —
    @PLN51's `forecast_test.loft`: the server detects imminent bounces *ahead of
    time*; the measured open question is how often an input change inside the
    lookahead window invalidates the forecast). Forecasting converts latency into
    **lookahead**: a forecast event (`bounce at t+Δ at X`) is broadcast *before its
    effect time*, so clients schedule it and render it **at the right instant** —
    effectively zero perceived latency exactly where latency is *felt* most in an
    arcade game (collisions, hits). Continuous motion is covered by
    newest-state/extrapolation (above); **discontinuities are covered by
    forecast-ahead events**, with a correction sent on invalidation (the Q2
    invalidation rate is what bounds how often a correction flickers). This is
    loft-side *meaning* over the kernel — it needs the event class + timestamps, not
    the parked snapshot ring — so it rides phases 01/05a, not 06.
  - **Cheating for feel is sanctioned** (the user's call, 2026-06-10): the game may
    deliberately bend truth where it makes play *feel* better — and the same-machine
    model makes this **fair by construction** (one authority → everyone sees the same
    bend; no per-client divergence, unlike internet-netcode trickery). The deepest
    form rides the forecast keystone: **promise-keeping physics** — once a forecast
    is broadcast, the server *prefers making it true* (nudging the sim within a
    tolerance) over issuing a correction; truth follows presentation. Corrections are
    reserved for genuine invalidation (a real input change), so correction flicker —
    the worst feel-killer — approaches zero. The same license covers the classic
    arcade feel-cheats at meaning level: input backdating (eat latency in the sim),
    favor-the-player collision tolerances, input buffering / coyote time. All loft
    *meaning*; the kernel never knows the game is lying.
- **Two participation tiers — and the phone is the on-ramp, not the end goal.** The
  **full arcade seat** is a *native client*: a real screen, real input (gamepad /
  keyboard / cabinet stick), full rendering — that is what the experience is designed
  toward, and what the native(/eventually UDP) path serves. The **phone-browser
  client** is the *walk-up tier*: QR-scan, zero install, in the game in seconds (the
  audience demo's proven flow), drop-in/drop-out, no lobby ceremony — how a bystander
  joins the fun instantly, within what a phone's input/display affords. Easy
  participation by design; never the bar the game is built against.
- **The shared spectacle** — the projector/big screen is the cabinet's screen
  (exactly the audience-demo + bumper-planes shape); personal screens are controls
  and auxiliary views.

This is also why the parked phases stay parked: prediction/rollback machinery
(06-snapshot-ring) exists to *fake* the single-machine illusion **over the
internet** — the LAN/same-machine model deliberately avoids needing it. The dogfood
consumers (@PLN6, @PLN51) are already this model; phases 00–04 give it its kernel.
The coverage evaluation ([ENGINE_HOST § Coverage](ENGINE_HOST.md#coverage--is-this-rich-enough-for-most-multiplayer-games-evaluated-2026-06-10))
guarantees nothing must be *undone* if a consumer ever outgrows the LAN; the parked
rows activate on a **consumer's measured need** (the dogfood loop), never
speculatively.

## Design

The full design is [ENGINE_HOST.md](ENGINE_HOST.md) (grown from the @PLN16
exploration; graduated to this plan). Its load-bearing sections:

- **The host-boundary principle** (the keystone): kernel = mechanics, loft = meaning;
  wire-schema-as-data; the residual that still recompiles routes to C71's library tier.
  Existence proof: @PLN51 built all its richness in loft over `poll_event()` with
  zero Rust changes.
- **Part 1 — tiered execution** (canonical model in LAVITION): tier 0 interpret /
  tier 1 wasm swap / native baseline; why wasm is the swap artifact (unload safety,
  sandbox-not-segfault, wire-shippable, the store is already the shared ABI); the
  risk register (bridge tax, rustc latency only acceptable because tier 0 exists,
  bulk loops stay native, **the dispatch table is the real build**).
- **Part 2 — the main-loop IO contract**: three traffic classes with three drain
  rules; head-of-line blocking as the failure mode; store-resident accumulation
  (claim on OPEN, write at offset, publish a `DbRef`); backpressure via the budget;
  three wire shapes (two-channel / chunked / **decompose-into-idempotent-events** —
  the @PLN6-proven preference); the dependent-event ordering decision to settle
  before the protocol freezes.
- **One kernel, two roles**: server and client are configurations of the same crate;
  loopback testing + single-player for free; the IDE's `--serve` host converges on
  the listener role; the browser stays a separate thinner host (frame-yield contract,
  not the kernel binary).
- **Prior art**: @PLN51 (`probe_server.loft` runs the whole loop; interpolation +
  forecast findings; the probe discipline) and @PLN6 (the event-world shape;
  snapshot-as-replayed-deltas).

## See also

- [@PLN16 IDE.md slice 6](../16-debugger/IDE.md) — the IDE surface that consumes
  phases 02/03 (6b/6c); slice 6a's `launchGame` child process is the interim
- [@PLN51 bumper-airplanes](../51-bumper-airplanes/README.md) — phase-04
  consumer; its `00a` probe is phase 00's harness
- [@PLN6 audience demo](../6-audience-generative-art/README.md) +
  `tools/audience-demo/` — phase-01 acceptance consumer
- [`lib_plans/08-server`](../../lib_plans/future/08-server/README.md) — receives the
  pump-mechanics work (phase 00b) as cross-linked commits
- [LAVITION.md](../../LAVITION.md) § Engine runtime architecture — the canonical
  architecture this builds

## Tier 0 and overload sets (@PLN162 step 14, 2026-09-14)

`LOFT_LIVE_RELOAD=1` is the language's OPEN profile: under it every call into an overload set
(`Disp-Key`) is lowered to a per-(name, spelling) synthesised function — the `__dyn_`
dispatcher a dynamic site already had, a `__sel_` stub at a static site — so that a `fn`
block ADDED to a watched file mid-run can join the set: the reload host parses it under its
own name, rebuilds every specialisation of the name in the new world and swaps each in behind
its old def through the same patch a body edit takes.  Refused, naming the cure: an add that
leaves a served tuple ambiguous or undecided, a removed or re-signatured overload, and a
second definition of a name that was one function.  The watcher keys blocks by their
declaration head, so a body edit reaches the overload it belongs to.  `LOFT_RELOAD_DEBUG=1`
lists every def a reload generated, with its parameters, and every swap.  The shadow session
parses the program exactly as the running program was parsed — `Parser::between_passes` and
`after_pass2` are one home each — because the parity check at install compares names only,
and a definition with one more hidden parameter in the running program than in the shadow is
a body that reads its buffer off the wrong slot.  `tests/live_world.rs`;
`doc/claude/plans/162-multiple-dispatch/IMPL.md` § Step 14.
