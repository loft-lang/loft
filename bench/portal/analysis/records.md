# The record shapes — why a vector of records is 3–18× Rust

Analysis of the portal's two record classes, taken 2026-09-21 on x86-64 from
`bench/16_consumer_shapes` (hot loops modelled on moros and crawler) and the record rows of
`bench/14_stdlib_vector`.  An ANALYSIS: it names the mechanisms, prices what a paired
source variant could price, and ranks what to build.  **All seven levers are built** (the
same day) — § Built, at the end, has what each measured against what this priced, and what
the building found that the analysis had wrong.

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

## Built (2026-09-21) — R1 to R7

| routine | before | after | × Rust before → after | levers that moved it |
|---|---:|---:|---|---|
| `mesh_emit` | 708.7 µs | 107.3 µs | **18.3 → 2.95** | R1 (−73 %), R4 (−43 %) |
| `enum_match` | 111.6 µs | 40.3 µs | **12.4 → 4.47** | R7 (−64 %) |
| `mesh_aabb` | 74.7 µs | 37.5 µs | **9.8 → 4.89** | R2 (−6 %), R3 (−47 %) |
| `entity_tick` | 299.8 µs | 147.5 µs | 4.67 → 2.28 | R2 (−30 %), R5 (−23 %), R7's window (−6 %) |
| `chunk_lookup` | 1,459 µs | 1,081 µs | 3.78 → 2.80 | R6 (−23 %) |
| `record_append` | 126.7 µs | 69.0 µs | 3.55 → ~2.0 | R4 (−45 %) |
| `record_walk` | 32.0 µs | 16.2 µs | 2.98 → 1.51 | R2 (−49 %) |
| `tuple_kernel` | 282.3 µs | 224.8 µs | 2.02 → 1.61 | R2 (−20 %) |

Lane 16's median 3.91× → 2.80×, its worst row 18.3× → 7.5× (`f32_build`, the vector-build
class, not this one); lane 14's median 3.03× → 2.41×.  Every hash unchanged.  Each lever is
a clause of an existing rule in `doc/claude/formal/rewrites.md` with its own switch, cells
(`tests/scripts/158-*.loft`) and emission pins (`tests/{field_mint,iteration_base,
nested_field,mint_window,copy_in_place,leaving_free,enum_record}.rs`).

What the building found that this analysis had wrong or did not see:

