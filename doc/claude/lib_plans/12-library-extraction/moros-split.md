<\!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-12 — moros world split + final cleanup phases

Part of [@PLAN12 library extraction](README.md).  Covers
the cross-project-consumer thread: **Phase 7a** (split moros
into shared `lib/world/` spatial primitives + moros-specific
remainder — cross-project unlock for moros + dryopea + bumper),
**Phase 7p** (cross-cutting primitives extracted before moros
migrates — physics-2body slot, particles slot, editor rename),
**Phase 6w** (extract `loft-libs-world`), **Phase 7b/7c**
(moros + dryopea project moves), and **Phase 8** (final
monorepo cleanup + `audience_crystal` test dir).

The shared-library matrix in the [main plan README](README.md#cross-project-consumers--moros--dryopea--bumper-airplanes)
sits alongside these phases — that matrix is the WHY for the
work in this file.

> **STATUS 2026-06-14 — the moros project repo move (Phase 7b) is DONE.**  Once
> the compiler fixes (#375/#374/#373 + the `&`-param deferrals) made the moros
> stack load + test-green on both backends, all five `lib/moros_*` packages were
> moved to the moros project's own repo (`workspace/moros/lib/`) and removed from
> this monorepo.  loft keeps fixture clones of the three it exercises in its own
> feature tests (`tests/fixtures/libs/{moros_map,moros_editor,moros_render}`) — the
> P150 leak guard, the `--html`/WASM smoke, the GLB exit-code guards, and the
> @P379 per-library-namespace test retarget to those.  The Phase 7a "build out
> `lib/world/`" narrative below is **historical** — `lib/world` already shipped to
> `loft-libs-world` as `hex_grid`, and moros shipped with its own `Hex`/`Chunk`
> rather than adopting it (a later adoption is the moros project's call).

---

### Phase 7a — moros world split (cross-project unlock; appears monorepo-internal)

Move the non-moros-specific spatial primitives into `lib/world/`: hex /
chunk types + addressing (from `moros_map`), wall / hex geometry
collision (from `moros_sim/collide.loft`), `lib/wall.loft` (folds in
whole — `DX`/`DY`/`DZ`/`STEP` + placement/edge helpers, load-bearing
for dryopea's build-order walls + rock faces), `lib/overland.loft`
(`OverlandMap` terrain layers), group/height handling.  Palette,
spawn, editor/UI/render stay in `moros_*`.  Preserve the existing
sparse Cell/Chunk shape (TTT v5 + audience demo) alongside the hex
additions (they share addressing — Open Q #10).  Unblocks dryopea
([@PLAN46](../../plans/49-dryopea/README.md)) AND
bumper-airplanes ([@PLAN50](../../plans/51-bumper-airplanes/README.md))
— see [README.md § Cross-project consumers](README.md#cross-project-consumers--moros--dryopea--bumper-airplanes)
for the shared-substrate argument.  Phase 7a is described as
"monorepo-internal" because no user-visible behaviour changes, but
**three downstream projects depend on its output** — it is the
top of the dependency chain for 6w/7b/7c.

Phase 7a also subsumes the **MapFile schema promotion**: the JSON
format currently buried inside `lib/moros_map` becomes a documented,
versioned `world::load_mapfile(path)` entry point in `lib/world/`,
since dryopea and bumper both consume it.

**Done so far:**

- `lib/world/` package created — sparse 32x32 chunk world model,
  4-byte `Cell` wire-format, save/load round-trip, 389 lines.  Smoke
  test (`lib/world/tests/world.loft`) passes under `--interpret`.
- `lib/wall.loft` (754L) + `lib/overland.loft` (7L) folded into
  `lib/world/src/` (2026-05-28).  These had zero `use` references at
  `lib/` root — physically dead code — so the move is non-breaking.
- **MapFile schema documented** as a cross-project contract in
  `lib/world/MAPFILE.md` (2026-05-28) — v1 JSON shape, versioning
  policy, per-consumer overlays (moros / dryopea / bumper), and a
  6-step migration plan to `world::load_mapfile()` for when the
  consumer stall lifts.  Schema is currently enforced by
  `lib/moros_map`'s `Map` struct (the contract names what's there).
- **The moros stack LOADS again (2026-06-14).**  The deep
  `moros_ui → moros_sim → moros_render/moros_editor → moros_map` diamond
  pushes `moros_map` to a high source number, which had broken
  cross-package type resolution on pass 1 — the load failed outright.
  Fixed in the compiler (a class of "pass 1 hard-errors on a still-
  resolving cross-package reference instead of deferring to pass 2"
  bugs: #375 field resolution, #373 forward-ref struct layout, and the
  `&`-param addressability / work-buffer / `change_var_type` sites).
  The full stack now loads clean and **all 427 moros library tests pass
  on BOTH backends** — interpreter and `--native` (moros_map 51,
  moros_editor 44, moros_render 154, moros_sim 137, moros_ui 41).  This
  was the standing prerequisite for validating the split against the real
  consumer: moros is now a loadable, test-green, dual-backend consumer, so
  the split work below can proceed against it.  Note `lib/world/` is now
  empty — its hex substrate already shipped to `loft-libs-world` as
  `hex_grid` (commit 6c9a03e9), so the "build out lib/world/" framing
  below is partly historical; the live remaining work is the moros→shared
  migration (step 6) and the moros project repo move, both of which the
  loadable moros now unblocks.

**Remaining work — UNBLOCKED today (re-evaluated 2026-05-29):**

A 2026-05-28 `sed` pass marking every wall.loft / overland.loft
item `pub` was reverted because the legacy moros syntax uses `enum`
for what current loft calls `struct` (`enum Tile { material: u8,
... }` — struct-shaped field list, no variants), and the parser
rejects it as `Expect enum values to be in camel case style`.  At
the time this was framed as consumer-blocked.  Re-validation 2026-
05-29 shows the framing was wrong: `grep -rn 'use wall\|use
world::wall' lib/` returns **zero** — wall.loft and overland.loft
have **no consumers anywhere in lib/**, so the translation breaks
nothing.

**Scope correction (2026-05-29):** a translation probe found
wall.loft carries *more* legacy-syntax issues than the
`enum`-as-struct claim:

- One `enum Tile { … }` to translate (line 165 — struct-shaped).
- *Two* duplicate struct names: `WallPoint` declared at lines 250
  AND 379 (different field shapes); `Line` declared at lines 262
  AND 393.  Loft rejects the duplicates — at least one of each
  pair must be renamed (design decision: which is canonical, or
  do both survive under distinct names).
- `assert!(...)` Rust-macro syntax at line 211 etc. — translate to
  `assert(...)`.
- `if flipped ^(steps < 0)` at line 317 — `^` operator semantics
  (XOR vs cast-shape) need a per-call check.
- Likely more once these clear and the parser proceeds further.

So the translation is **mechanical at the per-edit level** but
**design-shaped at the file level** (the duplicate struct rename
is a naming decision, not a transliteration).  Since wall.loft
has zero consumers, the safest path is: rename duplicates by their
*usage neighbourhood* (e.g. `WallPoint` near `Drawing` becomes
`DrawingWallPoint`), drop unused dead code, and validate by
parsing.  Estimated effort: **S–M** (half a day, vs the README's
prior "mechanical / XS" framing).  Treated as deferred-by-scope
until either Phase 7b moros migration unsticks (and the consumer
specifies which structs it needs) or a separate clean-up sub-phase
is filed.

The remaining mechanical / additive work below is still doable
today without any external-consumer coordination:

- Translate `enum`-as-struct declarations in wall.loft / overland.loft
  to current `struct` syntax (mechanical syntax migration; zero
  consumers to break).
- Mark items `pub` after translation.
- Wire `use wall;` / `use overland;` (or fold content directly) into
  `world.loft`.
- Move hex / chunk types + geometry collision **additively** from
  `moros_map` / `moros_sim/collide.loft` into `lib/world/` per
  MAPFILE.md migration plan steps 1–3 (leave the moros_map originals
  intact — non-breaking; consumers keep using their own types until
  ready).
- Add `world::load_mapfile()` / `world::save_mapfile()` entry points
  (MAPFILE.md step 4) — additive public API, no consumer surface.
- Add `pub use world::*;` shim in `moros_map` (MAPFILE.md step 5) —
  compatibility shim, moros_map's API to its callers unchanged.

Together these are five of MAPFILE.md's six migration steps and
constitute **the bulk of Phase 7a**.

**Remaining work — genuinely consumer-blocked (one item):**

- Migrate `lib/moros_*` to `use world;` (MAPFILE.md step 6) — this
  *replaces* `moros_map`'s internal `Hex` / `Chunk` types with
  `world::*` and changes consumer call sites.  Paired with the
  external moros project's migration; not done in advance.

The consumer stall affects step 6 only.  Steps 1–5 are unblocked.
**6w (`loft-libs-world` chunk extraction) depends on steps 1–5 but
NOT on step 6** — the chunk can ship at `world 0.1.0` carrying the
additive types + load/save API while moros continues to use its
own internal copies, then upgrade to `world 0.2.0` when the
consumer stall lifts.

**Coverage prerequisite (links to Phase 6t Tier 5):** `lib/world`
currently has only the smoke test.  Before 6w extracts, Tier 5
adds save/load round-trip + MapFile schema tests against
`world::load_mapfile` / `world::save_mapfile` (added in step 4
above) + sparse-write boundary tests.  Coverage growth pairs
naturally with the API growth.

**MapFile schema (concrete design — part of 7a):**

Today the MapFile JSON format lives implicitly inside `lib/moros_map`'s
save/load code.  Three projects consume it; before extraction the
schema must be:

1. **Versioned at the top level** — every MapFile carries a
   `schema_version: integer` field.  Loaders for older versions
   stay supported within `lib/world`; new fields land as
   backwards-compatible additions (default-on-absent).
2. **Documented as a stable contract** — a `MAPFILE.md` reference
   doc in `lib/world/` describing every field, its semantics, the
   versioning policy, and the loader's null-default behaviour.
3. **Loaded through one entry point** — `world::load_mapfile(path:
   text) -> Map` and `world::save_mapfile(self: Map, path: text) ->
   FileResult`.  Callers never see raw JSON; the format becomes
   the library's responsibility.

Sketch of v1 fields (extracted from current moros_map save/load):

```jsonc
{
  "schema_version": 1,
  "name": "test_map",
  "size": { "cx_min": -2, "cx_max": 5, "cz_min": -2, "cz_max": 5 },
  "palette": [
    { "id": 0, "name": "void",  "color": "#00000000" },
    { "id": 1, "name": "grass", "color": "#5fa030ff", "height_band": [0.0, 2.0] },
    { "id": 2, "name": "wall",  "color": "#808080ff", "extrude": "pillar:8,12" }
  ],
  "chunks": [
    {
      "cx": 0, "cz": 0,
      "cells": [
        { "hx": 0,  "hz": 0,  "palette": 1, "h": 0.5, "item": null },
        { "hx": 1,  "hz": 0,  "palette": 2, "h": 0.0, "item": null }
      ]
    }
  ],
  "spawn":  [ { "x": 0.5, "z": 0.5, "kind": "player" } ],
  "items":  [ ],
  "walls":  [ ]
}
```

**Per-consumer overlays.**  Bumper extrudes `palette[i].extrude`
strings into 3D pillars / cliffs; dryopea reads `palette[i].height_band`
for slope generation (paired with [lib-plan 20 terrain-heightmap](../71-terrain-heightmap/README.md));
moros reads the existing flat fields.  Unknown overlay fields are
preserved on round-trip (forward-compat).

**Targets / bumper-specific data.**  Per PLAN50 Open Q #6
(recommended: separate `targets.json`), bumper's target positions
ship in a sibling file keyed to the same hex coords — not inside the
MapFile.  Keeps the MapFile shape stable across all three games.

### Phases 6w / 7b / 7c / 8

Chunk extractions + cleanup; each follows the
[per-chunk template](REFERENCE.md#per-chunk-extraction-template).  6w
needs `world` complete (7a) + green CI (6.5); 7b needs graphics +
world published; 7c is greenfield ([@PLAN46](../../plans/49-dryopea/README.md));
8 adds `audience_crystal` package `tests/` and updates
[PACKAGES.md](../../PACKAGES.md) to the monorepo-free state.

**Phase 7p prerequisite for 7b — extract cross-cutting primitives
first.**  Moving `lib/moros_*` into the existing moros project is
naively a git filter-repo + path update, but if it ships before the
cross-cutting primitives are factored out, dryopea + bumper end up
copy-and-forking moros internals.  Concrete checklist before 7b
ships:

| Sub-arc | Driving slot / design | Verify |
|---|---|---|
| Editor rename + L1-L2 of universal-editor extraction | [`lib_plans/73-universal-editor/`](../73-universal-editor/README.md) L0 (architecture spike + naming) + L1 (`hex_grid`) + L2 (`hex_map`) | `lib/moros_editor/`'s name reflects its cross-game scope; the L0 naming decision is what unblocks the rename |
| Physics primitives | [`lib_plans/75-physics-2body/`](../75-physics-2body/README.md) Phase 1 (types + sphere-vs-AABB step) | `lib/moros_sim/collide.loft` items migrated into `lib/physics_2body/`; moros tests green using the new package |
| MapFile schema | Inline design in Phase 7a above | `world::load_mapfile()` + `world::save_mapfile()` are the only entry points; `MAPFILE.md` documents v1 |
| Particles slot | [`lib_plans/76-particles/`](../76-particles/README.md) Phases 1–2 (trail + burst types) | Slot READMEs exist; PLAN50 / dryopea use the slot's API in their (still-stalled) design docs |
| Broadcast QoS | [`lib_plans/future/08-server/` § Gap 8](../future/08-server/README.md) — `BroadcastTopology` + sight + rate-LOD + forecast | `lib/server/src/broadcast.loft` exposes the topology API; PLAN50 wires through it |

Each row is independently sized in its own slot; this table is the
order-of-operations checklist, not the implementation plan.

