<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# LAVITION.md — engine brand + architecture + library model

[lavition](https://github.com/lavition/lavition) is the universal editor
built on top of the loft language stack.  It edits hex-grid worlds with
walls, materials, items, and heights, supports multi-level layering,
stencil-driven authoring, and detection of round / curved structures.
This doc captures the brand layering with loft, the engine's
architecture, and the library model that organises lavition's plugins.

For the design vision (12-direction buildings, 24-direction walls,
curve detection, vertical layers, dual-use stencils) see the
[`lavition/lavition` README](https://github.com/lavition/lavition).

## Brand layering — loft vs lavition

Two distinct brand identities, by tier:

| Tier | Brand | Role | GitHub org |
|---|---|---|---|
| Language | **loft** | embedded scripting language + interpreter + standard libraries | [`loft-lang`](https://github.com/loft-lang) |
| Engine | **lavition** | universal editor product built on loft | [`lavition`](https://github.com/lavition) |

**Why two brands.** loft is the technical layer — a language with its own
identity, registry, and ecosystem of libraries.  lavition is the
user-facing product — the editor a game developer downloads, the brand
indie devs discover via Google.  Keeping them separate lets each evolve
on its own schedule without collateral churn.

**A game's relationship to both:**

- *Runtime:* depends on `loft` + the `loft-libs-*` packages it imports.
  No lavition dependency at runtime.  A built game ships its own binary
  containing the loft runtime + the libraries it links.
- *Authoring time:* uses lavition's editor to compose worlds, place items,
  edit walls.  The editor writes data files the game then reads at
  runtime via the same `loft-libs-world` primitives.

So games are *runnable without lavition*, *authorable with it*.

## Naming principle — brand in metadata, symbols stay descriptive

The brand belongs where users find the product: org URLs, repo names,
package homepages, plugin manifests, docs hero, marketing.  The brand
does NOT belong in the names users read inside script code.

| Place | Pattern | Example |
|---|---|---|
| GitHub org | brand-prefixed | `github.com/loft-lang/`, `github.com/lavition/` |
| Repo / chunk | brand-prefixed | `loft-libs-world`, `lavition/editor` |
| Crates.io crate | brand-prefixed where coupled | `lavition_core` |
| **`use X;` in `.loft` scripts** | **descriptive only — NO brand prefix** | `use hex_world;`, `use hex_walls;`, `use terrain;` |
| Plugin manifests (lavition-internal) | short descriptive | `[plugin] name = "hex_editor"` |

Same model as cargo: `use serde;` is descriptive; the brand
(`serde-rs/serde`) lives in the repo URL.  A cold reader encountering
`use hex_world;` knows what the library is from the name alone; no need
to also know what engine the script targets.

## Engine runtime architecture — the Rust main loop, loft, and live programming

lavition's runtime is **two cooperating layers over one shared store**:

- **The main loop is Rust** — a capable native host that owns the frame cycle:
  timing, rendering, physics, input, the platform event loop.  This is the
  performance-critical core, and it stays native.  loft does **not** try to be
  the main loop; Rust is better at it, and the split is deliberate.
- **loft is the live-editable layer** — the game's scripting and, more
  fundamentally, its **data model**.  The logic and data a developer edits while
  iterating live in loft, where they are predictable
  ([Goal E](GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth)),
  friction-free ([Goal F](GOALS.md#goal-f--friction-free-surface-the-language-serves-the-programmer-not-the-compiler)),
  and — the point of this layer — **survive being edited while the game runs**.

**The seam between them is the store.**  Both layers read and write **one shared
store** — the same word-addressed, schema-driven, `DbRef`-keyed substrate.  This
is the [@C71 shared-store dispatch](NATIVE.md#n9--native-library-shared-store-dispatch-c71)
generalised from "a native library + its interpreted caller share a store" to
"the host loop + the script share the game's data."  There are not two
representations kept in sync; there is one — the Rust loop ticks the world by
reading and writing store records, and loft reads and writes the same records.

```
 ┌───────────────────── Rust main loop (native, per frame) ─────────────────────┐
 │  input → update → physics → render → present   (capable, perf-critical core)  │
 └─────────────┬─────────────────────────────────────────────┬──────────────────┘
               │ reads / writes                               │ calls (per frame / on edit)
               ▼                                              ▼
       ┌────────────────────────── one shared store ──────────────────────────┐
       │  schema = data (Stores.types / Parts) ·  values = DbRef records       │
       └──────────────────────────────┬───────────────────────────────────────┘
                                       │ reads / writes
                                       ▼
                        loft — game logic + data model (live-editable)
```

### Live programming — "don't break the game while you're programming it"

The reason for this architecture is a single engine capability: **a developer
edits the game while it runs, and the running game stays alive.**  That is a
constellation, not one feature — data migration (the [GOALS.md
purpose](GOALS.md#why-a-language-not-a-store-bolted-onto-an-existing-one)) is the
first key node:

| node | what it preserves across an edit | status |
|---|---|---|
| **Data continuity** | game data survives struct/enum changes — migrate, don't reset | foundation in progress (own-format serialize + schema-driven migrate) |
| **Code continuity** | hot-swap a function body, add/remove fns + types, while running | partial — interpreted; the incremental-recompile path (`byte_code_from`) is mapped |
| **State continuity** | the running world + session survive the edit (store-resident session image) | **shipped** — [@PLN14 store-resident REPL session](plans/14-store-resident-repl-session/README.md) ([reference](REPL.md)): values live in a session store, observing reads it, and an on-disk image persists them behind a layout gate (not yet wired to auto-resume) |
| **Fault containment** | a mistake fails *recoverable*, never halts the game — see it, fix it, retry | seeded — REPL per-eval isolation; generalise to the live session |
| **Live interface** | a console into the running game to inspect + change live state | the REPL (`:vars` / `:type`) is the surface; its destination is the **debugger** [@PLN16](plans/16-debugger/README.md) (browser its natural home, but terminal/embedded too) — breakpoint → switch routine + callers to interpret → drop into a REPL at that line with the live frame's bindings → optional conditional/test breaks |

### Where the hard part is — reference stability

The serialize/parse round-trip is the *easy* half.  Integrating migration into a
live Rust loop is mostly a **`DbRef`-stability** problem: the host loop holds
references *into* the store across frames — entity handles, cached lookups — and
when data migrates (records move, a struct grows), those references must stay
valid or be **remapped**.  Keeping the host's live pointers coherent across a
migration is the genuinely hard node, more than the text round-trip — named here
so future migration work treats it as first-class.

### Why the costs land where they do

- **The loop never pays migration.**  A schema change is occasional (a developer
  saved a file), so migration is a one-time pass costing a single frame hitch,
  never the steady-state loop.  That is why *correctness over speed* is the right
  call for the serialization: the loop itself is never on that path.
- **The serialization is host-callable by construction.**  `show`/`show_loft` /
  `parse` are `Stores` methods — Rust APIs the main loop invokes directly,
  between frames, on a detected edit.  No bridge layer.
- **Frame-yield already exists.**  loft yields per frame for the
  [HTML/wasm runtime](HTML_EXPORT.md); the same contract lets a host loop
  interleave loft execution with live-edit checks at frame boundaries.

### Execution granularity — per-function interpret over a compiled baseline

> **The heart of the engine** (2026-06-10): this model's full form — the
> live build swap on BOTH hosts (native process cutover / browser module
> swap in the living page, one invariant: *the build is replaceable; the
> state and the connections persist*) — is designed with its test catalog
> in [plans/18-engine-host/08-live-build-swap.md](plans/18-engine-host/08-live-build-swap.md).

The editor does **not** interpret the whole program and compile it later.  The
baseline is **everything compiled / optimized** — the libraries (graphics,
physics, world: the *heavy* code) as native / wasm cdylibs, and most game logic
too.  Editing flips a **single function** between tiers:

1. **Edit → that function drops to the interpreter, instantly.**  Only the
   function you mutated is interpreted; everything else stays compiled.  No
   compile step, no roundtrip — zero-latency feedback on exactly the code in
   flux.  (The loft interpreter is itself wasm in the browser — the
   [wasm runtime](WASM.md) — so this needs no server.)
2. **Server → that function swaps back to optimized wasm.**  The roundtrip
   recompiles the one function (loft's native codegen → optimized wasm) and
   hot-swaps it in.  Per-function throughout.

```
 baseline (always):  [ compiled libraries ]  +  [ compiled game functions ]   ← native / wasm
                                   ▲  ▲
                      shared-store │  │ dispatch (N9 / @C71)
                                   │  ▼
 editing fn F:    F interpreted now ─▶ calls compiled libs over the store ─▶ near-native
                     └─▶ server recompiles F → wasm ─▶ F swaps back to compiled ─▶ full speed
```

**Why an interpreted function stays fast: it calls *compiled* code.**  The
function being edited is thin orchestration; the cycles live in the heavy library
calls it makes — graphics, physics — which are **always compiled**.  Those calls
cross the boundary as a **shared-store dispatch**
([N9 / @C71](NATIVE.md#n9--native-library-shared-store-dispatch-c71)): an
interpreted caller and a compiled callee read and write the **same records**, so
the crossing is a dispatch, not a copy.  **This is the prime reason the
interpreter must run against compiled libraries** — it lets you interpret the few
lines you are touching while the engine underneath stays native.  "Interpreted"
means slow *for the function you're editing*, never slow *for the game*.

**Parity is what makes the per-function swap sound.**  A function moving
interpreted ↔ wasm is invisible *only because the three backends produce identical
results* — [Goal D](GOALS.md#goal-d--cross-platform--cross-backend-parity).  Were
they ever to disagree, the swap would silently change the running game
mid-session — so the differential backend sweep (Goal D's Check — "identical
output and diagnostics, zero differences") is the **enabling mechanism** for
tiered live execution, not just a correctness gate.

**The data survives every swap for free**, because code and data are separate:
all tiers operate on the same store, so swapping a function's *backend* never
touches the *records* it reads.

**It degrades gracefully.**  The server roundtrip is asynchronous and optional:
offline, or with the server unreachable, the edited function keeps running
interpreted.  You lose the performance tier on that one function, never the
ability to edit a live game.

*Build-side notes:* the engine-host design exploration —
[plans/18-engine-host/ENGINE_HOST.md](plans/18-engine-host/ENGINE_HOST.md) — records the
entry-gate probes for this model (the wasm bridge-tax measurement, unload safety, the
N9 dispatch table as the real build), the **main-loop IO contract** (frame-boundary
drain with byte/time budgets, completion-as-event, store-resident accumulation of long
loads), and the in-house prior art that already runs pieces of it in pure loft.

So loft's value proposition is sharpened: it does **not** win the main loop (Rust
does) — it wins the **live-edit experience**.  Predictable memory, a
friction-free surface, and data that survives your edits are the axis it is built
to own.

## Library model

Lavition's library landscape splits into four kinds:

### 1. Data + system libraries (loft ecosystem; consumed at runtime by games)

Live in `loft-lang/loft-libs-*`.  Bare descriptive names.  Games
import these and ship them in their runtime binary.

**Six chunks by tier-of-concern.**  The grouping rule: a chunk's
libraries should share an answer to "what does a consumer pulling
this in get pulled in TRANSITIVELY?"  Cross-chunk consumers know
what they're opting into.  This avoids the previous mistake of
bundling unrelated things under "graphics" just because they were
visually adjacent.

| Chunk | Domain | Transitive deps | Headless? |
|---|---|---|---|
| `loft-libs-core` | utilities | (none meaningful) | yes |
| `loft-libs-net` | networking | ureq, tungstenite | yes (server-side) |
| `loft-libs-graphics` | rendering (GPU) | glutin, gl, winit, fontdue, image, rodio | no |
| `loft-libs-assets` | file formats | png (imaging), gltf parsing (glb) | yes |
| `loft-libs-game` | game systems | (no external system deps; integrates with graphics/assets per consumer choice) | yes |
| `loft-libs-world` | hex-grid world data | (loft data only) | yes |

**Library inventory per chunk:**

| Chunk | Libraries | Status |
|---|---|---|
| `loft-libs-core` | `arguments`, `random`, `crypto`, `regex` | all shipped.  `random 0.2.0` (repo PR #9, 2026-06-10) adds the VALUE tier — owned deterministic `RandStream`s (seed_stream/get/indices) for games whose RNG is part of their state; crawler consumes it (its three private LCGs deleted). |
| `loft-libs-net` | `web`, `server`, `game_protocol` | all shipped |
| `loft-libs-graphics` | `graphics`, `shapes`, `gridmesh` | all shipped 0.1.x |
| `loft-libs-assets` | `mesh3d`, `glb` (+ `imaging` migrating in at next major) | **chunk bootstrapped 2026-05-31** ([loft-lang/loft-libs-assets](https://github.com/loft-lang/loft-libs-assets)).  `mesh3d 0.1.0` + `glb 0.1.0` shipped (registry PR #8, validator 3-of-3 gates green).  imaging migration deferred to next major-version boundary. |
| `loft-libs-game` | `physics_2body`, `particles`, `input`, `time`, `audio_bus` | **chunk bootstrapped 2026-06-01** ([loft-lang/loft-libs-game](https://github.com/loft-lang/loft-libs-game)).  `time 0.1.0` shipped (registry PR #11, validator green).  **`input 0.1.0` extracted from the monorepo `lib/input/` 2026-06-14** (@PLN3; depends on `graphics`).  Other inhabitants pending design. |
| `loft-libs-docs` | `html`, `markdown` | **chunk created 2026-06-14** ([loft-lang/loft-libs-docs](https://github.com/loft-lang/loft-libs-docs)).  General-purpose document/markup libraries (no game or GPU dependency) — `html 0.1.0` + `markdown 0.1.0` extracted from the monorepo `lib/{html,markdown}/` (@PLN3). |
| `loft-libs-world` | `hex_grid`, `hex_world`, `hex_walls`, `hex_terrain`, `hex_items` | **chunk bootstrapped 2026-06-01** ([loft-lang/loft-libs-world](https://github.com/loft-lang/loft-libs-world)).  `hex_world 0.1.0` shipped (registry PR #10, validator green; renamed from `lib/world` in monorepo W.1).  **`hex_grid` added 2026-06-10** (repo PR #1) — the GEOMETRY axis the other hex_* axes build on: the canonical pointy-top/odd-r/`L=√3` lattice math + the 12-orientation `cell_*` square basis, extracted from the **crawler** roguelike (which consumes it live — the first cross-game consumer).  The family's convergence plan (axial = interchange/storage, odd-r = authoring, `hex_grid` bridges; the capability roadmap: round towers, 24-dir walls, cliffs, water flow, collision, LOS, hearing) lives in the repo's [CONVERGENCE.md](https://github.com/loft-lang/loft-libs-world/blob/main/CONVERGENCE.md).  `hex_walls`/`hex_terrain` pending design (source material: dryopea `lib/wall.loft`/`lib/overland.loft` + crawler `wallgeo`). |

**Naming the hex_* family.**  Each library covers ONE data axis of
the hex world.  The `hex_*` prefix marks the family + leaves room for
parallel `voxel_*` / `tile_*` families later.

**Future cleanup notes** (not blocking):
- `imaging` lives in `loft-libs-graphics` today but conceptually
  belongs in `loft-libs-assets`.  Migrate at the next major-version
  boundary (deprecation period publishes under both chunks for one
  minor release).
- Graphics's `audio_*` + `sfx_*` surface is the mixer / OS audio
  output device interface — it's effectively a small "audio
  rendering" sub-library inside graphics.  Game-side audio
  TRIGGERS (categorise sounds, ducking, spatial) live in
  `audio_bus` (game chunk).  A future cleanup could move the mixer
  into its own `loft-libs-audio` chunk if audio surface grows.

These libraries know nothing about lavition.  They're equally usable by
a pure-loft CLI tool, a game built on lavition, or a third-party engine
that wants the same data primitives.

**`lavition_stack` meta-package.**  To hide the 6-chunk granularity
from new users, a small `lavition_stack` meta-package with NO code,
just dependencies:

```toml
[package]
name = "lavition_stack"
version = "0.1.0"
loft = ">=0.8"

[dependencies]
graphics = ">=0.1"; imaging = ">=0.1"; glb = ">=0.1"
hex_world = ">=0.1"; hex_walls = ">=0.1"; hex_terrain = ">=0.1"; hex_items = ">=0.1"
physics_2body = ">=0.1"; particles = ">=0.1"; input = ">=0.1"; time = ">=0.1"; audio_bus = ">=0.1"
```

A new game's loft.toml then becomes one line: `lavition_stack = ">=0.1"`.
The meta-package lives in a small `loft-libs-meta` chunk; composition
evolves via meta-package version bumps without breaking individual
library versioning.

### 2. Engine core (lavition org; required by every plugin)

Live in `lavition/<repo>`.  Brand-prefixed crate names because they only
make sense inside lavition.

| Crate | Purpose |
|---|---|
| `lavition_core` | plugin host + UI framework + input router + history/undo stack + selection abstraction + viewport |
| `lavition_render` | editor viewport rendering (grid overlays, selection halos, stencil previews) — builds on `graphics` |
| `lavition_asset_pipeline` | import / export / file watching / hot reload |
| `lavition_stencil` | shared stencil format (capture, serialise, apply at building scale or item scale) |

### 3. Editor plugins (lavition org; pluggable, one per data type)

Live in `lavition/<plugin-name>` repos.  Each plugin knows how to author
ONE data primitive.  Pluggable so games extend with their own data
types without modifying core.

| Plugin | Authors which data | Tools |
|---|---|---|
| `lavition_hex_editor` | `hex_walls` | paint segment, erase, snap to corners, curve detection toggle |
| `lavition_terrain_paint` | `terrain` heightmap + materials | brush, raise/lower, material palette, smooth |
| `lavition_item_placer` | `items` | place, rotate, animate, smaller-scale stencil-as-item |
| `lavition_layer_panel` | vertical layers | basement / ground / 2nd / roof toggle, free-placement layer creation |
| `lavition_stencil_library` | reusable stencils | save current selection as stencil, browse + stamp |
| `lavition_curve_detector` | derived view of `hex_walls` | smooth-render preview, threshold tuning |

Plugin contract (sketch):

```toml
# lavition.toml in each plugin repo
[plugin]
name        = "hex_editor"
edits       = ["hex_walls"]
loft_deps   = ["hex_walls"]
core_deps   = ["lavition_core"]

[tools]
paint = "src/tool_paint.rs::PaintTool"
erase = "src/tool_erase.rs::EraseTool"

[widgets]
toolbar     = "src/widget_toolbar.rs::Toolbar"
layer_panel = "src/widget_layer.rs::LayerPanel"
```

### 4. Engine binary (lavition org; the product users download)

Lives in `lavition/lavition` (the meta-repo currently holds the design
intent; the binary will land here or in a sibling `lavition/engine`
repo when the core stabilises).

The binary:
- Bundles `lavition_core` + `lavition_render` + `lavition_asset_pipeline`.
- Loads plugins from a configurable plugin dir + the bundled defaults.
- Embeds `loft` as the scripting layer for plugin scripts + project
  automation.
- Ships per-platform installers + a portable archive.

### 5. Game-internal libraries (separate user orgs; NOT in either ecosystem)

Live in each game's repo under that game's owner.  Project-prefixed
descriptive names.  These are not extracted because they encode
game-specific data shapes (moros's faction model, dryopea's mission
graph) that have no value to other games.

| Pattern | Example |
|---|---|
| `<project>_<thing>` | `moros_map`, `moros_sim`, `moros_render`, `dryopea_*` (planned) |

## Hex-family libraries — per-library narrative

Sketches of each `hex_*` data library covering: what data it owns,
which lavition plugin authors it, which game roles consume it, and an
API outline.  These are the source material for the eventual
`lavition.io/docs/<lib>/` pages (which own the "lavition + generic-term"
search space — see [Discoverability](#discoverability--the-practical-reason-for-brand-visibility)).

### `hex_world` — the addressing primitive

**Owns:** the hex grid itself.  Sparse 32×32 chunked storage so a 5000-hex
world is mostly empty chunks; load/save round-trips the whole structure.

**Lavition pairing:** loaded by `lavition_core` as the canonical world
coordinate system; every plugin reads/writes against `hex_world`
addresses (`HexId { col, row }` or chunk-relative
`(ChunkId, sub_col, sub_row)`).

**Game roles:**
- Runtime address space for the game's world data.
- Save/load primitive (the engine reads `world_save` / `world_load`
  state from this library).
- Iteration helper (`hexes_in_chunk`, `neighbors_of(hex)`,
  `bbox_of_chunks(region)`).

**API sketch:**
```loft
pub struct HexId { col: integer not null, row: integer not null }
pub struct ChunkId { x: integer not null, y: integer not null }
pub struct ChunkedWorld { /* sparse storage */ }

pub fn world_new(name: text) -> ChunkedWorld
pub fn world_save(w: const ChunkedWorld, path: text) -> boolean
pub fn world_load(path: text) -> ChunkedWorld
pub fn neighbors_of(h: HexId) -> vector<HexId>     // 6 neighbors
pub fn chunk_of(h: HexId) -> ChunkId
pub fn hexes_in_chunk(c: ChunkId) -> iterator<HexId>
```

### `hex_walls` — linear features at sub-hex resolution

**Owns:** wall / cliff / road segments between or along hex edges.  Each
hex edge has 4 sub-segments per side (so 6 edges × 4 sub-segments = 24
orientations per hex).  Segments carry material, height range, and
vertical layer.

**Lavition pairing:** authored by the `lavition_hex_editor` plugin.
The plugin's curve-detection pass reads `hex_walls` data and renders
chains of straight segments as smooth shapes for preview + final render.

**Game roles:**
- Collision data for the physics layer (`physics_2body` reads
  `hex_walls` to know what's solid).
- Render input for the game's wall renderer (read-only iteration over
  visible segments).
- Pathfinding input (segments block movement between hexes).

**API sketch:**
```loft
pub struct WallSegment {
  hex: hex_world::HexId not null,
  orientation: integer not null,   // 0..23
  layer: integer not null,         // basement/ground/2nd/roof/free
  material: integer not null,
  height_min: float,
  height_max: float
}

pub fn walls_at(w: const ChunkedWorld, h: hex_world::HexId) -> vector<WallSegment>
pub fn walls_between(w: const ChunkedWorld, a: hex_world::HexId, b: hex_world::HexId) -> vector<WallSegment>
pub fn add_wall(w: ChunkedWorld, seg: WallSegment)
pub fn remove_wall(w: ChunkedWorld, hex: hex_world::HexId, orientation: integer)

// Curve detection — derived view, no separate storage.
pub fn detect_curves(w: const ChunkedWorld, layer: integer) -> vector<CurveChain>
```

### `hex_terrain` — heights + materials per hex

**Owns:** per-hex height + material palette.  Slope rules between
adjacent hexes so the renderer knows whether to draw a cliff (handled
as walls) or a smooth ramp.  Material palette indexed by integer so
storage per hex is one byte for material + one for height.

**Lavition pairing:** authored by the `lavition_terrain_paint` plugin
(brush, raise/lower, palette swap, smoothing).

**Game roles:**
- Surface render (the game's ground renderer reads height + material
  per hex and meshes the visible region).
- Movement cost (steeper slopes = more cost; material affects speed).
- Spawn site selection (palette type → spawnable items / NPCs).

**API sketch:**
```loft
pub struct HexTerrain {
  height: integer,                     // 0..255, mapped to world Z by game
  material: integer not null           // index into MaterialPalette
}

pub struct MaterialPalette {
  materials: vector<MaterialDef> not null
}

pub fn terrain_at(w: const ChunkedWorld, h: hex_world::HexId) -> HexTerrain
pub fn set_terrain(w: ChunkedWorld, h: hex_world::HexId, t: HexTerrain)
pub fn slope_between(w: const ChunkedWorld, a: hex_world::HexId, b: hex_world::HexId) -> float
pub fn palette_of(w: const ChunkedWorld) -> MaterialPalette
```

### `hex_items` — placed instances

**Owns:** item instances anchored at one of the 12 building-placement
directions per hex.  Each item carries type, orientation, optional
animation state, and the vertical layer it lives on (including
"free placement" layers that don't participate in collision).

**Lavition pairing:** authored by the `lavition_item_placer` plugin.
Stencils placed as items (smaller render scale) write into this library.

**Game roles:**
- World population at runtime (the game iterates `items_in_region` per
  frame to render + tick animations).
- Interaction targets (item type → interaction script).
- Save/load (items round-trip with the rest of the world via
  `hex_world::world_save`).

**API sketch:**
```loft
pub struct HexItem {
  hex: hex_world::HexId not null,
  direction: integer not null,        // 0..11 (12-direction placement)
  item_type: integer not null,
  orientation: integer,                // optional rotation around vertical axis
  layer: integer not null,             // basement/ground/2nd/roof/free
  anim_state: integer                  // optional animation track + frame
}

pub fn items_at(w: const ChunkedWorld, h: hex_world::HexId) -> vector<HexItem>
pub fn items_in_region(w: const ChunkedWorld, region: ChunkRegion) -> iterator<HexItem>
pub fn place_item(w: ChunkedWorld, item: HexItem)
pub fn remove_item(w: ChunkedWorld, hex: hex_world::HexId, direction: integer)
pub fn tick_animations(w: ChunkedWorld, dt_ms: integer)
```

## Game-system libraries — per-library narrative

The `loft-libs-game` chunk holds general game-runtime systems.  None
are hex-specific or rendering-coupled; they're the substrate any game
needs to tick state per-frame regardless of what data shape it uses.

### `physics_2body` — rigid-body collision + integration

**Owns:** pairwise collision math + integrator.  Sphere ↔ sphere,
AABB ↔ AABB, sphere ↔ AABB.  Velocity-Verlet or symplectic Euler
integrator.  NO N-body stacking (the "2body" in the name); for chains
/ articulated bodies, a future `physics_chain` or external library
applies.

**Lavition pairing:** none direct.  Game code (or a game-internal
hex-physics glue layer) wires hex_walls's collision shapes into
physics_2body queries.

**Game roles:** projectile + entity collision, movement constraints,
ground/wall contact resolution.

**API sketch:**

```loft
pub enum ColShape { Sphere(radius: float), Aabb(half_extents: Vec3) }
pub struct Body { pos: Vec3, vel: Vec3, mass: float, shape: ColShape }

pub fn collide_pair(a: const Body, b: const Body) -> Collision  // null if no contact
pub fn integrate(b: Body, force: Vec3, dt: float) -> Body
pub fn resolve_collision(a: Body, b: Body, c: const Collision) -> (Body, Body)
```

Plan slot: [`lib_plans/75-physics-2body/`](lib_plans/75-physics-2body).

### `particles` — emission + lifetime + sync state

**Owns:** particle emission, per-particle lifetime, transient state
(position, velocity, age, color).  Two flavours per the existing plan
slot — ribbon trails (ring-buffered points for plane smoke, exhaust)
and point bursts (short-lived sprites for explosions, score
confetti).

**Headless usable:** yes — a multiplayer server can simulate
emission/lifetime for sync; clients render with graphics
primitives.

**Lavition pairing:** no editor authoring today — particles are
emitted at runtime by game logic.  A future `lavition_particle_preview`
plugin could let authors tune emitter parameters visually.

**Graphics pairing:** particles expose a "renderable list" each frame;
graphics draws it.  Decoupled so headless usage stays clean.

**API sketch:**

```loft
pub struct ParticleEmitter { /* opaque: type + position + emission rate + lifetime */ }

pub fn emitter_trail(position: Vec3, max_points: integer) -> ParticleEmitter
pub fn emitter_burst(position: Vec3, count: integer, lifetime_s: float) -> ParticleEmitter

pub fn emitter_tick(e: ParticleEmitter, dt: float)
pub fn emitter_emit_point(e: ParticleEmitter, pos: Vec3, vel: Vec3, color: integer)
pub fn emitter_particles(e: const ParticleEmitter) -> vector<RenderablePoint>
```

Plan slot: [`lib_plans/76-particles/`](lib_plans/76-particles).

### `input` — abstract input state + key bindings

**Owns:** mouse position + button state, keyboard down/up, controller
axes/buttons, action bindings (map "Space key" → action "jump"),
debounce / edge detection.

**Coupling pattern:** `graphics` (the platform layer via winit/glutin)
owns the raw OS event loop and exposes raw events via
`graphics::poll_events() -> vector<RawEvent>`.  The `input` library
reads those events each tick and maintains the abstract state.  Game
code queries the abstract state.  No circular dep.

```
winit / glutin (in graphics cdylib)
   │
   ▼  graphics::poll_events() -> vector<RawEvent>
input library (loft-libs-game)
   │  resolves bindings, maintains abstract state
   ▼
game code: input::is_action_pressed("jump")
```

**Lavition pairing:** every editor plugin uses `input` for keyboard
shortcuts + mouse interaction; bindings managed by
`lavition_editor_core`.

**Game roles:** all gameplay input — entity control, UI clicks,
camera control, debug shortcuts.

**API sketch:**

```loft
pub struct Bindings { /* key/button → action_id mapping */ }
pub struct InputState { /* opaque — accessed via fns below */ }

pub fn input_new(b: Bindings) -> InputState
pub fn input_tick(s: InputState, events: const vector<RawEvent>)

// Query API (game code calls these per frame)
pub fn is_action_pressed(s: const InputState, action: text) -> boolean
pub fn is_action_just_pressed(s: const InputState, action: text) -> boolean   // single-tick edge
pub fn get_axis(s: const InputState, axis: text) -> float                       // -1.0..1.0
pub fn mouse_position(s: const InputState) -> Vec2
pub fn mouse_button_down(s: const InputState, button: integer) -> boolean
pub fn rebind(s: InputState, action: text, raw: RawEvent)                       // user remapping
```

`RawEvent` lives in `graphics` (it's the platform-event union).
`input` imports it; the game doesn't have to touch it directly.

### `time` — frame counter + dt + scheduling

**Owns:** frame counter, dt (delta since last frame), wall clock
access, scheduling primitives (timers, repeating tasks, defer-to-
next-frame).

**Migration path:** **shipped 2026-06-01.**  `time 0.1.0` is the
first inhabitant of `loft-libs-game` (W.0c + W.0d) — the package
moved from the monorepo `lib/time/` to the chunk repo unchanged
(preserving the existing API), tagged + released + added to the
registry.  The monorepo's own doc page for it resolved via
`tests/fixtures/libs/time/` (the Phase 6.12 fixture mirror) until
@PLN149 step 9 moved that page into the package as
`time/docs/01-getting-started.loft`; the fixture mirror stays for the
tests that still link against it.

**Lavition pairing:** editor uses time for animation playback + UI
transitions + auto-save scheduling.

**Game roles:** the game loop's tick driver; animation timing; AI
schedules.

**API:** version 0.1.0 preserves the monorepo `lib/time/` API
verbatim — post-0.1 iterations land via the chunk repo's own release
cadence (W.0c+W.0d-style tag + GH release + registry PR).

### `audio_bus` — game-side audio triggers

**Owns:** play_sfx, set_volume_for_category (music/sfx/voice),
spatial audio (sound at world position, listener position),
ducking, category mixing.

**Shipped 2026-08-25 as `audio_bus` 0.1.0** (@PLN146 E3) — the bus tree, the
gain composition and ducking.  Spatial audio is NOT in it: `pan` moves one voice
and a caller supplies the position, because placing a sound needs a listener and
a world, and neither belongs to a mixer.  That is the next slice, not a gap in
this one.

**Distinct from `graphics`'s audio:** graphics has `audio_load` / `audio_play`
(with `looping` / `pan` / `start` since 0.9.0), `audio_play_raw` and the `sfx_*`
helpers — those are the mixer / output device interface (talks to rodio, and to
Web Audio in a page).  `audio_bus` is the game-side abstraction (categorise
sounds, manage groups, duck); it composes graphics's low-level mixer as the
output.

```
game code
   │  audio_bus::play("sfx", "click.wav", at: hex(3,5))
audio_bus  (loft-libs-game)
   │  resolves bindings, applies category volume, spatializes
   ▼  graphics::audio_play_raw(samples, gain, pan)
graphics audio mixer  (loft-libs-graphics)
   │
   ▼  rodio → OS audio device
```

**Future cleanup:** mixing/output could itself migrate out of graphics
into a `loft-libs-audio` chunk; not blocking — flag in graphics's
README.

**API sketch (preliminary):**

```loft
pub struct AudioBus { /* opaque */ }

pub fn audio_new() -> AudioBus
pub fn set_category_volume(b: AudioBus, category: text, gain: float)
pub fn play(b: AudioBus, category: text, asset: text)
pub fn play_at(b: AudioBus, category: text, asset: text, world_pos: Vec3)
pub fn set_listener_position(b: AudioBus, pos: Vec3)
pub fn duck(b: AudioBus, category: text, gain: float, duration_s: float)
```

## Next library work — execution order

Three new chunks need to bootstrap: `loft-libs-assets`,
`loft-libs-game`, `loft-libs-world`.  Each chunk-bootstrap is
~30 minutes of work: GitHub repo + CI workflow from the
`loft-libs-net` template + registry-side validator setup.

Each library sub-step is a self-contained Stage A → Stage B mini-cycle
(same pattern Phase 5b just ran: tarball + chunk-repo + registry PR +
consumer migration + monorepo cleanup).

| # | Step | Effort | Depends on |
|---|---|---|---|
| ~~W.0~~ | ~~Promote `glb` out of `graphics` submodule into `lib/glb/` (monorepo).~~  **Dropped 2026-05-31** — superseded by direct-to-chunk path (W.0b writes the package directly into `loft-libs-assets`, no monorepo round-trip).  Monorepo-first pattern is legacy from the era when `lib/graphics/` lived there; with the chunk already extracted, the cheaper path is to harvest from the registry-shipped graphics 0.1.0 into the new chunk. | — | superseded |
| **W.0a** | Bootstrap `loft-libs-assets` chunk (GitHub repo + CI from net template). | S | — |
| **W.0b** | Write `mesh3d` + `glb` packages directly into `loft-libs-assets`; tag + release + registry PR. | M | W.0a |
| ~~W.0c~~ | ~~Bootstrap `loft-libs-game` chunk.~~  **Shipped 2026-06-01** — repo created at [`loft-lang/loft-libs-game`](https://github.com/loft-lang/loft-libs-game), CI workflow + LICENSE + README + `.gitignore` cloned from the loft-libs-assets template (matrix scoped to `[time]`). | S | — |
| ~~W.0d~~ | ~~Extract `lib/time` → `loft-libs-game/time 0.1.0`~~  **Shipped 2026-06-01** — Stage A: package copied into chunk, [time-v0.1.0 release](https://github.com/loft-lang/loft-libs-game/releases/tag/time-v0.1.0) (5937 bytes, sha256 `fb078def…`), registry [PR #11](https://github.com/loft-lang/registry/pull/11) (validator green).  Stage B: `lib/time/` removed from monorepo, `tests/fixtures/libs/time/` populated via `scripts/sync-fixtures.sh` (PINNED_REFS row), `tests/docs/32-time.loft` `@ARGS` repointed at the fixtures dir. | M | W.0c |
| ~~W.0e~~ | ~~Bootstrap `loft-libs-world` chunk.~~  **Shipped 2026-06-01** — repo created at [`loft-lang/loft-libs-world`](https://github.com/loft-lang/loft-libs-world), CI workflow + LICENSE + README + `.gitignore` cloned from the loft-libs-assets template (matrix scoped to `[hex_world]`). | S | — |
| ~~W.1~~ | ~~Rename `lib/world` → `lib/hex_world` (monorepo, internal churn) + update consumer loft.tomls + `src/wasm.rs` `include_str!` paths.~~  **Shipped 2026-06-01** — dir + entry-file renamed, `name` in loft.toml bumped to `hex_world`, all consumer `use world;` / `world::` refs updated (tests/integration/multiplayer, tests/fixtures/libs/game_protocol/examples, tests/multilib/p379_lib_namespace, lib/hex_world/tests/02-persist).  No `src/wasm.rs` `include_str!` paths were needed — `lib/world` was never WASM-bridged. | S | — |
| ~~W.2~~ | ~~Extract `hex_world` Stage A → Stage B → `loft-libs-world/hex_world 0.1.0`.~~  **Shipped 2026-06-01** — Stage A: package copied into chunk, [hex_world-v0.1.0 release](https://github.com/loft-lang/loft-libs-world/releases/tag/hex_world-v0.1.0) (18009 bytes, sha256 `e537960b…`), registry [PR #10](https://github.com/loft-lang/registry/pull/10) (validator green).  Stage B: `lib/hex_world/` removed from monorepo, `tests/fixtures/libs/hex_world/` populated via `scripts/sync-fixtures.sh` (PINNED_REFS row), `tests/issues.rs::p379_two_libs_same_struct_name` repointed at the fixtures dir, fixture broken-link false-positives suppressed via `link_source_is_fixture` filter in `tools/indexer/src/scan.loft`. | M | W.0e, W.1 |
| W.3 | Split `hex_walls` out of `lib/hex_world/src/wall.loft` into `lib/hex_walls/`.  Defines the API boundary. | M | W.2 |
| W.4 | Extract `hex_walls` Stage A → Stage B. | M | W.3 + curve-detection design |
| W.5 | Design + implement `hex_terrain` as `lib/hex_terrain/`. | MH | W.2 (uses hex_world addressing) |
| W.6 | Extract `hex_terrain` Stage A → Stage B. | M | W.5 |
| W.7 | Design + implement `hex_items` as `lib/hex_items/`. | MH | W.2, W.3 (12-dir placement, layer model shared with walls) |
| W.8 | Extract `hex_items` Stage A → Stage B. | M | W.7 |
| W.9 | Design + implement `lib/physics_2body/` (from existing plan slot). | MH | plan-26 design |
| W.10 | Extract `physics_2body` Stage A → Stage B → `loft-libs-game`. | M | W.9 |
| W.11 | Design + implement `lib/particles/` (from existing plan slot). | M | plan-27 design |
| W.12 | Extract `particles` Stage A → Stage B → `loft-libs-game`. | M | W.11 |
| W.13 | Design + implement `lib/input/`.  Wraps `graphics`'s polled key/mouse primitives → abstract action state, bindings, edge detection.  **Partial 2026-06-01** — design + API + 5-test suite drafted (`lib/input/{src/input.loft, tests/01-basics.loft}`, ~250 LOC), `loft.toml` declares the `graphics >=0.1` dep, but the lib is **PARKED on [@P391](PROBLEMS.md#open-issues--quick-reference)** (cross-package struct constructor lands in CONST_STORE → `Write to read-only store` panic on the first field write through `&InputState`).  Tests are gated by `LIB_PKGS_SKIP` in `tests/wrap.rs` + `tests/native.rs::LIB_PKGS_NATIVE_SKIP` + `tests/html_wasm.rs::LIB_PKGS_NODE_SKIP`.  Un-park when @P391 ships. | MH | @P391 blocker |
| W.14 | Extract `input` Stage A → Stage B → `loft-libs-game`. | M | W.13 unpark (waits on @P391) |
| W.15 | Audit graphics's existing audio surface (`audio_play_raw` + `sfx_*`) — clarify the boundary between graphics's mixer and game's `audio_bus`. | S | **Done** (@PLN146 E2/E3): graphics owns the OUTPUT (load, play, volume, pan, seek, stop, and the raw/`sfx_*` synthesis), `audio_bus` owns the GROUPING (a bus tree that multiplies, and ducking).  The line is that graphics moves one sink and knows nothing about why. |
| W.16 | Design + implement `lib/audio_bus/` + extract Stage A → Stage B → `loft-libs-game`. | MH | **Shipped** — `audio_bus` 0.1.0 in loft-libs-game, built there directly rather than through a `lib/` stage.  Spatial audio (a listener, a world position) is deliberately left out: it needs a world, and a mixer does not have one. |
| W.17 | Bootstrap `loft-libs-meta` chunk + ship `lavition_stack 0.1.0` meta-package. | S | enough chunks live to make the meta-package meaningful (typically after W.2 + W.0b ship) |

**Total:** ~17 sub-phases.  Realistic shipping cadence: 1-2 per
session, so the whole 6-chunk topology lands across 10-17 sessions.

**Branch model:** continue the established pattern — one cross-theme
branch (`doc-updates` or successor) accumulates the work; a PR opens
per chunk milestone (`loft-libs-assets` bootstrap + glb extraction →
one PR; `loft-libs-world` foundation = hex_world + hex_walls → next
PR; etc.).  Avoid one-PR-per-library — too much review overhead.

**Shipped so far (2026-05-31):**
- ✓ W.0a — `loft-libs-assets` chunk bootstrapped.
- ✓ W.0b — `mesh3d 0.1.0` + `glb 0.1.0` shipped directly into the
  chunk (registry PR [#8](https://github.com/loft-lang/registry/pull/8)
  merged at 21:02 UTC; both packages live + installable).

**Open sequencing notes:**
- W.0c / W.0d (game chunk + time migration) — independent of the
  world chunk work, can ship next.
- W.1 / W.2 (hex_world rename + extract) is the foundation for W.3-W.8.
- W.9 / W.10 (physics_2body) depends ON nothing new but lavition's
  glue layer would want it before authoring real plugins.  Ship after
  the data layer is stable.
- **Direct-to-chunk pattern** (used in W.0b) preferred over
  monorepo-first for libraries not already in the monorepo.
  Monorepo-first stays the right path for W.1 (hex_world rename) +
  W.3 (hex_walls split) since those libs already live at `lib/world/`.

## Discoverability — the practical reason for brand visibility

The brand isn't visible in metadata for marketing reasons.  It's visible
because **descriptive symbol names are not searchable on their own.**  A
user who encounters `use hex_world;` in a script and googles "hex_world"
gets Roblox games, Civilization map packs, and 2D puzzle clones — not
our library.  This is the same problem Python's `requests` has:
"requests documentation" returns garbage; the canonical query is
"python requests".

We accept the tradeoff (script readability over search ergonomics) but
mitigate the cost on multiple fronts:

### 1. Names unique enough that search has a fighting chance

`hex_world` > `world` (less collision).  `hex_walls` > `walls`.
`terrain` is borderline; `particles` is OK; `physics_2body` is unique.
The bad offenders in the current library set are short generic words —
`shapes`, `graphics`, `imaging`, `web`, `server`, `math`, `mesh`,
`scene`.  For these the SEO collision is severe; consider a soft
rebrand-via-prefix at the next major-version boundary (see
[§ Long-term rename candidates](#long-term-rename-candidates) below).

### 2. The package registry is the authoritative discovery layer

`loft-lang/registry` indexes every library with description + homepage
+ category.  Same role crates.io plays for Rust — a user who doesn't
remember a library's name browses the registry by category.

- `loft-lang.org/libraries/` (catalog HTML, planned) — human-facing
  index.
- `doc/claude/LIBRARIES.md` — the single in-repo catalogue of
  every installable registry library (the separate `doc/library-catalog.md` +
  `gen_library_catalog.py` were retired 2026-07-02 as a stale duplicate).
- `loft search <term>` (planned, on the registry MVP roadmap) —
  CLI-side fuzzy search.

A canonical query like "loft library catalog" or "loft package
registry" should rank high; the catalog itself then routes users to the
specific library's docs.

### 3. In-package branding loud (once they find it)

Every published library's `README.md` opens with the ecosystem context:

> # hex_world — chunked hex grid for the loft language
>
> Part of the [loft](https://github.com/loft-lang/loft) ecosystem;
> works standalone and with the [lavition](https://github.com/lavition)
> editor.

Once a user lands on the package (via any path — search, registry,
catalog, blog post), the brand is unmistakable.  No requirement that
the symbol carry the brand.

### 4. IDE / editor tooltips supply context the symbol omits

When lavition's editor (or any LSP-aware loft IDE plugin) hovers over
`use hex_world;`, the tooltip shows:

```
hex_world v0.1.0
chunked hex grid + addressing + save/load
loft-lang/loft-libs-world
docs:  https://loft-lang.org/libraries/hex_world
```

The symbol stays bare in source; the IDE provides the brand context.
Same pattern IntelliJ uses for bare Java imports (hover on `List` →
"java.util.List from rt.jar").

### 5. Lavition's docs own the "lavition + generic-term" search space

The discoverability target isn't "googling `hex_world` finds us" (it
won't, against Roblox + Civ + a dozen other things).  The target is
**"googling `lavition world` or `lavition wall` lands on our docs
directly."**  That query is achievable with normal SEO because
"lavition" is unique on Google — combine it with any common word and
the result space is small enough we can own it.

Concrete structure for the lavition docs site:

```
lavition.io/docs/                     overview
lavition.io/docs/world                ← hex_world + lavition editor integration
lavition.io/docs/wall                 ← hex_walls + hex_editor plugin
lavition.io/docs/terrain              ← terrain + terrain_paint plugin
lavition.io/docs/items                ← items + item_placer plugin
lavition.io/docs/stencils             ← stencil format + library + stamping
lavition.io/docs/layers               ← vertical-layer model
```

Each page covers the editor narrative + the underlying loft library
(`hex_world`, `hex_walls`, etc.) and links out to the library's own
docs (`loft-lang.org/libraries/hex_world/`).  The brand is in the URL +
page title + meta description; the library symbol stays bare in code.

So a user can find the right documentation via three paths:

- **"lavition world" / "lavition wall"** → directly hits the
  brand-disambiguated lavition docs page.  Brand goes in the search,
  not the symbol.
- **"loft library catalog"** → hits the registry / catalog page,
  browses by category to find `hex_world`.
- **Hover in IDE** → tooltip resolves the bare symbol to its docs URL.

### 6. Cross-linking between loft + lavition + game project sites

- `lavition`'s homepage: "Built on the [loft](https://github.com/loft-lang/loft) language."
- `loft`'s homepage: "Primary editor: [lavition](https://github.com/lavition).  Also usable from CLI, web, or any other host."
- Each game's README points at both.

So a user landing on any one site can navigate to the others within
one click.  Brand context discoverable through the link graph, not
required in source.

### Long-term rename candidates

For existing generic-name libraries that have severe SEO collision,
consider soft rebrand at the next major-version boundary.  Not urgent;
not all at once.

| Current | Better candidates | Notes |
|---|---|---|
| `graphics` | `loft_graphics`, `lgfx` | "graphics" alone is uncoupled from any specific ecosystem |
| `math` | `loft_math`, `linalg`, `vecmath` | rename to a less collision-prone domain term |
| `mesh` | `loft_mesh`, `geomesh` | overlaps with `mesh.js`, `Mesh` (many engines), Apple's Mesh framework |
| `scene` | `loft_scene`, `scenegraph` | overlaps with countless "scene" libraries |
| `shapes` | `loft_shapes`, `geo_shapes` | borderline; manageable with good SEO |
| `web` | `loft_web` | extremely generic; current size of the lib is small enough that a rename is cheap |
| `server` | `loft_server` | same |

These are not action items today — current 0.x versions stay as-is.
But at 1.0 it's worth a single rename PR per affected library, with a
deprecation period publishing both names for one minor release.

### What this section is NOT

This isn't a recommendation to brand-prefix EVERY symbol.  Unique-enough
names (`hex_world`, `hex_walls`, `terrain`, `particles`, `physics_2body`,
`gridmesh`, `imaging`, `arguments`, `crypto`, `random`) stay bare —
they're already searchable enough that the mitigations above carry the
discoverability load.  The prefix is only for short generic words
where SEO is genuinely broken without it.

## Migration / rename pending

- `lib/world` (currently in monorepo, planned for `loft-libs-world`
  extraction): rename to `hex_world` to disambiguate from voxel / tile /
  BSP "world" expectations.
- `walls` data (currently folded into `lib/world/`): split out as
  `hex_walls` matching the `hex_world` pairing.
- `lib_plans/73-universal-editor/` plan: keep the design content,
  retitle to reference lavition as the engine the plan delivers.
- 6 residual `lav` / `Lavition` references in the loft tree (pre-rename
  leftovers): keep — they reference the original codename for the
  language, which is now reused as the engine brand.  No harm, no
  cleanup needed.

## Anti-renames

- **Don't rename `loft` to anything else.**  It's a real shipped artifact
  with published packages, a registry, and consumer commitments.  loft
  stays loft; lavition is built *on* loft.
- **Don't add brand prefixes to data libraries.**  `use hex_world;` is
  cleaner and more portable than `use lavition_hex_world;` (which would
  falsely imply engine coupling).

## See also

- [`lavition/lavition`](https://github.com/lavition/lavition) — engine
  meta-repo with design vision + roadmap pointer.
- [`lib_plans/73-universal-editor/`](lib_plans/73-universal-editor)
  — the original design plan for the universal editor (predates the
  lavition brand; design content still authoritative).
- [`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction)
  — the multi-phase plan for getting libraries into `loft-lang/loft-libs-*`
  (the substrate lavition consumes).
- [`PACKAGES.md`](PACKAGES.md) + [`PKG_REGISTRY.md`](PKG_REGISTRY.md)
  — how loft libraries are packaged + distributed.