- **R2's form.**  The analysis priced R2 as a negative ("deriving the pointer costs what the
  three loads save") and proposed the base anyway.  The first build took the address from
  the element's `DbRef` — an identity test on its store and record — and measured **+46 %
  SLOWER**.  Hand-pricing the emitted Rust (which this analysis's method did not do for R2)
  found the form that works: the address from the INDEX (−40 %), and the iteration index
  stepped unchecked (−26 % more), which is a range proof the analysis did not list at all.
- **R1's hidden half.**  The § V-z element-first rewrite looked its header up by the
  bare-variable key, so with R1's gate alone a field-form element would have been minted by
  its template and finished through the header: 0 elements for 20, silently.  And a null
  record root handed to a parameter panicked indexing store 65535.
- **R3 must stay in the emitter.**  Folded in the parser, `v.pos.x` becomes an `(R-Scalar)`
  candidate typed by the parent, which a write through a sub-record view never evicts.
- **R5 does not elide the copy.**  The runtime already makes it a no-op; eliding it
  statically would drop the fault note of an out-of-range `v[i] = e`.
- **R6 is not small alone once R2 is in** (−23 %), and no program can make its rule answer
  wrong — it is falsified over synthetic IR.
- **R7's exclusion was one rule's reason read by all of them**: the (type, offset) key is
  `(R-Scalar)`'s problem, not the push header's or the address's.
- **Three cells could not fail when first sabotaged** (R4, R5, R6) and were replaced or
  supplemented by ones that can; and the checking form had a hole (R3: an offset summed
  wrongly re-reads the store at the same wrong offset) that is now closed.

Left in this class: `record_update` (4.6×: `v[i].x = v[i]?.y + …`, inline element accesses
rather than a view), the null-aware float compare in `mesh_aabb` (§ 8), R1's field-form
GROUP outside a held header (the parser reserves only for a local vector), a text-field
element's append (the text set grows a store), and `(R-Alias)` for a parameter root beside
another vector (a type-based alias rule would admit `sc.ops += […]` beside `src[i]`).


## Next (2026-09-25) — R8: a nested record rides the tuple

Priced, designed, NOT built.  The value-record family (`(R-ValueRecord)`, `(R-ValueLocal)`)
stops at a record whose fields are scalars: `hoist::type_layout` declines any field that is
itself a record, so mesh3d's `Vertex { pos: Vec3, normal: Vec3, uv: Vec2 }` — eight floats
two structs deep — keeps its return buffer while `Vec3` rides the tuple.  A census of the 42
library packages finds **23 structs whose fields are scalars and inline sub-records of
scalars** against 9 flat records wider than the six-field cap: the nesting is the wider
gap.  The rows it stands behind: `sphere` 14.5×, `mesh_to_floats` 21×, `save_glb` 9.6×,
stage `draw_list` 4.8× (`DrawRect { UiRect }`), tween `value` 5.4×, game_protocol `msg_ping`
11× (a nested record pair returned by value).

**Hand-priced** (`--native-release`, this box) on a probe of the `sphere` shape — per vertex
`p = vec3(…); n = vec3(…); add_vertex(m, vertex(p, n, u, v))`, 200 000 vertices:
**200 → 60–100 ns per vertex**, hash unchanged, with `n_vertex` returning the eight floats as
a tuple and `n_add_vertex` taking the tuple and writing its eight fields into the appended
slot.  What remains is the general record append — `OpNewRecord` (prefill), the eight
`set_float`s through `store_mut`, `OpFinishRecord` — which the push-window form of
`(R-PushRec)` would take to a handful of nanoseconds: that is the clause after this one.
Probe and patch: the session scratch `next/nv.loft`, `nvr.rs` → `nvr_v1.rs`
(`bench/portal/hand_price.sh`).

**The shapes, as the IR spells them** (from `loft introspect` of the probe):

* the callee's literal — `Vertex { pos: p, normal: n, uv: Vec2 { u: u, v: v } }` with `p`,
  `n` tuple parameters: `OpCopyRecord(p, OpGetField(__retbuf, 0, Vec3), Vec3)`,
  `OpCopyRecord(n, OpGetField(__retbuf, 24, Vec3), Vec3)`, and the nested literal already
  lowered in place (`(R-InPlaceLiteral)` C6): `OpSetFloat(OpGetField(__retbuf, 48, Vec2), 0, u)`,
  `OpSetFloat(OpGetField(__retbuf, 48, Vec2), 8, v)`;
* a site's nested read — `v.pos.x` is `OpGetFloat(OpGetField(v, 0, Vec3), 0)`;
* the parameter side — `add_vertex(m, av: Vertex)` appends `av` whole
  (`OpNewRecord` / `OpCopyRecord(av, elm)` / `OpFinishRecord`).

**The one invariant**, and the sites that re-assert it: *a tuple of a record is its scalar
fields in declaration order, an inline sub-record contributing its own fields at the
SUMMED offset* — one home, `type_layout`, which already serves a result, a local and a
parameter by construction.  Then: (1) `type_layout` recurses into an inline sub-record
field (`Reference(S)` where S lays out itself; the cap counts scalars, six → eight or
more — the comment on `VALUE_RECORD_MAX_FIELDS` says a wider tuple is spilled by the ABI,
which is still nothing beside a store record); (2) the site walk's read arm folds an
`OpGetField` chain with constant offsets into the summed key `(tp, P + X)` before it
looks the index up; (3) the callee's Object-block tuple build takes a sub-record COPY from
a tuple local as that local's elements, and a nested in-place field write at its summed
offset; (4) the parameter side reads `av.pos.x` through the same fold and hands a
tuple-carried record to an append by materialising it into the appended slot (the general
append first, the push window later); (5) the live-reload arm mints a record per tuple
parameter — its nested writes go at the summed offsets.  Falsifiers: the interpreter is
the values oracle; `LOFT_NO_VALUE_LOCAL` / the value-record switch are the A/B; the
existing cells `a-small-record-parameter-is-carried-as-a-tuple.loft` and the
value-record pins extend with a nested cell each; `LOFT_TRACE_VALUEREC=1` names the
admission.  Matrix axes to hold: nesting depth (1, 2), a nested field written after the
literal, a sub-record handed to another tuple parameter, a nested record with a narrow or
boolean member, an enum member (declines), a site that copies the whole sub-record
(`q = v.pos`).
