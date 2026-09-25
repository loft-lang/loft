<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN51 — Bumper-airplanes — the next audience demo

**Status:** Draft (2026-05-27, captured from chat).  No code yet.
Successor to [@PLN6](../6-audience-generative-art/README.md)
(painting/decay audience demo) — same projector+phones substrate,
new mechanic.

**Tunable parameters live in a sibling document.**  Every value in
the design narrative below is illustrative; canonical values +
playtest ranges + rationale are in [`NUMBERS.md`](NUMBERS.md).
Pattern mirrors dryopea's `docs/NUMBERS.md` + runtime JSON split.

**Current state of the work** — what's been done, what's in flight,
what to pick up next: [`STATUS.md`](STATUS.md).  Start there if
resuming after a context gap.

## Pitch

A static 3D world built from a **dryopea-style hex editor map** with
**extruded highs** (palette types `wall` → pillar, `wall_high` →
cliff, `hill` → ramp, etc.).  Audience members each fly an
**airplane / bumper-car hybrid** through the scene from their phone;
the projector shows the whole world with the camera attentive to
**where most planes currently are**.  Planes **cannot crash** — they
**bounce** off geometry and each other, and a bounce leaves the
plane in a **hard-to-control stall** the pilot has to recover from.

Each plane drags a **smoke-pot trail** so the projector view reads
as multiple coloured ribbons threading through the geometry —
audience can see who's where without needing to read labels.

## The pieces

### World

- **Source:** a saved dryopea editor MapFile (or future stencil-pipeline
  output — @PLN49 plan 06).  The painted palette types map to extrusion
  rules:

  | Palette type | Extrusion |
  |---|---|
  | `sea` / `water` / `sand` | flat (h = 0) — flyover, planes can graze surface |
  | `grass` / `hill` | low ramps (h = 0.5–2 m) |
  | `rock` / `steep_rock` | medium ramps (h = 3–6 m) |
  | `wall` | **pillar** — vertical column, 8–12 m, narrow footprint |
  | `wall_high` | **cliff** — vertical face, 12–20 m, wider footprint |

- **No edit at runtime.**  The map is loaded once at projector startup
  and stays static.  The audience demo is the consumer; the dryopea
  editor is the author tool.
- **One file format.**  MapFile JSON loaded via
  [`hex_world::load_mapfile()`](https://github.com/loft-lang/loft-libs-world/blob/main/hex_world/MAPFILE.md) (the
  cross-project schema documented for moros / dryopea / bumper), or
  the eventual path-backed `Store` (`store_persist_bind` from
  [@PLN43 phase 01c](../43-loft-store-durable/README.md)) — both
  give us the painted hash, and extrusion is a render-time decision
  on top.  Per-palette extrusion strings live on
  `MaterialDef.md_extrude` (`wall`/`wall_high`/`hill`/etc.) per the
  MAPFILE schema.

### Projector (big screen) — wide overview

- The projector is the **spatial map** of the play space.  Camera is
  high and wide enough to show the full extruded world plus most or
  all planes at once — audience members read positions and trails at
  a glance, recognise their own colour, see who's chasing whom.
- **Attention camera:** centroid of all planes currently outside
  their spawn-invincibility window (no plane ever despawns
  permanently, so "active" = "past the spawn grace period"), with
  weight decaying by recency-of-bounce (so the camera follows the
  action, not the strays).  Smooth lookat shifts toward the
  densest cluster.
  Zoom widens to keep the full active group in frame; tightens only
  when planes cluster naturally.  Pose interpolation borrowed from
  @PLN6 phase 3.
- **Render layers, back to front:**
  1. Sky / fog backdrop.
  2. Extruded world geometry (instanced per palette type).
  3. Plane meshes — single shared mesh with a **bright red nose**
     and a player-coloured body.  The red nose is identical across
     all planes; only the body recolours per player.  The visual
     rule "red = don't ram head-on" is intrinsic to the mesh and
     instantly legible at projector distance, no UI explanation
     needed.
  4. Smoke-pot trails — per-plane ribbon trail, ~3 sec of recent
     positions, alpha-faded.  Colour matches plane.  Trails are the
     primary "who's where" cue at projector zoom.
  5. **Score confetti** — on every score event, a brief burst of
     ~30 small particles in the scoring player's plane colour
     erupts at the collision point (target gate / impact centre),
     drifts upward with mild gravity, fades over ~1 sec.  The
     player-colour match means audience members spot "their own"
     confetti at-a-glance, even when the centroid camera is far
     from the action — the colour identity carries.  +5 hits
     produce a bigger burst (~80 particles) than +1 target taps;
     combos (if shipped) scale by combo position.
  6. HUD overlay: per-player score, joined-recently flash,
     countdown timer, GAME OVER + leaderboard splash on round
     boundary.

### Phone client (per player) — first-person cockpit

- Same connection pattern as @PLN6 (HTTP page + WebSocket on one
  port).
- **First-person view from inside the cockpit.**  The phone is the
  player's intimate viewport; the projector is the spatial overview.
  Splitting the two roles cleanly is what makes the demo legible —
  audience pairs their own phone-view with their tagged trail on the
  big screen.
