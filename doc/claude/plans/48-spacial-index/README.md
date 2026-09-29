# @PLN48 — `spacial<T[x,y]>` / `spacial<T[x,y,z]>`: a Morton/Z-order radix spatial index

**Status:** S1–S4 done — the plan is **complete**. `spacial<T[x,y]>` /
`spacial<T[x,y,z]>` is a working keyed collection on both backends (interpreter +
`--native`): construct, append, `for`-iterate (natural Morton/Z-order), `len()`,
and range-slice proximity queries (`xs[(x,y)..]`, `xs[(x,y)..:n]`,
`xs[(x1,y1)..(x2,y2)]`), 1–3 coordinate axes. The old "planned for 1.1+"
diagnostic is gone. S4 validated it in crawler (`near_mobs_test.loft`): the
spacial near-set matches a brute-force scan on every query, both backends, with
an 18× candidate reduction and an 8× (interpreted) / 3.7× (native) wall-clock
win. See [DATABASE_INDEXES.md § Spatial Index](../../DATABASE_INDEXES.md#spatial-index-srcradix_treers)
for the shipped operation reference.

## Why

Games need **proximity queries** — "find the near mobs from any mob's
perspective" (aggro/threat, interest management, flocking, broad-phase
collision) — over a chunked world.  Two dimensionalities, chosen by the
number of coordinate key fields:

- **`spacial<Mob[x,y]>`** — flat / top-down worlds (the common case).
- **`spacial<Mob[x,y,z]>`** — worlds with a *considerable vertical
  component* (towers, caves, flight).

The same Morton machinery serves both; 2 or 3 interleaved axes.

**loft already has the scaffolding for exactly this, both unfinished:**
- **`spacial<T[…]>`** (`Parts::Spacial(content, coord_fields)`,
  `src/database/types.rs::spacial`) — the type registers and takes N
  coordinate key fields, but every operation
  (`insert_record`/`find`/`remove`/`copy_claims`) is
  `panic!("Not implemented")`, and the parser gates it with the 1.1+
  diagnostic.
- **`src/radix_tree.rs`** — a store-backed binary PATRICIA tree whose key oracle
  consumes the key **one bit at a time**, so an interleaved Morton key is never
  materialised.  **Rewritten and unit-tested as deliverable R** —
  see [RADIX_TREE.md](RADIX_TREE.md).

An earlier revision of this plan claimed *"this is completion, not greenfield"*
for the radix tree.  That was **false**, and probing it is what produced
deliverable R: the sketch's `set_bits` silently stored `0` (its arguments were
swapped against `set_byte(rec, fld, min, val)`), and inserting a **second**
record segfaulted.  `RadixIter::next` returned `None` unconditionally, so the
bidirectional walk — the entire reason this plan wants the tree — did not exist.
The *interface intent* was sound and is kept; the representation was replaced.

## Design

- **Granularity** — quantize each coordinate to an integer at a chosen
  world resolution (cell size), configured per `spacial` type (sensible
  default, e.g. 1 world-unit).  Coarser cells pack more entities per code
  (bucket within a code); finer cells widen the key range.
- **Morton / Z-order key** — bit-interleave the N quantized axes into one
  key.  2D: interleave x,y (≤32 bits each → ≤64-bit).  3D: interleave x,y,z
  (≤21 bits each → ≤63-bit, fits `MAX_DEPTH 64`).  The radix tree's
  bit-key closure yields the interleaved bits on demand — no full key
  materialisation.
- **Radix tree storage** — entities keyed by Morton code.  Nearby codes
  share long prefixes (Z-order locality) → spatially close entities are
  tree-adjacent, compact, and **cache-resident** (the same locality lever as
  gridmesh).  Multiple entities per cell → a small bucket at a code.
- **Proximity iterator (the "loop left and right" core)** — from a query
  point's Morton code, walk **predecessor + successor simultaneously**,
  emitting entities in increasing Morton distance ≈ spatial proximity.
  - *Approximate near* (aggro/interest): the raw walk — fast, good-enough.
  - *Exact radius / k-nearest*: the walk yields candidates; verify with
    true distance (Euclidean or Chebyshev) and keep expanding the bit-window
    until the candidate ring's minimum-possible Morton bound exceeds the
    current k-th true distance (standard Morton-kNN termination).  Build the
    verification into the iterator so callers get correctness for free.

