# The record shapes — why a vector of records is 3–18× Rust

Analysis of the portal's two record classes, taken 2026-09-21 on x86-64 from
`bench/16_consumer_shapes` (hot loops modelled on moros and crawler) and the record rows of
`bench/14_stdlib_vector`.  An ANALYSIS: it names the mechanisms, prices what a paired
source variant could price, and ranks what to build.  Nothing here is built.

| routine | what it is | × Rust | loft instr / item | Rust |
|---|---|---:|---:|---:|
| `mesh_emit` | nested records appended to `m.verts`, `m.tris` | **18.0** | 2,314 per vertex | 65 |
| `enum_match` | struct-enum values appended, then matched | **12.4** | 969 per edit | 81 |
| `mesh_aabb` | `v.pos.x` … over 7,168 vertices | **9.7** | — | — |
| `record_update` | `v[i].x = v[i]?.y + …` | 4.66 | 44 per record | — |
| `entity_tick` | copy out, mutate, scan the vector, write back | 4.58 | 1,750 per entity-tick | 207 |
| `record_append` | `v += [P { … }]` | 4.40 | 175 per record | 18 |
| `chunk_lookup` | scan a chunk list, then a record in a vector in a record | 3.69 | 1,293 per lookup | 95 |
| `record_walk` | `for p in v` reading three fields | 2.99 | — | — |

(— : the routine was inlined into its caller in that lane, so callgrind's toggle could not
isolate it.)

## The finding in one paragraph

**None of this is missing machinery — it is existing machinery that declines.**  The
rewrites that make a record loop fast already exist and are verified: a push header that
builds an element in its slot (`@FR-R-PushRec`), a record view's address carried for direct
field loads (`@FR-R-RecPtr`), a held element base (`@FR-R-Base`).  The flat, bare-variable
shapes the drawing library uses take them, and sit at 1.3–2×.  The shapes a game writes —
a vector that is a FIELD of a record, a record nested INSIDE a record, a struct-enum
element, the copy-out / write-back idiom, a lookup loop that returns from inside — each
miss one admission rule and fall back to the general path, where every field access
resolves the store again (~25–48 instructions for a load or a store) and every append is
two trips through the runtime's type table.  The traces say so in their own words
(`LOFT_TRACE_RECPTR=1`, `LOFT_TRACE_HOIST_DECLINE=1`); each mechanism below quotes its
decline.

In `entity_tick` and `chunk_lookup` **89–91 % of the instructions are in the EMITTED
function itself**, not in the runtime: this class is a code-generation question, where
the keyed class was a runtime-library one.

## The mechanisms

### 1. An append to a vector FIELD takes the general path — `mesh_emit`, 18×

`m.verts += [Vertex { … }]` where `m` is a record.  `hoist::mint_path` — the ONE
definition of an admissible record mint — answers `None` for a field path, by its own
documented choice: *"A FIELD path (`h.pts += […]`) also answers `None` — this unit admits
the bare-variable root the raster loops use; widening to field paths is a measured decision
for later."*  So the loop keeps no push header and every element is
`OpNewRecord` → `record_new` → `vector_append` … `OpFinishRecord` → `record_finish` →
`insert_record`, with the type-table walk (`nullable_field_parent`, `sub_record_type`,
`nullable_some_variant`, `set_default_value_nullable`, `link_siblings`) run twice per
element: ~1,500 of the 2,314 instructions per vertex.

**Priced** by the same routine over two bare local vectors (`verts += […]`,
`tris += […]`), identical checksum: **678 → 184 µs (−73 %)**, 18× → ~4.9×.  This is the
measurement that `mint_path`'s comment was waiting for — and every builder a consumer
writes has this shape (`m.verts`, `sc.ops`, `world.items`); it is also the drawing
library's `parse` row, where the same lever was estimated at −1.5 µs of 17.

### 2. A field write into a just-minted element resolves the store per field — 4–5× after (1)

