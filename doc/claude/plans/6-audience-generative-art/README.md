<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN6 — Audience-driven generative-art demo (development plan)

## Status — DONE / CLOSED 2026-07-08 (delivered; residual phases WON'T-DO)

**The demo shipped and did its job — presented live multiple times.** Closed by decision: the
audience-generative-art effort now moves to a new demo of this kind (**"bumper plane"** — its own
plan when it starts), so the residual phases below will NOT be picked up here.

- **Shipped:** phase 0 (browser client — hex grid, palette, tap-paint, WebSocket, pan zones,
  jump-to-active, onboarding centring), **phase 1 (server) — fully done** (world state + decay,
  multi-client, the connect-latency fix via the `lib/server` event pump, crash-resistance,
  `WsGroup`, load-tested, ran live ~4 h zero crashes), phase 3 (native OpenGL projector — flat-2D
  MVP + auto-camera + GPU crystal growth + the `lib/gridmesh` chunk pipeline + per-group VBOs).
- **WON'T-DO (superseded — carried to the next demo, not deferred-with-a-trigger):** phase 6
  (server-authoritative crystal state + the reliable removal-sync fix — phones can show a decayed
  hex until they reconnect; a **known limitation of the shipped demo**), phase 0.5 (swipe-paint),
  phase 3 follow-ons (per-group frustum cull + ground VBOs). Phase 2 was already dropped.

Much of the demo's value graduated into reusable libraries — `lib/server` (event pump),
`lib/web` (`WsGroup`), `lib/gridmesh` + `lib/graphics` (`GroupVboSet`) — which the next demo and
moros inherit.

---

### Historical — the shipped MVP (branch `audience`, PR #214, through 2026-05-20)

Phases 0 + 1 + 3 shipped end-to-end:
phone tap → multi-client loft server → desktop OpenGL projector
view that rotates + tilts + look-at-shifts to the latest edit in
one smooth motion.  Deployed at [`doc/audience-demo/`](../../../audience-demo)
for static-client validation on GitHub Pages.  Local dev loop:

- [`tools/audience-demo/single_port_server.loft`](../../../../tools/audience-demo/single_port_server.loft)
  — combined HTTP + multi-client WebSocket on one port; phone URL
  is simply `http://<lan-ip>:18083/` (page auto-derives WS from
  `location.host`).
- [`tools/audience-demo/projector.loft`](../../../../tools/audience-demo/projector.loft)
  — native OpenGL projector view.  Auto-camera with FPS-style pose
  (tilt = pitch, rotation = yaw, look-at view shift); snapshot-
  request handshake (`6:`) so the projector resyncs after any
  reconnect; renders an empty-cell backdrop so audience can read
  the grid layout.

