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


## R8: a nested record rides the tuple — priced 2026-09-25, BUILT 2026-09-28 (§ Built below)

Priced and designed here; built as `(R-ValueRecord)`'s nested clause.  The value-record family (`(R-ValueRecord)`, `(R-ValueLocal)`)
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

## Built (2026-09-27) — an appended builder's heap record lands in its element

The record-build rows' commonest site is `out += [mk(…)]` over a record WITH A NAME — a
text field — and @PLN157 § V-d built the in-place delivery for all-scalar records only.  The
gate's stated reason was the synthetic-nullable field, which
`record_is_fully_written_by_a_literal` already excludes on its own; the element is fresh, so
a text, a plain vector or an inline record is written into it exactly as into a store of
its own.  Hand-priced first on `panel_build`'s emitted Rust (six `make_button(…, _elm)` calls
in place of lift + copy + free): 6.55 → 5.30 ms per op, hash `11c234`; built (the gate widened,
`LOFT_NO_APPEND_IN_PLACE` the A/B on one build): **6.7–7.0 → 5.4 ms (−20 %)**, hash equal.
What the row still pays, from its profile after the change: the per-frame free of the
previous `Panel` (`remove_claims_mode` 10 %, `owned_walk` 4 %), the store claims of its
texts (`claim_block`, `fl_insert`, `set_free_header` ~12 %), and `palette_items_for_tool`'s
copy of a constant vector into the list.  A keyed or struct-enum field keeps the copy
(h9, h11 of the cell file), and so does a VECTOR field at any inline depth (2026-09-28): built
in the element it keeps its quantised push block where the copy claims it at length, and the
persisted file grew 25 % (`store_rebuild_b1`; the @PLN123 guards caught it on the branch's
gate); `slope_path_with_undo` is unmoved — its builder site
(`m.m_chunks += [build_chunk(…)]`) runs only when a chunk is absent, and the row is the
two `map_get_hex` result stores per step, which is `(R-ValueRecord)`'s width and the `?`
discharge exit (priced next).

## Built (2026-09-27) — the seven-field accessor rides the value path

