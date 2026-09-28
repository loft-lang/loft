# lib-plan 19 — `gridmesh`: chunk-local, bounded-extent grid→mesh primitives

**Status:** FINISHED 2026-06-14 — the `gridmesh` library shipped
(`gridmesh 0.1.1`, in `loft-libs-world`, with an in-repo test fixture). The one
remaining phase (C — the moros consumer) is now **moros-side** and out of this
library plan's scope (see the close note at the end of the Status). Phase A + B
done; crystal **C1 done** (chunk-driven
`SegMesh` build, SET-equivalent to the legacy build cross-mode) and **G2
done** (render-group / tile layer, tunable `group_dim`; tests
`lib/gridmesh/tests/rendergroup.loft`).  Full-build perf characterised:
**O(N) flat (8 µs/seg at n=20 and n=100)** + **SegMesh ~2.1× smaller** than
the legacy CrystalMesh (`crystal_stress` bench).  **C3 (incremental two-level
reuse) is RE-ENABLED 2026-05-22** — both blockers are fixed: caching nested
struct-of-vectors no longer crashes ([@P311](../../PROBLEMS.md) +
[@P313](../../PROBLEMS.md)/[@P314](../../PROBLEMS.md)/[@P315](../../PROBLEMS.md)/[@P317](../../PROBLEMS.md)/[@P318](../../PROBLEMS.md)),
and the `c: &CrystalIncr` call-arg borrow conflict ([@P312](../../PROBLEMS.md),
native E0503) is fixed in codegen (the pre-eval pass now hoists a `c.field`
read passed alongside `&mut c`).  `CrystalIncr` two-level reuse is back on;
incremental == full build (regression `tests/scripts/133-crystal-incr.loft`,
both backends).