See [§ Sub-arcs](#sub-arcs) for what's left and
[§ Local dev tooling](#local-dev-tooling) for the iteration
workflow.

Scoped 2026-05-09 to support an upcoming local-meetup talk to game
creators + art enthusiasts.  Sibling presentation plan at
[`../../../presentations/audience-generative-art/`](../../presentations/audience-generative-art)
owns the talk shape, slides, and audience-participation flow.
This plan owns the **development work** the demo needs.

## Goal

Build the development pieces for an audience-participatory
plant/crystal-growth simulation on a hex world: audience members
seed growth from their phones / laptops via a single shared URL;
choose colors that bias growth direction; native projector view
auto-cameras to recent activity; a loft generative script
(live-tweakable) drives the simulation.

The work upstream-feeds three pre-existing plans
(`lib_plans/08-server`, `lib_plans/13-scriptable-scenes`,
`plans/23-event-loop`) by sharpening their scope to a concrete
deliverable; the demo's WebSocket primitives are already shipped
in `lib/server` + `lib/web`, so the work is application code on
top of them, not library extension.

## Sub-arcs

| # | File | Builds | Effort | Status |
|---|---|---|---|---|
| 0 | [00-audience-browser-page.md](00-audience-browser-page.md) | Pure HTML/JS smartphone client — 9×13 hex world view (with movement-zone outer rings), 9-color palette, clear + jump-to-active controls, tap/swipe paint, WebSocket | S | **Partial** — MVP shipped (PR #214): hex grid, palette, tap, WebSocket all work; deployed at [`doc/audience-demo/`](../../../audience-demo); coord system locked to pointy-top OFFSET `(x, y)` so 32×32 server chunks tile naturally (y+2 vertical, y+1 jogs half-hex right).  WS endpoint auto-derived from `location.host`.  **0.6 outer-ring pan zones shipped** (2026-05-19, commit `1af9d5ed`) with arrow indicators + parity-safe y-pan step of 2.  **Grid taller** (9×13) to use phone real estate.  **0.8 jump-to-active shipped** (2026-05-19): `msg_id 5` payload `x,y` recorded in `lastActivePos`; button-press centres the view on it (absolute set, not stepped pan).  Flash on receipt already in place; click clears the flash so the user knows the prompt was acknowledged.  **0.9 onboarding view-centring shipped** (2026-05-19): on first WS open, arm an 800 ms settle timer; during the window, record `lastSeenCell` from every paint delta.  At timer-fire OR on first `msg_id 5` (whichever comes first), one-shot-jump the view to `lastActivePos ?? lastSeenCell` (no-op if the world is blank).  Active-signal during onboarding wins because it's the freshest "where is the action right now" hint.  Remaining: 0.5 swipe |
| 1 | [01-server-state.md](01-server-state.md) | Loft server: hold world state, age cells, run automatic decay (older cells removed; filled-neighbour lease extends life so shrinking starts at the edges), broadcast world deltas + active-player signals.  No autonomous growth — placement is pure direct painting from audience taps | M | **Partial — MVP shipped** (PR #214): single-hash world state, multi-client connect/dispatch/broadcast, world-replay-on-connect, active-player signal on color_select.  **Snapshot-request handshake added** (msg_id 6, 2026-05-19 commit `2d1fadfa`) so any client can re-request a full replay at any time — projector + browsers use this on connect + as a watchdog for missed deltas.  **Combined HTTP + multi-client WS on one port** ([`tools/audience-demo/single_port_server.loft`](../../../../tools/audience-demo/single_port_server.loft)) so the phone URL is `http://<ip>:18083/` with no `?ws=` override.  **1.4 tick loop (growth/age half) shipped** (2026-05-19): main loop advances `world.tick` at 10 Hz (catch-up if a single iteration covers multiple windows so a long pause doesn't drop ticks).  Each cell records `birth_tick: u32` at creation so age = `world.tick - birth_tick` (in 100 ms units).  Persistence bumped to v2 format (magic + version + tick + per-cell birth_tick); v1 files load with `birth_tick = 0` so an existing world reads as "always existed."  Heartbeat log every 600 ticks (~1 min).  **1.4 decay step shipped** (2026-05-20): each tick boundary runs `decay_sweep` — for every non-empty cell, `effective_lifetime = base + lease × filled_neighbour_count` (defaults 3000 / 600 / 300 ticks = 5 min + 1 min/nbr + 30 s window, all env-tunable via `DECAY_BASE` / `DECAY_LEASE` / `DECAY_WINDOW`); cells past `effective_lifetime + window` are removed and broadcast as a `4:x,y,0` delta.  Neighbour count probes the cell hash for the 6 odd-row-offset keys (mirrors `audience_crystal::is_hex_nbr`), so erosion starts at cluster edges (fewest neighbours) and works inward — verified end-to-end under interp (14-cell world eroded to 0, persisted + reloaded).  **1.7 active-player heartbeat shipped** (2026-05-20): the per-`color_select` signal (rejected as overwhelming) is replaced by a 1 Hz emit of the most-recently-active painter's cell to all clients.  **Shrink animation dropped** (decision 2026-05-20): the eligible→removed crystal-shrink the design originally specced would have needed per-cell age on the wire — judged not worth it.  A removed cell simply disappears; an alpha fade-out on the `4:x,y,0` delta (shipped 2026-05-20, ~600 ms, no wire change) is the lightweight cosmetic substitute — in the **browser client** (`doc/audience-demo/index.html`, an rAF-driven alpha ramp) AND the **native projector** (`projector.loft`, which unified its cell shader on per-vertex RGBA + `GL_BLEND` and re-emits the erased cell's ground footprint at dwindling alpha).  This removes the shrink animation as a driver for age-on-wire.  `DECAY_WINDOW` now just adds a flat tail to total lifetime.  **1.8 multi-client load test shipped** (2026-05-20): [`tools/audience-demo/load_test.loft`](../../../../tools/audience-demo/load_test.loft) — one driver process opens N real WS connections, paints disjoint cells, drains the broadcast fan-out, and reports per-tier metrics; ramps 3 → 12 → 30 clients by default (`LOFT_AUD_CLIENTS` for a single tier).  **Findings** (3/12/30 clients × 10 paints, fresh world, interp): broadcast correctness is solid — **100% fan-out, 0 send failures, 0 drops at every tier** (every paint reached every client).  **The bottleneck is connection establishment, not throughput**: per-client connect latency degrades super-linearly (0.6 → 2.8 → 7.3 s/client), so all 30 clients took **~220 s** to connect.  **Root cause (corrected after a Tier-A prototype):** it is **not** the loft loop — a loft-level Tier A (incremental replay + batched accept + snapshot deferral, all shipped in `single_port_server.loft` as sound concurrency hygiene) was prototyped and **did not move connect latency** (210 s vs 220 s).  A TCP-layer experiment (2026-05-20) pinned the exact cause: WS streams accepted via the manual HTTP-upgrade path keep a **500 ms read timeout** that `n_ws_upgrade` never resets, so every idle-client poll blocks 500 ms → `connect[k] ≈ k × 505 ms` → O(N²).  `TCP_NODELAY` made no difference, and lowering the timeout / peek-gating broke `multiplayer_v3` (lossy `read_exact` framing).  **The real fix was that `lib/server` already had a safe path** — the event pump (`srv.run`/`n_ws_next_event`, 20 ms streams, `NoData`-aware `ws_read_frame_detailed`), the supported multi-client path validated by TTT v5.  **A′ (shipped 2026-05-20)** taught the pump to also serve plain HTTP (new `AcceptOutcome::Http` + `Server::poll_event` / `respond_*`) and rewrote the audience server onto it (clients keyed by `cid`, fan-out via `srv.broadcast`).  **Result: 30-client connect 220 s → ~10 s (21.6×), 100 % fan-out, page still served, `multiplayer_v2/v3/v5` green.**  See [§ Tier A′](01-server-state.md).  True O(ready) scaling (epoll, idle clients cost zero) is still Tier B.  **1.9 crash-resistance shipped** (2026-05-20): audited the pump path against hostile clients — found + fixed an unbounded WS-frame allocation (`payload_len` claiming ~u64::MAX → process abort; now capped at 16 MiB, regression test `p_oversized_ws_frame_rejected_without_alloc`); malformed text frames + dropped handshakes already survive (probed via [`crash_probe.loft`](../../../../tools/audience-demo/crash_probe.loft)).  **All of sub-arc 1 (server) is now done.**  Wire is `<msg_id>:<payload>` (MVP); spec'd migration to JSON-in / binary-out is filed as phase 1 step 2.  **WsGroup shipped** (2026-05-20): multiplexed non-blocking client receiver in `lib/web` — load test send phase dropped from ~295 s to 19 ms for 30 clients; server validated live for ~4 hours (18:04–22:00 Amsterdam, zero crashes) |
| ~~2~~ | ~~02-generation-script.md~~ | DROPPED — closed by the 2026-05-10 growth-model decision (pure direct painting; no autonomous growth).  Renderer-side ridge / edge classification covers what would have been the "plant / crystal aesthetic" generator | — | Dropped |
| 3 | [03-projector-view.md](03-projector-view.md) | Native loft beamer client: subscribe to server, render full hex world (flat-2D MVP shipped; frost-style 3D crystal mesh next), auto-camera follows activity, presenter hotkey overrides | M | **Partial — flat-2D MVP + auto-camera shipped** (2026-05-20, commits `2d1fadfa` → `a60f168c`): [`tools/audience-demo/projector.loft`](../../../../tools/audience-demo/projector.loft) opens a 1280×800 OpenGL window, renders the hex world flat-2D with the empty-grid backdrop, auto-camera with FPS-style pose (tilt = pitch, rotation = yaw, look-at view shift toward latest edit, snap-on-event tilt kick + ease-in over 0.2 s, orient → 0.1 s pause → translate sequence).  Catchup via the msg_id 6 handshake.  Remaining: 3D crystal mesh (frost ridges + bridges + hard-edge palette triangles per [`03-projector-view.md` § Visual model](03-projector-view.md#visual-model--3d-crystal-mesh)) — the auto-camera was the prerequisite, and is now ready for the mesh layer to drop in.  **Per-frame mesh-cost stress test shipped** (2026-05-21, [`tools/audience-demo/crystal_stress.loft`](../../../../tools/audience-demo/crystal_stress.loft)): ramps 1→100 hexes × 4 fill patterns and times `crystal_segments_aged`.  Found the original furthest-on-axis mains fork-explode on spread clusters (ring/80 = 318 ms/build, 10130 segments → OOM).  **Crystal algorithm then revised** (2026-05-21): mains capped at `MAX_MAIN_HEXES` and drawn old→new to the nearest older cell on each axis (gap > 1 → separate crystals that merge when a paint narrows the gap).  This bounds forks by construction — **ring/80: 318 ms → 9.9 ms (32×), 10130 → 810 segments**; every pattern now scales the same.  Remaining cost is O(N²) per-cell scans → next win is the spatial index (PF2); fork-bound (PF3) is now intrinsic.  Full numbers + PF1–PF4 in [§ Performance](03-projector-view.md#performance--crystal-mesh-rebuild-cost-stress-test-2026-05-21); leak-validated clean on both backends, original + revised.  **GPU crystal growth shipped** (2026-05-21): a freshly-painted crystal blooms in over ~1.5 s entirely in the vertex shader (mesh tags each vertex with its cell centre + birth frame; `CRYSTAL_VERT` scales it out from the centre by age), so growth animates with ZERO per-frame CPU rebuild — PF1 caching stays intact.  See [§ Growth animation](03-projector-view.md#growth-animation--gpu-side-shipped-2026-05-21).  Diagnostics: the `store_memory()` builtin (added this session) pinned the scale bug — a large mesh fragments into ~100 k uncoalesced free blocks (`vector +=` reallocates per append + no free-block coalescing → L1/L2 follow-ups).  **Gridmesh chunk-pipeline migration G1 + C1 shipped (2026-05-21):** the crystal mesh-gen is now a consumer of the reusable [`lib/gridmesh`](https://github.com/loft-lang/loft-libs-graphics/tree/main/gridmesh) chunk toolkit (lib-plan 19) — this realizes the [§ Performance](03-projector-view.md#performance--crystal-mesh-rebuild-cost-stress-test-2026-05-21) I3 "extract a reusable grid→mesh primitive" and M1 "narrow element types".  **G1**: `ChunkField` carries wired-in per-chunk cell buckets so dirty-chunk extraction is O(dirty), not an O(N) re-partition.  **C1**: `crystal_segments_aged` builds the mesh chunk-by-chunk as a narrow **`SegMesh`** (u8 kind/colour, single coords — M1's ~60 % memory cut, GPU-VBO-native) instead of the fat i64/f64 `CrystalMesh`; `chunk_shift`/`halo_k` are tunable parameters (consumer policy, not engine constants).  Validated: `tests/scripts/130-gridmesh-crystal-equiv.loft` (new SegMesh build SET-equals the legacy `CrystalMesh` build, cross-mode interp + native) + first dedicated `lib/gridmesh/tests/segmesh.loft` unit tests (empty / emit narrowing / 9-column parallel-array alignment / u8 boundary).  Remaining gridmesh arc: G2 render-group (tile) layer, C3 two-level incremental reuse (= I1 dirty-region rebuild), tuning/flat-cost bench, C4 projector per-group VBOs.  Side-finding **@P310** (native graphics FFI `*const i32` vs `vec<i64>`) filed; FIXED 2026-05-22.  **Update 2026-05-21 (perf pass):** **G2 shipped** (render-group/tile layer, tunable `group_dim`; `collect_dirty_groups` O(dirty groups × group_dim²); guard `lib/gridmesh/tests/rendergroup.loft`).  **Bench characterisation** (`crystal_stress`): full build is **O(N) flat — 8 µs/seg at both n=20 and n=100** — and **SegMesh is ~2.1× smaller** than the legacy CrystalMesh (50 hexes 38 KB vs 81 KB; 100 hexes 81 KB vs 171 KB).  **C3 (incremental rebuild) RE-ENABLED 2026-05-22** — @P311 (nested-keyed store crash), @P314 (narrow-vector concat divergence), and @P315 (`vector<single>` 8-byte-into-4-byte width) are all FIXED; `CrystalIncr` two-level reuse is back on, incremental == full build on the interpreter (regression `tests/scripts/133`).  Residual **@P317**: symptom 2 (native incremental store-leak panic) FIXED 2026-05-22; symptom 1 (`crystal_incr_new(non-empty snap)` all-at-once non-determinism) FIXED 2026-05-22 — `crystal_incr_new` now builds its field IN PLACE (no deep-copy of a populated `ChunkField`); deterministic == golden on both backends (regression `tests/scripts/133::test_c3_all_at_once_side24`).  The underlying CORE deep-copy bug (a populated hash field corrupted on copy when `claim` over-sized the dest bucket record) is FIXED 2026-05-22 (@P318).  The last native-only gate **@P316** (`vector<u8> ?? <int>` miscompile) is FIXED 2026-05-22 — narrow-element `?? <int>` widens both branches to `i64`, so the native C3 path is unblocked end to end (`tests/scripts/135` now runs on both backends).  **C4 (per-group VBOs) DONE 2026-05-23**: the projector renders the crystal as one VBO per render group (`GroupVboSet` in lib/graphics — a reusable primitive moros will share) and re-uploads ONLY the dirty groups per edit; a persistent `CrystalIncr` mirrors every paint/erase/recolor (new `crystal_incr_erase`/`_recolor` + gridmesh `field_remove_cell`), so per-edit cost is O(dirty) not O(N) — the scaling fix for big (10+ painter) sessions.  Side-bugs filed: @P319 (moros closure-capture interp crash), @P320 (keyed-remove `&` false positive).  Also landed: a modular **library-test CI** (`tests/wrap.rs::library_suite`) — the libraries had no CI of their own before.  Visual smoke (the demo render) is the human verification step (GL can't run headless).  Remaining: per-group frustum cull (bounds stored) + per-group ground VBOs — both deferred follow-ons |
| 4 | [04-hosting.md](04-hosting.md) | Public URL reachable from venue WiFi (VPS / hotspot / ngrok / cloudflared).  Operational, not code | XS | Open |
| 5 | `05-rehearsal-and-backup.md` (not yet written) | One full dry run on demo hardware; record both demos as fallback for catastrophic failure | XS | Open |
| 6 | [06-server-authoritative-state.md](06-server-authoritative-state.md) | Move the crystal CENTRE/LINE role decision to the server (single source of truth): decide at paint time + persist (v3) + broadcast, so roles can't flip, two-adjacent-hexes can't become two crystals, and **multiple OpenGL clients render the same crystal** from a shared seed (share inputs, not pixels).  Includes: only full crystals grow + line→centre promotion; optional shared-clock for identical multi-client animation; and the **reliable removal sync** fix (phones keep showing decayed hexes until they reconnect — the `4:x,y,0` removal must reach + apply on the browser) | M | **Open (design 2026-05-22)** — prompted by live-demo feedback; two-phase old→new growth + per-segment bloom anchor already shipped in sub-arc 3 |

Phase 2 was dropped (see above).  Phases 0 and 1 land in parallel —
they share only the seed-event schema (sketched on paper first; the
MVP wire format is the temporary `<msg_id>:<payload>` shape).  Phase 3
depends on phase 1 (server-driven state).  Phase 4 unblocks public
testing.  Phase 5 depends on everything else working.

The seed-event schema is the only cross-phase contract.  Lock it
in before anyone codes phase 0 or phase 1.  Suggested first cut:

```json
{ "type": "seed", "x": <hex_q>, "y": <hex_r>, "color": "<hex>" }
```

The world-update broadcast (server → projector + audience clients)
schema needs locking too — minimum: a per-tick delta of
`{ filled_hexes: [(x, y, color), ...] }` so the projector renders
the growth without recomputing it.  Phase 1 + 2 + 3 all consume
this; phase 1 produces it.

## World shape — 3D crystal mesh

The world the projector renders is **not flat** — and it is
**not a closed surface** either.  The mesh is built from filled
hexes only; empty hexes are background lattice with no geometry.
Each filled crystal's height grows monotonically with its filled-
neighbor count (lone filled hex = lowest crystal point, fully-
surrounded = highest plateau).  Two filled hexes one cell apart
form a small **bridge** by each extending laterally toward the
other and meeting over the empty cell between them.  A line of
filled hexes forms a continuous **ridge**.

Reference aesthetic: **ice forming on a cold window pane** —
dendritic / fern-like silhouettes with a central spine, feathery
secondary branches, and needle tips.  Mostly negative space.
The camera can see *through* a bridge to crystals behind, and
through the gaps of any single crystal to what is past it.

Crystal **tops are never flat**.  The mesh-generation algorithm
produces ridge lines at even height (the frost spines) flanked
by triangles that slope into shallow crevices (between
branches).  Across many adjacent filled hexes, ridge lines
preferentially connect into longer fern-like features.

Each crystal's mesh is built from triangles, **each triangle one
solid palette colour with hard edges to its neighbours** — no
gradients, no shader-side blending.  Most triangles take the
cell's own colour; some take a 1-away neighbour's colour; a few
take a 2-away tile's.  At projection distance the eye fuses the
mosaic into a perceived colour-bleed across cluster boundaries,
but the underlying geometry is always solid-colour triangles
with hard edges.  The cell's own colour reads unambiguously at a
glance.

When a new hex becomes filled, the crystal **grows over ~5
seconds** in the direction extruded from the cluster of older
nearby hexes toward the new one.  Direction is derived
client-side from comparing this cell's age with its neighbours'
ages, so no per-cell direction metadata travels on the wire.

### Three views, three roles

The same world is rendered three different ways depending on
who is looking:

| View | Renderer | Camera | Hex grid visible | 3D crystals visible |
|---|---|---|---|---|
| **Projector** (the spectacle) | WASM 3D | Auto, follows activity | NO — pure dark stage | YES |
| **Desktop client** | WASM 3D (same renderer) | User-controlled (mouse-drag + WASD pan, scroll zoom) | YES — base plane under the crystals | YES |
| **Phone client** | Plain HTML/JS 2D | Pan via outer-ring touch | YES — the entire view IS the grid | NO — pure orthogonal flat hexes |

The phone deliberately does not need the WASM renderer at all,
keeping it responsive on modest hardware.  Desktop and projector
share the renderer; the desktop adds base-plane render +
click-to-paint, the projector adds auto-camera + hides the base
lattice.

### Tempo philosophy — sluggish by design

The whole demo is **deliberately sluggish**: 10 Hz server tick
(not 20+); ~5 second crystal growth animation per placed cell;
~2 second aesthetic-swing window; gentle decay over tens of
seconds; placed cells don't snap on instantly, removed cells
don't disappear instantly.  This is **a feature, not a bug**.

Many audience members tapping at once would otherwise overwhelm
the spectacle — bursts of input would produce visual chaos no
single person could parse.  The slow tempo absorbs input rate
spikes into a flowing visible river of growth + decay.  Each
audience member sees their input land but with enough delay and
animation that the result reads as part of the collective
rather than as a private input → output reaction.

Concrete consequences this principle locks in:

- Server tick rate stays at 10 Hz (not faster).
- Growth animation stays at ~5 seconds (not snappier).
- Aesthetic-swing window stays at ~2 seconds (not 500 ms).
- Direct-paint actions don't get optimistic local echo —
  the lag IS the desired pacing.

If a tuning question later asks "should X be faster / snappier?"
the default answer is **no, lean into sluggish** unless
rehearsal proves the lag has crossed from atmospheric into
disconnected.

### Self-cleaning lifecycle

Together with the decay rule, the system **self-cleans its
data structures over time**: cells expire, chunks empty out and
get garbage-collected, in-flight growth/decay animations finish
themselves.  No manual cleanup is needed during the talk and no
persistence layer is needed after.  The server can run
indefinitely without operator intervention; a crash-recovery
restart from blank produces the same end-state the decay rule
would have produced anyway given enough time.

### World data layout

The world reuses the **`lib/moros_map` chunk pattern** — sparse
32×32 chunks at integer chunk coordinates, with the same
`chunk_idx_32` / `hex_idx_32` addressing helpers (which handle
negative-coordinate floor-division correctly).  A chunk is
created on first non-empty write at coordinates inside it; a
chunk is **removed when all 1024 of its cells are empty**.

Per-cell payload is **4 bytes**: 1 byte colour (0 = empty, 1-9
= palette), 1 byte height (0..255, derived from filled-neighbour
count), 2 bytes age (0..65535 ticks).  No per-cell `(q, r)`,
direction, or planting-author metadata — all derivable from the
chunk address + cell index + neighbour ages.

Detail in [`01-server-state.md` § State model](01-server-state.md#state-model).

Full rendering rules + open mesh-shape questions in
[`03-projector-view.md` § Visual model](03-projector-view.md#visual-model--3d-crystal-mesh).
The audience client (phone) stays 2D top-down for input
clarity; the projector is the spectacle that shows the world's
true 3D shape.

## Generation algorithm — plant / crystal growth

The seed list is a set of `{ position, color, planted_at_tick }`
records.  Each simulation tick:

1. For every empty hex adjacent to a "live" cell, decide whether
   it gets filled this tick (probability based on neighbor count
   + growth-rate parameter).
2. If filled, pick its color from the weighted majority of live
   neighbors, **biased toward the dominant color in the direction**
   of the new cell relative to the seed clusters.  Audience color
   choices form a vector field that pulls growth-color decisions.
3. Optionally: cells "die" / "freeze" after age N — relevant for
   plant aesthetic (older branches lignify) more than crystal
   (freeze on contact).

Plant variant: directional bias creates branching toward
concentrated color votes.  Crystal variant: directional bias
creates faceted boundaries between competing color territories.

Phase 2 produces 2-3 variants (plant / crystal / hybrid) so the
presenter can switch between them between rounds.

## Auto-camera (phase 3 detail)

The projector view doesn't pan manually — it follows activity.
Each recent change (filled hex from a tap or from the growth
simulation) contributes a brief heat-trail at its position.  The
camera derives:

- **Target position** — centroid of recent activity (last few
  seconds, exponentially weighted)
- **Zoom level** — inverse of activity spread.  All activity in
  one small region → zoom in; spread across the map → zoom out

Smooth motion: lerp toward target each frame; never snap.

Tuning knobs (defaults + adjust in rehearsal): zoom min/max,
smoothing constant, idle behaviour (slow pan vs hold), presenter
override hotkey to lock the camera.

## Open design questions

1. **Plant vs crystal aesthetic** — RESOLVED: they are not
   separate variants.  Both emerge from a single renderer based
   on the local shape of the filled region.  Thin lines (dotted,
   1-wide, 2-wide) read as **plant** with ridges following the
   line tangent and curving smoothly through bends (anticipated
   2 cells before, trailed 2 cells after).  Wider blobs read as
   **crystal** with the default radial pattern.  A single drawn
   line **swings between the two** as audience input continues.
   Detail in [`03-projector-view.md` § Plant vs crystal — emergent
   from filled-region shape](03-projector-view.md#plant-vs-crystal--emergent-from-filled-region-shape).
2. **Color palette size** — RESOLVED at 9: indices 1-3 RGB
   primaries, 4-6 CMY additive mixes, 7-9 white / grey / brown.
   Index 0 is reserved for "empty hex" in the world state and is
   never sent in a `seed` event.  Detail in
   [`00-audience-browser-page.md` § Zone 2](00-audience-browser-page.md#zone-2--color-palette-middle).
3. ~~**Direction-bias mechanic**~~ — MOOT (autonomous growth
   removed).  With pure direct painting, no generation step
   needs colour-direction bias.  Closed by the 2026-05-10
   "growth model" decision.
4. ~~**Round structure**~~ — RESOLVED 2026-05-10: no rounds.
   Demo runs continuously from start of talk to end; the
   automatic decay rule keeps the canvas alive without manual
   resets.  Presenter narrates and shows code at chosen
   moments without segmenting the painting timeline.
5. ~~**Audience platform**~~ — RESOLVED 2026-05-10: both
   phone (portrait touch) and desktop (laptop pointer + larger
   world view) are first-class targets.  Phone is the primary
   layout; desktop is a separate UI tuned for mouse + keyboard.
   Detail in [`00-audience-browser-page.md` § Desktop variant](00-audience-browser-page.md#desktop-variant).
6. ~~**Presenter as a special role**~~ — RESOLVED 2026-05-10:
   no.  Presenter uses the regular phone or desktop client like
   everyone else.  Operational actions (server restart, clear
   canvas if needed) happen out-of-band via SSH or the local
   shell on the server box; nothing is exposed in the audience
   UI.  Avoids a third client surface to build + test.

## Check-in points (regular validation moments)

This plan is built for interactive work — each check-in is a
deliberate pause where the previous phase gets validated and the
next phase's open questions get answered.  Don't skip them; the
open questions exist precisely so they can be answered with
information from the prior phase, not pre-committed.

| Check-in | After phase | What's validated | What's decided |
|---|---|---|---|
| **CI-0** | Before any code | (none — agreement that the schema sketch in Sub-arcs § works) | Lock the seed-event + world-update schemas as the cross-phase contract |
| **CI-1** | Phase 0 (browser page) | Tap → seed event reaches a stub WebSocket on localhost; phone-touch ergonomics tested on 2-3 actual phones | Q5 (audience platform — phone vs laptop), Q2 (palette size after seeing real-thumb-pick) |
| **CI-2** | Phase 1 (server state) | Stub client → server holds state → broadcasts back.  Multi-client test (2-3 connections) | Q4 (round structure: continuous vs timed), since round structure shapes server reset logic |
| **CI-3** | Phase 2 prototype (generation) | Simulated audience taps + generation algorithm produces visuals.  Both plant + crystal variants prototyped before this gate | Q1 (plant vs crystal — pick after SEEING both), Q3 (direction-bias: local vs global, picked from observed visual character) |
| **CI-4** | Phase 3 (projector view + auto-camera) | End-to-end on demo machine: audience client → server → projector renders.  Auto-camera tuning observed in motion | Q6 (presenter special role: do the controls actually feel needed in practice?) |
| **CI-5** | Phase 4 (hosting) | External machine reaches the server via the public URL; latency under acceptable threshold (~200ms round-trip) | (none — pure validation) |
| **CI-6** | Phase 5 (talk content draft) | Slide deck + presenter script reviewed for narrative arc; presentation-side open questions resolved (see [`presentations/audience-generative-art/`](../../presentations/audience-generative-art)) | Decisions in sibling presentation plan |
| **CI-7** | Phase 6 (rehearsal) | Full dry run on demo hardware; backup recording tested as fallback path | **Ship or postpone** — explicit go/no-go with the user |

## Decision log

Decisions get recorded here as each check-in resolves them.  The
log carries the resolution + a one-line *why* so the rationale
survives the talk.

| Question | Resolved at | Decision | Why |
|---|---|---|---|
| Q1 plant vs crystal | 2026-05-10 (design review) | Single renderer, both aesthetics emerge from local-shape edge detection | Thin-line cells follow line tangent with curve lookahead 2 cells before/after a bend (plant); wider blobs fall back to default radial pattern (crystal); the swing between aesthetics as audience input changes is part of the spectacle |
| Q2 palette size | 2026-05-10 (design review) | 9 colours, indices 1-9; index 0 = empty | RGB primaries (1-3) + CMY mixes (4-6) + white/grey/brown (7-9) covers spectrum + neutrals; 9 is comfortable for thumb-pick on phone; 0 reserved for world-state "empty hex" so colour and emptiness share one field |
| Q3 direction-bias mechanic | 2026-05-10 (design review) | MOOT — no autonomous growth | Closed by the growth-model decision: pure direct painting needs no per-direction colour bias |
| Q4 round structure | 2026-05-10 (design review) | No rounds — continuous demo from start to end | Automatic decay keeps the canvas alive without manual resets, removing the original reason for round boundaries.  Presenter narrates + reveals code at chosen moments without segmenting the painting timeline.  Simplifies server (no reset RPC), client (no "round X" UI), and presenter flow |
| Q-growth model | 2026-05-10 (design review) | Pure direct painting + automatic age-based decay (no autonomous growth) | Cells appear only from audience taps + swipes.  Server runs no generation simulation.  Decay is an automatic per-tick step: older cells expire and are removed; filled-neighbour count extends the effective lease so decay starts at the edges of clusters and works inward (inverse-growth aesthetic).  Removes Q3 (direction-bias) from scope and reshapes Q4 (round structure) |
| Q-decay tuning | 2026-05-10 (design review); shrink-animation dropped 2026-05-20 | base_lifetime = 5 minutes (3000 ticks at 10 Hz); lease_per_neighbour ≈ 1 minute; decay_window ≈ 30 seconds | Sluggish-by-design: no cell decays before 5 minutes.  An isolated edge cell expires at 5 min + 30 s; a fully-surrounded cell survives ~11 min + 30 s.  ~~once eligible, removal animates over ~30 s rather than snapping~~ — shrink animation dropped 2026-05-20; the window is now just a flat lifetime tail and a removed cell simply disappears (client-side fade, shipped in `index.html`).  Shipped 2026-05-20; values env-tunable via `DECAY_BASE`/`DECAY_LEASE`/`DECAY_WINDOW` |
| Q5 audience platform | 2026-05-10 (design review) | Both phone (touch) and desktop (pointer + keyboard) are first-class | Phone layout is the primary; desktop is a separate UI optimised for larger world view + mouse + keyboard.  Same WebSocket protocol, same world model — just two layouts.  Doubles input testing surface but lets non-phone audience members participate fully and gives the presenter a usable laptop fallback |
| Q6 presenter special role | 2026-05-10 (design review) | No special role — presenter uses regular phone or desktop client | Operational actions (restart, clear canvas) handled out-of-band via SSH / local shell on the server box.  Avoids a third client surface to build + test; presenter blends in with the audience |
| Q-three-view-roles | 2026-05-10 (design review) | Three distinct view roles — projector (3D, no base grid, auto-camera), desktop (3D + base grid, user camera, paints), phone (2D pure orthogonal grid, no 3D, paints) | Projector is the pure spectacle (no input, no lattice); desktop is the projector renderer + base-plane input; phone is the smallest possible flat paint surface.  Phone does not need the WASM renderer at all |
| Q-growth-direction | 2026-05-22 (live demo) | Crystals grow **from the existing crystal toward each new point** (nature-like): a connecting main extends old→new first (phase 1), then the new point fills outward (phase 2).  Shipped in the projector via per-segment GPU bloom anchoring (a main blooms from its src = the existing crystal's centre; branches' birth delayed by `GROWTH_FRAMES`) | The opposite (growth blooming out of the newly-clicked hex) read as backwards; this matches how frost/dendrite crystals actually accrete |
| Q-authoritative-roles | 2026-05-22 (live demo) | The CENTRE-vs-LINE role (which hexes are full crystals that grow) is **server-authoritative**: decided once at paint time, persisted, broadcast.  Clients are deterministic renderers of the shared seed (cells + role + birth) — **share inputs, not pixels** — so multiple OpenGL clients show the same crystal | Per-client role derivation flipped frame-to-frame, made two adjacent hexes into two crystals, and let two projectors diverge.  Server ownership makes the role stable, persistent, and identical across all clients.  See [06-server-authoritative-state.md](06-server-authoritative-state.md) |

## Local dev tooling

The `tools/audience-demo/` directory holds two servers covering
different iteration loops.  Both are loft programs that link
against `lib/server`; neither requires a redeploy to iterate.

| File | Loop | When to reach for it |
|---|---|---|
| [`tools/audience-demo/dev_server.loft`](../../../../tools/audience-demo/dev_server.loft) | Single-port HTTP + WebSocket, one client at a time, echoes events as deltas | Default for client-side iteration.  Edit `doc/audience-demo/index.html`, refresh the browser, see the change.  No PR, no Pages deploy. |
| [`tools/audience-demo/server.loft`](../../../../tools/audience-demo/server.loft) | WS only, true multi-client, pair with `python3 -m http.server -d doc` on a separate port | Anything needing two or more tabs at once: multi-client world replay, audience-scale broadcast smoke tests, projector-view co-development. |

`dev_server.loft` two design properties worth knowing about when
extending it:

1. **HTML re-read per request** — `read_index()` calls `file(...)`
   inside the request handler so saving the HTML is enough to ship
   the change to the next browser refresh.  Tiny per-request cost;
   acceptable for single-client dev.
2. **All-URL discovery at startup** — reads `/etc/hostname` and
   walks `/proc/net/fib_trie` looking for `/32 host LOCAL` IPv4
   entries (non-loopback), prints every reachable URL.  The client
   derives its WebSocket URL from `location.host`, so opening
   *any* of the printed URLs auto-routes WS back to the same
   instance — phone testing on the LAN needs no client edit.

Full workflow + the multi-client combo's `?ws=...` override are
documented at [`tools/audience-demo/README.md`](../../../../tools/audience-demo/README.md).

## Cross-arc dependencies

**WebSocket primitives the demo needs are already shipped** (post
TTT v5):

- `lib/server/src/server.loft` ships multi-client WebSocket
  (`srv.run(on_event)` + `ws_clients_*`, `ws_event_*`) plus
  `send_binary` + `last_opcode` on the single-client path.
- `lib/web/src/web.loft` ships the WebSocket client (`ws_handler`,
  `ws_connect`, `ws_send`, `ws_recv`) plus `send_binary`,
  `last_opcode`, and the `pack_*` / `byte_at` binary helpers.
- `lib/world/src/world.loft` ships the sparse 32×32 World/Chunk/
  Cell + `tick_and_decay`.

Sub-arcs 1 + 3 are application code on top, not library
extensions.

**Hard prereqs (sibling plan):**

- [@PLN41 — `lib/server` hardening](../41-server-hardening/README.md)
  items (a), (b), (e):
  - (a) `srv.broadcast_binary()` / `srv.send_binary_to()` — the
    projector's world-snapshot + delta broadcasts depend on this.
    Without it, @PLN6 has to inline `n_ws_send_binary` in loft
    (TTT v5 t4 already did this as a workaround; not the right
    long-term shape).
  - (b) Server-side binary recv via `from_utf8_unchecked` — the
    phone client's drag events stay ASCII-safe today, but any
    future client→server binary path (e.g. compact swipe
    encoding) would silently corrupt without this.
  - (e) `srv.run_with_tick(on_event, on_tick, tick_ms)` — the
    1 Hz `active_player_signal` heartbeat needs a tick-driven
    broadcast scaffold.  Without it, the server can only emit
    on event arrival, which breaks the "audience always sees
    fresh activity" property.
- Items (c), (d), (f) of @PLN41 are post-launch hardening (the
  30-client soak, panic isolation, observability metrics).
  Useful for production rehearsal; not blocking phase 1.

**Soft dependencies — would benefit but not block:**

- `lib_plans/65-scriptable-scenes` — hot-reload of the
  generation script between rounds.  Without it, presenter
  restarts the script (acceptable; the talk script can frame
  "now let me change the rules and re-run").
- `plans/28-error-messages` phases 4-7 — nicer errors if something
  goes wrong on stage.  Not blocking; presenter has rehearsed
  fallback.
- `plans/36-developer-experience` DX.1 / DX.3 — talk
  content overlaps with quick-start `examples/` and the "Learn
  loft in 30 minutes" walkthrough.  Can write the talk inline OR
  land both at once.
- `plans/finished/22-mutable-closures` (shipped 2026-05-13) + the [TTT v6 retrofit](../39-tic-tac-toe/README.md#tic-tac-toe-v6--ergonomic-retrofit-using-writable-closures)
  — drops the `Reference<T>.inner` ceremony from the server's
  pump callback so the loft snippets shown on stage during the
  "loft snippet highlights" beats read at their best.  If
  @PLAN22 lands before the talk, @PLN6 server uses writable
  closures; if not, it uses `Reference<T>` exactly like TTT v5.
  Demo functions either way; only on-screen elegance differs.

**Risk dispositioned (2026-05-12)**: @PLAN15 phase 03 confirmed
the closure-DbRef leak feared in LIFETIME.md does NOT manifest
— closure records free at scope exit via standard local-cleanup
(D1) or `Parts::ChildRec` cascade (D3).  Heavy closure usage in
the generation script does not accumulate state across iterations.
See `tests/leak.rs::p15_phase03_closure_text_capture_*_no_leak`
for the 100-iteration tight-loop guards.

**Upstream-feeds (this work sharpens scope for):**

- `lib_plans/future/08-server` — phase 1 patterns (state hold +
  per-tick broadcast) reusable as server primitives
- `lib_plans/65-scriptable-scenes` — phase 2 is a
  proof-of-concept for the script architecture
- `plans/32-event-loop` — phase 1 is a practical
  EVENT_PROTOCOL instance

**Wire-protocol primitives validated by TTT v5:**

[`plans/39-tic-tac-toe`](../39-tic-tac-toe/README.md) § "Tic-tac-toe v5" carries the
binary-frame extension to `lib/server` + `lib/web`, the
session-tagged blob protocol, the N-client routing pattern,
the catch-up recovery handler, and the sluggish-tempo tick-loop
behaviour — each validated with the smallest possible text-mode
test program.  The TTT board uses the same `World` / `Chunk` /
`Cell` data model this plan defines, so primitives proven there
translate to @PLN6 with zero protocol glue.

Build TTT v5 first; this plan's phase 1 (server) and phase 0
(phone client binary decoder) become consumers of proven
infrastructure rather than co-developing it.

**Deliberately does NOT depend on:**

- `plans/33-multiplayer-editor` — audience client is a
  dumb tap-emitter, not the full moros editor
- `lib_plans/64-game-client` — not needed yet

## Potential problem blockers — open P-issues that could bite

Open P-issues in [PROBLEMS.md](../../PROBLEMS.md) that could surface
while building / running the demo.  Each row carries the workaround
and an assessment of demo-time impact.  If the demo hits one of
these unexpectedly, fall back to the workaround first — fixing the
underlying issue mid-build risks unrelated regressions.

| P# | Subsystem | Demo impact | Workaround |
|---|---|---|---|
| **@P281** | parser (pass-1 fn return types) | **High** — any new demo file with a caller defined BEFORE its callee in the same file type-checks against `unknown(0)` and trips "Expect token ;".  Easy to hit when authoring fresh code under pressure. | Define call-graph leaves first; aliases bridge old names. Audit phase-1 server code with this rule before merging. |
| **@P284** | both backends (for-loop float) | **Medium** — projector view's mesh iteration or world cell positions may use `vector<float>`; loop yields 3 values then garbage forever.  Triggers cleanly with `for f in v`; not always obvious. | Replace `for f in v` with `i = 0; while i < v.len() { ... v[i] ... i = i + 1; }`. |
| **@P279** | typer (conditional reassign → unknown(0)) | **Medium** — typical text-building pattern in event handlers: `s = ""; if cond { s = ... }` consumed by something text-typed.  Easy to hit in phase 1 server code. | Add explicit `: text` to the binding consuming the reassigned variable. |
| **@P277** | parser typer (sorted append type collision) | Low — only triggers on local `sorted<T[K]> = []; sorted += [T{...}]`.  Unlikely in demo code unless we add per-event sorted scratch. | Wrap the sorted in a one-field struct (see scan.loft's `DistinctSets`). |
| **@P278** | parser (method chain after self-slice) | Low — only triggers on specific method-call shapes after self-slice rebind.  Unlikely in event-handler code. | Hoist the method chain into a helper fn taking the param by-value. |
| **@P285** | typer (hash[key]==null warning misattribution) | Cosmetic only — typer mis-attributes "Redundant null check" warning to the inner key.  No runtime impact.  Will fire during dev when writing membership tests for the world / session hashes. | Ignore the warning. `if !hash[key]` sidesteps the shape; functionally equivalent. |
| **@P282** | tooling (warnings to stdout under --native) | Low — only affects machine-readable consumers of the demo binary's stdout.  The projector view doesn't pipe stdout to anything machine-readable. | Run with `--no-warnings` for the on-stage projector binary. |
| **@P316** ✅ FIXED 2026-05-22 | native codegen (`vector<u8> ?? <int>`) | ~~**Medium** — the native projector reads u8 cell fields (kind / colour); reading one with `?? <int-default>` mis-compiles (the `??` result stays `u8`, not widened to `i64`).  Bites the projector specifically (it runs native).~~  Fixed: narrow-element `?? <int>` widens both `if` branches + the result to `i64` (`build_null_coalesce_default`).  `tests/scripts/135` (`v[i] ?? 0` on `vector<u8>`) now runs on both backends. | n/a (fixed) — was: avoid `vector<u8>[i] ?? <int>` reads. |
| **@P318** | core DB (hash deep-copy) | **FIXED 2026-05-22** — deep-copying a populated `hash<T[k]>` corrupted its entries when `Store::claim` over-sized the dest bucket record (a hash reads `room` from the size header; `copy_claims_hash_body` laid the dest out for the SOURCE room → `find` probed the wrong slot, missing entries; size-dependent + non-deterministic).  Fixed by re-inserting via `hash::add` (mirrors `copy_claims_index_body`), with a deterministic unit-test regression.  Was already sidestepped in the demo (in-place field build, also faster); now fixed at the core too. | n/a (fixed). |

### Recently fixed but worth knowing about

- **wasm_gl integer-i64 arg widths** (closed 2026-05-18, commit
  `3ecd13f8`) — `lib/graphics` gallery and any browser-side `gl_*`
  consumer was bricked by misaligned argument reads since April.
  Re-affects the demo ONLY if the projector view falls back to the
  browser (which is the documented fallback if the native build
  breaks on demo hardware).  The doc/pkg/loft_bg.wasm bundle is
  fresh as of that commit.
- **WebGL2 shader translation** (closed 2026-05-18, commit
  `4cb19ae5`) — `lib/graphics/src/*.loft` ships shaders as
  `#version 330 core` (desktop GLSL); WebGL2 needs GLSL ES 3.00.
  Bridges in `doc/loft-gl.js` + `doc/loft-gl-wasm.js` translate
  transparently.  Same browser-fallback caveat as above.
- **C3-blocker P-issue family** (closed 2026-05-22, branch `audience2`) —
  @P311 (nested-keyed store crash), @P314 (narrow-vector concat divergence),
  @P315 (`vector<single>` 8-byte-into-4-byte width), and @P310 (native graphics
  FFI storage stride) are all fixed, so `CrystalIncr` two-level reuse (C3) is
  re-enabled.  @P317 is now FULLY fixed too — symptom 1 (`crystal_incr_new`
  all-at-once non-determinism) by building the field in place (no deep-copy of a
  populated `ChunkField`).  The underlying core deep-copy bug is now FIXED too
  (@P318 — `copy_claims_hash_body` re-inserts each entry via `hash::add` instead
  of a slot-for-slot copy that broke when `Store::claim` over-sized the dest
  bucket record).  Diagnosis tooling added the same session: `LOFT_LOG=zero_claim` /
  `copy_check`, leak-report-by-type, and an enriched store-allocator tripwire
  (see [DEBUG.md § Debugging store-ownership bugs](../../DEBUG.md)).

### Risks (operational)

| Risk | Mitigation |
|---|---|
| Conference WiFi blocks WebSocket / firewalls outbound | Host server on presenter's laptop + phone hotspot; have audience join via local AP if needed |
| Generation script crashes during live tweak | Rehearse 3-5 known-good scripts; presenter switches to fallback if a tweak crashes |
| Audience > expected concurrent connections | Load-test up to 2× expected count; lightweight WebSocket library |
| Native projector view crashes mid-talk | Pre-recorded video backup of full demo; presenter narrates over it if needed |
| Audience hesitates to participate ("am I supposed to tap?") | Presenter starts the demo by tapping their own phone; "look, my hex appeared.  Now everyone try." |
| Generation script needs language features that don't exist yet | Lock the script's loft surface area early (phase 2 first cut); only use shipped features |
| Open P-issue surfaces in fresh demo code unexpectedly | Follow workaround table above; defer fix to post-demo unless it blocks a rehearsed beat |

## See also

- [`../../../presentations/audience-generative-art/`](../../presentations/audience-generative-art) —
  sibling presentation plan: talk shape, slides, audience flow
- [`../../../presentations/par/`](../../presentations/par) —
  reference for slide-deck structure
- [`../24-multiplayer-editor/`](../33-multiplayer-editor) —
  adjacent plan (full multi-paint moros editor); this demo is a
  simpler subset
- [`../../../lib_plans/future/08-server/`](../../lib_plans/future/08-server) —
  server library; this plan's phase 1 sharpens its scope
- [`../../../lib_plans/65-scriptable-scenes/`](../../lib_plans/65-scriptable-scenes) —
  scriptable-scenes; this plan's phase 2 is its proof-of-concept
- `lib/moros_editor/` — existing 3D hex editor (potential basis
  for phase 3 projector view)
- `lib/server/` — shipped multi-client WebSocket support
- `lib/web/` — shipped WebSocket client
- `lib/graphics/examples/` — existing creative-coding examples
