<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 164 — Activation arenas, adopt-at-bind and build-in-place

Tracker: [@PLN164](https://github.com/loft-lang/plans/issues/164) · `status:active` ·
`subject:store-lifetime`.

## Status (REQUIRED)

Active (the owner's go, 2026-09-15).  **P0 done**, **B1, C4, B2, C3, C1 and C2 shipped** (B2's caller side and
C2 both a wash on the parse row, structural gain only — § B2 and § C2 *Measured*) — the
measurements are under § P0 below and the mechanism under § B1.  What P0 changed in the
plan: tier 1's ceiling is ~10 % of the parse row, not a fifth, and its store-identity cost
(287 `store_nr !=` sites in one emission) is real, so A1/A2 stay behind B and C in the
queue and the owner's E1 pick is still open.  B1 took the minimal sound shape — adopt the
callee's minted store, keep the caller's buffer null — and the matrix found the reason the
wider one (reuse the buffer across activations, E7) must wait: the interpreter's rebind of a
promoted buffer local frees a caller-supplied buffer where native guards it (§ B1b).
Routes here per the docs-vs-plans rule: the mechanisms belong to `LIFETIME.md` /
`OWNERSHIP_MODEL.md` / `formal/ownership.md` once shipped; this directory is the design and
the matrix until then.

## Goal (REQUIRED)

**The principle this plan serves (owner, 2026-09-15; GOALS.md § Goal F):** the
programmer is not assumed to know the store model inside, so the most NATURAL form of a
program is its CANONICAL form and therefore the one the compiler optimises — not an idiom
the author learns from a survival guide.  **And the scoping (owner, same day):** this is
not "optimise everything" — each spelling is first asked *is this natural to write?*, the
mechanism goes to the spellings that pass, and a contrived spelling keeps today's correct
copy and is deferred with its row kept.  Most cells of the matrix below may be deferred
at any moment; the natural ones are where the gain sits.  **And the licence (owner,
2026-09-15, C122, `(R-Escape)`):** the contract is semantics, not representation — a
rewrite needs its validated conditions and nothing else; the one boundary is a library API,
whose callers are unseen, and a construction that does not escape it may be rewritten in
any way its conditions allow.  The compiler removes the per-call temporaries a record-returning style mints — a store
per hidden buffer per activation, a deep copy at a first bind, at a last-use field
assignment and at an element overwrite, a copy for a read-only `?`-discharge — without a
line of the consumer's code changing, and the drawing library's `parse` row goes from
7.6× the Rust reference toward 3× as the measurement of it.

## Effort + design

- **Effort:** M (tiers 1–2) · MH (tier 3)
- **Design:** ~ — the invariants are named; the store-identity question (§ Edge cases E1)
  is open and decides tier 1's shape.
- **Last touched:** 2026-09-16 (C2 shipped, and then measured on the bench: the rewrite has no
  site in the consumer, so the parse row's points class stays with C5 — § C2 *Shipped*)

## The evaluation — what one parsed line costs, and why

The owner's premise, stated 2026-09-15: *knowing this fact of loft I would look at the
parse implementation to make fewer objects; but what I really want is a compiler that
determines this problem itself and makes efficient code for it.*  So the library
(`loft-libs-graphics`, `drawing/src/drawing.loft`, `parse_poly` and its callees) is the
FIXED input; it is written in the natural style — build a record, return it, put it in a
field — and the question is what the compiler mints around that style.

A census of the emitted Rust (`--native-emit` of the bench's `parse_only.loft`, counting
store mints, record mints, deep copies and frees per function and attributing each to its
loft line) says, for ONE `Poly` line:

| temporary | mechanism that mints it | needed by the program? |
|---|---|---:|
| 4 stores at `parse_poly` entry (`__ref_3..6`) | every record-returning callee gets a caller-minted return buffer, one STORE each, freed at exit | no — dead at exit |
| a store for `pp_raw` + a deep copy of the `PointList` (4 vectors) | the first bind of a call result copies: `OpBindOrCopy` adopts only a store the local already holds, and a fresh local holds none (`O-Buffer`) | no — the buffer could be adopted |
| a store per `?`-discharged record element (`ap_e = sc.elems[idx]?` in `acc_pts`) + a copy of the `Elem`, its `ename` text included | `?` on a record element materialises a record; `Elem` owns text, so § V-ad's all-scalar buffer does not apply | no — only read, field by field |
| a store for `Elem {…}` + a copy into `sc.elems[idx]` | an indexed overwrite from a literal builds in a temp and copies | no — the slot exists |
| a deep copy of `Paint` (with its `spec` vector) into the `Op` | `paint: pp_paint` copies at `pp_paint`'s last use | no — a move would do |
| the points vector copied into `Op.pts` and again into `Mark.pts` | `smooth_pts` builds in its own buffer; each destination copies | one copy at most |
| a default prefill of every minted record | a partial literal defaults the rest field by field (`set_default_value_nullable`) | no — a per-type image is one block write |

Per line that is a dozen store mint/free pairs, four deep copies and three copies of the
points, on a scene of thirteen lines.  The `Op` itself is ALREADY built in place in the
scene's store (`@FR-R-Mint`/`@FR-R-PushRec`), and the `Mark` already lands in a buffer
`parse_scene_at` mints once for the whole scene and hands down — so two of the objects
already live in a longer-lived container; everything below `parse_poly` does not.

The profile agrees (program samples only, 20 000 calls, this box):

| class | share | routines |
|---|---:|---|
| record and vector lifecycle | ≈ 57 % | `copy_claims` 8, `set_default_value_nullable` 7, `begin_write_inner` 4, `remove_claims_mode` 3.5, `store_mut` 4.7, `free_named` 3, `claim` 2.7, `Store::init` 1.8, `store_budget` 1.8, `close_file_handle` 1.2, `owned_walk` 1.4 … |
| the library's byte scan | ≈ 19 % | `matches_at`, `find_option` (rescans the line per key) — the library's own |
| the parse logic | ≈ 8 % | `parse_scene_at`, `acc_pts`, `fronds`, the trig |

## The three tiers — the invariant each rests on

Each tier is a situation the compiler PROVES (C120: no rewrite may change a value after a
fault; only situations we know), switchable and falsifiable in the @PLN157 style, and
lands on BOTH backends where it changes the IR (the interpreter is the values oracle, and
`O-NoDiverge` says the two translate the same `deps` facts).

**Tier 1 — the activation arena.**  *Invariant:* a hidden return buffer (`__ref_N`, a
`__ref_p2_N` discharge buffer, a literal's temp) never outlives the activation that minted
it (`O-Buffer`: the buffer is the caller's store, freed at frame exit; a result that must
survive is adopted, copied or moved OUT of it).  So every buffer of an activation may be a
RECORD in one store instead of a store each, allocated past a mark taken at entry and
released to that mark at every exit.  The store is the CALLER's, threaded down like the
buffer itself is today (`R-Callee`: "a return buffer may be a record the caller offered"),
so a loop such as `parse_scene_at`'s line loop resets one arena per iteration — the
compiler can see nothing minted since the loop head survives except what was appended to
`sc`.  Removes the store mint/free pair per temporary: `Store::init`, `database_named`,
`store_budget`, `close_file_handle`, `owned_walk`, `free_named` — about a fifth of the row.
*What it must keep:* every rule keyed on STORE IDENTITY (§ Edge cases E1).

**Tier 2 — adopt at first bind, move at last use.**  *Invariant:* `O-Move` — a returned
heap value's ownership transfers to the caller's binding.  Today `OpBindOrCopy` honours it
only when the local already holds a store; the first bind copies.  Adopting the buffer at
the first bind makes iteration one what iteration two already is.  And `B-Copy`'s copy at
`paint: pp_paint` is a copy of a value nobody reads again: when the ownership oracle says
the local OWNS its store and liveness says it is dead on every path after the assignment,
the field takes the record by MOVE (bytes relocate, heap handles never change store —
`@FR-R-MoveAppend`'s relocation, applied to one record).  Removes the deep-copy class, about
a sixth of the row.

**Tier 3 — build where it will live.**  *Invariant:* a value whose ONLY destination is
known at the site it is built may be built THERE (`@FR-R-ElemFirst` states it for a vector
consumed by one append; this generalises the destination).  Three shapes: an indexed
overwrite from a literal writes the slot's fields (the old element's owned heap released
first, a partial literal's omitted fields set to their defaults); a call whose result is
stored exactly once gets the destination slot as its return buffer, so `smooth_pts` fills
`Op.pts` directly; a `?`-discharged record element that is only read is a VIEW (`B-View`),
not a copy.  The per-type prefill image belongs here.  Removes the remaining copies and
most of the prefill.  The one copy that stays: a vector stored in TWO destinations
(`Op.pts` and `Mark.pts`) — a record field owns its data, and that is the right answer.

## P0 — what one activation's buffers cost, measured (2026-09-15, this x86-64 box)

The parse bench (`parse_only.loft --n 2000`, `--native-release`, hash `33f6d2b8`) read
**50.5–54.8 k ns/op** before any change.  Its emission (`--native-emit`) carries **287**
`store_nr !=` and **39** `store_nr ==` tests — the sites E1 would touch — 37 `OpDatabase`
+ 45 `OpDatabaseNP` mints and 26 `OpCopyRecord` copies.  The runtime census
(`LOFT_TRACE_DB=1`, two parses) puts the store mints per parse at **~88**: Paint 19, Mark
19, `vector<Pt>` 18, `vector<float>` 16, PointList 6, Elem 6, Sketch 2.

**What a mint/free pair costs.**  A vector declared inside a loop re-mints its store per
iteration under `LOFT_NO_LOOP_BUFFER_REUSE=1` and keeps it by default (§ V-al), so the A/B on
one program prices the pair: **28 → 88 ns/op, ≈ 60 ns** for the smallest buffer (a mint, a
first claim, a free).  Perf on that probe splits the 60 ns about evenly between the
store-specific half (`op_database_inner`, `Store::init`, `Stores::clear`, the zeroing
memset) and the buffer's first claim (`claim`, `claim_block`, `finish_claim`,
`pre_alloc_vector`), which an arena RECORD pays too.

**So tier 1's ceiling is 88 × 60 ns ≈ 5.3 µs ≈ 10 % of the row**, half of that the
store-specific part an arena or a pooled store can remove; the plan's "a fifth" counted the
profile's lifecycle routines, which include per-access costs (`store_mut`, `begin_write`)
that no store discipline changes.  The hand patch of `parse_poly`'s four eager buffers is
therefore not worth building: those are 3 of 4 unused per activation (one `smooth_pts` path
runs), 12 pairs per parse, ≈ 0.7 µs — the arithmetic answers it.  Verdict for the queue:
**B (the copies, 18.5 % inclusive) and C (the prefill and the discharge copies) before A**,
and for E1 the pooled-store option is the one to price first when A1 is cut, because it
keeps identity as it is.  The leak-gate extension (E19) waits for A1's shape.

## B1 — adopt at first bind (SHIPPED 2026-09-15, `@FR-O-Move`)

*The two protocols, side by side.*  `a = mk_lit(i)` (every return a literal, deps `[]`)
and `b = mk_loc(i)` (`o = P { … }; …; o`, the local promoted onto the buffer, deps
`["o"]`) lowered differently: `a` adopts a buffer the caller mints once and reuses
(`OpFreeRefIfDistinct(a, __ref_1)`); `b`'s caller minted a SECOND store and deep-copied
(`OpCopyRefOrNull` / the `_dst`/`_src` delivery), freeing the callee's — two mints, a copy
and two frees per call where the callee's one mint would do.  The parse row's `pp_raw =
read_points(s)` (a `PointList` of four vectors) and the bench's `bs_sk = parse_scene(…)`
(the whole `Sketch`) are that shape.

*The rule.*  `use_analysis::adopts_minted_at_bind` — ONE home for the three readers — admits
a direct call to a loft-defined callee whose return deps name exactly its hidden return
buffer attribute, bound to a plain local (not a parameter — a promoted local is one — not a
caller-hidden buffer, not handed to the call as its buffer).  `scopes::scan_set` then strips
the local's deps (it OWNS the minted store) and pairs it with the call's buffer for the
identity-guarded free the fresh-adopting shape already takes; the interpreter's first-bind
arm delivers by `OpPutRef` and native's dispatch takes the plain assignment.  A rebind keeps
the in-place copy on both backends.  Switch `LOFT_NO_ADOPT_FIRST_BIND=1` (parse time).

*What the matrix found.*  `bytecode-comparisons/B1-adopt-first-bind-cells.loft` (c1–c17,
hand-computed, both backends under `LOFT_STRICT_STORES`, `LOFT_POISON`, `LOFT_POISON_CLAIM`
and the leak gate) was green before the change and RED after the first cut on the
interpreter alone: c6, plan 51 cluster 3's `render_lit_then_call`.  The pairing enrolled the
buffer in `reuse_record_buffers` (the entry-time pool, `@FR-R-Reuse`), so the callee received
it non-null, its promoted local's literal built into it, and the rebind `cv = alloc_canvas(…)`
FREED it — the interpreter's reassignment path has no twin of native's `_rb_w_` witness — after
which the caller's next `Q` took the recycled slot and `p.tag` read it.  The callee's IR is
identical before and after; the free was a no-op on a null buffer.  So B1 excludes its
pairings from the pool (`Scopes::minted_pairs`), the buffer stays null, and the callee mints
per call: **139 → 108 mints** over the cells on the interpreter, 137 → 106 on native.

*Receipts.*  Guard `tests/scripts/164-adopt-first-bind.loft` (its `@falsified-at:` is the
pool sabotage above), pins `tests/adopt_first_bind.rs`, the ten plan-51 guards green on
both backends under both switch states.

*Measured.*  The parse bench (`parse_only.loft --n 2000`, `--native-release`, this quiet
x86-64 box, hash `33f6d2b8` throughout): **50.5–54.8 k → 43.3–44.0 k ns/op**, about
−14 % on the row — the `PointList` copy at every `read_points` bind and the whole-`Sketch`
copy at the bench's own `bs_sk = parse_scene(…)` are the binds it removes.  The consumer
table (`compare.py`, 14 rows) is re-measured when the next unit lands, per the plan's
phase 5.

**B1b — reuse the buffer across activations for this callee shape** (E7's steady state, 0
mints per call): blocked on the interpreter's reassignment path freeing a caller-supplied
buffer held by a promoted buffer local (`143`'s shape); native guards that free with its
entry-time `_rb_w_<buffer>` witness (loft#1126).  Closing the divergence is the whole of
B1b, and c6 under the pool without the exclusion is its falsifier.

## C4 — the prefill image (SHIPPED 2026-09-15, `@FR-R-Prefill`)

*The mechanism.*  `Stores::set_default_value_nullable`'s `Shape::Record` arm walked the
type's fields on every mint that is not proven complete-write — a store resolution per
field, a recursion per inline record, a typed setter per sentinel — for bytes that are the
same on every mint of a type (7 % of the parse row).  The arm now asks
`prefill_from_image` first: the type row carries a `PrefillImage` cell beside its
`TypeFacts`; the first `Absent::Prefill` of a COMPLETE type zeroes the span, runs the walk
once and reads the span back as the image; every later mint is one `Store::write_image`
(a bounds-checked block copy, shadow-tagged as `zero_range` is).  The image is what the
walk writes BY CONSTRUCTION — it is never computed a second way — and
`LOFT_PREFILL_VERIFY=1` is the falsifier: the walk re-run after every image write, a panic
naming the type where the bytes disagree.  Declines, each keeping the walk: a type with a
field not laid out yet (`u16::MAX` content — `layout_complete` walks the inline record
tree; an image captured then would freeze the field's zero where the walk, once the field
is laid out, writes its sentinel), an image whose length no longer matches the layout, and
`Absent::Final` (the `text as Struct` fill: declared defaults and interned text are not a
fixed byte pattern).  A table rollback forgets the image with the facts.  The all-zero
fast path (`heap_facts(tp).1` → `zero_range`) stays in front of it.  One runtime home, so
the interpreter, `--native` and wasm take it together.  Switch `LOFT_NO_PREFILL_IMAGE=1`.

*What the matrix found.*  The prefill is VALUE-INVISIBLE in every shape the cells reach:
a literal writes each omitted field's default explicitly (`to_default`, loft#914), a
`?`-discharge of an absent element answers `construct_default(T)` (LOFT.md § `?`:
`points[i]?` → `Point{}`, the declared default and the inline record's defaults included
— c8 measured it equal to a bare `Mix {}` on every binary), and the `text as` fill is
`Final`.  So the value channel cannot move under a wrong image; the receipt for "no
value changes" is the VERIFY census: all 1432 `tests/scripts` files on the interpreter
(the one panic is `75-native-stub`'s `@EXPECT_FAIL`), the parse bench on native, and the
cells on both backends under `LOFT_PREFILL_VERIFY`, `LOFT_POISON`, `LOFT_POISON_CLAIM`,
`LOFT_STRICT_STORES` and the leak gate.  Two side-findings, neither C4's: a NEGATIVE
declared default (`n: integer = -1`) was dropped by a `text as Struct` cast because
loft#876's fold matched plain literals and `-1` is `OpMinSingleInt(1)` — fixed in
`typedef::fold_declared_default` with guard
`tests/scripts/a-negative-declared-default-survives-a-cast.loft`; and a `single`-typed
declared default (`a: single = 1.5`) writes 0 in a struct LITERAL on the interpreter and
fails native compilation (`set_single` handed an `f64`), on the pre-B1 binary too — to be
filed (`hit-by:loft`, `silent-wrong`), repro in the next session's notes.  And a doc
inconsistency for C3: `formal/binding.md` `(B-View)`'s discharge clause says "an absent
element discharges to null" where LOFT.md § `?` and every backend answer the type's
default record; the rule text is the one to correct when C3 is cut.

*Measured.*  The parse bench (`parse_only.loft --n 2000`, `--native-release`, hash
`33f6d2b8`, interleaved A/B on one binary): **46.4–48.5 k (`LOFT_NO_PREFILL_IMAGE=1`) →
42.3–42.8 k ns/op** (≈ −11 %).  The 14-row `compare.py` table is re-measured when B2
lands.

*Receipt and gate (the next session).*  The sabotage receipt: `prefill_from_image` made to
capture the image BEFORE the walk (an all-zero image), and under `LOFT_PREFILL_VERIFY=1`
both backends panic at the first image USE — but only after cell c11 was added: on native
every `Mix` literal is a complete-write mint (`OpDatabaseNP`) and `mk`'s buffer is minted
once per activation of its caller, so c1–c10 CAPTURE the image on native and never use it
(the native half of the receipt was vacuous, which `LOFT_TRACE_PREFILL=1` now makes
visible: it names each capture and each use, and `tests/prefill_image.rs` pins a use on
BOTH backends — 2368 / 15 uses of `Mix`, interpret / native).  The guard is
`tests/scripts/164-prefill-image.loft` with the panic text as its `@falsified-at:`; the
five pins pass; fmt and both clippy legs are green.  The negative-default guard's
`make falsify` against `7fd2a664` could not run on this box (`/tmp` is a 7.4 GB RAM tmpfs
and the control build needs more; point `LOFT_FALSIFY_CACHE` at a disk path) — its receipt
stays the hand measurement recorded in the file.

## B2 — the result built where it will live (SHIPPED 2026-09-16, `@FR-R-Place`, `@FR-R-MoveLast`)

*What the instrument found first.*  `loft introspect` on the B2 cells' `read_paint` (four
literal exits, the library's shape) showed the callee answering a DIFFERENT store per
exit: the three mid-body `return Paint { … }` each minted a `__ref_p2_N` work-ref store
and returned it, and only the tail literal wrote the `__retbuf` the caller handed
(@PLN157 § V's `BuildIntoBuffer` rewrote the tail alone).  `(R-Place)` declines exactly
that callee — *on some exit answers a store other than the buffer it was handed* — so the
row's own callee could not be placed until the callee clause held.

*Unit 1 — every literal exit writes the handed buffer (SHIPPED).*
`Parser::literal_exits_into_buffer`, run on the function body right AFTER the tail's
delivery is dispatched: when the buffer is still the unpromoted `__retbuf` and every
mid-body exit is a fresh literal of its type, each `return S { … }` takes the tail's own
rewrite (`build_into_return_buffer`: the literal's `OpDatabase` behind the "caller offered
a record" guard, its writes on `__retbuf`, the work-ref `skip_free`).  It is decided after
the tail and not at the `return` because the first cut — a mirror inside `parse_return` —
ran BEFORE the tail promoted a local onto the buffer, so `mk_mix`'s early literal and its
promoted local `o` shared one buffer and the caller freed a stale ref (the corpus's
`164-adopt-first-bind` c3, a `BUG (#306)` stack-store free refusal).  A function with any
non-literal exit keeps its per-exit stores.  Switch `LOFT_NO_LITERAL_EXIT_BUFFER=1`; pins
`tests/literal_exit_buffer.rs` (the IR shape on `read_paint`: 4 of 4 exits on `__retbuf`,
the switch restores 1 + 3, the promoted-local shape declined, the cells on both backends).
No value moves and, under B1's null buffer, no store count moves: the unit changes the
callee's CONTRACT, which is what units 2–3 consume.

*Units 2–3 — the caller side (SHIPPED 2026-09-16).*  `src/place_result.rs`, one IR pass
for both backends, run inside `scopes::check` after the scan phases have settled the
function's frees (E18).  For a plain local `v` bound once from a loft-defined callee whose
every exit is a fresh literal into its `__retbuf` (unit 1's contract), read only as the
receiver of native operations while it holds the record, and stored ONCE per path into a
record-literal field of an `_elm_` element `OpNewRecord` appended to a PARAMETER's
collection, three sites move: `__ref_N = null` → `OpPlaceRecord(X, tp)` (a record claimed in
`X`'s store, prefilled through the C4 image), `OpCopyRecord(v, dst, tp)` → `OpMoveRecord(v,
dst, tp)` (the bytes relocate, the heap handles keep their claims, the source's block is
released on the spot), and the exit pair → one `OpFreeRecordIn(v, tp)` on a path that still
holds the record and NOTHING on a path that stored it.  The ops are `#rust` templates
(`default/01_code.loft`); the runtime is `Stores::place_record_prefilled`, `move_record_out`
and `free_record_in` (with a store-root arm for the null-host fallback, which answers a
fresh-store buffer).  Switch `LOFT_NO_PLACE_RESULT=1`; `LOFT_TRACE_PLACE=1` names every
admission and decline.

*What the matrix found — four things the design did not say.*  (1) The candidate gate is
not B1's: `adopts_minted_at_bind` admits the promoted-local callee shape; an all-literal
callee (`read_paint`) takes the older fresh-adopting pairing, so the pass gates on its own
`placeable_bind` and asks the callee's exits directly.  (2) An exit has THREE spellings —
`return { Object …; __retbuf }`, `{ Object …; return __retbuf }` (a function with text
work-refs frees them between the writes and the return), and the parser's bare tail
`{ Object …; __retbuf }` before the scope pass wraps it (what the copy notice's preview
sees) — and the local's exit free has two
as well: `OpFreeRef(v)`, or in a record-returning caller `OpFreeRefIfDistinct(v, __retbuf)`
(the library's `parse_poly` answers a `Mark`; the first cut declined it).  (3) The design's
"`v = null` after the move" is unsound: the interpreter lowers a rebind of an owned record
local as a store-level free of what it held — it would free the HOST — where native emits a
plain null; and the first cut's zeroing of the source was a runtime stand-in for the path
state (the owner's question: why zero a source once zero-on-claim is gone?).  So the pass
records the state at every exit and writes the free that is right there, and a path that
stored the record rejoining one that holds it declines; `(R-MoveLast)`'s mechanics clause
now says so.  (4) The copy notice reads the parser's IR on the program path (the lint family
runs before the scope check there and after it under `loft test` — a pre-existing difference
the dead-store lint's tests pin, so it stays), and reported a relocated field as a copy that
"could not be moved"; it now asks the pass for its verdict in preview
(`place_result::admits`, the same walk on the parser's IR, tolerant of the null init the
scope pass has not yet prepended).

*Receipts.*  Cells b1–b15 exact on both backends under `LOFT_POISON`, `LOFT_POISON_CLAIM`,
`LOFT_STRICT_STORES`, `LOFT_HOIST_VERIFY` and `LOFT_NATIVE_LEAK_CHECK`; every intended decline
fires for its own reason (`LOFT_TRACE_PLACE`); the census 184 → 50 mints, one fewer per
admitted call (`add_poly` ×64, `add_px` ×30 no-heap, `add_card` ×40 text-owning).  Guard
`tests/scripts/164-place-result.loft`, `@falsified-at:` the pass made to free on every exit
(both backends die with `Store access out of bounds … the reference is corrupt`).  Pins
`tests/place_result.rs`: the IR shape of `add_poly`, the switch, the silent copy notice, the
census.  The corpus (`tests/scripts` + `tests/docs`) has NO admitted bind — the shape is the
library's, not the corpus's — so the guard is the only coverage.
The gate's one red, the browser kernel differential, was a pre-existing dangling `Data`
pointer in `wasm::resume_frame` that the changed bytecode made visible (loft#1541, fixed in
the same arc with `State::rebind_data`).

*Measured.*  The parse row (`parse_only.loft --n 2000`, `--native-release`, hash `33f6d2b8`,
this x86-64 box, 2026-09-16): `pp_paint` admitted (three moves, one held exit), and a WASH.
Wall time on this box now spreads ±10 % between runs of one binary, so the receipt is
`perf stat -r 5` on the cached binaries: user instructions 8.39 G against 8.39 G, cycles
3.18 G against 3.23 G, cache misses 6.09 M against 6.11 M (placement / switch off), every
difference inside its own run-to-run spread.  The reason is the scene: no gradient is used,
so the `Paint` relocated carries no `spec` data — the copy it replaces was ~30 bytes and the
store it removes is matched by a claim and a delete in the host store.  What B2 buys on this
row is structural (no per-call store, no per-call copy), not time; the copy class the
profile measured at 18.5 % is the points (`pts: smooth_pts(…)`, `Mark.pts`), which are C2
and C5.  A leak-probe build (the placed block leaked instead of deleted) measured the same,
so the host store's free-tree churn is not a cost either on this row.

## C3 — the read-only `?`-discharge (SHIPPED 2026-09-16, `@FR-B-Disturb`, `@FR-B-View`)

*The premise did not survive the matrix, and that is the result.*  C3 was cut to make
`ap_e = sc.elems[idx]?` a VIEW instead of a copy.  It already is one, on both backends:
the bind carries the base's dep (`ap_e(1):ref(Elem)["sc"]`), no copy op is emitted, and
the `__ref_p2_N` store is minted only on the ABSENT arm — which `acc_pts` never takes,
because `elem_index` appends the element before `acc_pts` is called.  A store census of
the shape (`LOFT_TRACE_DB=1`, the library's `acc_pts` reproduced against the same `Elem`)
mints exactly THREE stores: the `Sketch`, the points vector, and the `Elem` temp of
`sc.elems[idx] = Elem { … }`.  Thirteen discharge shapes were measured — off a parameter,
off a local, a hash lookup, a nullable field, an all-scalar element, an element with a
vector field, a struct-enum element, a `&`-based base, a `const` bind, a nested field, a
rebind in a loop, an explicit `?? default`, a plain undischarged bind — and every one is
already a view.  The only copies the sweep found are the two that are RIGHT: a
materialise under `(B-Disturb)`, and the return of a view as a value.

So the evaluation table's row *"a store per `?`-discharged record element + a copy of the
`Elem`"* is the SAME store and copy as its own next row, *"a store for `Elem {…}` + a copy
into `sc.elems[idx]`"*, counted twice.  That row is C1's, and C1 is where the `acc_pts`
temporary actually is.

*What C3 turned out to be.*  Its own condition, as the rewrite list states it: *"`sc.elems`
not disturbed between bind and last read, **in this frame or any callee**"*.  The view
existed; the condition was only half-checked.  `ViewWalk::disturb` asserted `(B-Disturb)`
at one site but computed its input from THIS frame's ops alone, so a view live across a
call kept reading an address its elements had left:

| shape | inline | through a callee |
|---|---:|---:|
| `e = sc.els[0]?; grow(sc, 200); e.a + e.b` | 3 | **4294967401** |
| the same with two appends (no reallocation) | 3 | 3 |
| `len(e.nm)` after the growth | 2 | **1** |
| `e = sc.els[2]?; remove_first(sc); e.a` | 30 | **40** |

Both backends, identical — the divergence is IR-level.  One shape with two meanings,
decided by which side of a call the append sits on, and by an allocation the author cannot
see.  `(B-Ref-Reshape)` already states the reach — *"the disturbance may be in this frame
or in anything the frame CALLS … at any depth"* — and `(B-View)` keys its materialise on
the same four events, so the rules settled it and only the implementation was short.

*The mechanism.*  `disturbed_params_map`: the container places each definition GROWS or
REMOVES FROM through a visible parameter, closed over the call graph by the same worklist
`removed_params_map` uses, unioned into `ViewWalk::disturb` at every call site.  Read
through the same two producers the inline walk uses, so the `OpClearVector` subtraction
comes with them and a callee that REBUILDS the field it was handed disturbs nothing,
exactly as that statement written inline does.  The forward edge does not ask whether the
parameter is spelled `&`: a plain heap parameter aliases the caller's container identically
(`calls.md` F-ParamHeap, probe 40 cell X9), so keying on the spelling would let an author
lose the materialise by taking loft's own `warn_redundant_amp` advice.  Switch
`LOFT_NO_CALLEE_DISTURB=1`, trace `LOFT_TRACE_DISTURB=1`.

*What the matrix found that the design did not say.*  (1) A binding that names a container
WHOLE (`d = &cv.data`) resolves to the same place as one that names an element inside it
(`e = sc.els[i]?`), because the place model carries one field offset — but a growth moves
every ELEMENT while it merely repoints the field SLOT the first one re-reads.  Shaking it
broke `157-view-header`'s `grown_between` (11 → 0).  `view_source_place_indexed` answers,
off the same walk that answers the place, whether the chain crossed an element read.
(2) The notice had to name the CALLEE: nothing in `c_grow_callee` appends to `sc`, and a
reader given only *"`sc` grows"* goes looking for a statement that is not in the function —
the same failure the `Grown` sentence was split off from `Reshaped` to avoid.
(3) A hidden RETURN BUFFER is an argument slot, so without excluding it every
record-returning function that fills a vector field reports a disturbance at each of its
call sites.

*Receipts.*  `tests/scripts/164-callee-disturb.loft`, fifteen PAIRS — each callee cell
beside the same program written inline, because the inline path has implemented
`(B-Disturb)` since loft#1373 and is therefore the file's own oracle — with hand-computed
absolute values beside them, clean on both backends under `LOFT_POISON`,
`LOFT_POISON_CLAIM`, `LOFT_STRICT_STORES` and `LOFT_NATIVE_LEAK_CHECK`.  Five cells are
CONTROLS that must keep aliasing and prove it by writing through the view and reading the
container back.  `@falsified-at:` is measured, not claimed: under the switch seven cells
move, the same seven on both backends.  Pins `tests/callee_disturb.rs`.

*The two honest costs.*  c7 — a removal BELOW the viewed element — now materialises where
today's alias answers correctly, because `(B-Disturb)` ends the place a removal renumbers
whatever happened to sit where, and because the same program written inline has answered
that way since @PLN130 F2.  That is probe 38 cell C1's objection, and it is a behaviour
change with no wrong answer behind it.  And the `&`-link twin `link_inline` reads `0` where
`11` is due — a `&` link silently downgraded to a copy, which `(B-Ref-Reshape)` says loft
will not do.  It is pre-existing (it reads `0` on `main` and under this unit's switch
alike), it is a `&` question rather than a plain-view one, and it is FILED rather than
widened into here.

*Measured on the parse row:* nothing.  C3 removes no copy, because there was none to
remove — what it removes is a silent wrong answer.  The row's copies are C1, C2 and C5.

*The cost it does have is COMPILE time, and it was worth checking.*  `disturbed_params_map`
walks every definition once, and `scopes::check` runs per FILE LOAD, so a program with several
`use`d libraries runs it again over an ever-larger definition table — the shape that would be
quadratic.  A/B on one binary (`LOFT_NO_CALLEE_DISTURB`, three runs each): a stdlib-only script
72 ms against 70, `07-control-flow` 57 against 57, and the two library-heavy corpus files
145 ms against 145 and 144 against 142 — inside the run-to-run spread at the size where it would
have shown.  The whole-program map is built once per `check` rather than re-derived per call
site, which is what keeps it there.

## C1 — the element overwritten from a literal (step 1 SHIPPED 2026-09-16, `@FR-R-InPlaceLiteral`)

*Re-measured first, per C3's lesson, and the row is real this time.*  `sc.elems[idx] = Elem {
… }` mints a `__ref_p2_N` store, builds the literal in it and `OpCopyRecord`s it into the
slot — confirmed on the `acc_pts` shape and on four element shapes (full literal, partial
literal, all-scalar element, element with a vector field).  The store census of the library
shape mints exactly three stores and this is the third.

*And the re-measurement found the phase is half built already.*  `(R-InPlaceLiteral)` names
two destinations, an element `v[i] = R { … }` and a field `o.f = R { … }`, and the FIELD one
has taken the in-place road since long before this plan: its writes take the place as their
receiver, it mints no store, and it already writes the declared default of every omitted
field.  Only the ELEMENT destination copies.  So C1's optimisation is narrower than the rule
reads.

**Step 1 — the staging clause, which was broken (SHIPPED).**  That shipped in-place road
staged nothing, so an initialiser read storage an earlier field's write had already replaced:

| shape | got | want |
|---|---:|---:|
| `o.f = El { a: o.f.b, b: o.f.a }` over `{a:1, b:2}` | **2,2** | 2,1 |
| the same with the second field read through a call | **2,2** | 2,1 |

Both backends, in silence.  The ELEMENT twin answers correctly — because it copies — which is
what makes it the oracle and what makes this the prerequisite: redirecting elements to the
place FIRST would have propagated the defect to them.

#330's hoist is exactly the cure and already existed; it only ever armed for a WHOLE-VARIABLE
rebind (`v = S { … }`), because `in_place_var` doubles as the retry path's target and that is
meaningful only for a variable.  `parse_object` now takes a separate hoist root from a
PROJECTION destination too, read through `projection_container_place`.  The root carries the
destination's field OFFSET and `Parser::reads_place` spares a read of that variable at a
DIFFERENT field, so `tw.a = El { a: tw.b.a }` takes no temp; the spare is taken only on proof,
because sparing wrongly costs the value and staging wrongly costs a stack temp.

*The one conservatism, stated because it is a choice.*  Only `OpGetField` proves disjointness.
A field read also arrives through the typed accessors (`bx.tag` is `OpGetText(bx, 28)`), whose
second argument is an offset for some ops and a SIZE for others, so separating them needs a
list of ops that reads offsets — and such a list drifts silently against a new op, which is
PERFORMANCE.md § Design P8's measured failure mode for the five mutation deny-lists.  So a
sibling read spelled with a typed accessor stages a temp it does not need.  It is bounded by
the literal's own field count, it is pinned either way so a later narrowing is deliberate, and
closing it wants the field's declared SPAN rather than another list.

*Receipts.*  `tests/scripts/164-in-place-literal.loft`, eighteen cells hand-computed from the
declarations, clean on both backends under `LOFT_POISON`, `LOFT_POISON_CLAIM`,
`LOFT_STRICT_STORES` and `LOFT_NATIVE_LEAK_CHECK`; `@falsified-at:` measured by disabling the
projection arm — c1 and c8 go red on both backends and every other cell is unmoved, which is
what says the arm stages the initialisers that read the place rather than staging everything.
Pins `tests/in_place_literal.rs` hold the IR shape.

**Step 2 — the element destination written in place (SHIPPED 2026-09-16).**  The gate was one
op name.  `parse_object` builds into whatever `code` it is handed — the probe shows it receiving
`Call(OpGetField)` for a field and `Call(OpGetVector)` for an element alike — and the arm that
mints a work-ref instead asked `is_field(code)`, which is `Call(OpGetField)` and nothing else.
`Parser::builds_into_element` admits the element place; switch `LOFT_NO_ELEMENT_IN_PLACE=1`.

*Four declines, and every one was bought by a measurement rather than by the design.*  (1) The receiver is emitted ONCE PER FIELD
WRITE, so the place must be re-derivable with no effect the program can see: a repeatable base
and an index that is a LITERAL or a BARE VARIABLE.  `v[bump()]` would call `bump` once per
field; `v[len(v) - 2]` would re-read a length the writes may have moved.  (2) The index is read
from the END of the argument list — `OpGetVector(base, size, index)` and `OpVectorRef(base,
index)` put it at different positions, and a fixed slot read the SIZE as the index and declined
every `OpVectorRef` place by accident.  (3) A COLLECTION field declines the whole type, and this
is the one that was MEASURED rather than reasoned: the staging clause binds a field expression
to a temp, and what that bind MEANS depends on the type — `(B-Copy)` copies a text, so a
literal reading the slot's own text is safe, while `(B-View-Base)` makes a collection projection
a VIEW.  In place, `v[i] = S { c: e.c }` staged a view of the slot's own vector, the write
released that vector before storing the handle back to it, and the field read `0` on BOTH
backends.  Deciding it per FIELD needs the self-read answer, which is not known until the fields
are parsed and the road is already taken; deciding it by TYPE is decidable up front.  Coarse,
and the safe direction — and the shape this phase exists for keeps the road, because
`acc_pts`'s `Elem` is text and scalars.

(4) A member of a LINKED COLLECTION GROUP declines, and the CORPUS bought this one — the cells
did not reach it.  `@FR-Col-Group` makes two collections over one element type two ROUTES to a
single record set, so an element write owes them the unlink-and-relink that `group_elem_write`
wraps the copy with; written straight into the slot the whole of it is skipped and `by_k` goes
on holding the record under the hash of its OLD key, which is loft#900's defect by another road.
`a-group-element-written-through-the-vector-member-reaches-every-member` is what caught it, and
its own header had already written the sentence.  The predicate is new
(`is_grouped_vector_elem`) rather than the existing `vector_group_elem_site`, because that one
recognises `OpVectorRef` alone — the site it serves sees nothing else — while this admission also
allows `OpGetVector`, so reusing it would have let the other spelling through.

*The asymmetry between the two lists is deliberate:* the ADMISSION names two element ops, the
DECLINE names four including both nullable spellings.  A miss in the admission costs the
optimisation; a miss in the decline costs the group's agreement.

*And `reads_place` grew a clause for it:* a read of a variable whose DEPS name the root counts
as a read of the place.  `e = sc.els[i]?; sc.els[i] = El { a: e.b, … }` reads the very slot the
literal overwrites, under another name — `acc_pts`'s own shape — so without it step 1's swap
would have come back one spelling over, on the very road step 2 opens.

*Measured:* the `acc_pts` shape's store census **3 → 2**, which is the `Elem` temp the
evaluation table charged to the `?`-discharge and C3 showed belongs here.  The row's remaining
copies are the POINTS, which are C2 and C5.

*Receipts.*  The guard reaches 25 cells, clean on both backends under every falsifier; all four
declines are cells, because a decline that silently stopped declining is the expensive
direction.  Step 2's falsifier is STRUCTURAL and the pins carry it — with the switch every cell
still passes, since the copy is correct, so what the switch costs is a store and a deep copy
that no value can see.  The pin asserts BOTH element spellings, having first passed vacuously by
asserting one.

## C2 — the destination as return buffer (SHIPPED 2026-09-16, `@FR-R-Place`, `@FR-O-Buffer`)

*Re-measured first, and this row IS real* — unlike C3's.  `h.v = mkints(n)` on an EXISTING
field place emits, per assignment:

```
__ref_1(1):vector<integer> = null;                        // the callee's buffer, a store of its own
__p154_rhs_1(1) = n_mkints(n(0), __ref_1(1));             // the callee fills it
OpClearVector(OpGetField(h(0), 8i32, 24i32));             // the destination is emptied
OpAppendVector(OpGetField(h(0), 8i32, 24i32), __p154_rhs_1(1), 0i32);   // every element copied in
OpFreeRef(__ref_1(1));                                    // the buffer freed
```

and what it should emit is

```
__ref_1(1):vector<integer>["h"] = OpGetField(h(0), 8i32, 24i32);   // skip_free, inline_ref
n_mkints(n(0), __ref_1(1));
```

— a store, an element-by-element copy, a free and a temp gone per assignment.  The caller's
clear is subsumed: the callee's own FIRST op is `OpClearVector(o)`.

**It is a pure-IR rewrite, and a first reading of this section said otherwise.**  That reading
assumed the buffer must become a place-valued ARGUMENT, which would break the dep model — the
call site mints the buffer as `Value::Var(vr)` and gives the result `Deps::frame1(vr)`, and B1's
adopt, the delivery arms and the scope pass's free sweep all key on that variable.  The buffer
does not have to stop being a variable.  Only what it HOLDS changes: the destination's DbRef
instead of a store of its own.  Every downstream reader is untouched, and the one real edit is
re-pointing the variable's deps from `Deps::none()` (owned) to the destination's base, plus
`skip_free` / `inline_ref` — the pattern `group_elem_write` already uses for its `found` temp.

What licenses it is the CALLEE's side: `n_mkints(n, o)` only ever does `OpClearVector(o)`,
`OpPreAllocVector(o, …)`, `OpPushInt(o, …)`, `return o` — it never mints a store for `o`, and
the caller already emits those same ops against a field DbRef today, so a field place is a valid
buffer.

⚠ **But that is a property of SOME callees, not of the lowering, and a first cut read it as the
lowering.**  Three callees were sampled and all three filled the buffer they were handed, which
made `(R-Place)`'s *"a callee that may hand back a store it did not mint"* look vacuous.  It is
not: a callee returning a vector LITERAL lowers to `OpDatabase(__vdb_1)` on its buffer
PARAMETER — it MINTS into the buffer — so handed the destination it mints over the place, and
the write lands in a record the destination does not name.  The corpus found it
(`1152-a-vector-value-into-a-group-reaches-every-member`, a refusal from loft#810's guard:
*"record N claims size 0 … freed or never written"*).  The admission therefore READS THE CALLEE
and declines a body that mints into any argument slot — the positive form of the rule's decline,
as B2 unit 1 is for records.  Generalising from three samples is the error, and the rule had
already written the answer down.

### The restrictions, re-derived against the measurements

`(R-Place)` lists five declines.  Measured against what the vector lowering actually emits, they
do not all survive as written, and saying which is the point of this section:

| the rule's decline | verdict, measured |
|---|---|
| an ARGUMENT reaches the destination's store | **essential, and true by construction** — `grow`'s first op is `OpClearVector(o)`, before any read of `v`, so `h.v = grow(h.v)` would read an emptied vector |
| a path READS the result after the store | **not needed for this clause.**  It belongs to the other `(R-Place)` clause — a buffer claimed in S and then relocated, where the source is moved-from.  Here the result NAMES the destination, so a read of it reads exactly what was written |
| a callee that may hand back a store it did NOT mint (`O-Opaque`) | **survives, and the first reading of this row was wrong.**  `passthrough` copies its parameter INTO the buffer and answers the buffer, which made the decline look vacuous across three sampled callees — but a callee returning a vector LITERAL mints into its buffer parameter (`OpDatabase(__vdb_1)`) and would mint over the destination.  Implemented as a positive admission that READS the callee's body |
| a callee that on some exit answers a store OTHER than the buffer | **already true for every callee measured** — `two_exits`' early `return [99]` still clears and appends into `o` and answers `o`.  This is the contract B2 unit 1 had to CREATE for records; the vector lowering establishes it already.  Kept as a cheap ASSERTED check, never re-derived |
| a `?` / `??` discharge on the result | **structurally excluded** — the right-hand side is an `ncc` BLOCK, not a `Call`, so the admission never sees it |

And two the rule does not list.  A destination that is a member of a LINKED COLLECTION GROUP is
**not** a decline here, where it was in C1: the maintenance is `OpClearKeyed(g.by_x)` …
`OpIndexGroup(g.es, g.by_x)` BRACKETING the fill, and a call sits inside that bracket, which
C1's in-place literal had nowhere to hang.  It needed
`group_reindex_after_vector_write` to learn the shape — it keyed on an `OpAppendVector` naming
the field, and this rewrite emits none, so the vector filled and the keyed view stayed empty
(`3,0,0` for `3,3,4`).  And a field of a struct-ENUM VARIANT **is** a decline: it lowers through
a variant check whose other arm is the null sentinel, so the place exists only if the enum holds
that variant and does not unconditionally EXIST as `(R-Place)` requires.

### Where the rules are short — `(O-Buffer)`

`(O-Buffer)` says a hidden return buffer IS THE CALLER'S STORE, freed at frame exit, with the
result's free guarded by store identity against it *"which, no static bit can say
(`O-Opaque`)"*.  A buffer that IS the destination place is not a store, is not the caller's to
free, and outlives the frame.  The two rules describe different objects.

This is **not** an open design call.  `(R-Place)` is the newer rule, written for this plan, and
it already decides it — *"the buffer IS the place and nothing moves"*; `(O-Buffer)` predates it
and is simply short.  So `(O-Buffer)` gains the clause rather than `(R-Place)` giving way, and it
is recorded as a change to a SHIPPED rule's text, which is the kind that gets said out loud.

### What it needs

A `LOFT_NO_BUFFER_IS_PLACE` switch, one aliasing test, a positive admission (direct call,
loft-defined, hidden vector buffer, every exit answers it), the group bracketing preserved, and
the 11-cell oracle in `bytecode-comparisons/C2-buffer-is-the-place-cells.loft` — whose values are
TODAY's answers and which C2 may not move.

### Shipped — and what the bench then measured

*Mechanism.*  `Parser::buffer_is_the_place` (`src/parser/expressions.rs`) re-points the buffer
VARIABLE at the destination — `inline_ref` plus `skip_free`, the way `group_elem_write` re-points
its own temp — where the right-hand side is a direct call to a loft-defined callee with a hidden
vector buffer, the destination place exists, no argument of the call reaches it, every exit
answers the buffer and no exit MINTS into it; `group_reindex_after_vector_write` keeps a grouped
destination's maintenance around the fill.  Switch `LOFT_NO_BUFFER_IS_PLACE=1` (parse time, both
backends).  Guard `tests/scripts/164-buffer-is-the-place.loft` (11 cells, the two corpus-bought
declines among them), structural pins `tests/buffer_is_place.rs`, registered under the `scopes`
subject.

*Measured, and the measurement is a FINDING rather than a confirmation.*  The row is real where
it is spelled: on `h.v = mkints(n)` the rewrite removes a store, an element-by-element copy, a
free and a temp per assignment.  **The parse bench spells it nowhere.**  `--native-emit` of
`bench/parse_only.loft` is byte-identical with the switch on and off (25 686 lines either way,
under `LOFT_NO_CACHE=1` too, with `parse_poly` and `smooth_pts` in the emission), and the row is
where B1 left it: 41.8–44.8 k ns/op, hash `33f6d2b8`.  The naturalness question — § How a verdict
is reached, question 2, which this cut asked of a probe of its own construction instead of the
corpus — answers the same way: the 15 library files of `loft-libs-graphics` carry **13**
`place = call(…)` sites and not one of them has a VECTOR result (twelve scalar, one `text`).

*So the row the rewrite list credits to C2 is a different DESTINATION.*  `pts: smooth_pts(…)` is
a record-LITERAL field of an element being appended, reached through a local
(`pp_pts = smooth_pts(…)`) — B2's `place_result` shape with a vector instead of a record — and it
declines today because `pp_pts` has TWO owning destinations, `Op.pts` and the returned `Mark.pts`.
C5 removes the second one; only then is there ONE destination to place into.  C2 therefore ships
with its row credited honestly: **the rewrite for the spelling it admits, structural, zero sites
in the consumer measured so far** — and the parse row's points class stays with C5 and the
vector `place_result` that follows it.

## The rewrite list — the natural `parse_poly` to its optimal form

**This is not about how a programmer writes loft.**  The programmer writes the natural
`parse_poly` as it stands in the library; the version below is the SPECIFICATION of what
the compiler must reach from it, written by reading the two side by side and asking, for
every difference, what fact on the IR licenses the rewrite and which rule says so.  The
rules were written first (2026-09-15, `formal/rewrites.md` § *A result is built where it
will live…*, `formal/ownership.md` `(O-ViewField)`, `formal/binding.md` `(B-View)`), so a
question met while building a phase is answered there and not decided in the code.

*The target, per `Poly` line:* one `Op` record created in the scene's store, its two
vectors grown in place, nothing else minted, nothing copied, and the `Mark` a value
tuple whose `pts` is a view of the op's own vector.  In hand-written form:

```loft
fn parse_poly(sc: Sketch, s: text, raw: PointList) -> Mark {     // raw: the caller's scratch
  read_points_into(raw, s);
  pk = read_paint_kind(s);
  if len(raw.pts) < (if pk == Stroked { 2 } else { 3 }) { return Mark { matched: true, bad: true } }
  sc.ops += [Op { kind: Stroke }];          // the one record, built where it lives (R-Mint)
  o = sc.ops[len(sc.ops) - 1];              // a view (B-View)
  read_paint_into(o.paint, s);              // the field is the buffer (R-Place)
  if pk != Stroked { o.kind = Fill; smooth_into(o.pts, raw.pts, raw.smooth, true); }
  else {
    o.w = read_width(s, 3); o.color = read_stroke_colour(s);
    smooth_into(o.pts, raw.pts, raw.smooth, false);
    if raw.any_width { /* widths appended into o.widths; smoothed in place */ }
  }
  Mark { matched: true, bad: false, pts: o.pts }   // pts a VIEW LEAF (O-ViewField), Mark in registers
}
```

What the compiler cannot do is the one thing a programmer could: change the contract
(`Mark` carrying an op index instead of the points, or `parse_poly` growing the element's
box itself).  `(O-ViewField)` reaches the same cost with the contract as written, which
is the whole point of Goal F.

| rewrite (per `Poly` line) | IR fact to establish | licence | exists today | phase |
|---|---|---|---|---|
| bind `pp_raw`, `pp_paint` without a copy | the callee's return deps name exactly its own buffer; the destination a plain local | `O-Move`, `O-Buffer` | shipped | B1 |
| `pp_paint`'s buffer claimed in the scene's store; `paint: pp_paint` a relocation, not a deep copy | the result's ONE owning destination is a field of a record in store S on every path that keeps it; `pp_paint` owned and dead on every path after the literal | `R-Place`, `R-MoveLast`, `O-Complete` | the liveness exists as the `avoidable-copy` lint's *"still used after this point"*; codegen does not read it; a buffer claimed in another store is new | B2 |
| `pts: smooth_pts(…)` fills `Op.pts` directly, no buffer | as above, with the destination place existing at the call and no argument reaching it; the callee writes only its buffer and answers it at every exit | `R-Place` ("the buffer IS the place"), `R-Callee`, E15, E16 | `retbuf_only_writer`; `R-ElemFirst` already builds a vector inside an appended element; C2 shipped the EXISTING-place road, which this is not — this destination is a literal field reached through a local, so it is a vector `place_result` and it declines on `pp_pts`'s second destination | C5, then a vector `place_result` |
| `sc.elems[idx] = Elem{…}` written into the slot | the slot exists; every field expression evaluated before the first write (`ename: ap_e.ename` reads the slot); omitted fields defaulted | `R-InPlaceLiteral`, E13, E14 | the FIELD destination already writes in place and defaults the omitted fields; E13's staging shipped as C1 step 1; the ELEMENT receiver is what is left | C1 step 2 |
| `ap_e = sc.elems[idx]?` as a view | `ap_e` only read; `sc.elems` not disturbed between bind and last read, in this frame or any callee | `B-View` (the discharge clause), `B-Disturb` | the VIEW already; the disturbance walk was this frame only, which is what C3 closed | shipped C3 |
| `Op { kind: Stroke, … }` prefilled by one block write | the literal's field set against the type's defaults | `R-Prefill` | complete-write has the set; the per-type image is missing | C4 |
| the four `smooth_pts` buffers not minted at entry | one path runs one call | trivial | goes away with C2 | with C2 |
| the `PointList` scratch reused across lines | the callee's literal rewrites every field when handed a live buffer; the buffer-holder's rebind never frees it | `R-Reuse`, § V-y | D-own-43 (the interpreter's rebind free) must close first | B1b, A2 |
| the points written once: `Mark.pts` a view of `Op.pts` | the field's source has an owning destination in `sc` (outlives the frame); every call site only reads it before any disturbance of `sc.ops` | `O-ViewField`, `R-ValueRecord`'s view leaf | no — the rule is new; § V-aa's site fixpoint is the home to extend | C5 |

*What each phase must build,* in the @PLN157 shape: a `LOFT_NO_<unit>` switch, cells with
hand-computed values on both backends under `LOFT_STRICT_STORES`, `LOFT_POISON`,
`LOFT_POISON_CLAIM` and the leak gate, a guard with its `@falsified-at:` receipt, pins, the
subject registered.  Each rule's decline list is its falsifier list: a cell per decline,
proving the copy still runs there.

*Analyses the phases share* (each wants ONE home, read by the scope pass and both
backends, the loft#810 discipline B1 followed): the def-use classes of a heap value (its
owning destinations against its read-only uses — `R-ElemFirst`'s "consumed exactly once"
generalised); per-path liveness at a set; the disturbance walk between two points for a
named container; destination existence and aliasing against a call's arguments; the
callee writer summary; the return shape over all call sites.

## Composition matrix — Stage A (REQUIRED)

The axes the tiers touch, each with the domain the language offers; every phase's cells
are drawn from these, hand-computed, run on both backends under `LOFT_STRICT_STORES=1`,
`LOFT_POISON=1`, `LOFT_POISON_CLAIM=1` and `LOFT_NATIVE_LEAK_CHECK=1`:

| axis | domain |
|---|---|
| callee kind | user fn · fn-ref (`O-Opaque`: empty deps) · `#rust` native · generic instance · twin (`__inv`) |
| result shape | all-scalar (a § V-aa value record) · record with vectors · record with text · nullable record · struct-enum · tuple |
| binding | first bind · rebind in a loop · a value-branch join (`O-Complete`) · a `?? default` discharge · `return f(…)` (§ V-an's phantom) |
| activation | plain · recursive · generator (`yield` inside) · `par` arm · a closure body |
| exit | fall-through · early `return` · `break`/`continue` · a fault (`LOFT_DEV_SOFT_HALT`) |
| destination | fresh local · existing local · record field · vector element (append / overwrite) · the function's own result · two destinations |
| aliasing | none · the source IS the destination's old value (`ename: ap_e.ename`) · a `&` view live across the write (`B-Disturb`) |

`python3 scripts/matrix_axes.py file <cells>` measures which values a cell corpus reaches;
`cross activation destination` is the pair the corpus must cross, since that is where the
arena and the placement meet.

## How a verdict is reached — the five questions, in order

The owner reviews the PROCEDURE, not twenty tastes (2026-09-15).  Each row is put to
these questions in order, and the first one that decides, decides:

1. **Spelling, or mechanism property?**  A mechanism property (an identity, a mark
   discipline, a gate's accounting) is not natural or contrived — it is an invariant the
   tier must hold for whatever it admits, or the tier is not built.  Its verdict is a
   falsifier and a named failure mode, nothing else.
2. **Is the spelling natural?  MEASURED, not judged.**  Two facts: does the consumer
   corpus write it (a grep over the 15 library files of `loft-libs-graphics`, and the
   games and the crawler when the row matters), and would a programmer write it without
   knowing the store model?  Absent from the corpus and explainable only by the store
   model → contrived → keep today's copy, defer the row.  Present → natural → continue.
   The corpus is the oracle for "natural" the way the interpreter is the oracle for a
   value: a spelling absent today may appear, and then the row is re-measured, never
   re-argued.  Measured 2026-09-15: a record `?`-discharge bound to a local (E17) 45
   times; a field assigned from a local (E9) 382; a bind-then-return (E20) 96; an
   element overwritten from a literal (E13) 3; a record fn with two or more literal
   returns (E15) 6; `x = f(x, …)` on a RECORD or vector (E16) 0 — its 6 hits are scalar
   `min`; a `yield` statement (E3) 0; a `par` block (E4) 0; an `OpDrop` type (E12) 0.
3. **Does a written rule already answer it?**  `formal/*.md` first (the debugging
   policy's *read the formal spec first*).  A rule that answers settles the row and the
   mechanism implements the rule exactly, no narrower, no wider.  A question no rule can
   express means the rule wants extending — the OWNER's design call, recorded under
   § Open design questions; the mechanism never decides it silently.
4. **What falsifies the verdict?**  A cell with a hand-computed value on both backends
   under `LOFT_STRICT_STORES`, `LOFT_POISON`, `LOFT_POISON_CLAIM` and the leak gate, and
   a soft-halt note count where a fault path exists.  A verdict with no falsifier is not
   a verdict; it is the plan-51 shape.
5. **What does being wrong cost?**  Admitting wrongly is silent-wrong, a leak or a double
   free — the freeze-axis class.  Declining wrongly is a missed optimisation the compiler
   pays itself (Goal F).  The asymmetry decides every doubt: DECLINE, and the decline is
   always available.

*Where each row was decided.*  At 1: E1, E2, E5, E6, E18, E19.  At 2 (contrived, deferred):
E3, E4, E12, E16.  At 3 by an existing rule: E7 (`O-Buffer`'s steady state), E8
(`O-Opaque`: empty deps cannot license an adopt), E9 (`O-Complete`: per path), E10
(`O-Borrow`: not owned, no move), E11 (`R-MoveAppend`'s zeroing), E13 (the language's
evaluation order — a literal's fields are evaluated before the assignment stores, so an
in-place build STAGES them), E14 (loft#914: an omitted field takes its default), E17
(`B-View` under `B-Disturb`), E20 (§ V-an's phantom, `O-Buffer`).  At 3 with NO rule —
the owner's call: E1's identity (`O-Buffer` names STORE identity; nothing names a record's
— open question 1).  At 5 (doubt → decline): E15 (a callee with more than one exit writes
its destination only at the single exit or declines), E2's handed-up record (copy when the
mark cannot be re-drawn).

## Edge cases to inspect before a phase is cut

Numbered for the review.  **Verdict** is the proposal; the owner confirms or moves it.
Every row is first judged on NATURALNESS: `natural` means a programmer writes it without
knowing the store model and the mechanism owes it the efficient code; `contrived` means
the spelling keeps today's copy and the row is deferred — a verdict that is always
available and never a defect.  The parse library is the reference for "natural": every
shape it uses (E7, E13, E15, E16, E17, E20) is natural by construction.

| # | naturalness |
|---|---|
| E1–E6, E18, E19 | mechanism-internal — not a spelling; they must hold whatever is admitted |
| E7, E13, E15, E16, E17, E20 | natural — the library writes them today |
| E8, E9, E10, E11, E14 | natural — a branch, a parameter, a partial literal are ordinary |
| E3 (generator), E4 (`par`), E12 (`OpDrop` types) | contrived for this plan — decline the mechanism there, keep the copy, defer |

| # | case | why it bites | proposed verdict | probe |
|---|---|---|---|---|
| E1 | **store identity is the free protocol's key.**  `OpFreeRefIfDistinct(v, __ref_N)` declines the local's free when `v.store_nr == __ref_N.store_nr` (`O-Buffer`), `OpDistinctStore` answers `store_nr !=`, and § V-af's witnesses are store identities | two buffers in one arena share a `store_nr`, so "same store" no longer means "same object": a free declines that should fire (leak) or a witness matches the wrong buffer | identity becomes `(store_nr, rec)`; `free_displaced` and the witness compare both; `emission_audit.py` R-State counts holders per record | count the sites (`grep -c store_nr !=` on an emission), then a hand-patched arena of `parse_poly`'s four buffers under `LOFT_STRICT_STORES` |
| E2 | recursion | a caller-threaded arena grows with depth; a mark/release at each activation exit keeps it a stack, but a result adopted from the arena by the CALLER's local (tier 2) sits above the callee's mark | release-to-mark only what the activation minted and did not hand up; a handed-up record is re-marked to the caller | a recursive record builder, depth 1 000, leak gate |
| E3 | a generator | the activation SUSPENDS; a local arena dies at the yield | the arena is a persistent field of the generator, or tier 1 declines a body with `yield` (as every hoist does) | the coroutine corpus under the switch |
| E4 | `par` arms | one arena, two threads | one arena per arm, marked and released by the arm | THREADING.md's corpus |
| E5 | records that own heap in an arena (`Elem.ename`) | release-to-mark is O(1) only for no-heap records; text handles must be released (`@FR-H-ClearRelease`) | the mark carries a heap-owner list; a walk over that list, never over the store | a cell with 1 000 text-owning temporaries, `LOFT_NATIVE_LEAK_CHECK` |
| E6 | an arena record's vector GROWS | growth relocates within the store; `@FR-R-Base` bases and `@FR-R-Header` headers into arena records | an arena mint counts as growth for the enclosing loop's `growth_free`, as a null-discharge buffer does today | `LOFT_HOIST_VERIFY=1` on the parse bench |
| E7 | adopt at first bind when the buffer is the CALLER's caller's (`ps_p = parse_poly(…, __ref_5)`) | the local and the buffer are one store; the steady state after iteration one already | MEASURED (B1): admitted for a fresh-adopting callee already; for a promoted-local callee the reused buffer is freed by the interpreter's rebind of that local (c6) — B1 keeps the buffer null, B1b takes the reuse | `B1-adopt-first-bind-cells.loft` c6; the plan-51 guards under the switch |
| E8 | adopt when the callee handed back a store it did not mint (`O-Opaque`: a fn-ref, a return that borrows a parameter) | adopting a borrowed store frees someone else's | SHIPPED (B1): `adopts_minted_at_bind` admits only a return whose deps name exactly the callee's buffer attribute; a visible-parameter dep, a `__closure` dep, a `CallRef` and a nullable return decline | c7 (a parameter read), c12 (a branch), c4 (nullable) |
| E9 | move at last use on ONE path only | `if c { s.p = pp } else { use(pp) }` — dead after on the then-path, live on the else | per-path liveness (`O-Complete`); the move only where dead on EVERY path after, else copy | a branch cell with the value read after on one arm |
| E10 | move from a `const` parameter, a view, an element | not owned; a move would steal | decline: the oracle's OWN verdict is the gate, `O-Proxy` is not enough here | cells over each source kind |
| E11 | the moved-from local at scope exit | its free must not run (the record is gone) | the move nulls the source (`@FR-R-MoveAppend` zeroes it); `LOFT_POISON` is the falsifier | poison run |
| E12 | a type with `OpDrop` (@PLN163 copy leases) | a move must not run the drop; a copy must take a lease | a moved record keeps its lease; interacts with @PLN163's rule — decide there | @PLN163's cells |
| E13 | **self-referential overwrite**: `sc.elems[idx] = Elem { ename: ap_e.ename, … }` where `ap_e` views the SAME slot | building in place overwrites a field the literal still reads | evaluate every field expression before the first write (a stage), or decline in-place when any field reads through the destination | the `acc_pts` shape as a cell, hand-computed |
| E14 | a partial literal overwrite | omitted fields must become defaults, not keep the old element's values | write the defaults (the prefill image) before the named fields | a cell with an omitted field that was non-default before |
| E15 | the destination as return buffer when the callee exits EARLY | `if len(pts) < 2 { return Mark{…} }` leaves the destination half-written | a callee that returns on more than one path writes the destination only at its single exit, or declines | `parse_poly`'s three returns |
| E16 | the destination as return buffer when the callee READS the destination (`v = f(v)`) | source and destination alias | decline when any argument reaches the destination's store | a cell `s.pts = grow(s.pts)` |
| E17 | a `?`-discharged read used as a view while the container is DISTURBED before the last read | `ap_e = sc.elems[idx]?` then `sc.elems += […]` then `ap_e.bx0` | `B-Disturb` — a view is admitted only when no disturbance stands between the bind and the last read | a cell with an append between |
| E18 | the interpreter | tier 1 and 3 change the IR (where buffers live, where a literal builds), so both backends must agree — and the interpreter's `OpDatabase` per buffer is the same IR | parse-time change, one IR; the emitter's hoists read the arena as a store | the whole `tests/scripts` corpus on both backends |
| E19 | `LOFT_STRICT_STORES` / the leak gate's accounting | a store that hosts many buffers is one store to the gate; a leaked RECORD inside it is invisible to a store-count gate | the gate counts records above the mark at exit, not stores | extend the gate first (a probe-first phase) |
| E20 | a buffer adopted by a local that is then RETURNED (`n = f(); …; n`) | § V-an's phantom: the result lands in the function's OWN buffer, which is the caller's arena, not this activation's | the phantom is above the mark by construction; pin it | `V-an-chain-cells` under the switch |

## Sub-arcs (REQUIRED)

| Item | Source | Verify | Status |
|---|---|---|---|
| **P0** — probe first: count the store-identity sites; price tier 1; the hand patch judged not worth building by the arithmetic; the leak-gate extension (E19) deferred to A1's shape | § P0 | 287 identity sites in one emission; ~88 mints per parse at ≈ 60 ns a pair: tier 1's ceiling ≈ 10 % of the row | Done 2026-09-15 |
| **A1** — arena for one activation's own buffers (`__ref_N`, `__ref_p2_N`, literal temps), mark/release at every exit | § Tier 1 | `tests/scripts/164-arena-activation.loft` both backends; plan 51's ten graduated guards under the switch; `emission_audit.py` R-State per record | Blocked on P0 |
| **A2** — the caller-threaded arena, reset per loop iteration | § Tier 1 | parse row −20 %; E2/E3/E4 cells | Blocked on A1 |
| **B1** — adopt at first bind | § B1 | cells c1–c17 both backends; the store census 139 → 108; plan-51 guards under both switch states | Shipped 2026-09-15 |
| **B1b** — reuse the buffer across activations for a promoted-local callee (E7's steady state) | § B1 | c6 under the pool without `minted_pairs` — the interpreter's rebind free must first match native's `_rb_w_` guard | Blocked on that divergence |
| **B2** — the result's buffer claimed in its destination's store, the field taking it by relocation at the last use (`R-Place`, `R-MoveLast`) | § B2 | cells b1–b15 both backends under the falsifiers; the census 184 → 50; `parse_poly`'s `paint: pp_paint` emits `OpMoveRecord` and `read_paint`'s buffer is a record in the scene's store; the parse row a wash (`perf stat`) | Shipped 2026-09-16 |
| **C1** — element overwrite from a literal in place (`R-InPlaceLiteral`) | § C1 | step 1 (the STAGING clause, a both-backend silent-wrong on the shipped FIELD road) and step 2 (the element receiver): 25 cells both backends under every falsifier, four declines pinned, the `acc_pts` census 3 → 2 | Shipped 2026-09-16 |
| **C2** — the destination as return buffer (`R-Place`'s "the buffer IS the place") | § C2 | the 11-cell oracle (today's answers, which C2 may not move); the aliasing decline; `(O-Buffer)`'s new clause | Shipped 2026-09-16 — a pure-IR rewrite, the buffer variable re-pointed at the destination; ZERO admitted sites in the parse bench (the emission is byte-identical under the switch) and none in the 15-file consumer corpus, so the gain is structural |
| **C3** — read-only `?`-discharge as a view (`B-View`'s discharge clause) | § C3 | the discharge ALREADY views (13 shapes measured, both backends), so the phase's content was its other half: `(B-Disturb)` across a CALL.  15 pairs both backends; 7 move under the switch | Shipped 2026-09-16 |
| **C4** — per-type prefill image (`R-Prefill`) | § C4 | cells c1–c11 both backends under `LOFT_PREFILL_VERIFY`; the verify census over all 1432 corpus files; the image USED on both backends (`LOFT_TRACE_PREFILL`); parse row −11 % | Shipped 2026-09-15 |
| **C5** — a returned record's heap field as a view leaf (`O-ViewField`, `R-ValueRecord`, `R-Escape`) | § The rewrite list | the points written once per line: `Mark.pts` names `Op.pts`; an E17 site that appends between the call and the read must read the copy; an escaping `pub fn` result reads the copy at the bridge | Open — last; rule admitted (C122) |

Every phase: a switch (`LOFT_NO_<unit>=1`), a falsifier, cells in
`bytecode-comparisons/`, a guard in `tests/scripts/` with its `@falsified-at:` receipt,
the pins in `tests/<unit>.rs`, `scripts/test_subjects.sh` extended — the @PLN157 shape.

## Phase ordering

1. P0 — done: tier 1 priced at ≈ 10 % of the row, E1's site count measured; the owner's
   pick between `(store_nr, rec)` identity and a pooled store stays open until A1 is cut.
2. B1, C4 and B2 — shipped.  B2 measured a wash on the parse row (the `Paint` it relocates
   carries no heap on the bench scene); the copy class the profile measured is the points,
   which C2 and C5 take.
3. ~~**C3 then C1**~~ — both shipped.  C3 moved no copy (the discharge was already a view, and
   the `acc_pts` temporary the evaluation table charged to it was C1's, counted twice); C1
   closed the staging clause on the FIELD road first, because the element road inherits it, and
   then opened the element road — census 3 → 2.
4. ~~**C2**~~ — shipped, and the bench then said the spelling it admits (`h.v = mkints(n)`,
   a call result into an EXISTING vector place) has no site in the consumer: the emission is
   byte-identical under the switch and the corpus's 13 `place = call(…)` sites are all scalar
   or `text`.  The gain is structural; § C2 *Shipped* records it and names where the parse
   row's points class actually sits.
5. **C5 is next** — and it is the keystone for the points, not the last polish the ordering
   first made it: `pp_pts` has TWO owning destinations (`Op.pts` and the returned `Mark.pts`),
   which is what declines a vector `place_result`.  The view leaf removes the second, and the
   `Mark` store per parsed line (12 of the ~68 mints a parse still makes) goes with it.  It
   extends `(O-ViewField)` and the value-record gate, both of which the owner has signed off
   (C122).
6. **B1b** beside them whenever D-own-43 closes (the interpreter's rebind guard); then
   A1 → A2, whose remaining share is re-measured after the copy phases have moved the mix.
7. Re-measure the 14-row bench after each phase (`compare.py`, 14/14 hashes).

## Open design questions

1. **E1 — what is an object's identity once buffers share a store?**  `(store_nr, rec)`
   is the proposal; it touches `free_displaced`, `OpDistinctStore`, the § V-af witness
   and the audit.  Or: keep one store per buffer and make the STORE cheap (a pooled
   `Store` with no file handle, no budget row) — a smaller change that keeps identity as
   it is and takes less of the fifth.  The owner picks; P0 measures both.
2. **Is the arena a parse-time IR fact (both backends) or an emission fact?**  The
   buffers are IR (`OpDatabase(__ref_N)`), so tier 1 is IR-level unless the interpreter
   keeps stores and only native pools them — which `O-NoDiverge` allows for a lifecycle
   detail with no value, but the leak gate must then read both.
3. **Where does a moved-from local's `deps` go?**  Tier 2's move needs the scopes pass to
   record "handed off", the same predicate the double-move lint counts.
4. **`(O-ViewField)` — DECIDED (owner, 2026-09-15, C122): admitted.**  *"The current
   contract is about semantics, not about optimisations; we can do anything for that as
   long as we can validate the conditions where it is correct.  The biggest problem here
   is a library API where we cannot know how it will be used.  But if a construction
   doesn't escape a library then we can rewrite whatever we want."*  Recorded as
   `(R-Escape)` in `formal/rewrites.md`, which every rule in the rewrite list now reads:
   a rule needs its validated conditions and nothing else; a construction that escapes a
   library's API keeps the promised representation and the boundary materialises; one
   that does not may be rewritten freely.  C5's cells therefore include an ESCAPE row —
   a `pub fn` whose result a consumer could keep — which must read the copy at the bridge.
   And the unit is a BUILD decision (owner, same day): a release copy of a program already
   compiles its `use`d loft libraries into the one program (the bench emission carries
   `parse_poly` itself), so in that lane the drawing library's API is no boundary and C5
   crosses it; only the library's own published cdylib, a `#rust` native, the live-reload
   arm, a stored layout and a placed library keep the promised representation.
5. **`(R-Place)` across stores.**  A result's buffer claimed in the DESTINATION's store
   (the scene's) rather than a store of its own is what makes B2's move a relocation;
   it is also the first place a temporary lives inside another record's store, which is
   E1's identity question in miniature.  If the owner picks the pooled-store answer for
   E1, this rule still stands: the placement is per call, decided by the destination.

## Cross-arc dependencies

- **@PLN157** — the parse row's bar is that plan's; this plan is its parse unit, split
  out because it is a memory-model change, not a rewrite of a loop.
- **plan 51 (hidden-buffer aliasing, finished)** — its five clusters are the shapes an
  arena must not reopen; its ten graduated guards run under every switch here.
- **@PLN163 (copy leases)** — E12: a moved `OpDrop` record and its lease.
- **loft#1336 (owner witness)** — a witness is a store identity; E1 changes it.

## See also

- `LIFETIME.md` (`OpBindOrCopy`, the adopt-or-copy delivery), `OWNERSHIP_MODEL.md`,
  `formal/ownership.md` (`O-Buffer`, `O-Move`, `O-Complete`), `formal/binding.md`
  (`B-Copy`, `B-View`, `B-Disturb`), `formal/rewrites.md` (`R-Callee`, `R-ElemFirst`,
  `R-MoveAppend`, `R-Mint`, `R-PushRec`), `formal/heap.md` (`H-ClearRelease`).
- @PLN157 `DESIGN.md` § V-an (the parse table this plan starts from) and § V-ao (the
  census).
- The tracker issue: [@PLN164](https://github.com/loft-lang/plans/issues/164).