**Close note (2026-06-14):** the gridmesh *library* is complete and published, so
@PLN7 is closed.  **Phase C — the moros consumer integration** (build_chunk_mesh
over the moros world, LOD, per-chunk VBOs) is **not** library work: `moros_*`
moved to the moros repo (#379), so applying `gridmesh` to the moros world is the
moros project's call.  The Phase-C design below is retained here as the reference
the moros project picks up from.

**Full implementation design:** [DESIGN.md](DESIGN.md) — concrete types,
the rule contract, the pipeline API, the crystal-consumer mapping, ordered
build steps (gridmesh B1-B3 → crystal C1-C3), and verification.

## Why

The crystal routine (`lib/audience_crystal`) is the prototype for a CLASS
of world-building algorithms — wall placement, edge rounding, surface
generation — that must run as **local routines over a limited set of
world chunks** and produce **meshes that don't extend much outside their
chunk**, so chunks build / update / cull independently. Two consumers
need the identical pipeline but share no code:

- `lib/audience_crystal` — generative-art prototype (offset/pointy hex);
  already O(N) + memory-bounded (P5/P6).
- `lib/moros_render` — the real world: `build_hex_meshes` meshes the
  WHOLE world monolithically into per-material buffers, with no per-chunk
  meshes, no halo (seams ignored), flat per-hex geometry, no culling, no
  dirty tracking.

`gridmesh` houses the shared pipeline as a **toolkit of primitives, not a
framework**: spatial-index neighbour queries → per-cell pattern
classification → bounded geometry emission keyed by owning cell →
dirty-region / per-chunk rebuild. Each consumer supplies only its
per-cell RULE. Mostly extraction + generalization of existing code, not
greenfield.

## The real goal — CACHE LOCALITY (data availability, not compute)

The deep reason for chunking + bounded extent + narrow types is **memory
locality, not parallelism or tidiness**.  A chunk's working set — its
cell payload arrays, the spatial index, the ≤k halo apron, and the mesh
accumulator it writes — should **fit in the CPU's primary caches (L1/L2)**
so processing a chunk is bound by ALU throughput, not by waiting on RAM.
For grid→mesh work the bottleneck is almost always **data availability**
(cache/DRAM latency), not the arithmetic.  Every structural choice serves
this:

- **Bounded chunk working set** — size `chunk_shift` so one chunk's cells
  + halo + index + mesh fit a target cache level (rule of thumb: keep the
  hot per-chunk bytes well under L2; smaller is better for L1).  This is
  the primary tuning knob, ahead of `par` worker count.
- **Struct-of-arrays + narrow types (M1)** — `vector<single>` /
  `vector<u8>` / `vector<u16>` pack far more cells per 64-byte cache line
  than i64/f64; SoA means a pass that touches one attribute streams it
  contiguously instead of striding over fat structs.  The ~60 % memory
  cut from M1 is really a ~2.5× increase in cells-per-cache-line.
- **Contiguous over pointer-chasing** — prefer dense indexable arrays
  (cell payload, the par work-list) over scattered records.  The spatial
  index is a `hash` (open-addressed: a contiguous bucket array + records
  in one store), kept COMPACT so a chunk's index stays cache-resident —
  hence the keyed-collection compactness work (plan-44: `coll[key]=value`
  upsert dedups in place and reclaims, [C68](../../DESIGN_DECISIONS.md#c68--keyed-collections-dedup-on-insert--and-collkeyvalue);
  duplicate keys would bloat the index out of cache).
- **Halo bounded to ≤k rings** — the apron a rule reads is the only
  cross-chunk data pulled in; keeping it minimal keeps the working set
  (and `clone_for_worker`'s per-worker copy) small.

So: chunk so the data fits cache → narrow + SoA so more fits per line →
keep the index compact → process contiguously.  Parallelism (below) is a
*second-order* win layered on top of locality, not the main event.

## moros world architecture (decided 2026-05-21)

The world consumer (the Phase-C payoff) is now pinned:

- **Chunks are fixed 32×32, always** — no other chunk size will exist.
  So a chunk's cells are a **dense 1024-hex grid** and cell access is pure
  arithmetic (`hex_idx_32`, `y*32 + x`) into flat **SoA** arrays — *no
  per-chunk hash*.  With narrow types (~4 B/cell: `u16` height + `u8`
  material + `u8` wall/flags) the cell core is ~4 KB → L1-resident.  This
  is the cache unit.
- **The world index is a SPARSE hash `hash<Chunk[cx,cy,cz]>`** — sparse not
  because chunks are sparse internally (they're dense) but because the
  world is **sparse in the vertical axis**: a few columns have many stacked
  layers, most have a single layer with lots of empty space around them.  A
  dense 3D array would allocate max-layer depth everywhere → huge waste; the
  hash pays only per existing chunk.  This is loft's **indirection hash** —
  the *chunk holds its own `(cx,cy,cz)`*, the hash slot holds a pointer to
  the chunk record, lookup compares the chunk's stored coord.  No new
  structure: the existing `hash<Chunk[cx,cy,cz]>` is the fit.  *(It replaces
  `moros_map`'s current linear scan over `m_chunks`.)*  Inserting a chunk at
  an already-occupied coord **replaces** it (dedup) — now correct via plan-44
  (@P305/@P306).
- **A `Chunk` holds**: its `(cx,cy,cz)` (the hash key) + the dense SoA cell
  grid (source of truth) + a near **mesh** (full geometry, current) + later
  a far **height profile** (below).
- **LOD — near mesh, far height-map profile.**  Near chunks render their
  full mesh.  For long-distance viewing a chunk carries a compact
  **height map + deformation** that captures the building *silhouette /
  skyline profile* with minimal detail (correct profile, not much else).
  That height map is **stored in a GPU texture and rendered by a shader** —
  so CPU-side it's just a small 2D array to upload (height → texture, the
  same shape as SoA → VBO), and the shader does the profile/silhouette work.
  Keep the profile tiny: the far pass streams *many* chunks, so per-chunk
  profile bytes must stay cache-resident.

Net: **`hash<Chunk[cx,cy,cz]>` (sparse, indirection) → each `Chunk` = dense
32×32 SoA grid + near mesh + (later) a far height-texture profile.**

## Reused primitives (extraction sources)
- `lib/moros_map`: `chunk_idx_32`/`hex_idx_32` (global↔chunk-local),
  `hex_distance`, `map_get_hex` (free cross-chunk halo reads), 32×32
  `Chunk`/`Hex`.
- `lib/audience_crystal`: `enc_coord`, `axial_dq/dr`, `step_x/step_y`,
  `idx_at`, `nbr_count_idx`, `CellRef`, the `cidx` spatial-index pattern,
  the `CrystalMesh` SoA layout + per-segment `cell_ix`.
- `lib/graphics`: `loft_gl_upload_mesh`/`gl_upload_vertices` (f32 VBO),
  `Mesh`/`Vertex`.
- Substrate: keyed collections (efficient via P5/P6), amortised vectors,
  narrow element types (M1: `single`/`u8`/`u16` for compact meshes).

## Phases

| Phase | Scope | Status |
|---|---|---|
| **A** | Extract coord + spatial-index primitives (`CellRef`, `enc_coord`, `build_index`, `idx_at`, `step_x/step_y`, `axial_dq/dr`, `nbr_count_idx`) into `lib/gridmesh`; re-point `audience_crystal` + the projector onto them. `build_index` replaces the inline @P300 workaround (now fixed). | ✅ **Done** — crystal output + memory byte-identical on both backends (block/100 = 2379 segs; block-500 = 12 738 segs / 2.83 MB / 9 free-blocks). |
| **B** | Chunking + halo + bounded extent + dirty rebuild: `build_chunk(layout, field, cx,cz,cy, rule) -> Mesh` reading a ≤k-ring halo, emitting geometry owned by in-chunk cells bounded to the chunk; per-chunk/dirty-cell incremental rebuild (O(dirty) compute + O(N) copy). A `HexLayout` adapter (axial flat-top for moros, offset pointy-top for crystal). Carry M1 narrow types into the mesh accumulator. | **B1 + B2 done** — B1: `SegMesh` accumulator (M1 narrow). B2: `ChunkField` + `ChunkKey` + chunk partition (`chunk_of`/`chunk_div`) + wired-in `dirty: hash<ChunkKey[ck]>` (keyed set) + `field_new`/`field_add_cell`/`field_mark_dirty`/`clear_dirty`/`chunk_is_dirty`/`dirty_count`, all cross-mode (guard: `lib/gridmesh/tests/chunkfield.loft`). Surfaced + **fixed** @P308 (struct-literal keyed-HASH field init / whole-field assign now deep-copy via `OpReplaceKeyed`; `field_new` uses the clean `cidx: build_index(...)`); sorted/index struct-field deep-copy deferred as @P309. **B3 done** — `ChunkInput{key, cell_ixs, halo_ixs}` (no `cidx` — passed to the rule separately for cache locality) + `all_inputs`/`collect_dirty_inputs`: single-pass O(N) cell→chunk partition (in-place keyed-bucket accumulate) + ≤halo_k 6-axis cross-border halo gather (deduped); dirty-only via the wired-in set (guard: `lib/gridmesh/tests/chunkinput.loft`, cross-mode). **B fully done.** **G1 refinement (2026-05-21):** the per-call O(N) `partition_cells` re-scan is RETIRED — `ChunkField` now carries wired-in per-chunk `buckets: hash<ChunkBucket[ck]>` built once in `field_new` and appended in `field_add_cell`, so `all_inputs`/`collect_dirty_inputs` read the buckets directly and dirty extraction is **O(dirty), not O(N)** (this is the persistent-index half of @PLN6 § Performance I1). First dedicated `SegMesh` unit tests added (`lib/gridmesh/tests/segmesh.loft`: empty / emit narrowing / 9-column parallel-array alignment / u8 boundary). **C1 done** — see crystal C1 below. |
| **C** | moros consumer (see "moros world architecture" above): fixed **32×32 dense** chunks in a **sparse `hash<Chunk[cx,cy,cz]>`** world index (replaces `moros_map`'s linear `m_chunks` scan); `build_chunk_mesh(map, cx,cy,cz)` over `gridmesh` (replacing the global `build_hex_meshes` path incrementally) with a moros RULE — surface + wall placement + edge rounding from the `Hex` neighbour pattern (the crystal technique applied to real world geometry); per-chunk VBOs + dirty IDs + frustum culling; **LOD**: near = full mesh, far = compact height-map profile uploaded to a texture + rendered by a shader (building silhouette). Own sub-plan; the payoff. The per-hex height field it meshes is produced upstream by [lib-plan 20 — terrain height-map](../71-terrain-heightmap/README.md) (slope-based generation; shared with the **dryopea** tower-defence game). | **Moros-side** — `moros_*` moved to the moros repo (#379), so this consumer integration is the moros project's call; this design is the reference it picks up from. Not gridmesh-library scope. |

## Parallelism — baked in from Phase B (chunks are embarrassingly parallel)

Chunks are independent (bounded meshes, read-only halo), so building many
chunk meshes is a parallel map.  loft's `par(...)` for-loop clause
supports exactly this and is designed in from the start:

```loft
for ci in chunk_inputs par(cm = build_chunk_mesh(ci, rule), N) {
  meshes += [cm];   // per-chunk Mesh, collected in order
}
```
- **Struct returns work** — `par(...)` deep-copies a worker-built struct
  (the chunk `Mesh`) inline into the result vector (THREADING.md
  "Supported return types … `struct`/reference"); the rule may be passed
  as a forwarded **context arg**.
- **Workers get locked read-only store copies** (`clone_for_worker`), so
  reading the shared field/halo in parallel is safe; the only rule is
  *workers may not write shared state* — chunk-local generation never
  does (each builds its OWN bounded mesh).

**Design invariants that make parallelism cheap AND keep meshes bounded
(the same discipline):**
1. `build_chunk_mesh(input, rule) -> Mesh` is a PURE function — no shared
   mutation — so wrapping the chunk loop in `par(...)` is trivial.
2. The worker INPUT is a COMPACT, self-contained `ChunkInput` = the
   chunk's cells + a ≤k-ring halo apron, extracted BEFORE the par loop —
   NOT the whole map.  This keeps `clone_for_worker`'s per-worker store
   copy small (it clones in-use stores per worker), and it structurally
   guarantees a chunk's mesh can only depend on its chunk + halo → bounded
   extent by construction.  Locality and parallelism reinforce each other.
3. Dirty-region rebuild = a `par(...)` map over the dirty chunks' inputs.

### Dirty tracking — a wired-in dirty INDEX on the chunk container

The chunk-holding structure carries a **first-class, wired-in index of
dirty chunks** — a keyed SET of dirty chunk coords
(`dirty: hash<ChunkCoord[ck]>`), maintained as part of the structure, NOT
a per-chunk bool that has to be scanned for, and NOT a plain append
vector that would accumulate duplicates.  This gives O(dirty) enumeration
directly, with dedup by construction:

> **Primitives now exist (plan-44, 2026-05-21).**  The keyed-set
> operations this design assumed are all implemented + cross-mode:
> idempotent set-insert via `dirty[ck] = ChunkKey{…}` (@P305 `OpSetKeyed`)
> or `dirty += [ChunkKey{…}]` (@P306 — both dedup by key, latest wins);
> membership via `if dirty[ck]`; and the wired-in clear via `f.dirty = []`
> (@P307 `OpClearKeyed`), with bounded memory under churn.  So
> `dirty: hash<ChunkKey[ck]>` as a struct field is directly buildable now.

- **Marker:** marking a chunk dirty is an idempotent **set-insert** into
  the wired-in dirty index — many edits to the same chunk coalesce to one
  entry.  (A per-chunk `built_gen` vs the field's edit generation can
  still validate "is this chunk's mesh current?", but the dirty index is
  the canonical "what to rebuild" — no scan of all chunks.)
- **Edit → propagation:** an edit at cell `(q,r)` inserts its containing
  chunk into the dirty index AND, when `(q,r)` is within the halo radius
  of a chunk border, the adjacent chunk(s) too (their mesh reads `(q,r)`
  via the halo).  This border coupling is the only cross-chunk dependency.
- **Rebuild:** iterate the dirty index DIRECTLY (O(dirty)) to build the
  `par(...)` work-list (a transient `vector<ChunkInput>` — extract cells +
  halo per dirty chunk), run the parallel map, replace those chunks'
  meshes/VBOs, then CLEAR the dirty index.

This belongs in `gridmesh`'s chunk-field abstraction so every consumer
(moros, crystal) gets it; moros's `Map` (which has no dirty tracking
today) gains it via the field.  The only vector is the short-lived par
work-list, collected by iterating the dirty index — so the
`par`-over-any-iterator extension is still not a prerequisite (vector-`par`
handles the work-list today).  Once par-over-keyed lands (THREADING.md
materialise path) you could `par` the dirty index directly and skip the
collect — a convenience, not a requirement.

Caveats: `par(...)` input must be a `vector<T>` (chunk inputs are a vector
— fine); tune `N` to cores (chunks ≫ cores → good balance); the @P229
worker-stack-snapshot flake is Linux-clean (Windows half still open).

## Phase A notes
- New: `lib/gridmesh/{loft.toml,src/gridmesh.loft}`.
- `audience_crystal/loft.toml` depends on `gridmesh`; `crystal.loft`
  `use gridmesh::*`, deleted its local copies, and builds the index via
  `gridmesh::build_index(snap.xs, snap.ys)`.
- `tools/audience-demo/projector.loft` `use gridmesh;` and builds `pidx`
  via `gridmesh::build_index` (was `audience_crystal::CellRef`/`enc_coord`
  inline).
- Verified: `crystal_stress` segment counts unchanged interp + native;
  projector compiles native; `store_memory` profile unchanged.

## Verification
```bash
./target/release/loft --interpret --lib lib tools/audience-demo/crystal_stress.loft   # segs unchanged
./target/release/loft --native    --lib lib tools/audience-demo/crystal_stress.loft   # same, both backends
cargo test --release --test wrap --test native --test issues --test leak --test doc_hygiene
```