- **Layout** (portrait):

  ```
  ┌─────────────────────────┐
  │ ▓      ┌─────┐        ▓ │   ← score centred at the top (the ONLY
  │ ▓      │ 42  │        ▓ │     overlaid UI element besides the
  │ ▓      └─────┘        ▓ │     two touch strips)
  │ ▓                     ▓ │
  │ ▓  first-person view  ▓ │   ← centre: out the cockpit windscreen.
  │ ▓                     ▓ │     World geometry + other planes +
  │ ▓                     ▓ │     their smoke trails.  No HUD, no
  │ ▓                     ▓ │     altitude, no speed gauge, no
  │ ▓                     ▓ │     dashboard frame.  Your own plane
  │ ▓                     ▓ │     is fully invisible.
  └─────────────────────────┘
  ```

- **Each strip is a vertical touch slider** with absolute thumb
  position `0.0` (bottom) → `1.0` (top).  Both thumbs default to
  `0.5` (centre) — that's "level flight, current speed."  The
  strips overlay the cockpit view as semi-transparent bands; the
  player's thumbs naturally rest on the screen edges where the
  bands sit, leaving the centre clear for the view.

- **No HUD beyond the score.**  This is a load-bearing design
  choice, not an omission to fill in later.  Altitude, airspeed,
  bank angle, heading, where-am-I-on-the-map — none of it is
  numerically displayed.  The player reads:

  - **Altitude + airspeed** from the *feel* of geometry rushing
    past the windscreen (cliffs growing tall, ground texture
    flowing fast or slow).
  - **Bank angle** from the horizon's tilt.
  - **Where-am-I-on-the-map** by *glancing at the projector*.

  Splitting the navigation responsibility this way is the demo's
  social hook: the projector is genuinely necessary to play well,
  not just decorative.  Audience members alternate between the
  intimate phone view and the spatial projector view, and the
  shared screen earns its place in the room.

- **Sight-filtered peer rendering with distance-LOD update rate.**
  The server applies two compounding filters when deciding which
  peer pose-frames to send to a given phone:

  1. **Hard sight cutoff** (`net.peer_sight_range`, default
     80 m).  Beyond this radius the peer's pose is not sent at
     all — out-of-range planes never enter the phone's wire
     stream.
  2. **Three-tier rate-LOD** within the sight range, by
     distance to the recipient's own plane:

     | Distance band | Update rate | Why this rate suffices |
     |---|---|---|
     | 0 to `peer_rate_full_radius` (default 25 m) | Every broadcast tick (~30 Hz) | Knife-fight range; every twitch matters; interpolation can't hide latency |
     | `peer_rate_full_radius` to `peer_rate_half_radius` (default 25–60 m) | Every 2nd tick (~15 Hz) | Visible motion per-frame is half as fast in pixels; client interpolation makes 15 Hz indistinguishable from 30 Hz |
     | `peer_rate_half_radius` to `peer_sight_range` (default 60–80 m) | Every `peer_rate_outer_factor`-th tick (default 4, so ~7.5 Hz) | Near sight-range edge; visible motion per-frame is small; interpolation smooths the gap |

  - **Throughput.**  Compounding the sight cutoff with rate-LOD
    drops per-phone outbound bandwidth substantially.  Worst case
    (all N peers in the inner ring) is the same as before; typical
    case (peers spread across all three bands plus many beyond
    sight) sees roughly **3–4 × less** outbound traffic than the
    uniform-30 Hz design — which itself was already light because
    of the sight filter.
  - **The projector is never filtered.**  Full N planes at full
    30 Hz, unconditionally.  The projector IS the overview by
    design — every plane, all the time.  The asymmetry is
    intentional: the room knows where everyone is, the
    individual pilot knows where their neighbourhood is.

- **Phone-side smooth rendering (v1, intrinsic to the renderer).**
  The bandwidth shaping above is only invisible to the player if
  the phone hides the discrete-frame nature of what it receives.
  Three client-side responsibilities:

  1. **Inter-frame interpolation.**  Each peer's pose updates
     arrive at 30 / 15 / 7.5 Hz depending on its rate-LOD band.
     The phone keeps a small per-peer history
     (`net.peer_interp_buffer_frames`, default 3) and at render
     time **cubic-Hermite-interpolates** the displayed pose
     between the two most recent received frames, using each
     frame's velocity field as the curve-tangent endpoint.  At
     60 fps render with 7.5 Hz input, the phone draws 8
     intermediate frames per real frame — motion looks
     continuous regardless of the underlying rate-LOD band.

     **Why Hermite and not pure linear:** the
     [interpolation test](../../../../tools/audience-demo-50/interp_test.loft)
     shows that **linear interp on a constant turn at 7.5 Hz
     produces 67 mm peak position error, Hermite produces 0**
     (cubic-with-end-velocities fits a circular arc exactly
     between two endpoints with their tangent velocities).
     The cost is a few extra multiplies per peer per render
     frame — negligible.  Default `view.interp_method =
     "hermite"`; "linear" is retained as a fallback for
     low-power phone targets.

  2. **Fade-in on sight-range entry (implicit signal).**  When
     a pose-frame arrives for a plane_id the phone has not seen
     recently, the renderer treats it as a fresh entry: ramps
     the peer's alpha from 0 → 1 over `view.peer_fade_in_secs`
     (default 0.4 sec) and starts building the interp history
     during the ramp.  No explicit ENTER signal is needed — the
     first pose-frame for a previously-unseen plane_id is
     itself the trigger.

  3. **Fade-out on sight-range exit (explicit `EXIT` signal +
     silence backstop).**  The server sends an explicit `EXIT:plane_id`
     event the instant a peer's distance crosses
     `net.peer_sight_range` outward.  On receipt the phone
     starts a fade-out at the peer's last-known pose, ramping
     alpha from 1 → 0 over `view.peer_fade_out_secs` (default
     0.5 sec).  Backstop: if the EXIT message is somehow lost
     (rare on LAN, more relevant on poor connections), the
     phone falls back to silence-timeout detection — if no
     pose-frame OR EXIT for a peer has arrived in
     `view.peer_silence_timeout_secs` (default 0.25 sec; sized
     as a packet-loss backstop, not a primary detector), the
     phone starts the same fade-out independently.  Self-healing
     via the backstop also covers the case where the server
     crashes — all peers go silent and fade out gracefully
     rather than freezing at their last position.

     If a pose-frame for a peer arrives **mid-fade** (server
     re-included them: transient packet-loss recovery, a peer
     re-entering sight range, or a stale state-sync), the fade
     reverses and normal rendering resumes.

  These three behaviours are not QoS — they're how the phone's
  first-person renderer turns whatever pose-frame schedule it
  receives into smooth visual motion.  Same code path runs
  whether the v1 static defaults or the v2 adaptive scaling is
  shaping the stream.