Even on the push-header path the element's fields are written as
`stores.store_mut(&db).set_float(db.rec, db.pos + off, v)` behind an `if db.rec != 0`:
a store lookup, a record-validity read and two bounds checks per FIELD — **143
instructions for the three fields of a `P`** (`record_append`: 175 per record against
Rust's 18), eleven such writes per vertex-and-triangle in `mesh_emit`.  The slot's address
is known — the push header just produced it.  `(R-RecPtr)` is asked and declines
(`_elm_3`: *"the remainder may grow a store"*): the rest of the block contains the NEXT
push, and the rule promises the address for the whole remainder rather than up to the
element's own finish, which is all a mint group needs.

### 3. A nested field path is not a fusable read — `mesh_aabb`, 9.7×

`v.pos.x` is `OpGetField(OpGetField(v, pos), x)`.  The loop holds a vector header and an
element base, and `(R-RecPtr)` still declines the loop variable — *"no fusable field read
or write of the view"* — because only a DIRECT scalar field of the view counts, and every
read here goes through the inline sub-record.  Each of the twelve reads per vertex is
emitted as a `DbRef` rebuilt with two offset additions and a full
`stores.store(&db).get_float(…)`.

**Priced** by binding the sub-record first (`p = v.pos;` then `p.x` …), which
`(R-RecPtr)` admits: **86 → 41 µs (−52 %)**, 9.7× → ~4.6×.  An inline record's field is
the parent's field at a constant offset; the path can be folded at generation time and
needs no binding.

### 4. A loop variable's address is re-derived through the store although the loop holds the base — every `for e in v`, 3×

Where the pointer IS admitted, a `for c in m.chunks` iteration emits
`get_vector_hoisted(…)` to build a `DbRef`, then `vector::rec_ptr(&c, &stores.allocations)`
to turn that `DbRef` back into an address — a second store resolution per element — in a
loop that already holds `__vb`, the address of element 0.  The element is at
`__vb + index × size`.  **Priced, as a negative:** making `chunk_lookup`'s loop variable
admissible changed nothing (916 → 955 µs), because deriving the pointer costs what the
three direct loads save.  This is `record_walk`'s whole row (2.99×), the inner scan of
`entity_tick`, and the floor under (3).

### 5. The write-back idiom blocks the record pointer — `entity_tick`, 4.6×

```loft
e = ents[i]?;  e.energy += e.speed;  …  ents[i] = e;
```

The consumer wrote the copy-out / mutate / write-back a Rust or C author writes.  In loft
`e` is a VIEW of `ents[i]` (`(B-View)`), so the last statement copies the element onto
itself — and that whole-record element assignment is not on the hoist allow-list, so
`(R-RecPtr)` declines `e` for the entire block: *"the remainder may grow a store"*.  The
~20 field reads and writes of `e` per tick then resolve the store each.

**Priced** by deleting the write-back (same checksum): **299 → 221 µs (−26 %)**, 4.6× →
~3.4×.  Two separate facts are owed here: a self-copy `v[i] = e` where `e` views `v[i]` is
no work at all, and a copy of a NO-HEAP record grows no store, whatever it copies onto.

### 6. An early return from a lookup loop blocks it too — `chunk_lookup`'s `map_get`

`for c in m.chunks { if c.cx == cx && … { return c.hexes[i]?.h_height } }` — the `?`
mints a per-site discharge buffer, the `return` frees it, and that free stands in the same
statement as the uses of `c`: *"the remainder frees a record before a use of the view"*.
The free is of the BUFFER, never of what `c` views, and it is on the path that leaves the
loop.  (With (4) unfixed this one buys nothing on its own — see the negative result
above — but it is the shape of every "find the record and answer a field" helper.)

### 7. A struct-enum element is excluded from all of it — `enum_match`, 12.4×

`plain_record_type` excludes *"a nullable, an enum payload and a synthetic
`__nullable<S>`"*, so a `vector<Edit>` gets neither the push header on append
(`pre_alloc_vector` + `OpNewRecord` + generic field stores + `OpFinishRecord`: ~420 of 969
instructions per edit) nor the record pointer in the `match`.  The match itself reads the
variant TAG three times — once per arm, all three hoisted ahead of the first test — each a
full store resolution, then three generic field reads in the arm taken.  A variant's
fields sit at fixed offsets behind a one-byte tag; the layout question the exclusion
names is answerable per arm, because the arm's tag test is exactly the fact that fixes it.

### 8. What is NOT waste: the float comparison

`v.pos.x < lox` emits the null-aware comparison (a NaN is the float null and orders
first): several tests where Rust's is one.  That is the language's semantics on operands
no proof says are non-null (a record field), and it is part of why `mesh_aabb` stays ~4×
after (3) and (4).  A range/non-null proof for fields a loop never writes is the
admissible lever there — the float twin of `(R-Range)` — not a change of meaning.

## What to build, in order

| | lever | rows it moves | measured / expected |
|---|---|---|---|
| R1 | `mint_path` admits a vector FIELD of a record variable (a path, not a root): the push header keyed by path as the read headers already are | `mesh_emit`, the drawing `parse` row, every consumer builder | **−73 %** measured on `mesh_emit` |
| R2 | the loop variable of `for e in v` takes its address from the held base (`__vb + i·size`), no `DbRef`, no `rec_ptr` | `record_walk`, `chunk_lookup`, `entity_tick`'s scan, `mesh_aabb` | est. −30…50 % of each walk; it is the floor under R3 and R5 |
| R3 | a field PATH through inline records folds to one offset and counts as a fusable read/write | `mesh_aabb`, `mesh_emit`'s nested writes | **−52 %** measured on `mesh_aabb` (by hand-binding) |
| R4 | a minted element's field writes go through the slot address until its own finish | `record_append`, `mesh_emit` (after R1), `entity_tick`'s fill | est. 143 → ~30 instructions per 3-field record |
| R5 | a self-copy `v[i] = e` of a view of `v[i]` is elided, and a no-heap record copy is on the allow-list | `entity_tick`, the write-back idiom everywhere | **−26 %** measured |
| R6 | a buffer's free on a leaving path is not a free "before a use of the view" | `map_get` and every find-and-return helper | small alone; unlocks R2 for them |
| R7 | struct-enum elements: the push header on append, and per-arm field access behind ONE tag read | `enum_match` | est. −50 % |

R1 is the clear case: the largest measured gain in the portal (−73 % on its worst row),
one documented gate to widen, and a shape every consumer has.  R2 comes next because it is
the floor under three other levers.  Together with R3 they address the three rows over
9×; R4–R7 are what takes the class from ~4× toward 2×.

## Method

`valgrind --tool=callgrind --toggle-collect='*::n_<routine>'` on an unstripped
`--native-release --native-debug` build for instructions per item, both lanes;
`LOFT_TRACE_RECPTR=1`, `LOFT_TRACE_HOIST_DECLINE=1`, `LOFT_TRACE_BASE=1` for the reason
each rewrite gives for declining; the emitted Rust (`--native-emit`) for what the declined
form costs; and ONE program holding each routine beside a source variant that differs in
exactly the suspected statement, both timed in-process with equal checksums — the variant
that admits the rewrite prices the mechanism without building anything.  Bisecting the
`entity_tick` decline took five one-statement variants under the trace: only the
write-back moved it.