## Caveats (design around)

- **Z-order ≠ Euclidean.**  The curve has discontinuities at quadrant
  boundaries — Morton-adjacent is *usually* spatially close, not always.
  Approximate `near` tolerates it; `within`/`nearest` must distance-verify.
- **Granularity trade-off** — resolution vs key range vs entities-per-code.
  Support a per-code bucket so multiple entities can share a cell.

## Architecture fit (per-chunk vs global)

The world is a sparse `hash<Chunk[cx,cy,cz]>` of fixed 32×32 chunks (lib-plan
19).  Two placements, both expressible (it's just *where* the `spacial`
field lives):
- **Per-chunk index** (recommended) — a small `spacial` per chunk keeps the
  working set cache-resident; a cross-chunk `near` query touches the ≤9 (2D)
  / ≤27 (3D) neighbour chunks.  Best when local queries dominate.
- **Global index** — one tree; simplest for sparse worlds / large variable
  radii; less locality.

## Phases

| # | Scope |
|---|---|
| **R** ✅ | **The radix tree, as a standalone deliverable** — [RADIX_TREE.md](RADIX_TREE.md).  A store-backed binary PATRICIA tree over an abstract bit-key oracle: `init`/`free`, `insert` (with growth + relocation), `find`, `seek` (lower bound), bidirectional `first`/`last`/`next`/`prev`, `remove` with a node free list, and `validate`.  Rust unit tests only — no schema, no parser, no `Parts::Spacial` — so everything downstream inherits a structure that is already proven.  Steps R1–R7 green. |
| **S1** ✅ | The Morton bit-key: interleave the 2 or 3 quantized coordinate axes into the oracle deliverable R already consumes.  Rust unit tests (2D + 3D keys). |
| **S2** ✅ | `spacial<T[…]>` wired to the radix tree: insert/find/remove/`copy_claims` (`src/radix_db.rs`, `src/database/allocation.rs`), Morton-encoded coord key fields (offset-binary so signed axes compare like `sorted`/`index`), the parser's "planned 1.1+" gate lifted, both backends (interpreter + `--native`, including a local-only `spacial` type).  Also landed beyond the original scope: struct-field `spacial`, `for`-iteration in natural Morton order, `len()`. |
| **S3** ✅ | **Proximity via range slicing, not methods** — deliberately pivoted away from the originally planned `coll.near`/`coll.within`/`coll.nearest` stdlib methods (user direction: *"I do not want any keywords or function related to the new data structures.. use the current iterate syntax and slicing syntax"*).  `spacial` needs no new keywords or functions — it is queried with the same range-slice syntax any keyed collection uses: `xs[(x,y)..]` (open outward walk, caller `break`s), `xs[(x,y)..:n]` (capped at `n`), `xs[(x1,y1)..(x2,y2)]` (bounding box — the raw Morton-code interval, a superset of the geometric box since Z-order threads through codes outside it; caller filters/breaks for an exact shape).  Slices carry 1–3 axes (`xs[(x,y,z)..(x2,y2,z2)]`), guarded by a new parser diagnostic (`MAX_AXES = 3`: `spacial<T[a,b,c,d]>` now a clean error instead of the runtime panic an unbounded axis count would cause). |
| **S4** ✅ | **Consumer validation in crawler** — `crawler/near_mobs_test.loft` (the `combat` branch).  Crawler stores enemies as a flat `vector<Enemy>` keyed on hex axial coords (`src/sim.loft`), so every proximity query (`sim_enemy_dist`, aggro/threat) is a linear scan.  The demo builds the SAME 2000-mob set as both a `vector<Mob>` and a `spacial<Mob[q,r]>`, and for 500 query points ("who is within 5 hexes of me?") compares the bounding-box slice `idx[(q-R,r-R)..(q+R,r+R)]` + exact `hex_distance` filter against the brute-force scan.  Result (both backends): all 500 near-sets **identical** to brute force; ~**18× fewer candidates** examined (54 732 vs 1 000 000); wall-clock **8× faster interpreted** (261→33 ms) and **3.7× faster native** (101→27 ms).  Also exercised the outward `idx[(q,r)..:k]` interest-walk, negative coords (offset-binary), and `integer not null` coord keys (crawler's idiom). |

**S4 harvested lessons (dogfood):**
- The win **scales with mob density**, not with the feature itself.  Crawler's *per-dungeon-level* enemy count is tiny (`MAX_SPAWNS = 32`, ~15 live), so a linear scan there is already fine — the index earns its keep on the **large** populations: the town's civilian daily-loop NPCs, overland/wilderness, or any crowd/broad-phase.  Point the adoption there, not at the 15-enemy dungeon floor.
- The `spacial` query's cost is **allocation-bound, not compute-bound**: each slice materialises a scratch rec-vector, so native's compute speedup barely moved `spac time` (27 ms) while it cut `brute time` 2.6×.  This is the concrete motivation for the **direct-cursor walk** follow-up (a compact `RadixIter` over the parent links, no scratch vector — resembling `index` `on=1`).

## Critical files
- `src/radix_tree.rs` — the tree + bidirectional iterator (deliverable R, done); the Morton bit-key plugs in as a `KeyFn`.
- `src/radix_db.rs` — the DB↔tree bridge (the `Radix`-kind counterpart of `src/hash.rs`): `add`/`find`/`remove`/`count`/`records`/`range`, Morton/Z-order key interleaving, `MAX_AXES = 3`. **Resolved** — the four ops that used to `panic!("Not implemented")` are implemented; `allocation.rs::for_each_owned_child` has a `Radix` arm (mirrors `Hash`) so `remove_claims`/`copy_claims` no longer leak or panic.
- `src/spatial.rs` — the underlying near/within/nearest geometry algorithms `radix_db.rs`'s range primitives build on.
- `src/database/types.rs` — `spacial()` registration + the Morton key; keyed-kind handling. (This finally implements the keyed kind @P295/@P305/@P309 had to exclude.)
- `src/parser/*` — the `spacial<T>` 1.1+ diagnostic is lifted; residual diagnostics are missing-key-fields and the `MAX_AXES` guard; keyed-field handling mirrors hash/sorted/index from @P305/@P307/@P308.
- Proximity is **not** a `default/*.loft` API — see S3: it is range-slice syntax the parser/runtime already support for every keyed collection, no new stdlib surface.
- Reuse: `src/keys.rs` (key extraction), the keyed-collection deep-copy machinery, the gridmesh chunk model (per-chunk indexing).

## Verification
- Rust unit tests for `radix_tree` (insert / find / bidirectional walk /
  remove; 2D + 3D Morton keys).
- loft cross-mode: `tests/scripts/48-spacial-construct-free.loft` (construct /
  append / iterate / `len()` / struct-field / teardown, no leak) and
  `tests/scripts/48b-spacial-slice.loft` (2D + 3D bounding-box / open-walk /
  count-limited slices, asserted against hand-computed expectations); both
  backends.
- Parser diagnostics: `tests/parse_errors.rs::spacial_needs_coordinate_keys`,
  `::spacial_rejects_more_than_three_axes`.
- Consumer validation (S4): `crawler/near_mobs_test.loft` — spacial near-set ==
  brute-force scan on 500 queries over 2000 mobs, both backends; 18× candidate
  reduction; 8×/3.7× (interp/native) wall-clock win.  Harvested lesson: the win
  scales with population, so adopt on the town/overland crowds, not the ~15-enemy
  dungeon floor; the per-query scratch-vector allocation is the next efficiency
  lever (direct-cursor walk).

Acceptance (S1–S4, met): `spacial<T[x,y]>` and `spacial<T[x,y,z]>` work end to
end — construct, append, iterate in natural Morton order, `len()`, and range
slices for proximity; the 1.1+ diagnostic lifted; both backends green; reuses
`radix_tree.rs` + `spacial<T>` rather than inventing new structure; no new
keywords or stdlib functions; validated against a brute-force oracle in a real
consumer (crawler).  The plan is complete.
