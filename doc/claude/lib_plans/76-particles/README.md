<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# lib-plan 27 — `particles`: ribbon trails + point bursts

**Status:** FUTURE (slot created 2026-05-28).  Driven by the
cross-project consumer audit in
[lib-plan 12 § Cross-project consumers](../12-library-extraction/README.md#cross-project-consumers--moros--dryopea--bumper-airplanes):
two games ([@PLAN46 dryopea](../../plans/49-dryopea/README.md),
[@PLAN50 bumper-airplanes](../../plans/51-bumper-airplanes/README.md))
both need ribbon trails and point-burst particles; without a shared
package, PLAN50 ships bespoke smoke trails + confetti and dryopea
copies-and-modifies for explosions + scramble exhaust.

Scope is **intentionally narrow** — two particle flavours that cover
the demonstrated game needs, not a general-purpose particle engine.

## Two flavours

### Ribbon trails

A per-emitter ring buffer of recent positions; rendered as a quad
strip with alpha fading from head to tail.

**Consumers:**
- PLAN50: smoke trails per plane (3-sec history at 30 Hz = 90
  positions), colour-per-player.  Critical for "see who's where"
  read on the projector.
- dryopea: scramble-rocket exhaust trail; potentially missile
  trails on enemy projectiles.
- moros: not currently planned, but trivial to add (player
  movement trail in third-person mode).

### Point bursts

A short-lived particle cloud emitted at a moment in time; each
particle has position + velocity + colour + lifetime.  Drift,
gravity, alpha-fade over lifetime.

**Consumers:**
- PLAN50: score confetti (30 particles for +1, 80 for +5, combo
  scaling) emitted at collision points.
- dryopea: explosions on tower hits, base destruction, enemy
  death.  Spark showers from scramble-rocket ignition.
- moros: not currently planned.

## Proposed API (sketch — not locked)

### Trails

```loft
pub struct Trail {
  positions:    vector<Vec3>,   // ring buffer, length = capacity
  head_idx:     integer,
  capacity:     integer,        // typically 90 (3 sec × 30 Hz)
  colour:       integer,        // packed RGBA
  fade_curve:   FadeCurve,      // Linear / EaseOut / etc.
  width:        float,          // ribbon thickness in world units
}

// Add a new position; oldest drops off when capacity is reached.
pub fn trail_emit(self: &Trail, pos: Vec3)

// Render-time: build a triangle strip representing the ribbon.
// Returns vertices + UVs + per-vertex alpha for the consumer's
// renderer to draw.
pub fn trail_geometry(self: Trail, camera_up: Vec3) -> RibbonMesh
```

### Bursts

```loft
pub struct Burst {
  particles:    vector<Particle>,
  birth_t:      integer,    // ms since session start
  duration_ms:  integer,    // particle lifetime
  gravity:      Vec3,
}

pub struct Particle {
  pos:          Vec3,
  vel:          Vec3,
  colour:       integer,
  size:         float,
}

// Emit a burst.  `count` particles at `origin` with velocities
// distributed in a hemisphere (or cone if `direction != Vec3::ZERO`)
// at `speed_range`.  Returns a Burst owned by the caller (drops
// itself out of relevance after `duration_ms`).
pub fn burst_spawn(
  origin:        Vec3,
  count:         integer,
  colour:        integer,
  direction:     Vec3,           // Vec3::ZERO for omnidirectional
  speed_range:   (float, float),
  duration_ms:   integer,
) -> Burst

// Step a burst forward by dt_ms.  Returns true if still alive.
pub fn burst_step(self: &Burst, dt_ms: integer) -> boolean

// Render-time: per-particle billboard positions + alpha.
pub fn burst_geometry(self: Burst, now_ms: integer) -> PointCloud
```

## What's NOT in this library

- **Stateful "particle systems"** with continuous emission, attractor
  fields, curl noise, sub-emitters, etc.  Two finite-lifetime
  primitives only.
- **GPU-side simulation.**  Trails + bursts are step-on-CPU; the
  consumer's renderer uploads the geometry per frame.  If GPU
  simulation ever becomes a requirement, that's a separate library.
- **Texture-driven particles** (smoke puffs with sprite atlases).
  Out of scope — particles are coloured points + ribbons are
  coloured quads.  PLAN50 explicitly chose this constraint.

## Implementation phases

| # | What ships | Effort | Notes |
|---|---|---|---|
| 1 | `Trail` type + ring-buffer emit + ribbon geometry builder | S | Closes PLAN50 phase 3's smoke-trail need |
| 2 | `Burst` type + step + point-cloud geometry | S | Closes PLAN50 phase 5's score confetti |
| 3 | Per-game tuning helpers — `score_burst(player_colour, count)` factory etc. | XS | Convenience layer; tunables live in consumer's NUMBERS.md |
| 4 | Pooling (optional) — reuse `Burst` allocations within a `BurstPool` to avoid GC churn | S | Only if profiling shows it matters |

## Open questions

1. **Ribbon orientation:** trails face camera (billboard) or use a
   fixed up-axis?  Bumper's smoke is at all altitudes — camera-facing
   probably right.  Dryopea ground-level explosions can use
   camera-facing too.  Default: camera-facing.
2. **Colour space:** packed RGBA u32 (existing `lib/graphics`
   convention) or float-RGBA?  Packed matches existing libraries.
3. **`lib/graphics` dependency:** is the geometry builder in this
   library or in `lib/graphics`?  Probably here (so it's
   game-agnostic), with the consumer's `lib/graphics` call drawing
   the returned mesh.

## Cross-references

- [lib_plans/12-library-extraction](../12-library-extraction/README.md)
  — Phase 7p of that plan blocks on this slot existing.
- [@PLAN46 dryopea](../../plans/49-dryopea/README.md) —
  consumer for explosions + exhaust.
- [@PLAN50 bumper-airplanes](../../plans/51-bumper-airplanes/README.md)
  — consumer for smoke trails + score confetti.
- [`lib/graphics`](https://github.com/loft-lang/loft-libs-graphics/tree/main/graphics) — the renderer the
  consumer draws the generated geometry with.