`slope_path_with_undo` (19.8×) was two `map_get_hex` result stores per step — a mint, a
seven-field copy and a free, twice — plus the lookup loop's declined hoist.  Hand-priced on
the emitted Rust (the accessor answering `[i64; 7]`, its two hot callers taking it): 565 →
297 µs per op, hash `3f61de`.  Built as four admissions of the value-record family, none of
them a new rule: the width six → eight; a `return v[i]?` exit as a value leaf; a field write
on a value local; and the tuple-parameter gate's terminal-copy exemption with the narrower
"existing record" question for a body `body_writes` cannot type — plus the hoist gate
admitting the frame's own return-buffer mint.  On one build: 572 → **293 µs (−49 %)**, hash
equal; `LOFT_NO_VALUE_RECORD=1` is the A/B.  What the row still pays, from its profile: the
two linear chunk scans per step (`map_get_hex` and `map_set_hex` walk `m.m_chunks` comparing
three fields per chunk — the library's algorithm, now hoisted), and `undo_push`'s
`s.us_redo = []` clear plus the `UndoEntry` append.  Beside it, `resolve_move` (10×) calls the
same accessor and is the next row to re-read.  One lesson the corpus paid for: the
discharge-return's copy source is a served leaf read, and the copy clause of the local-use
accounting counted that mention a second time, so a local with a bare in-place mint beside
a field write passed as a value local and its mint was emitted into a tuple
(`85-struct-copy-return-owned` failed to compile natively).  The accounting counts a
mention once; cell v9 holds the shape.

**The lane against the pre-arc commit** (2e4199b7 built into its own target dir, the two
binaries interleaved twice on the idle arm64 box, `--n 10`, every hash equal):

| row | pre-arc | now | |
|---|---:|---:|---:|
| `resolve_move` | 6.24 ms | 3.78 ms | −40 % |
| `panel_build` | 6.74 ms | 5.66 ms | −16 % |
| `slope_path_with_undo` | 579 µs | 301 µs | −48 % |
| `emit_to_material` | 1.53 ms | 1.14 ms | −25 % |

`resolve_move` and `emit_to_material` were not worked on: they call `map_get_hex` and the
`vec3` / `vertex` builders, which the width and the discharge-return leaf reached.

## Built (2026-09-28) — R8: a nested record rides the tuple

Built as designed in § R8 (`formal/rewrites.md` `(R-ValueRecord)` § The nested clause): the
layout recurses into an inline sub-record (one home, `hoist::type_layout`), the sites fold a
nested read to the summed key, the builder's literal takes a tuple source's elements for a
sub-record copy, and a sub-record of a value local crosses a call or a copy as a range of the
tuple.  On the sphere-shaped probe (`p = vec3(…); n = vec3(…); add_vertex(m, vertex(p, n,
vec2(…)))`, 2 000 000 vertices, this arm64 box idle): **0.27 → 0.17 s (−37 %)**, values equal
on the interpreter; `LOFT_NO_VALUE_RECORD=1` 0.43 s.  The emitted `n_vertex` answers
`(f64 × 8)` from its three tuple parameters with no store touched, and `add_vertex` takes the
eight floats and writes them into the appended slot — the general record append (`OpNewRecord`,
eight `set_float`s, `OpFinishRecord`) that the push-window form of `(R-PushRec)` is the clause
after this one.  What the building found the design had not said: a write through a
sub-record must count as a write into EVERY enclosing record, or a `Vertex` tuple parameter
would not see `other.pos.x = …` (the classifier typed such a write by the sub-record alone,
which the flat family never had to notice — cell n12 is the falsifier, and a `&` link to a
sub-record reaches every type holding it inline); and a bind from a sub-record (`q = w.pos`)
is a VIEW by the oracle, not a copy, so that local keeps its record (n10).  The rows it
stands behind (`sphere`, `mesh_to_floats`, `save_glb`, stage `draw_list`, tween `value`,
game_protocol `msg_ping`) are re-read on the x86-64 lane at the next portal re-measure;
`msg_ping` returns a nested record PAIR by value, so it is the row to read first.  Cells
`tests/scripts/158-nested-value-record.loft`; the `nested_field` pin's n12 row moved
(2 → 9 reads through the address: the callee's parameter is a tuple now, filled at the site).

## The record lifecycle, profiled (2026-09-29) — what the elimination rules still leave

The four slow record rows of the moros/dryopea lane, each run alone (60 iterations of its
driver, `perf` at 499 Hz over the native binary on the idle arm64 box), attributed by module:

| routine | store lifecycle — mint, claim, `copy_claims`, free | the routine's own code | the rest |
|---|---:|---:|---|
| `panel_build` (10.3×) | 55 % + 9 % libc allocator | 25 % | text formatting |
| `emit_to_material` (11.6×) | 47 % | 31 % | 17 % vector append and finish |
| `resolve_move` (13.4×) | 47 % | 47 % | `map_get_hex` alone 21 % — a hash lookup, five per call |
| `truncate_to` (12.1×) | 4 samples: too fast at 60 iterations; the same copy-and-free shape as `panel_build` | | |

Half of each row is records the Rust twin keeps on the stack or moves.  The elimination
rules are not the limit; their ADMISSION conditions are, and every rule's trace names the
condition it declined on for these functions:

- **`(R-ValueRecord)` is blocked by consumers, not shape.**  `Rect`, `Hex`, `Vertex` and
  `Vec2` travel as tuples; `Vec3` does not, because `resolve_move` declines to carry `from`
  and `dxyz` as tuples — *"the body may write a record of its type"*.  The body writes `Vec3`
  records only into the fresh return buffers of its own `vec3` calls, but the callee write
  set is keyed by TYPE, so a fresh buffer reads as a possible alias of the parameter.  That
  one imprecision keeps every `vec3` a store record: four mints and frees per `resolve_move`,
  eight per hex in `emit_hex_surface`.  It is the store-identity question `(R-Base)`'s growth
  clause answered for loops (`hoist::StoreFacts`), asked of a callee's write set.
- **`(R-Place)` admits one destination shape.**  A call result into a record-literal field of
  an element appended to a PARAMETER's collection, from a literal-exit callee.  Declined here:
  `panel_build`'s three locals (built for the fields of the frame's OWN return literal — the
  return buffer is never a host), `hex_to_world` (exits through `vec3(x, y, z)`, a call),
  `e` in `c_truncate_to` and `up` in `emit_hex_surface` ("handed to a loft-defined call").
  Each is a local with exactly one owning sink.
- **`(R-MoveLast)` relocates within one store, so it inherits `(R-Place)`'s gap.**
  `Panel { p_toolbar: pb_buttons, … }` finds `pb_buttons` in its own store and the return
  buffer in the caller's: the "move" is a deep copy of six buttons with two texts each, then
  a free of the source — `copy_claims` 5.5 % and `remove_claims_mode` 6 % at the top of the
  profile.
- **No move crosses a call.**  `history_push(tl, e)` appends its by-value parameter; a
  by-value parameter IS the caller's record (`(B-Ref-Uniform)`), so the append deep-copies
  the `Stroke` tree and the caller frees `e`.  The twin writes `h.entries.push(e)`.
- **A loop-local record a push takes is minted per pass.**  `tri = Triangle {…};
  m.triangles += [tri]` declines `(R-LoopRecord)` ("a native op takes it otherwise"): six
  mint-copy-free per hex where the inline spelling would build in the slot.
- **The appended tuple** (`m.vertices += [vertex(…)]`) is still a mint, eight stores and a
  finish — `vector_finish`, `record_finish`, `insert_record` in the emit profile; the
  `(R-PushRec)` clause priced in § R8.
- **The per-frame free is a full walk** (`owned_walk`): `p = panel_build(…)` in a loop frees
  the previous tree; no rule hands the old store back as the buffer.

### Order, with the ceiling

The lifecycle share above is the ceiling: these rows can lose about half their time to the
rules, not reach parity — `map_get_hex`, the text formatting and the geometry stay.

1. **Callee write sets by store identity** — a write into the callee's own fresh buffer
   cannot alias a parameter (`hoist::StoreFacts` in the value-record parameter admission).
   No new rule; flips `Vec3` to a tuple; reaches three of the four rows.  Expected:
   `resolve_move` −40 % (four of the 47 %), `emit_hex_surface`'s eight `vec3` mints per hex.
2. **Destination-directed builds** — `(R-Place)` widened from "a call result into a
   parameter's element literal" to "a local with one owning sink": a field of the frame's
   return literal or an appended element, built by a call, a literal or a vector build, the
   return buffer a host.  `(R-MoveLast)` then relocates in-store.  `panel_build`'s three
   locals, `tri`, `up`, `centre`.  Expected: most of panel_build's 55 %.
3. **The appended tuple** (§ R8's next clause): `vertex`, every `add_vertex` shape.
4. **A move through a by-value parameter** — caller's local dead after the call, callee's
   parameter used once as an appended element or stored field: relocate, not copy.  Needs a
   design note: it changes what a callee may assume of its parameter's store after the append.
5. **Store reuse across a rebind from a call** for a heap-owning tree — a per-frame UI's
   shape; a new rule with a lifetime argument, so last.

Evidence: 182 / 36 / 120 / 4 samples at 499 Hz, so the shares are within a few points;
the decline reasons are exact (`LOFT_TRACE_VALUEREC`, `LOFT_TRACE_PLACE`, `LOFT_TRACE_POOL`,
`LOFT_TRACE_LOOP_RECORD`, `LOFT_TRACE_RECPTR` over the lane's emission).

## Built (2026-09-29) — a callee's own buffer is a fresh record

§ Order's first item, as designed: the fallback walk of the tuple-parameter gate
(`hoist::may_write_existing`, the one a body `body_writes` cannot type) now seeds a callee's
own return buffer as fresh in the callee's walk, keeps the frame's buffer fresh through a
rebind of the local it was promoted onto, and counts a mint into a local that owns its store
(`hoist::owning_local`) as a fresh record.  No new rule and no switch of its own
(`LOFT_NO_VALUE_RECORD` covers it); `LOFT_TRACE_VALUEREC=1` now names the op that keeps a
parameter a record.  On the moros/dryopea lane `resolve_move`'s `from` and `dxyz` are carried
as tuples, and `vec3` and `hex_to_world` become value records (14 functions as tuples, 9
parameters, from 12 and 7).  Measured on this arm64 box, old compiler against new, same
hashes: **`resolve_move` 4.19 → 1.65 ms (14.0× → 5.33×, −60 %)**, **`emit_to_material`
1.21 → 0.97 ms (10.6× → 8.1×, −20 %)**, `panel_build` and `truncate_to` unmoved as
predicted (their cost is the copy-and-free pair, § Order 2 and 4).  Cells
`tests/scripts/158-a-callees-own-buffer-is-a-fresh-record.loft`: the resolve_move shape, a
builder into a fresh local's field, a builder two calls deep, and the aliasing negatives
(c4b / c5b: a parameter naming the record written reads the write back) — the falsifier is
the sabotage recorded in the cell header.  What `resolve_move` still pays is `map_get_hex`,
five hash lookups per call (the keyed class), and the `MoveResult` return.

**`smooth` (loft#1570), profiled the same day** (`perf` over the lane binary, 300 samples):
`n_smooth_pts` 40 % self, `length_vector` 20 %, `get_vector` 12 %, `n_ctrl` 12 %,
`vec_header` 6 %, `OpFreeRef` 4 %.  `pt`, `half_chord` and `ctrl` are already tuples.  Three
mechanisms, in the order they were taken:

1. **The appended tuple through the window — BUILT.**  `sp_out += [raster::pt(…)]` is a mint,
   two `store_mut` writes and a finish per point.  The mint window (`(R-RecPtr)`'s mint
   clause) declined the element because a copy FROM a value local INTO it was not a
   "fusable write" — it lowers to setters, so it is one now — and the record push window
   (`(R-PushFill)`'s record clause) declined because `sp_a` "may view the pushed vector": a
   view of a no-heap RECORD reads fields and never a length, so it cannot observe a deferred
   length bump, and the copy into the fresh element is the slot's own write.  Both refined;
   the window's write test is asked as the header hoist asks it (in-place tier, the variable
   table).  Hand-priced first on the emitted Rust (−13 %, same hash), then built: **884 →
   742 ns per op (5.4× → 4.6×)** on this box.  Cell m15 of `158-mint-window.loft`; m12's
   pin moved (its tuple result is now delivered through the element's address).
2. **The outer loop's hoist — BUILT.**  `LOFT_TRACE_HOIST_DECLINE` (which now names the
   innermost blocking node) said the `half_chord(…)` call blocks loop 14: `ctrl`'s
   `pts[j]?` discharge mints a buffer and writes its defaults, `half_chord` frees the
   buffers of `ctrl`'s tuple results, and the two callee tests (`in_place_only_writer`,
   `retbuf_only_writer`) read every such native op as a write.  They now carry the header
   hoist's own allowances (`hoist::callee_allowance`: a discharge buffer's mint, a lazy
   buffer's, a record free, a DEAD buffer's mint/clear/free, the last read off a
   program-wide per-function table `HoistOwned::dead_by_fn`), and the memo never pins a
   verdict reached without those facts.  Loop 14 hoists: `pts` and `flags` hold headers,
   `half_chord__inv` takes `pts`'s header and a base derived at the call and hands both on
   to `ctrl__inv`, whose `len(pts)` is the header's length and whose `pts[j]?` is one
   fused read.  **742 → 500 ns per op (4.6× → 3.09×)**; the verifier run of the whole
   drawing bench is green.
3. **A parameter beside a pushed return buffer — MEASURED, not the hazard it read as.**
   `sp_out` ADOPTS the return buffer; `(R-Alias)` drops a parameter's header only beside
   a push to the `__retbuf` variable itself, and adoption keeps the local's name — so
   `pts` kept its header, and the question was whether that is sound.  Probed: `v =
   grow(v)` where `grow` adopts its buffer and holds `src`'s header answers the same on
   both backends and under `LOFT_HOIST_VERIFY=1`, because the adopted buffer is a store
   minted for the call, never the argument's — the parser hands no live store in as a
   buffer while a view of it is passed (the cross-call growth refusal).  Documented as the
   rule's reading; no select needed.
4. **The frees — BUILT, two shaves.**  At 3.09× the profile's remaining share was ~70
   `OpFreeRef` calls per op on buffers that hold nothing: `half_chord`'s `__ref_1`/`__ref_2`
   for `ctrl`'s tuple results (dead by any reading, but `dead_buffers` iterated the frame's
   MINTED set, and these are minted by no one — now a never-minted buffer whose every
   mention is an admitted callee's buffer argument or a free is dead too: 500 → 487 ns),
   and `ctrl`'s two `?` discharge buffers, minted only on the absent arm and freed on every
   exit (now a VARIABLE's free is guarded by its own null test inline, the compare the
   runtime made after the call: 487 → **433 ns, 2.67×**, range 2.59–2.74).  The whole
   drawing bench under `LOFT_HOIST_VERIFY=1` prints the same hash.  **`smooth` is under the
   bar on this box**; the closing measurement for loft#1570 is the x86-64 laptop's.

## `mesh_to_floats` (2026-09-29) — a null test on a view declined the whole loop

Profiled alone (`perf` over the lane binary, symbols): `vector_append` 36 %, `append_done`
13 %, `append_f32` 13 %, `pre_alloc_vector` 4 % — every one of the six `mf_buf += [x, y, z]`
singles per vertex went through the GENERAL append, because the loop's hoist declined on
`if mf_v != null`: `OpNeRef` / `OpEqRef` compare two `DbRef`s and touch no store, but they
were not in the gate's store-free list (a `reference` parameter is not a scalar, so the
generic test refused them).  Listed beside `OpRefIsNull`: **6.48 → 2.85 ms per op
(36.9× → 15.8×)** on this box, same hash.  Cell c18 of
`158-a-base-survives-a-growth-of-another-store.loft`.

What the row still pays, re-profiled: the routine's own code 59 %, and a per-triangle
`[mf_tri.a, mf_tri.b, mf_tri.c]` — a scalar vector LITERAL walked by `for`: a loop buffer
reset, a reservation, three general `append_i64`s and a length and element read per step
through the store (`vector_append` 13 %, `length_vector` 6 %, `append_i64` 5 %, `get_vector`
4 %).  The nullable view `mf_v` holds no address: its store is the parameter's and the
remainder grows the adopted result buffer, which the store oracle keeps apart from no
parameter on purpose.  The lever is a scalar vector literal of constant length carried as
an ARRAY — a local (`for i in [a, b, c]`, walked without a store) or a record field
(`Mat4 { m: [16 floats] }`, mat4_mul 38×): one representation clause beside the value
record, § Order item 7 of the evaluation, and the next unit for both rows.

**BUILT 2026-09-29, the local half** (`(R-LiteralWalk)`, `LOFT_NO_LITERAL_WALK`): the literal
walk is n scalar temps and a counted select on both backends, no vector.  `mesh_to_floats`
**2.85 → 1.84 ms per op (15.8× → 10.5×)**, same hash.  What the row pays now is the six field
reads through `stores.store(&db).get_float` (a store lookup and a null test each) and the six
pushes through the header per vertex, against a Rust twin that reads a struct and extends a
`Vec<f32>` — the `(R-RecPtr)` remainder for a nullable view whose store is the parameter's.
The record-field half (`Mat4 { m: [16 floats] }`, `mat4_mul` 38.6×) is still the
representation clause and stays open.

## `sphere` (2026-09-29) — the append's runtime dispatch, not its writes

Profiled alone (`perf`, the scratch driver over mesh3d by path): `record_new` 10 %,
`record_finish` 8 %, `nullable_field_parent` 8.4 % and `nullable_some_variant` 6.8 % (a
`__nullable<` NAME test, twice per append), `insert_record` 4.4 %, `link_siblings` 4.5 % and
`settle_displaced` 3.6 % over empty lists — 27 % of the row in the runtime's general
record dispatch for `self.vertices += [av]`, which the emitter had already reduced to eight
`rec_set`s through the element's address.  The bare-vector short path (§ V-k) never reached a
FIELD.  It does now (BOTH backends, `LOFT_NO_PLAIN_FIELD_APPEND`): **1.70 → 1.22 ms per op
(10.0× → 7.1×)**, same hash; the interpreter shares the path (0.22 → 0.20 s for twenty spheres, inside
its noise).  What the row still pays:
`vector_append`'s growth (the twin's `Vec` pays it too), the prefill of the eight fields
the tuple then overwrites (`OpNewRecord` with prefill — `(R-CompleteWrite)` does not see the
tuple delivery's writes, which sit in one block statement the coverage walk does not enter),
and the `add_triangle` half.  **The prefill went the same day** (`(R-CompleteWrite)`, a
whole-record copy into a heap-free element covers the type): **1.22 → 0.94 ms per op
(7.1× → 5.4×)**.  What is left is the growth, the two `add_triangle` writes and the
trigonometry the twin also pays.

## `pack_instances` (2026-09-29) — one text set kept three callees writing

Traced (`LOFT_TRACE_HOIST_DECLINE`): the loop declined at `out += [frame_of(self, i) as
single]`.  `frame_of` discharges `self.st_seqs[n.nd_seq] ?? Sequence { …, q_name: "" }`; the
fallback's mint into its `__ref_p2_` buffer was already an allowance, its text field's
`OpSetText` was not — outside the twelve scalar setters, it made the callee a writer, and
`lit_colour`'s chain the same.  With the SET admitted as the mint is (the buffer's store is
the site's own): **920 → 309 µs per op (24.3× → 8.4×)**, hash unchanged — 16 pushes through
the push header, three vector headers, where before there were 16 general appends and 20
store reads per node.  What the row still pays: the twenty field reads of `n` through the
store (`(R-RecPtr)`'s remainder: `out` is the return buffer, never proven apart from `self`)
and the three calls per node.

## cbor `encode_bytes` and `decode` (2026-09-29) — profiled, not yet built

`encode_bytes` (31.7× on the laptop): the byte-wise copy IS admitted (`LOFT_TRACE_BYTE_COPY`
names `n_encode`'s `_mv_value_4` as one append), and the row's 100 encodes of a 64 × 256-byte
array cost 33 µs each here — 2 ns a byte against a memcpy.  The profile is flat and all of
it is per-CHILD buffer churn: `encode(items[i])` mints a result store (`op_database_inner`,
`claim_block`, `claim_best_fit`), `head()` mints another and pushes one to three bytes into
it, the parent appends the child (`vector_add`, `copy_block_cross_store`, `resize`) and frees
it (`free_named`, `clear_vector_release`, `set_free_header`, `fl_insert`) — about 500 ns
around 259 bytes.  The lever is the destination-directed build (§ Order item 6): `buf +=
encode(x)` handing `buf` to the callee as the buffer it appends INTO, so a child neither mints
nor is copied; `encode` is recursive, which is why the frame's buffer pool does not reach it.

`decode` (21.8×, 42.6 ms against 2.0 ms — the largest absolute gap on the list, and
pluginabi's `check_request` at 59.9× decodes every frame twice through it): `read_value`'s
own code 19 %, and the CLAIMS machinery 40 % — `remove_claims_mode` 12 %, `owned_walk`
9.6 %, `copy_claims` 6.8 %, `holds_no_heap` 3.2 %, `OpFreeRef` 2.9 %, and `malloc` /
`cfree` / `finish_grow` 8 % from the child lists the walks allocate per record.  The shape:
`sub = read_value(bytes, p); items += [sub.value]` deep-copies the child's heap into the
parent's vector and then walks the temporary to free it, per node.  Two levers, the second
cheap: (a) the cross-call MOVE — the child built where it will live (§ Order item 6 again),
which removes both the copy and the free; (b) the walks themselves — `owned_walk` pushes a
child per FIELD of a struct, scalars included, into a `Vec` allocated per record, and
`remove_claims` re-asks `holds_no_heap` per child; a walk that visits heap-capable fields
only, over a scratch kept in `Stores`, is a runtime unit for every free of a heap-owning
record on both backends.  Beside them, `bytes[pos] ?? 0` reads through the runtime
(`length_vector` 5.9 %, `get_vector` 5.3 %): a parameter read outside any loop holds no
header, and the byte-range copies `for k in 0..arg { bs += [bytes[argpos + k] ?? 0] }` are
the `(R-VecCopy)` shape with a RANGE, one `append_bytes` each if the rule takes it.

## pluginabi `check_request` (2026-09-29) — the worst row, taken apart

58.8× here (12.1 ms per op of 2 048 checks: 5.9 µs a check against the twin's 101 ns), 59.9×
on the laptop.  A check decodes a three-entry CBOR map (`op` text, `state` 88 bytes, `arg`
24 bytes) TWICE — `pa_decode_ok`, then `pa_decode` — walks the entries for `op`, copies its
text out and compares it with six constants.  The twin does the same two decodes over
borrowed slices and allocates nothing.

**Counted** (`LOFT_ALLOC_REPORT=1`, one round of 2 048 checks minus the next): **26 store
alloc/free cycles and 5.5 records per check**; live stores stay at 21 (reuse works), the
CYCLES are the cost.  **Profiled** (`perf` over the `--native-release` child, symbols kept):
store allocation and free 27 % (`claim_block`, `claim_best_fit`, `fl_insert`, `finish_claim`,
`set_free_header`, `free_named`, `database_named`, `op_database_inner`, `OpFreeRef`), the
claims walks 24 % (`copy_claims` 10.7 %, `remove_claims_mode` 5.8 %, `owned_walk` 4.9 %,
`holds_no_heap` 2.6 %), the program's own code 11 %, vector reads 7.6 %, byte copies 4.9 %,
the Rust heap the walks allocate 4.8 %.  `decode` is 55 % inclusive (two calls), the rest
is the copy `pa_decode` makes, the entry walk, and the frees.

**The structure, per CBOR node** (`n_read_value`'s map arm, read off the emission): the
child's `Decoded` is built in a store of its own (`__ref_7` / `__ref_8`, one per key and
value), its `value` deep-copied into the parent's entry (`OpCopyRecord` → `copy_claims`,
recursing through the text or byte vector), the child's store cleared; the finished map
deep-copied into the frame's return buffer (copy 2, the whole tree walked again); `decode`
returns it; `pa_decode` copies `d.value` into a local (copy 3 — a workaround for loft#425,
which is CLOSED); `check_request` frees.  Every payload byte is copied three times and
every node's claims are walked three times and freed three times.  Eleven hidden buffers
per `read_value` frame are freed on each of its return paths (187 `OpFreeRef` sites), and
byte strings are pushed a byte at a time (`bs += [bytes[argpos + k] ?? 0]`, 2 ns a byte).

**The six structural problems**, most costly first:
1. *A heap-owning record that crosses a call boundary is a STORE.*  Each `read_value`
   answer mints an arena (header, free tree, footer) for a three-field record, and frees it
   after one read.  Twenty-six per check; the twin has zero.  The value-record rule
   (`(R-ValueRecord)`) declines every record with a vector or text field, so the whole
   decoder is outside it.
2. *Composition copies where Rust moves.*  `items += [sub.value]`, `Decoded { value: CMap
   {…} }` and `v = d.value` each deep-copy a tree between stores; a move would be a handle
   write.  The destination-directed build (§ Order item 6 — the callee builds its answer in
   the slot it will occupy) is the lever for both 1 and 2.
3. *The claims walks allocate.*  `owned_walk` pushes a child per FIELD into a `Vec` per
   record, scalars included, and `remove_claims` re-asks `holds_no_heap` per child.
4. *Byte ranges are pushed one at a time.*  The `(R-VecCopy)` shape with a range is one
   `append_bytes`.
5. *Frees are per variable, per return path.*  A frame that mints nothing still tests
   eleven buffers at every exit; a frame that did mint pays a store free each.
6. *In the library:* the loft#425 workaround copy is dead weight now, `pa_get` discharges
   each entry through a fresh record with two text fields (`entries[i] ?? CborEntry {…}`),
   and `pa_text` copies the text out (`"{value}"`) where a borrow would do.

**What 3× needs.**  3× is ≈ 300 ns a check here — at most two or three store cycles.  Items
2, 4, 3 and 6 take the row to an estimated 1.5 µs (≈ 15×): the copies and their walks go,
the byte loops become memcpys, the library stops copying.  The rest of the distance is
item 1 — records that own heap answered as VALUES (a `Decoded` crossing the call as a tuple
whose payload is a handle), so a decode allocates once for the tree, not once per node —
and that is a rule extension of `(R-ValueRecord)`, not a site fix.  The double decode is
the library's design and the twin's too; it does not move the ratio, only the absolute.

## The placement matrix (2026-09-29) — what a decoder's shapes need from `(R-Place)`

Built before any code (`tests/scripts/a-chain-exit-hands-the-placed-buffer-through.loft`),
hand-computed, stores counted per call with `LOFT_ALLOC_REPORT`.  The boundary today:

| shape | decision | stores/call |
|---|---|---:|
| p2 a literal-only callee's result into a literal field of a parameter's element | placed | 1 (the host literal) |
| p3 the same through a chain (`return mk(n)`) | placed since the chain clause (was: "not a fresh literal on every exit") | 1 (was 2) |
| p0/p1 the result appended WHOLE (`r.items += [v]`) | "the copy carries a flag" | 2 |
| w2 the wrapper's field copied out and returned (`v = d.value; return v`) | "a reference into the local leaves the receiver chain" | 3 |
| w1 the decoder loop (`sub = read(…)` in a loop, `items += [sub.value]`, `p = sub.next` after) | never a candidate: the bind is inside a loop and the host is a local | — |
| w4 recursion, w7 two heap fields, w9 a branch join, w3 an alias | not candidates / declined | — |

So the chain clause is landed and the decoder is still whole: what it needs next, in
order, is (a) a bind inside a loop with the per-iteration buffer, (b) a destination in a
local vector's store, (c) the placed thing being the ONE heap-owning field of a wrapper whose
scalars are read after the move (a field move that zeroes its source so the wrapper's free
finds nothing), and (d) the local vector returned inside the exit literal built in the
return buffer's store.  Each is an ownership argument with cells already in the matrix.

## `check_request` after the loop clause and the exit vector (2026-09-29)

Both clauses the matrix asked for are built (`(R-Place)`'s loop clause: w1/w9; `(R-ExitVector)`:
v1–v13, w1, w4, l3), and the cbor `read_value` is admitted at every site: `sub`, `kd`, `vd`
placed in the local vector's store and moved out by field; `bs`, `tb`, `items`, `entries`
claimed in the return buffer's store and taken by a handle move.  The tree is built in ONE
store — the outermost buffer's — and copied nowhere on the way up.

**Measured** (this box, `--native-release`):

| row | before | loop clause | + exit vector |
|---|---:|---:|---:|
| cbor bench `decode`, ms per op | 42.6 | 34.7 | 26.2 |
| stores per `check_request` | 26 (20.4 after the fixes since) | 20.4 | 7.4 |
| `check_request` driver, 40 rounds of 2 048 checks | 0.47 s | 0.39 s | 0.39 s |

**The A/B the census demanded** (`scripts/perf_check.py`, this box, the clauses off against
the clauses on over the seven libraries and two consumer lanes the census flagged): with both
clauses OFF `check_request` is 18 % slower (49.6× → 58.6×) and cbor's `decode` 17 % slower
(23.8× → 27.7×); no hex_* routine moved 15 % either way, so the native header and base
hoists the placed vectors lose cost nothing the bench can see.  Blessed in
`bench/portal/rewrite_census.tsv`.

So the decoder's own row moved 38 %, the store cycles per check fell to a third, and
`check_request`'s TIME did not move with the second clause.  The profile says why: what the
store mints cost is now paid by BLOCK claims and releases in the shared store —
`place_record_prefilled` 11 % (a best-fit search of the store's free tree per placement, then
the prefill), `free_record_in` 10.6 % (the claims walk, then a free-tree delete per block), and
the free-tree maintenance behind both (`claim_best_fit`, `fl_insert`, `fl_set_red`, `fl_balance`,
`delete`: ≈ 16 %).  A claim in a store is not cheaper than a store mint: both are an allocator
round-trip.  The other half of the profile is unchanged: `OpCopyRecord` 15 % (the library's
`v = d.value` copy in `pa_decode`, item 6), the claims walks 18 % (`remove_claims_mode`,
`owned_walk`, `holds_no_heap` — item 3, and `owned_walk` still allocates a `Vec` per record),
`read_value`'s own code 13 %.

**What this settles.**  Items 2 and part of 1 of the six are done — no copy composes the tree
and only two stores a decode remain (the `d` buffer and `pa_decode`'s copy).  The distance to 3×
is now in three places, each a runtime or library change rather than a rewrite:

1. *The arena's allocator.*  A tree built once and cleared whole pays a red-black free-tree
   claim per node and a delete per node at the clear.  A store whose content is one tree wants
   BUMP claims and a one-step reset (`(R-Header)`'s store-free list already resets a root
   vector's store in one step); the heap the texts own is the only thing a reset must still
   find.
2. *The claims walk.*  `owned_walk` allocates per record and `remove_claims` re-asks
   `holds_no_heap` per child (item 3): the same walk over a type table computed once would
   cost a fraction.
3. *The library.*  `pa_decode`'s copy (`v = d.value; return v` — the w2 shape, 15 % of the
   profile) and `pa_get`'s discharge (item 6).

The first two move every store-based program, not this row; the third is pluginabi's and is
this stream's to make.

**A side-finding the perf-check surfaced.**  hex_shape's `wall_chain_walk` died under the
exit vector with a corrupt reference: its `(mq, mr, md) = chain_marks(…)` locals carry
wrappers typed `main_vector<vector<integer>>` over `vector<integer>` (loft#1757), harmless
while a wrapper is a store released whole, fatal once released as a block by its type.  The
clause declines such a wrapper (54 sites in hex_shape); the typing itself stays open.

## `check_request` on the library AS WRITTEN — the copies and claims per check (2026-09-30)

Owner's rule (`(Perf-Teach)`): the row is measured on cbor and pluginabi exactly as they are
written, and the code generation earns the speed.  After the loop clause, the exit vector and
`(R-Header)`'s function clause, the check driver runs 0.38 s per 40 rounds (49.6× calibrated)
and the profile is these, in order:

| share | what | where it is decided |
|---:|---|---|
| 13.9 % | `OpCopyRecord` in the caller: `pa_text(pa_decode(frame), "op")` LIFTS the call result into `__lift_1`, and a lifted bind takes the adopt-or-copy protocol with an unconditional free of the passed buffer — so the first bind always deep-copies the tree.  A NAMED bind `z = f(…)` adopts the callee's store with a witnessed free (`OpFreeRefIfDistinct(__ref, z)`). | the lift's lowering in `scopes` (`new_lift_var` marks it `inline_ref`; the pairing never runs) and both backends' bind arms |
| 10.9 % | `free_record_in`: the placed buffers' and wrappers' releases (a claims walk that allocates, then a free-tree delete) | runtime |
| 10.3 % | `place_record_prefilled`: the placement's claim (a best-fit search once the store has frees) and the prefill of a record every exit literal rewrites whole | runtime; the prefill is `(R-CompleteWrite)`'s to skip |
| 6.9 % | `remove_claims_mode`: the per-turn `OpClear` of a placed buffer | runtime |
| 13.2 % | `read_value`'s own code | — |
| 5 % | `set_str` + `text_from_bytes_native`: the text payloads copied out of the frame (the cbor spans branch reads them in place; a text FIELD that borrows the frame is the language change the tree form cannot reach 3× without) | language |

The runtime copy census (`LOFT_COPY_DUMP=1`, one round): per check, one copy at
pluginabi:71 (`v = d.value` in `pa_decode`), one at :218 (the lift above), and 1.75 each at :86
(`r = e.value` in `pa_get`) and :94 (`pa_text`'s match arm) — six deep copies of a CborValue
tree per check, every one a compiler matter:

1. **A lifted call result adopts like a named bind** (`__lift_N = f(…, __ref_N)`): the same
   witnessed pairing a plain bind gets, so `f(g(x))` copies nothing.  −14 %.
2. **`(R-ReturnField)`**: `p = mk(…); return p.a` hands `p`'s store over at the field's
   position instead of minting a store and copying the field into it (`pa_decode`); cells
   written and hand-checked in the scratchpad (`return_field_cells.loft`, r1–r7, values hold on
   both backends today; 3 stores per call, 2 after).
3. **Arena buffers**: a store that holds one tree cleared whole (a lazy `__ref` result buffer)
   claims by bump and releases nothing until its root's `OpClear`, which RESETS it — the
   placement's claim, the block releases and the per-turn walk all go.  A runtime policy
   behind a switch; needs the fact that a record owns nothing outside its store.
4. **Skip the prefill** of a placed buffer whose callee writes every field on every exit
   (`(R-CompleteWrite)` already proves that for a mint).
5. **`pa_get`'s and `pa_text`'s match copies**: a `match` on a view's variant binding a field
   copies the payload; a read-only arm wants a view.
6. **Text payloads as spans of the frame** — the language change.

Beside the row, measured as a CEILING and kept in scratch (`cbor.patch`, `pluginabi.patch`):
a reader that skips instead of decoding puts the row at 5.3× against a scanning twin, 1.97×
against the tree twin.  It is not the answer; it is what the compiler is measured against.

## Built (2026-09-30) — a lifted call result adopts like a named bind

Item 1 of the list above.  `pa_text(pa_decode(frame), "op")` lifts the inner call into a
`__lift_1` temp, and the temp is null-initialised in the function prologue (`lift_vars`) — so
its one bind read as a REBIND, and a rebind of a record from a callee whose return carries its
buffer's dep (`pa_decode` returns the local it copied out of `d`) takes the copy protocol on
both backends: a store minted for the copy, `OpCopyRecord` over the whole tree, the callee's
store freed.  `d = pa_decode(frame); pa_text(d, "op")` never paid that: a named first bind
adopts the callee's minted store (`(O-Move)`, @PLN164 B1) with the buffer's free guarded by
identity.

The fix is one home for both spellings: `Scopes::lift_set` binds every record-lift temp, marks
its bind after the prologue's null-init as its first (`deferred_first_bind`, the fact both
backends' bind arms already read) and pairs the call's buffer through `pair_call_buffers` —
`scan_set`'s pairing block, extracted byte-identically — so the buffer's free is
`OpFreeRefIfDistinct(__ref_1, __lift_1)` exactly as the named bind's is.  The stdlib's one such
site (`exists` lifting `file(path)`) moves with it.

**Measured** (this box, `--native-release`, the check driver's 40 rounds of 2 048 checks, seven
interleaved runs each): 0.38 s → 0.34 s, −11 % (the profile's `OpCopyRecord` share was 13.9 %).
Per cell of the guard (`tests/scripts/a-lifted-call-result-adopts-like-a-named-bind.loft`,
`LOFT_ALLOC_REPORT=1`, stores per call): the lifted form 3 → 2 (the named control's 2), a lift in
a loop of three 9 → 4, two lifts in one call 6 → 4, the early-return shape 3 → 2; a fresh
(dep-empty) callee and a lift inside a `??` block were already at the named cost.
`tests/lift_adopt.rs` pins the count on both backends, the IR shape, and the switch
(`LOFT_NO_ADOPT_FIRST_BIND=1` restores the copy).  Falsified by sabotage: without the mark the
copy is back and the loop cell answers wrong on `--native`; without the pairing every check
stays green (the buffer holds the null sentinel today) and only the IR pin reads.

What remains of the list is unchanged in order: `(R-ReturnField)` for `pa_decode`'s own copy
(`v = d.value; return v`), the arena buffer, the placed buffer's prefill, the match copies, and
the text-as-span language change.

## Built (2026-09-30) — `(R-ReturnField)`: the returned field of an owned local hands its store over

Item 2 of the list.  `pa_decode`'s `v = d.value; return v` was a `materialized_view_return`
exit: the return buffer minted, the CborValue tree deep-copied into it, `d` freed — the
loft#425 workaround's copy, 15 % of the profile in `OpCopyRecord`.  `return_field.rs` (run in
the scope pass after the exit vector) rewrites that exit into a hand-over when the copy's
source is a field path rooted at an owned record local (or a local view of one), the returned
record owns heap, and everything between the copy and the return is a free: the mint, the copy
and the root's own free go, the exit returns the field's address into the root's store, and
every other store free in the exit is re-witnessed against the root — the buffer a pooled loop
root adopted (r12) and an alias's owner (r5) are skipped, a distinct buffer is freed as before.
The caller is not consulted: it adopts with the witnessed free or copies with the source freed
exactly as for any buffer-carrying return.

**Measured** (this box, `--native-release`, the check driver's 40 rounds, seven interleaved
runs): 0.34 s → 0.33 s, on top of item 1's 0.38 → 0.34.  In the guard's cells
(`LOFT_ALLOC_REPORT=1` / `LOFT_COPY_DUMP=1`, stores and copies per call): `return p.a` 1
copy → 0; the struct-enum wrapper (r9, `pa_decode`'s shape) 4 stores → 2; a pooled caller loop
(r7) stays at 1 store, its copy gone.  Reach: seven corpus files (the return-field guards of
`85-*` and `h9`, one nested-literal root in `h12`), no stdlib function.  The `avoidable-copy`
advice on `return p.b` beside a second exit (r6) goes silent with the copy.

What remains, in order: the arena buffer (item 3), the placed buffer's prefill (item 4), the
match copies in `pa_get`/`pa_text` (item 5), text payloads as spans (item 6).

## Built (2026-09-30) — the lazy-free phase (item 3) and the walk's enum arm

Item 3 of the list, as a runtime policy of every store rather than a buffer kind: a store
starts in a LAZY phase (`(H-LazyFree)`, `LOFT_NO_LAZY_FREE=1`) in which a delete of a small
block (at most 64 words: a record, a text) merges with its neighbours as ever and leaves the
result out of the tree (a block ending the store becomes the wilderness; a larger block — a
rung, a table — is tracked as ever), and a claim takes the tail; the first claim once the untracked words reach 256 and a
fifth of the extent written sweeps once (`coalesce_free` + `fl_rebuild`) and ends the phase.
A store bound to a file never enters it: the first run's six reds were the layout guards of
persisted and paged stores (a reclaim that trimmed nothing because claims had landed on the
tail, a keyed lookup five bytes over its page budget, a read-repeat census that differed by the
scratch blocks) — the phase is for the store that is built and released, not the one that is
kept.  And the perf-check's one SLOWER row set the trigger: with the sweep gated on the
wilderness running out, `mesh_emit` (a pooled buffer reset per round, a vector ladder per
round) placed every rung tail-first in a wilderness that always held it — +24 %, the process
at 10 MB against 5.9 — so the sweep now fires at the claim's entry on the dead-words bound
alone, and the row reads −2 %.
The decoder's store — nine placed buffers and the vector rungs freed per decode, the store
released whole — never sweeps, so its claims never leave the tail and its deletes cost one
header write.  Beside it,
`holds_no_heap` learned the struct-enum field: a `Decoded` whose value moved out answered "may
hold heap" and walked (allocating a `Vec` per record) to find nothing.

**Measured** (this box, `--native-release`, the check driver's 40 rounds, three runs each,
same binary, the switch as the OFF arm):

| build | check_request, 40 rounds | what moved |
|---|---:|---|
| before (1c010450) | 0.318 s | — |
| the enum arm | 0.30 s (−6 %) | `remove_claims_mode` 4.0 → 2.3 %, `owned_walk` 3.3 → 1.6 %; the walks left are real teardowns (a truncated frame's partial map) |
| + the phase | 0.286 s (−6 %, −10 % in all) | `claim_best_fit`, `fl_insert`, `fl_set_red`, `fl_delete_node`, `delete`'s merge: 27 % of the samples → the tree ops gone, `claim_block` + `set_free_header` stay |

**What the profile reads now** (share of the row): `read_value`'s own code 20 %, the placed
buffer's claim and prefill (`claim_block`, `set_free_header`, `place_record_prefilled`,
`set_default_value_nullable`, `prefill_from_image`, `enum_parent_size`) 12 %, the remaining
walks 11 %, `store_mut` 4 %, the vector header and push path 6 %, the text payload copy 3 %.

**Side-findings, each a codegen matter and not this unit's:**
- A UNIT variant written into an enum field (`Decoded { value: CNull, … }`) mints a store
  (`OpDatabaseNP`), writes one tag byte into it, deep-copies it into the field (`OpCopyRecord`)
  and frees the store — on every `ok: false` exit of `read_value`.  A tag write in place is the
  whole job.
- `decode`'s `d = read_value(bytes, 0); …; return d` beside a literal exit orphans the
  caller's buffer every call (`free #5 name=__shared_dest_orphan`): two store cycles per
  check that place nothing.  The bind could take the caller's buffer as its own `__ref` when
  the literal exit's fields read from `d` before the write and `d`'s heap is released first.
- `owned_walk` still builds a child per FIELD, scalars included, into a `Vec` per record; the
  enum arm removed most of its callers on this row, not the allocation.

What remains of the list: the placed buffer's prefill (item 4), the match copies in
`pa_get`/`pa_text` (item 5), text payloads as spans (item 6).

**Order from here (owner, 2026-09-30): prevent objects before making them cheaper.**  The
lazy phase and the enum arm stay; they make a store's lifecycle cheaper, and that work comes
back later.  But for a record the program never needed, a cheaper free only speeds up what
the compiler should not have made.  So the next units remove objects, in this order:

1. **A heap-owning record crossing a call as a value** (`(R-ValueRecord)` extended to records
   with one heap field): `Decoded { value, next, ok }` answered as a tuple whose payload is a
   handle, so a decode claims once for the tree and not once per node.  This is the "item 1" of
   the six structural problems and the only lever the ledger prices at 3×.
2. **A unit variant written into an enum field in place**: one tag byte, where today each
   `ok: false` exit mints a store, copies it into the field and frees it.
3. **`decode`'s orphaned caller buffer**: two store cycles per check that place nothing.
4. **The match copies in `pa_get` / `pa_text`**: a read-only arm binds a view.

Store efficiency (the placed buffer's prefill, `owned_walk`'s per-field `Vec`) resumes once
these stop producing the objects it would speed up.

## Built (2026-10-02) — a decoder's texts are no objects, its byte strings one copy

Measured first on cbor as written (the per-check store census, `LOFT_STORE_CENSUS` on an
`op-census` build): 4 stores, 46 claims and 26 deletes a check, no deep copies left.  Priced by
hand on a scratch copy before anything was built (`loft-optimize` step 4): reading each text
off the frame with `text_from_byte_range` instead of through a `tb` vector 3 558 → 2 525 ns a
check (−29 %); the byte strings as one slice on top, 2 349 ns.  A `Decoded` returned as a tuple
was SLOWER (7 812 ns) — so item 1 of the order above is a design, not a respelling.

Both forms are now the compiler's, on the library as written: `(R-ByteCopy)`'s vector clause
(a byte run copied one at a time is one guarded slice append) and `(R-TextRun)` (a byte run
read once as text is never built), and the function header of `bytes` kept beside the slice
(`(R-Header)`'s function clause had read the slice as a write).  `check_request` 3.62 →
2.26 µs (−38 %, interleaved on one core), past the hand-written forms' 2.35.  Reach outside cbor: the vector clause fires wherever a
byte vector variable is copied by index (pluginabi's bench); `(R-TextRun)` is cbor's alone in
the bench body today.  An `integer` or `float` run (`members[st + m] ?? 0` in Moros,
`?? 0.0` in hex_fit) keeps its loop: its elements can hold the null the `??` replaces, so it
needs an append that substitutes the default — the next clause, priced on those rows first.

## `mat4_mul` and `check_request` re-profiled (2026-10-02) — what is left is store traffic

**`mat4_mul`** (mesh3d, 23.0× Rust after `(H-SwapIn)` and `(R-RepeatRun)`).  A probe running
only `mo_c = mat4_mul(mo_a, mo_c)` 10⁶ times: 213 ns a call, `n_mat4_mul`'s own code 33 % —
the other two thirds is store work done on every call.  The caller hands the call a NULL
return buffer each round (`__ref_5` is never assigned), so the callee creates a store, claims
a 16-element vector, fills it; the caller then resets `mo_c`'s store (`OpDatabase`), exchanges
the two, and frees the released one.  Priced by hand-editing the emitted Rust (same output,
`taskset`, three interleaved runs each):

| form | time | |
|---|--:|---|
| as emitted | 0.19 s | |
| A: exchange without resetting `mo_c` first, free the released store | 0.15 s | −21 % |
| C: keep the released store as the next round's buffer, reset it | 0.14 s | −26 % |
| B: keep it, and the callee refills the vector the buffer already holds | **0.07 s** | **−63 %** |

So the prize is B, double-buffering: no store created or freed per round, and the literal
`Mat4 { m: [16 × 0.0] }` written into a buffer of the same type refilling its existing vector
in place instead of zeroing the field (which orphans the old vector) and claiming a new one.
Both halves are needed: C shows the store reuse alone buys a third of it.

**`check_request`** (pluginabi, 22.6×).  87 % of a check is cbor's `read_value`, called twice
per frame (`pa_decode_ok` reads only `.ok`, `pa_decode` the value).  By leaf, under
`check_request`: the record allocator (claim, best fit, free list, prefill, delete) ~28 %; a
heap `String` made and freed for every decoded text (`malloc`/`free`, `from_utf8`,
`text_from_bytes_range`) ~11 %; store lookups (`store_mut`) 6.5 %; `holds_no_heap` 4.6 %
(already the fast path that skips the release walk); the decoder's own code ~15 %.  The levers,
largest first: the first decode builds a whole tree only to read `.ok` (about half the work,
a demand-driven specialisation, not priced); the per-text `String` round trip (write the text
into the store from the byte range directly, runtime, both backends); then the per-record claims.

## `check_request` after `(R-PureReuse)` and `(M-Match)` (2026-10-02) — where data moves, objects are made and freed

`pa_decode_ok` and `pa_decode` decoded the same frame twice; `(R-PureReuse)` computes it once,
and `(M-Match)` stops `match pa_get(m, k) { … }` from calling its subject once per arm.  Per
check, native: 4 stores / 30 claims / 10 deletes → 2 / 16 / 5; `check_request` 22.7× → 12.4×
Rust (`make worst`, one quiet run).  The other worst rows did not move with this step beyond
noise — the census agrees: `(R-PureReuse)` fires only in pluginabi's bench, the match fix only
where a subject is a call.  Ratios swing ±30–40 % run to run where the Rust lane moves
(`build_vis` 12.7× → 19.3× with its native time flat), so read under 15 % as no change.

**One decode, by frame** (`LOFT_STORE_CENSUS`, `op-census` build, native):

| frame | stores | claims | deletes | grows | bytes moved |
|---|--:|--:|--:|--:|--:|
| empty map | 1 | 2 | 1 | 0 | 0 |
| one text → text entry | 1 | 7 | 3 | 0 | 32 |
| one text → bytes(64) entry | 1 | 8 | 4 | 1 | 32 |
| the request (3 entries) | 1 | 13 | 5 | 1 | 96 |

Attributed to `read_value`'s IR: the base is the result `Decoded` and the entries vector; the
first entry adds the two `Decoded` temporaries of the key and value sub-calls (placed once,
reused after), the key text, the value payload and the entry; every 32 bytes moved is a
`CborValue` moved out of a temporary into the entry.  `pa_get` adds a store and 3 claims: a
copy of the matched value that `pa_text` reads once.

| per check | count | needed? |
|---|---|---|
| the decoded tree's store, root, entries vector, three entries | 1 store, 5 claims | yes |
| six payloads copied out of the frame | 6 claims, ~100 bytes | yes, unless a text shares the frame's bytes |
| `kd` / `vd` temporaries | 2 claims, 5 deletes, 96 bytes moved | no — the value can be built in the entry's slot |
| `pa_get`'s copy of the found value | 1 store, 3 claims | no if the caller only reads it while the map lives (an ownership question, the owner's call) |

**Next, in order:** (1) destination-passing — `read_value` writes a sub-value straight into the
place its caller names (the entry's `key` / `value` field), no temporary and no move; it is in
cbor, so `decode` (13.4×) and `encode_bytes` (22.1×, now the worst row) are in its reach;
(2) `pa_get` read in place; (3) the per-claim cost (`set_default_value_nullable`,
`holds_no_heap` ~5–6 % each) where a literal writes every field.  Estimated 1+2 → ~0.7 µs a
check (~7×); under 3× (~300 ns) also needs the payload copies gone.  Price each by hand on
the emitted Rust before building it.

**Side-finding:** `make rewrite-census` is red on `main` on the aarch64 box — `R-ExitVector`
and `R-Header` read 1–3 lower than `bench/portal/rewrite_census.tsv` in cbor and every hex_*
program, with the library commits equal to the baseline's.  Same on this branch, so not its
change; not yet explained.