- **Per-phone adaptive QoS (post-v1).**  The static sight + LOD
  defaults above assume every phone has a comparable connection.
  In practice, audience phones at the same venue can span a 10×
  range of WS round-trip time (someone on the venue WiFi vs.
  someone tethering to a slow 4G).  A v2 refinement classifies
  each phone into a quality tier from passive WS measurements
  and **scales its peer-rendering envelope per tier**:

  | Tier | RTT | Loss | sight_range × | full_radius × | half_radius × | interp_buffer |
  |---|---|---|---|---|---|---|
  | **Good** | < 50 ms | < 1 % | 1.5× | 1.0× | 1.0× | 3 |
  | **OK** (default) | 50–150 ms | < 5 % | 1.0× | 1.0× | 1.0× | 3 |
  | **Limited** | > 150 ms or > 5 % loss | (caps activate) | 0.6× | 1.5× (smaller full-rate ring) | 1.5× | 4 (deeper buffer to ride out jitter) |

  **The server is the sole authority for QoS *classification*
  — no client-side measurement code, no extra protocol
  round-trips.**  (The phone still does the v1 smooth-rendering
  work — interpolation + fade in/out — but that's renderer
  work, not QoS work; same code path runs regardless of which
  tier the server has placed the phone in.)  Each input frame
  the phone already sends carries the phone's local `ticks()`
  timestamp (a handful of bytes within the existing input-frame
  budget).
  The server already records its own receive-time per frame.
  From these two streams alone, the server passively computes:

  - **One-way uplink latency** = `(server_recv_t) - (phone_send_t)`
    — clock-skew is removed by the standard "minimum recent
    one-way" estimator (the smallest delta in a window is
    treated as the offset; everything above it is RTT
    contribution).
  - **Loss rate** = gaps in the per-client sequence number as
    the server sees them.
  - **Jitter** = standard deviation of recent one-way latencies.

  No ping/pong protocol.  No client-side measurement code.  No
  extra round-trip messages.  The phone keeps doing exactly
  what it was doing in v1; the server quietly classifies it
  from data already flowing.

  Tier classification updates every few seconds with hysteresis
  (must hold tier-up condition for ~5 sec before upgrading;
  tier-down kicks in on a single bad window to protect the user
  from a stuttering experience).

  Effect at a real audience demo: phones with strong WiFi see
  the largest neighbourhood at the highest fidelity (good play
  is rewarded with rich situational awareness); phones with
  marginal connections still get a playable experience scaled
  to what their link can carry, instead of dropping out
  entirely.  Players don't see their tier explicitly — they
  don't know it exists; it just feels right.  Server-side log
  shows tier transitions for diagnostics; player-side shows
  nothing.

  Captured as a post-v1 sub-arc — see [§ Sub-arcs](#sub-arcs-sketch--phase-the-work-like-plan36)
  phase 7.  Static defaults from NUMBERS.md ship in v1; the
  multipliers and tier thresholds become tunables when phase 7
  starts.

### Wire protocol — message kinds (v1)

The phone ⇄ server WS protocol carries a small set of message
kinds.  Fixed-format (not JSON) to keep per-frame size under
budget.  Indicative shape — final framing decided in phase 0a /
phase 1.

| Direction | Kind | Approx. wire size | When |
|---|---|---|---|
| Phone → server | `INPUT` | ~16 B (cid + L + R + L_on + R_on + ticks) | At `net.input_rate` (30 Hz default) |
| Server → phone | `POSE` | ~32 B per plane (see `net.pose_frame_bytes_budget`) | At per-recipient per-peer rate-LOD (30 / 15 / 7.5 Hz by distance) |
| Server → phone | `FORECAST` (bounce prediction) | ~40 B per plane | **Look-ahead.**  Emitted as soon as the server detects an imminent bounce (plane's ballistic projection intersects geometry within `net.bounce_lookahead_ms`, default 100 ms).  Payload: `(plane_id, t_bounce, pos_at_bounce, vel_after_bounce)` — i.e., the *future* state the plane will be in, time-stamped with the server-clock moment it becomes valid.  Sent to every in-sight recipient regardless of their normal-pose LOD band.  Client receives the forecast ahead of the actual bounce instant; uses it to plan a local visual animation that completes synchronised to `t_bounce` on its own clock.  Bounces happen at the same wall-clock moment on every viewer's phone (and the projector), regardless of network-latency variance |
| Server → phone | `EXIT` | ~6 B (plane_id) | When a peer's distance crosses `peer_sight_range` outward, sent once |
| Server → phone | `EVENT` | ~8 B (kind + position) | Own-plane events (bounce, target hit, plane hit, stall, score) — triggers phone-local audio + score increment |
| Server → projector | `POSE` (all planes) | ~32 B × N | At `net.broadcast_rate` (30 Hz); projector receives unfiltered |
| Server → projector | `EVENT` (all events) | as above | Triggers projector confetti + countdown ticks + leaderboard splash |

No `ENTER` signal — first pose-frame for a previously-unseen
plane_id is itself the trigger for the phone's fade-in.

**Bounces as forecast, not as report.**  Normal flight is
continuous motion well-approximated by linear or Hermite
interpolation between sparse samples (per the
[interpolation test](../../../../tools/audience-demo-50/interp_test.loft)).
But a bounce is a *discrete* event: the velocity vector flips
in one frame; no interpolation between pre-bounce and
post-bounce *samples* recovers the actual path (test shows
0.3–1.0 m peak error across all interp methods when the bounce
instant is missed).

The fix isn't to send another sample — it's to send the
*future*.  When the server detects an imminent bounce (the
plane's ballistic projection will intersect geometry within
`net.bounce_lookahead_ms`, default 100 ms), it broadcasts a
`FORECAST` message immediately, containing:

- `t_bounce`: the server-clock moment the bounce will happen
- `pos_at_bounce`: where the plane will be at that moment
- `vel_after_bounce`: the post-bounce velocity (already
  reflected by the server's bounce math)

Each in-sight recipient receives the forecast *ahead* of
`t_bounce`.  The client uses it on its own schedule: continue
the plane's current trajectory until `t_bounce`, then transition
to `pos_at_bounce` and continue along `vel_after_bounce`.  The
visual bounce happens at exactly `t_bounce` on the client's
wall clock — **synchronised across every phone in the room and
the projector**, regardless of variable network latency to each
device.

**Acted on at the client's leisure.**  Between forecast-receipt
and `t_bounce`, the client animates locally at its render rate
(60 fps).  The forecast is a *plan*, not a snapshot — the
client owns the visual presentation.  Normal POSE updates
continue to flow at LOD rate; they serve as **gentle
correction** for any divergence from the forecast (e.g., if the
player twitched the controls during the lookahead window and
the trajectory changed slightly, the next regular POSE absorbs
the drift).

**Cost analysis.**  Bounce events are rare per-plane (geometry
bounces ~0–2/sec under typical flight; plane-on-plane bounces
much rarer).  At 30 players × ~2 events/sec/plane × ~5 in-sight
peers × 40 B per FORECAST ≈ **~12 KB/sec total** across the
broadcast — negligible compared to the steady-state pose-LOD
traffic.  No distance cap is needed; the event rate is
self-limiting.

**Edge cases.**
- *Prediction wrong (player twitched mid-lookahead).*  Forecast
  is invalidated; client snaps via the next POSE update.  Brief
  visual glitch, very rare in practice.
- *Plane-on-plane bounces.*  Harder to forecast (both planes'
  trajectories matter, both have inputs).  Forecast lookahead
  shrinks to one tick (~33 ms) for these; visual quality is
  slightly less smooth than geometry bounces but still better
  than reactive keyframes.
- *Bounce + bounce (chain).*  Forecast a single bounce; the
  second bounce gets its own forecast at the appropriate time.
  No multi-bounce forecasting in v1.

### Control mapping (the novel bit)

Let `L_on`, `R_on` be whether each thumb is in contact with the
screen, and `L`, `R` the thumb positions in `[0, 1]` while in
contact.  Three control modes depending on contact state:

**Both thumbs in contact — pitch + roll.**
- `pitch_input = (L + R) / 2 - 0.5`  — symmetric: both thumbs up =
  climb, both thumbs down = dive.  Range `[-0.5, +0.5]`.
- `roll_input  = R - L`              — differential: right above
  left = roll-right.  Range `[-1, +1]`.

**Exactly one thumb in contact — slow yaw, no roll.**
- Lifting one thumb off the screen while keeping the other in
  contact engages a flat-turn (yaw) mode: the plane rotates slowly
  around its vertical axis without banking.
- `yaw_input = +1` if only one specific side is in contact, `-1` if
  only the other.  Magnitude is constant — yaw rate doesn't depend
  on the remaining thumb's position (its position is ignored in
  this mode so the player can yaw without inadvertently pitching).
- Pitch + roll inputs zero out while in this mode; the plane holds
  its current pitch and decays roll back toward level.
- Direction-mapping ("left-thumb-lifted yaws which way?") is a
  playtest decision; see open questions.

**Neither thumb in contact — coast / level out.**
- All inputs zero.  Pitch decays toward level, roll decays toward
  level, no yaw.  The plane coasts under whatever momentum it has,
  losing energy slowly.

Mapped to plane physics:

- `pitch_input > 0`: **climb**, but speed-limited — slow climb-rate
  proportional to value.  Bottom of climb is energy-cost (lose
  airspeed → eventually stall).
- `pitch_input < 0`: **dive**, fast.  More dive = more speed
  gained.  Hitting the ground is a bounce (see below).
- `roll_input` sets bank angle.  Bank without pitch = lazy circle.
  Hard bank + dive = combat-style cut.
- `yaw_input` slowly rotates the plane around the vertical axis,
  unbanked.  Useful for fine course corrections through narrow
  gaps where you don't want the altitude loss of a banked turn.
- **Throttle is automatic.**  Speed is a function of pitch + recent
  bounce damping; the player flies by angle alone, not by thrust
  control.  Keeps the controls to two thumbs.

The three-mode mapping covers all six core axes a flying-game
controller usually allocates separate buttons for (pitch +, pitch −,
roll +, roll −, yaw +, yaw −) using just thumb presence + position
on two strips.  Mode transitions happen naturally — a player
intending a yaw simply lifts one thumb; intending a roll simply
opposes the two thumbs; intending a climb lifts both.  No mode
toggle, no dedicated button.

**Mode-transition hysteresis.**  Capacitive touch screens can
drop a contact briefly (~30–50 ms) during a hard manoeuvre or a
sweaty thumb.  Without filtering, that 50 ms one-thumb-only blip
would engage yaw mode and apply a yaw impulse the player didn't
ask for.  The phone client therefore requires a stable
single-thumb-only state for `controls.mode_transition_hysteresis`
(~150 ms, see [NUMBERS.md](NUMBERS.md)) before yaw engages.
Exiting yaw — re-engaging the second thumb — is detected with
the same hysteresis to avoid a touch dropout flipping yaw back on
mid-manoeuvre.

### Sound — own-plane only, no projector audio, no music

The projector emits **no sound** at all.  Each phone plays sounds
**only for events involving its own plane**, as short discrete
samples through its own speaker:

- Engine drone pitched by airspeed (continuous, the only non-event
  audio).
- Bounce *bonk* on geometry contact (pitched lower for cliffs,
  higher for ground).
- Plane-on-plane *clang* when your plane hits or is hit by another.
- Target-hit *chime* when you score on a world target.
- Stall *whirr* during the 3-sec recovery window, fading as
  control returns.
- Score *ding* on the +1 / +5 events (single-shot, pitched up by
  combo position if a multiplier ships).

**No "neighbour-awareness" simulation.**  A phone never plays
sounds for events involving *other* planes — no spatial
attenuation logic, no proximity-based filtering, no
broadcast-and-cull.  Players will physically hear nearby phones'
speakers anyway (a phone 3 m away is audible), so the room
delivers the spatial ambience acoustically rather than each
phone simulating it through software.  This is more authentic
than any spatial-audio implementation: it's the literal sound of
the room.

**Why no music + no projector audio:**

- The room IS the soundtrack.  With 12–30 phones each emitting
  discrete event sounds, plus audience reactions (cheers, groans,
  trash-talk), the ambient noise self-organises: calm during
  opening minute → escalating chaos as "the storm" kicks in (if
  that mechanic ships) → cheering crescendo at GAME OVER.  No
  need for a music bed, and any music bed would force the room
  to talk over it.
- **Each phone is its own arcade cabinet.**  Deep arcade-era
  authenticity: each player has their own audio experience local
  to their hand.  The audience-experience is the SUM of all
  phone-experiences plus the room — exactly how an arcade hall
  sounded, and exactly the auditory model the physical
  distribution of phones delivers without any software work.
- Per-phone audio dodges synchronized-projector-audio engineering
  (no need for low-latency network audio dispatch to a shared
  sink; each phone just plays its own samples on its own clock).

**Tech:** Web Audio API, samples bundled in the HTML page, single
`AudioContext` per phone unlocked on first tap (iOS gesture
requirement).  Server sends own-plane events as targeted WS
messages to the specific client whose plane is involved — no
broadcast, no client-side filtering needed.  ~XS effort, slots
into phase 0 (phone client) rather than a separate sub-arc.

### Bounce physics (no crashing)

- On any geometry contact: reflect velocity along surface normal
  with energy loss (~0.4 retained).
- On plane-to-plane contact: equal-and-opposite reflection plus a
  random angular impulse applied **once at bounce-instant** (not
  a sustained force).  Both planes enter stall mode for
  `stall.duration` (~3 sec) during which control authority is
  dampened (`stall.control_damp`, ~30 %).  The random angular
  impulse decays via natural angular drag (`stall.angular_drag`)
  over the stall window — recovery is therefore guaranteed: the
  player fights *settling* tumble that has a finite energy
  budget, not a sustained random force that could pin them
  unrecoverably.
- Stall mode is the **only** consequence of a bounce.  Planes never
  despawn.  The smoke trail also "puffs" on bounce as a visual cue.

### Scoring

- **Hard-to-reach world targets:** scattered through the world are
  **target bumpers** — spherical objects with classical red/white
  bull's-eye banding, readable as targets at projector distance
  and from any direction in flight.  Diameter ~2 m, fixed
  position, no rotation.

  A plane **bounces off a target as a strong, directional kick**
  back along the impact normal: 0.7 energy retention along the
  outward-radial direction and only 0.2 along the surface
  tangent.  In practice the plane is **redirected back roughly the
  way it came** — head-on hits reverse direction; glancing hits
  kick the plane firmly away rather than skimming.  Every bounce
  on a target scores **+1** to the player who hit it.  No stall
  (stall only triggers on plane-on-plane hits).

  **Spent state — tilted down, pass-through.**  Each target has
  two states:

  - **Primed (rest):** upright, full bull's-eye facing the
    approach.  Collider active, bounces planes, scores +1.
  - **Spent (cooldown):** the target visibly **tips down at an
    angle** — as if knocked askew on a horizontal hinge,
    hanging diagonally.  The bull's-eye is still rendered (the
    audience knows it's a target, just spent), but the **collider
    is off** — planes pass straight through it without bouncing
    and without scoring.

  After a hit:

  - `t = 0`: hit registers, plane bounces, +1 scores.  The
    target snaps to its tipped-down pose in one frame.
  - `t = 0 .. ~15 s`: target holds tipped-down.  Pass-through.
  - `t ~= 15 s`: target rotates back to upright (~0.3 sec ease).
  - `t > 15 s + 0.3 s`: re-primed.  Collider on, ready to score.

  The carnival-target / duck-target metaphor: hit knocks it
  over, it slowly rights itself.  Audience reads "primed vs
  spent" at projector distance from the orientation alone —
  upright = ready, tilted = wait.

  **15 seconds is a deliberately long cooldown.**  At a casual
  flight speed of ~20 m/s through a ~100 m × ~100 m map, a plane
  traverses the full diagonal in ~7 sec — so 15 sec is two full
  cross-map flights.  This is the point: a single target is
  *not* a farmable resource, and players are pushed to **find
  other targets** rather than loiter on one corner.  Map design
  follows from this: 20–40 targets distributed across the map so
  there's always a primed one within reasonable flight time,
  even with 12–30 active players.

  Bounded scoring keeps games close: max per-player target score
  per 5-min round is ≤ 20 × +1 = 20 (if you could perfectly chase
  one target after another).  Plane-on-plane +5 hits stay
  uncapped, so skilled offensive play scales the leaderboard
  above what target-farming alone can.

  **Two anti-spam mechanisms combine:**

  1. **Cooldown + pass-through** — 15 sec deadtime during which
     the same target is non-interactive.  The plane literally
     flies through a spent target, removing both the bounce
     reroute AND the score.  Visible at projector distance from
     the tilt angle alone.
  2. **Placement** — targets are placed in pockets (narrow
     gorges, between pillars, under cliff overhangs) such that
     the kick sends the plane back through the only available
     approach corridor.  With 15 sec cooldown, returning to a
     just-hit target is almost always futile — by the time
     navigation could plausibly route you back, the target is
     either re-primed (fine, score again) or some other player
     has already hit it on their pass-through.

  Skill comes from **reaching** the target and **chaining** to
  the next one: getting there at all is the challenge, and
  chasing multiple targets in quick succession is a flow-control
  problem on top of the navigation.  A skilled pilot routes
  through the map hitting fresh targets in sequence, never
  doubling back to a still-cooling one.
- **Player-on-player hits:** the score depends on *which part of
  the other plane your collision touched*:

  - **Your contact on the other plane's red nose:** **no score**
    for you.  You still bounce, you still stall.
  - **Your contact on the other plane's body (anything not the
    nose):** **+5** for you.  Both planes bounce, both stall.

  The test is per-plane, so a single collision can produce
  asymmetric scores:

  | Scenario | Plane A contact | Plane B contact | A score | B score |
  |---|---|---|---|---|
  | Head-on (nose to nose) | B's nose | A's nose | 0 | 0 |
  | T-bone (A noses B's side) | B's body | A's nose | +5 | 0 |
  | Wing-tip clip (parallel pass) | B's body | A's body | +5 | +5 |
  | Tail-chase nose-tap (A's nose on B's tail) | B's body | A's nose | +5 | 0 |

  The bounce sends both planes apart, so a single collision
  produces at most one score event per plane (no
  bounce-touch-bounce-touch chain within one physics tick).

- **The red nose is the anti-coordination mechanism.**  Two
  players agreeing to fly head-on at each other for trade-points
  produces 0+0 every time — the rule has zero exploits for
  trivial head-on collusion.  Wing-tip-trade coordination (both
  fly parallel and brush wingtips for +5+5) is theoretically
  possible but requires the kind of precise formation flying that
  is itself skilled play, not exploitation — encourage rather
  than punish it.

  Result: skilled offensive play looks like T-bone strikes,
  diving-onto, climbing-past, broadside passes.  The red-nose
  warning is visible to every pilot on every plane on the
  projector, so even non-pilots watching can see who's attacking
  whose flank — the read is immediate.

- **Implementation:** each plane has a 2-sphere collider — a
  small nose-sphere (~0.5 m radius, positioned at plane front)
  and a larger body-sphere (~1.5 m radius, at centre-of-mass).
  Physics resolves the bounce + stall identically regardless of
  which sphere touched; the score event fires `+5` to plane X
  iff plane X's contact was on plane Y's body-sphere.

## Why this is interesting for loft

The shipping mechanics of @PLN6 — phone HTML page + loft server +
projector — are already proven.  What this demo would surface as
**new** language / library asks:

| Surface | What's needed | Status |
|---|---|---|
| Twin-strip touch input | Phone-side: capture two simultaneous touches by x-coordinate band, report `{L, R}` ∈ `[0, 1]²` over WS at ~30 Hz | New on the HTML/JS side; pure client work |
| Per-frame WS pose sync | 30 Hz `(L, R, L_on, R_on)` input per phone → server; 30 Hz per-plane pose broadcast back, **sight-filtered per recipient** (`net.peer_sight_range`) so each phone only receives poses for OTHER planes within visual range.  Total throughput scales with typical visible-peer count, not N − 1 — substantially under naïve worst-case.  Pose frames are ~32 bytes fixed-point (see `net.pose_frame_bytes_budget`), not JSON | Reuses [`lib_plans/future/08-server/` § Gap 8 — `BroadcastTopology`](../../lib_plans/future/08-server/README.md#gap-8--per-recipient-broadcast-qos-sight--rate-lod--forecast); the sight + rate-LOD + forecast pattern is generalised there for dryopea reuse |
| Phone-side first-person 3D | WebGL view from the cockpit: extruded world geometry + other planes' positions + their smoke trails.  Own plane mostly invisible (nose section + frame overlay only) — no chase-camera tuning since the camera IS the cockpit | New on the HTML/JS side; same world payload as projector, simpler camera |
| Static world load | Read a dryopea MapFile JSON OR a `store_persist_bind`'d hash, extrude per palette; serve as a single download to phone + projector at session start | Loader is [`hex_world::load_mapfile()`](https://github.com/loft-lang/loft-libs-world/blob/main/hex_world/MAPFILE.md) (Phase 7a wraps the existing moros_map shape behind a documented schema); palette extrusion fields live on `MaterialDef.md_extrude` per the schema doc.  Stencil-pipeline supplement after @PLN49 plan 06 lands |
| 3D continuous physics | Plane integrator (pose + velocity + angular vel); sphere-vs-geometry + sphere-vs-sphere collision; reflection with damping | Uses [`lib_plans/75-physics-2body/`](../../lib_plans/75-physics-2body/README.md) — shared rigid-body library; PLAN50's "0.7 outward / 0.2 tangent" target kick is `reflect(v, n, 0.7, 0.2)` in that slot's API |
| Smoke trails + score confetti | Ring buffer of recent positions per plane (~90 entries at 30 Hz / 3 sec); ribbon mesh.  Plus point-burst score confetti at collision points | Uses [`lib_plans/76-particles/`](../../lib_plans/76-particles/README.md) — two-flavour particle library (Trail + Burst) shared with dryopea explosions / scramble exhaust |
| Centroid camera (projector only) | Weighted-mean position with decay; smooth lookat lerp; auto-zoom to keep active group in frame | Trivial helper; lib/graphics extension |
| Off-axis collision scoring | Cross-product / dot-product on velocity vectors | Trivial math; no library need |

**No language gaps surface.**  The plan is application code on top
of shipped libraries — same pattern as @PLN6 phase 3 was.  The
language has all the primitives this needs.

## Why it's a better audience demo than @PLN6

@PLN6 was generative-art on a 2D hex grid — the audience watched
patterns emerge from collective painting.  Lovely as art, but the
**participatory loop** was thin: tap a hex, see it appear.

This demo:

- **Continuous control** — each player is steering at 30 Hz, not
  tapping at 0.1 Hz.  Engagement-per-minute is much higher.
- **Visible individuality** — coloured smoke trails make each
  audience member's path visible on the big screen.  People can
  pick out their own trail in real-time.
- **Emergent social play** — anti-coordination scoring rewards
  flying *near* other players (broadside hits) but punishes
  collusion (head-on agreement).  Skilled play involves chasing,
  evading, ambushing.
- **Static authored world** — the dryopea editor doubles as the
  authoring tool, demonstrating dryopea's downstream use-case the
  same way moros's editor doubles as a level builder for moros.
- **Same physical setup** — projector + phones over LAN, no new
  hardware ask vs. @PLN6.

## Open questions (not blockers for the draft)

1. **Player cap.**  @PLN6's load test validated 30 simultaneous
   clients.  Does plane physics + per-frame WS scale to 30?  Or do
   we cap at ~12 and have the rest spectate?  Trivial network test
   can answer.
2. **Onboarding.**  New phone joins mid-flight — spawn point?
   Probably a "departure pad" at the edge of the map, off-camera.
3. **Anti-grief.**  A player spawn-camping the departure pad would
   stall new joiners.  Maybe `4-sec invincibility` on respawn (no
   bounce, no collision).
4. **Round structure.**  Continuous-running clock + leaderboard?
   Fixed-length rounds (5 min)?  Probably 5-min rounds with a
   between-round leaderboard slide.
5. **Smoke colour assignment.**  Per-player random?  Or audience
   chooses from a palette page-1?  Probably random — keeps the
   join-flow to one tap.
6. **Target authoring.**  Target bumpers have intrinsic visual
   (red/white sphere) and intrinsic size (~2 m), so authoring is
   pure position-plus-elevation.  Two options for the format:
   - **Add a `target` palette type to the dryopea editor.**  The
     editor paints positions; a separate elevation field per
     painted hex picks the height of the sphere above the ground.
     One source-of-truth file; targets ship with the map.
   - **Separate `targets.json` keyed to the same hex coords.**
     Keeps dryopea's scope unchanged; demo-side loads both files
     at startup.  Simpler to ship without touching dryopea.

   Recommend separate JSON for the first version, palette-type
   migration only if it becomes a friction.
7. **Yaw direction mapping.**  Which thumb-lift yaws which way?
   Two candidates:
   - *Press-the-side-you-want:* left thumb stays in contact →
     yaw left.  Maps onto real-aircraft rudder-pedal intuition
     ("push left rudder to yaw left").
   - *Lift-the-side-you-point-to:* left thumb lifted → yaw left.
     More gestural (raising a hand to indicate direction);
     possibly more discoverable for non-pilots.

   Decide by playtest with 3–4 audience-naive testers each way.
   The thumb-position-while-yawing is intentionally ignored so
   the lifted-thumb player can't accidentally also pitch the
   plane.

8. **Per-target parameter overrides.**  Currently every target
   uses the global `target.cooldown_secs` / `target.score` /
   `target.tilt_angle` from [NUMBERS.md](NUMBERS.md).  If a
   specific target turns out to be too easy or too hard for its
   placement (a sheltered one becomes a +1 farm, an exposed one
   stays uncontested), there's no per-target knob to tune.
   Options:
   - **Global only (v1):** keep authoring simple; if a target
     misbehaves, move it geometrically.
   - **Override fields per target:** the target authoring data
     carries optional `cooldown_secs` / `score` overrides that
     fall back to global defaults.  More authoring complexity
     but finer balance control.

   Recommend global only for v1, revisit if a specific target
   keeps breaking the round economy.

## Sub-arcs (sketch — phase the work like @PLN6)

| # | What ships | Effort | Builds on |
|---|---|---|---|
| 0a | [Network throughput probe](00a-network-probe.md) — synthetic 30 Hz × N WS load against the existing @PLN6 server; resolves the dominant unknown (does the broadcast pump hold at 12 / 20 / 30 clients?) before phase 0 commits substantial code | XS | @PLN6 phase 1.9 (Tier A′ pump + WsGroup) |
| 0  | Phone client — twin-strip + first-person canvas + WS skeleton + Web Audio samples + **per-peer smooth-rendering (linear interp between received frames + fade in/out on sight-range crossings)** | S-M | phase 0a verdict + @PLN6 phase 0 |
| 1  | Loft server — per-client pose state, 30 Hz broadcast loop, event dispatch (collisions, scores, stalls) | S | @PLN6 phase 1 (Tier A′) |
| 2  | Static world loader — `hex_world::load_mapfile()` + per-palette extrusion (palette-`md_extrude` → 3D pillars / cliffs / ramps) | XS | [`hex_world/MAPFILE.md`](https://github.com/loft-lang/loft-libs-world/blob/main/hex_world/MAPFILE.md); @PLN49 plan 01 E4 |
| 3  | Projector renderer — world + planes + trails + centroid camera + score-pop overlays + countdown | M | @PLN6 phase 3; [`lib_plans/76-particles/`](../../lib_plans/76-particles/README.md) Phase 1 (Trail) + Phase 2 (Burst) |
| 4  | Physics — plane integrator + bounce + stall | M | [`lib_plans/75-physics-2body/`](../../lib_plans/75-physics-2body/README.md) Phases 1-5; the existing [`tools/audience-demo-50/forecast_test.loft`](../../../../tools/audience-demo-50/forecast_test.loft) (Q1/Q2/Q4/Q5 — 4/4 PASS) is the acceptance rig for the slot's reflect-with-energy-split + nose/body 2-collider |
| 5  | Scoring + ambience — targets file + off-axis collision rule + leaderboard + "the storm" difficulty ramp | S | particles Phase 3 (score-burst factory) |
| 6  | Live playtest + tuning (controls, audio mix, palette) | S | all above |
| 7  | **Per-phone adaptive QoS** (post-v1) — passive RTT/loss estimation, three-tier classification (Good / OK / Limited), per-phone scaling of `peer_sight_range` and rate-LOD radii.  Phones with strong connections see more peers at higher fidelity; weak connections degrade gracefully instead of dropping | S | [`lib_plans/future/08-server/` § Gap 8](../../lib_plans/future/08-server/README.md#gap-8--per-recipient-broadcast-qos-sight--rate-lod--forecast) — the QoS tier-classification layer lives in `lib/server`'s `PerPeerScaling` strategy, reusable by dryopea multiplayer |

Each phase ships standalone (incremental playable state); the
cadence is the same flat-2D-MVP-first sequence that worked for
@PLN6.

## Cross-references

- [@PLN6 audience-generative-art](../6-audience-generative-art/README.md)
  — phase 3 renderer pattern, single-port HTTP+WS server,
  multi-client load test (the reusable substrate).
- [@PLN43 phase 01c — store_persist_bind](../43-loft-store-durable/README.md)
  — eventual home for the static world file (alternative to JSON).
- [@PLN49 dryopea](../49-dryopea/README.md) — editor that authors
  the world; plan 06 stencil pipeline gives us authored prop
  libraries when it lands.
- [`lib_plans/12-library-extraction § Phase 7p`](../../lib_plans/12-library-extraction/README.md#phases-6w--7b--7c--8)
  — the cross-cutting primitives (physics, particles, server QoS,
  MapFile schema) PLAN50 depends on land via Phase 7p.
- [`lib_plans/75-physics-2body/`](../../lib_plans/75-physics-2body/README.md)
  — shared rigid-body physics; sub-arc 4 consumes Phases 1-5.
- [`lib_plans/76-particles/`](../../lib_plans/76-particles/README.md)
  — Trail + Burst particle library; sub-arcs 3 + 5 consume.
- [`lib_plans/future/08-server/` § Gap 8](../../lib_plans/future/08-server/README.md#gap-8--per-recipient-broadcast-qos-sight--rate-lod--forecast)
  — broadcast QoS layer (sight + rate-LOD + bounce-forecast);
  sub-arc 7 consumes.
- [`hex_world/MAPFILE.md`](https://github.com/loft-lang/loft-libs-world/blob/main/hex_world/MAPFILE.md) —
  cross-project MapFile schema; sub-arc 2 consumes via
  `hex_world::load_mapfile()`.
- [`lib/graphics`](https://github.com/loft-lang/loft-libs-graphics/tree/main/graphics) — projector renderer
  base.
- [`lib/server`](https://github.com/loft-lang/loft-libs-net/tree/main/server) — multi-client WS hub.
