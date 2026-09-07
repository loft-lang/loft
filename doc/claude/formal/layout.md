<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/layout.md — the store byte layout (strict)

**Catalogue:** @F3 (heap/store), @PLN97 (layout contract), #477 (the nested-vector stride that
motivated it), #399 (narrow-int storage).

> **Rules then deviations** (see [README](README.md)). This is the **layout relation** for loft's
> store — the function `layout(τ)` that turns a type into bytes: record size, field/element byte
> offsets, scalar widths, the reference encoding. It is the FORMAT counterpart to
> [heap.md](heap.md): heap.md gives the STEPS (`H-Read` reads at `pos + field_offset`), this doc
> DEFINES that `field_offset`. The two meet at every access — heap.md's semantics operate on the
> bytes this doc lays out.
>
> **One format — no in-memory / on-disk split.** loft's store is ONE thing: the durable file is
> bit-for-bit identical to the in-memory store (`src/store.rs`: *"durability is a metadata layer,
> not a payload-layout change"*). So `layout(τ)` is a single relation, and its rules hold equally
> for the bytes in RAM and the bytes on disk. `layout` is computed once, in `src/data.rs`
> (`element_size` / `element_align` / `element_offsets`) and `Stores::finish` (struct positions);
> **both backends read the same offsets from the one type table** — a divergence is a bug, not a
> second layout.

## The store byte model (the ground the rules stand on)

- **`Store`** — a word-addressed byte region (`src/store.rs`): a fixed header
  `[0 = SIGNATURE (4 B), 4 = free-space index (4 B), 8 = record size (4 B), 12 = content …]`, then
  records. `layout(τ)` governs the CONTENT bytes of a record; the header is a fixed store frame.
- **arena frame** — records live in one arena. **Record 0** is the header (above); **record 1 =
  PRIMARY** is the root container (the program's top `hash` / `sorted` / struct value), its type-id
  at `(rec = 1, pos = 8)`. Every record begins with an `i32` **size word** (in 8-byte words; a
  NEGATIVE size marks a free block), so the chain is walkable by `rec += |size|`. Addressing is
  **word-based**: `rec` is a word index, byte offset `= rec · 8`; a sub-reference is a `u32` at
  `(rec, pos)`. This frame is what makes a store file **self-describing and native-endian** (native ↔
  wasm, RAM ↔ disk) and lets a reader walk it by byte offset — the basis of the #522 remote
  working-set range-reader. The multi-byte words (the `i32` size, the `u32` sub-references, the
signature) are written **native-endian** (`store.rs` `to_ne_bytes`/`from_ne_bytes`), so a store
is portable among **like-endian** hosts — every current target (native and wasm) is
little-endian; a big-endian reader needs a byte-swap (only the `.dcache` sidecar is fixed
little-endian). Endianness is thus a **physical-encoding** detail, outside the frozen logical
layout contract (which pins field/enum identity + order, not byte encoding).
- **record** — a value of type τ occupies `size(τ)` contiguous bytes; a field/element sits at a
  fixed byte offset within it. `H[r ⊕ n]` (heap.md) reads at `r.pos + n`.
- **`DbRef`** = `(store_nr: u16, rec: u32, pos: u32)` (`src/keys.rs`) — the universal pointer. A
  STORED reference to another record is a **4-byte record pointer** (the `rec` into the target
  store); the full 12-byte `DbRef` is stored only where a value must round-trip its whole pointer
  (the closure half of a fn-ref field, `Parts::DbRef`).
- **null is in-band** — a nullable field uses a SENTINEL inside its own bytes (`i64::MIN` for
  integer, `nullref` for a reference), not an extra byte. Nullability is therefore NOT a layout
  fact (rule `L-Null`).
- **the identity** — `layout_algo_hash` + the per-type `layout_dump` (`src/database/types.rs`) is
  the store's **layout identity**: a stable fingerprint of every rule below applied to the
  program's types. It is the version the `.dschema` sidecar records (`src/schema_sidecar.rs`).

## Notation

- `layout(τ) = (size(τ), align(τ), offs(τ))` — the total byte layout of τ: its record size,
  natural alignment, and the offset of each field/element.
- `off(τ, f)` — the byte offset of field/element `f` within a τ record (`offs(τ)` indexed).
- `width(τ)` — the stored width of τ as a FIELD of another record (a scalar's size, or 4 for a
  record pointer). `id(P)` — the layout identity of a program's type table `P`.
- `τ?` is the nullable form of τ; `τ ≈ σ` means "same byte layout" (`layout(τ) = layout(σ)`).

---

## Rules

### The layout function is total and shared

```
  (L-Total)   layout(τ) is a TOTAL function of the finished type τ: every registered type has
              exactly one (size, align, offset-vector), computed in src/data.rs + Stores::finish.
              BOTH backends (interpreter store, native DbRef ABI) read these SAME offsets — a
              backend that computes a different offset is a bug (D-op-1), not a second layout.
```

**In words.** There is one layout per type, and it is decided by the type alone (plus the fixed
algorithm) — never by which backend, which run, or which call site reads it. This is what makes a
store written by one build readable by another *of the same layout*.

### Scalar and narrow-int widths

```
  (L-Scalar)  width(boolean)=1  width(character)=4  width(single)=4
              width(float)=8    width(integer)=8    width(text)=4 (a Str handle)
              align(τ) = its natural alignment (1,4,4,4,8,8,4 respectively).
  (L-Narrow)  a range-annotated integer stores in the SMALLEST width that holds its range (#399):
              u8 → 1 B, u16/i16 → 2 B, i32 → 4 B, else 8 B.  The narrowing is a WIDTH change,
              so it moves offsets and record size — a layout fact the golden pins.
```

**In words.** Each base type has a fixed stored width. A narrow integer (`i32`, `u8`, …) stores in
fewer bytes — which is exactly why a narrowing is a layout change: it shifts every following
field. (#399 is the change class; the golden test pins each width.)

### References, collections, and child records

```
  (L-Ref)     a stored reference / collection field (Reference, Vector, Hash, Sorted, Ordered,
              Index) is a 4-byte RECORD POINTER into the target store.  A ChildRec is a 4-byte
              co-located rec id.  The full 12-byte DbRef is stored only for Parts::DbRef (a
              fn-ref field's closure half).  A collection's ELEMENT stride is width(element).
```

**In words.** A field that points at other records holds a small (4-byte) pointer, not the data
inline. The data lives in the target store; the collection's per-element stride is the element's
own width — so a change to an element's width (e.g. the nested-vector stride, #477) is a layout
change even though the field pointer is unchanged.

### Structs, enums, tuples

```
  (L-Struct)  a struct record packs its fields by DESCENDING alignment; off(τ, fᵢ) is the packed
              position; size(τ) is the packed total.  A field access is H[r ⊕ off(τ, f)].
  (L-Enum)    an enum is a 1-byte discriminant; a data-carrying variant (EnumValue) is
              [tag byte] followed by the variant's fields (L-Struct packing).  Variants are
              numbered from 1: 0 is the absent value (L-Null) and 255 is the null a write
              spells, so an enum holds at most 254 variants and the parser refuses the 255th.
  (L-Tuple)   a tuple (τ₀,…,τₙ) is a synthetic __tuple<…> struct.  Element offsets are
              natural-alignment packing — off = the next position ≥ the element's alignment —
              and a tuple has TWO layout views that must compute the SAME offsets: the STACK
              view (data::element_stack_offsets / element_stack_size) and the STORAGE view
              (the synthetic struct, calc::calculate_positions_with_groups, read back by
              data::stored_tuple_offsets).  Their agreement is part of the rule, not an
              implementation detail.
```

**In words.** Fields are packed largest-alignment first, so the record has no wasted padding and
every field lands on its natural boundary. Enums carry a 1-byte tag; a variant with data is that
tag plus the variant's own fields. A tuple is stored as a hidden struct, packed the same way.

⚠ A tuple lives in two places — on the stack and in a record — and the two are computed by
different code. That is why `L-Tuple` names both and requires them to agree: @PLN114 split the
one ambiguous `element_offsets` into the two named views precisely so a site has to declare which
it means, and a site that picks the wrong one reads a plausible offset from the wrong model.

### Nullability is a sentinel, not a layout

```
  (L-Null)    τ ≈ τ?  —  layout(τ) = layout(τ?).  A nullable field has the SAME bytes as its
              not-null form; absence is a sentinel IN those bytes (i64::MIN / nullref), never an
              extra byte or a moved offset.  Nullability is a SCHEMA fact (semantics), not a
              LAYOUT fact.
```

**In words.** Making a field nullable does not change the bytes — the "missing" value is a
reserved bit pattern inside the field. So a nullable and a not-null integer have identical layout;
the difference is meaning, carried by the schema (`data_to_json`), not the layout hash. (Verified:
the golden renders both identically.)

⚠ **The rule as written is true for a SENTINEL type and false for an INLINE STRUCT, measured
2026-08-28 (loft#1134).** `LOFT_DUMP_TYPES=1` on two pairs of structs that differ only in a `?`:

```
IntD[16/8]   k:integer[0]  n:integer[8]        IntN[16/8]   k:integer[0]  n:integer[8]
Dense[24/8]  k:integer[0]  item:S[8]           Nble[32/8]   k:integer[0]  item:__nullable<S>[8]
```

The scalar pair is identical, exactly as the rule says. The struct pair is not: the record grows
by eight bytes and `S`'s own fields move from offset 8 to offset 16, behind a discriminant. That
is not a sentinel — a struct stored inline has no reserved bit pattern to spend, which is why
[types.md](types.md) gives it the tagged `__nullable<S>` (discriminant + payload) and calls the
property bought *"no collision"*. So `(L-Null)` holds for every type that HAS a sentinel and does
not hold for the one representation that needs a tag, and the two documents did not agree.

**The incomplete half is what licensed a defect.** A reader of this rule alone concludes that
`S?` and `S` share their offsets, which is precisely the belief that wrote a dense `S` into a
tagged slot: field `a` landed on the discriminant, presence became a data byte, and a present
`S { a: 0, … }` read back absent (loft#1134, and its layout-side account in
[tuples.md](tuples.md) § D-tup-6). A rule stated for four of its five cases is more expensive
than no rule, because it is CITED.

The rule wants splitting rather than weakening, and the split is decidable from the type alone:

```
  (L-Null)     τ ≈ τ?  for every τ that reserves a null VALUE  —  layout(τ) = layout(τ?);
               absence is a sentinel in those bytes (i64::MIN / NaN / 255 / nullref / codepoint
               0 / an ENUM's discriminant 0, which its variants are numbered away from), never
               an extra byte or a moved offset.  So a variant test over a `vector<E?>` element
               decides absence for free: an absence is discriminant 0 and every variant is 1 or
               above, which is `(M-Variant)` needing no null test of its own (loft#1410).
  (L-Null-Tag) a struct stored INLINE — a `vector`/keyed element, an embedded field, a tuple
               member — has no sentinel to spend, so `S?` is the tagged `__nullable<S>`:
               layout(S?) = discriminant ++ layout(S) at the payload base, and
               size(S?) > size(S).  Absence is discriminant 0.  Every writer and reader of such
               a slot goes through the tag; the pair that holds this is
               `Parser::emit_nullable_slot_write` / `emit_nullable_slot_read`.
  (L-Null-Which) which of the two governs a FIELD is decided by the field, not by the type it
               names: `Type::Reference(S, deps)` is the IR spelling of BOTH an embedded `S`
               and a `reference<S>` pointer, and the `u16::MAX` share marker in `deps` (#328)
               is what tells them apart — the same bit `Data::has_value_cycle` reads to skip
               pointer edges.  Marked ⟹ `(L-Null)`, a 12-byte pointer with `nullref` for
               absence; unmarked ⟹ `(L-Null-Tag)`.  One home: `synth_nullable_struct_fields`.
               And what is NOT a slot takes `(L-Null)`: a local, a parameter, a return, the
               subject of `??` or `?` spell `S?` as the pointer, so a tagged value reaching one
               is read through its tag AT THAT POINT — `Parser::read_through_tag`, the read half
               of `(L-Null-Tag)` applied once, both passes.  Left in the slot's spelling, a
               local took whichever spelling its last assignment parsed (loft#1367).  A view
               taken this way holds the PAYLOAD's address: a later clear of the slot is not
               visible through it, exactly as through a `reference<S>?` field after the field
               is cleared; the slot's own reads see the tag.  The base of a FIELD READ or a
               METHOD CALL is not a slot either: `v[i].n` and `v[i].m()` on a tagged element
               read the receiver through its tag first, so an absent element's field is the
               typed null and its method receives `nullref`, as a `S?` local's would.
               And the pointer half has ONE value spelling: a read that names no record — an
               index past the end or before the front, a keyed miss, a zero child pointer —
               answers `nullref` AT THE READ (`DbRef::or_null`, the exit of
               `vector::get_vector`, the two `vec_get_or_raise` twins, `Stores::get_ref`,
               `State::get_record` and `codegen_runtime::OpGetRecord`).  The slot spellings
               (`ABSENT_REC`, discriminant 0, a zero pointer) never leave their slot; a
               `DbRef` carrying a live `store_nr` with `rec == 0` is a reference whose
               HOLDER has no record, not an absent value, and only the total `rec` tests
               (`OpEqRef`, `OpConvBoolFromRef`, `is_absent_collection`) read it.  Handed up
               instead, that shape was present to the handle test and garbage to the copy
               (loft#1374).  Read from the DESTINATION end the same clause says a write to a
               place naming no record is DROPPED: `v[i] = e` past the end changes nothing and
               raises nothing, so the record copy answers `nullref` on both sides and returns
               (`State::do_copy_record` and `codegen_runtime::OpCopyRecord`, which guarded a
               null SOURCE for @PLN25 and needed the twin once an absent read began answering
               `nullref` — without it the copy indexed `allocations[65535]` and the process
               died on a program the compiler accepts).
  (L-Null-Text) `text` reserves TWO spellings of absence and they are ONE value: an UNSET
               handle (str_rec 0 — the `nullref` above) and an ALLOCATED record holding the
               `STRING_NULL` (`"\0"`) bytes.  A reader tests the CONTENT, never the handle
               alone: `get_str` maps an unset or out-of-range handle onto `STRING_NULL` too,
               so the content test is TOTAL and subsumes the handle test.  An allocated `""`
               is a present value, not an absence (@P375).  One home: `Store::text_is_null`.
```

`(L-Null-Which)` is there because the split above is decidable from the type and the code still
got it wrong: both halves are reachable through ONE `Type` variant, so a site that reads the
variant and not the marker picks a representation by accident.  Measured (loft#1316): the field
rewrite discarded `deps`, `reference<Leaf>?` and `Leaf?` laid out byte-identically, and writing
`?` on a pointer field silently replaced sharing with an inline copy — while on a reference graph
that returns to its own struct the inline form has no finite size, so a linked list's terminator
could not be declared at all.  A rule that says WHICH half applies but not HOW a site decides is
the incomplete kind this doc has been bitten by twice already (see the ⚠ above).

`(L-Null-Text)` is the same kind of record for the one scalar whose sentinel is not a bit
pattern in its own slot.  A text slot holds a HANDLE, and absence is spelled twice because the
two writers spell it differently: a never-written slot keeps the zero handle a fresh record
starts with, while writing `null` — from a literal, an assignment, a call, or any native text
path — allocates a record holding the sentinel bytes.  Neither writer is wrong; what is wrong is
a reader that knows only one.  Measured (loft#1270): `Stores::is_null` tested the handle alone
and is the predicate deciding whether a struct field is OMITTED, so `NT { a: 1, t: null }`
serialised `{"a":1,"t":null}` while the same value parsed back serialised `{"a":1}` — one value,
two documents, and a document that is hashed, signed or diffed changed for a value that did not.
The render arm applied the content test only under `json`/`loft`, so the plain form put the
sentinel on the wire AS TEXT: `"{x}"` answered `{a:1,t:"\0"}`, a present one-character string
where the program meant nothing.  `native.rs` had carried the total test for its own reader since
loft#769 — the rule was discoverable in the code and simply not written down here.

Neither half of the tag split is a new decision — both are what the code has shipped since
@PLN25 — so this records the boundary rather than moving it. The falsifier below covers `(L-Null)`; `(L-Null-Tag)`
is covered behaviourally by
`tests/scripts/1134-a-nullable-tuple-element-is-stored-behind-its-tag.loft`, whose zero-valued
first field is the cell that a size-and-offset golden cannot see — the same blind spot the ⚠
under the falsifier list already records for the sentinel half.

### The soundness invariant — no silent cross-version misread

```
  (L-Sound)   a reader whose layout identity id(P) differs from a store's RECORDED identity MUST
              reject-or-migrate, NEVER read the bytes raw.  A raw handoff is admitted ONLY when the
              two identities are equal.  Discharged by the .dschema sidecar (schema_sidecar): the
              stored id is compared on load; a mismatch is CorruptReason::SchemaMismatch → rebuild.
```

**In words.** The layout is allowed to change between builds — but a store written under an old
layout must never be *read* under a new one as if nothing moved. The sidecar records the layout
identity the store was written with; on load it is compared, and a mismatch routes to
migrate-or-rebuild. This is the layout analogue of heap.md's `H-Sound`: this doc defines the
cliff (what the bytes mean under a given layout), and the sidecar keeps the program from walking
off it (reading bytes under the wrong layout). Its absence is exactly the #477 failure — a layout
change noticed only by broken data. The gate matters most across a boundary the reader cannot see
behind: the **#522** working-set loader range-reads a REMOTE store over HTTP — it MUST verify the
remote store's layout identity (its `.dschema` / `layout_algo_hash`) before walking the bytes, or
it silently misreads data fetched over the network. `schema_sidecar::check_beside` / `classify` is
that gate, now applied across a network boundary.

---

## Deviations

**OPEN: 1.**
- **D-layout-1** — residual: the load-time schema gate is built and opt-in, and closes fully when a persistence consumer wires `check_beside` into its open path

The full register — these entries in full, plus every closed one with its dates and
issue numbers — is the companion [layout-history.md](layout-history.md).

## Conformance

Every rule is a program-independent fact both backends must agree on, and each has a standing
falsifier ([@PLN97](../plans/97-layout-contract/README.md)):

- **`L-Total` / `L-Scalar` / `L-Narrow` / `L-Ref` / `L-Struct` / `L-Enum` / `L-Tuple`** — pinned
  byte-exact by the **golden layout test** (`tests/layout_golden.rs` + `tests/golden/layout/`):
  record size + every field position + narrow encoding + collection element stride, over a corpus
  spanning every storage kind. Any change is a red diff; proven to fail on a #477-class
  perturbation. The **coverage audit** (exhaustive over `Parts`) keeps a new storage kind from
  slipping in unpinned.
- **Both backends** — `tests/scripts/509-layout-parity.loft` constructs the corpus and asserts the
  read-path (narrow widths, the nested-vector stride at runtime, tuple packing, enum tag+fields) on
  interpreter, `--native`, and wasm: a value-corrupting ABI divergence fails on the divergent
  backend. (D-op-1's differential falsifier applies here as elsewhere.)
- **`L-Null` / `L-Null-Which`** — the golden renders a nullable and a not-null field identically
  (same size, same offsets); nullability lives in the schema, not the hash.  The golden compares
  a `τ?` with its own dense twin, so it cannot see a `τ?` given the WRONG half's representation —
  `reference<Leaf>?` laid out as a well-formed `__nullable<Leaf>` and every size check passed.
  `tests/scripts/1316-a-nullable-reference-field-is-still-a-pointer.loft` scores that on
  behaviour instead, on both backends: the marked field must SHARE (write through the source, read
  it through the field) and the unmarked one must COPY, which is the pair no layout dump
  distinguishes.  The VALUE spelling — a read that names no record answers `nullref` —
  is `tests/scripts/1374-an-absent-pointer-leaves-its-slot-as-nullref.loft`, both backends
  under strict stores: an index past the end and before the front, a `hash`, `sorted` and
  `index` miss, each through a local bind, a declared local, a `S?` argument, a `-> S?`
  return and its bind, a copy-bind, an inline test and a field read, with every source
  PRESENT as the control (a write through the handle must reach the container, which a fix
  that copied would fail).  The tagged receiver of a field read and a method call is
  `153-a-tagged-element-read-by-a-variable-index-is-one-null.loft` and
  `153-a-method-call-on-a-tagged-element-reports-the-undischarged-receiver.loft`.
- **`L-Null-Text`** — `tests/scripts/1270-an-absent-text-is-one-absence.loft`, on both backends:
  every way of SAYING absent (literal `null`, an omitted field, an assignment, a call, a parse)
  writes ONE document and round-trips to itself, while an allocated `""` stays a present value.
  The `""` cells are the control — a fix reading "no characters" as absence passes every null
  cell and fails those.  A layout golden cannot see this rule at all: both spellings occupy the
  same four bytes, so what differs is the VALUE in them.

  ⚠ **That falsifier covers the first half of the rule only, and the second half broke under
  it (2026-08-22).** `L-Null` says two things: a nullable field has the same BYTES as its
  not-null form, and absence is a SENTINEL in those bytes. The golden tests the first — sizes
  and offsets — and cannot see the second, because a sentinel is a value and the golden
  compares layout. So a writer may spell absence any way it likes and the golden stays green:
  loft's `JsonValue` JSON walker wrote the zero code into one-byte widths, so an absent `u8?`
  read back as the VALUE `0` and an absent `boolean?` as `false`, while every layout gate
  passed. The sentinel half is now enforced structurally instead — the encodings live at one
  address (`Stores::write_absent_value` / `Stores::write_narrow_value`, the write-side twins of
  `Stores::is_null`) and both JSON walkers call them, so a second spelling has nowhere to live.
  Behavioural guard: `tests/scripts/json-walker-absent-field.loft`, both backends.
- **`L-Sound`** — `src/schema_sidecar.rs` tests: an unchanged store → `Identical` (raw handoff); a
  changed layout → detected (`SchemaVerdict::Changed`), a garbage sidecar → `Unreadable`, each
  mapping to `CorruptReason::SchemaMismatch`. The layout identity a store records is
  `Stores::layout_algo_hash`, itself pinned by the golden.
