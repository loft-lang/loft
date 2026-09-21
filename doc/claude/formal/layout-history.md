# formal/layout-history.md — the deviation register for [layout.md](layout.md)

> **The rules are next door.**  [layout.md](layout.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

- **D-layout-1 — no version guard on persisted bytes** (the motivating gap). Before @PLN97 the
  layout was **nowhere written** and **nothing recorded which layout a store was written under**,
  so a layout-changing fix (#477: nested-vector stride 4→8) **silently misread** existing data —
  caught only by breakage, never by a check. `L-Sound` is the rule it violated.

  **Status — mechanism shipped, auto-enforcement pending a consumer.** The rule is now
  *enforceable*: the golden test (`tests/layout_golden.rs`) catches a layout change at commit time,
  and the `.dschema` sidecar (`src/schema_sidecar.rs`, `CorruptReason::SchemaMismatch`) detects a
  stale store at load and routes it through the durable store's `on_corruption` rebuild. **Residual:**
  the durable store (`plans/43`) is not yet driven by a loft builtin, so nothing *calls* the
  load-time gate automatically yet — the deviation closes fully when a persistence consumer wires
  `check_beside` into its open path. Until then the guard exists but is opt-in.

  **CLOSED 2026-09-21 (loft#1562).** Re-measured on main `9f5cf6a96`, the residual was narrower
  than written and silent: the gate had become the default on `store_load` (loft#700) and the
  paged loaders (@PLN97 arc G), and three paths still read an existing image raw.
  - `store_persist_bind` on an existing file.  A `hash<Rec[id]>` written as `{id, v}` and bound
    as `{id, extra, v}` answered `true` and read `v` from the neighbouring bytes, on both
    backends.  The bind then wrote the `.dschema` with the READER's layout, so the same file's
    later `store_load` answered `true` too, and the program that wrote it read the records its
    reader had inserted in the other layout.  Reshaped, dropped and nested fields, `sorted`,
    `index` and a struct container behaved the same; a garbage sidecar was bound and relabelled.
  - `store_load_url_trusted` and `store_load_url` (whole-image fetches) — no gate at all, over
    `file://` and HTTP alike.  A SHA-256 pin does not help: it says the bytes are the ones
    published, not which layout wrote them.

  Now all three ask the gate before they read: the bind before `Store::open`, so a refusal
  leaves the file, its sidecar and the collection untouched; the URL loaders before the image
  fetch, reading `<url>.dschema` over the same transport (a failed fetch is an absent sidecar,
  as a server's 404 is).  The refusal names its own builtin.  Both backends, and the browser
  build, whose node harnesses had answered every whole-file GET with the store bytes and now
  answer a `.dschema` request as a server does.  Guards are named in layout.md.

- **D-layout-2 — the `?` changed the layout** (2026-08-28, loft#1125). `L-Null` says
  `layout(τ) = layout(τ?)`, and three sites decided layout by naming `Type` variants BARE, so a
  wrapped shape reached none of them.

  The visible one: the walk that gives an `index` its bookkeeping triple a position runs at the
  end of `fill_all` precisely so `#left_N / #right_N / #color_N` are appended to the ELEMENT
  struct before it is sized. `Optional(Index(…))` matched nothing there, the triple was appended
  afterwards, and `finish_type` returns early for a type that already has a size — so all three
  kept `position: 0`, on top of each other and of the first real field. The nullable form then
  refused to lay out at all while its dense twin was fine.

  Its two siblings, same rule: the `null` → sentinel conversion asked for `Type::Vector` alone
  and was short by the five KEYED kinds and by the wrapper, so a `spatial<P[x,y]>? = null` local
  kept a bare `Value::Null` — which writes nothing — and the scope-exit `OpFreeRef` read the
  untouched bytes as store #0 (BUG #306); and an OMITTED nullable collection FIELD took the zero
  its type gives, where zero is the EMPTY collection and absence has its own reserved id
  (`DbRef::ABSENT_REC`, loft#917), so `c: vector<τ>? = null` read back present-and-empty.

  **Status — CLOSED.** All three read through `base()`. Guard:
  `tests/scripts/a-nullable-collection-lays-out-like-its-dense-twin.loft`, which gives every
  keyed kind its OWN element struct: the layout half is invisible whenever the same index type
  also has a dense local somewhere in the program, because that one registers the bookkeeping in
  time and the nullable form inherits a correct layout.

- **D-layout-3 — three writers did not go through the tag** (2026-08-30, loft#1198). `L-Null-Tag`
  ends *"every writer and reader of such a slot goes through the tag; the pair that holds this is
  `emit_nullable_slot_write` / `emit_nullable_slot_read`"*, and the sentence was a description of
  one writer out of four. Deciding to tag needs the SOURCE's type, and a nullable struct has two
  spellings that mean one thing — the dense `S` and the `S?` a function returns or a local
  declares. The tuple's writer asked `needs_nullable_wrap`, which reads both. The struct field
  (`objects.rs::handle_field`), the element store (`collections.rs`) and the append
  (`vectors.rs`) each spelled `let Type::Reference(src_d, _) = src_tp` instead and so could see
  only the dense one.

  For every `S?`-spelled source the dense record therefore went in untagged, which is `L-Null`'s
  layout applied where `L-Null-Tag` governs — the same confusion of the two halves that D-tup-6
  and D-layout-2 are, arriving this time from the WRITE side. Two faces: a present value landed
  one field low so every read came back one field high, and a value the callee withheld at
  runtime wrote nothing at all, leaving the slot reading PRESENT with its previous value. With
  the discriminant aliased onto the payload's first field, `S { a: 0, … }` read back ABSENT.

  **Status — CLOSED.** All three route through `emit_nullable_slot_write`, which now also
  releases the payload the slot held on its PRESENT arm — one of the three carried that free and
  the shared home did not, so absorbing them without it would have traded a wrong answer for a
  leak. Guard: `tests/scripts/1198-a-nullable-source-is-tagged-into-its-slot.loft`, whose
  controls are the dense source (the half a corpus of literals can see) and the tuple member
  (the writer that already obeyed the rule).

- **D-layout-4 — `τ?` picked the WRONG HALF of the split for a pointer field** (2026-09-03,
  loft#1316). `L-Null` and `L-Null-Tag` divide on a property of the type: a τ that reserves a
  null VALUE keeps its own bytes and spends the reserved pattern, and only a struct stored INLINE
  needs the tag. A stored reference reserves `nullref`, so `reference<T>?` is `L-Null`'s case and
  must lay out as the 12-byte pointer it already is. The rewrite that mints `__nullable<S>` gave
  it `L-Null-Tag`'s instead.

  Both notions travel as `Type::Reference`, told apart by the FIELD's own `u16::MAX` share marker
  (#328) — the same bit `has_value_cycle` reads to skip pointer edges. `synth_nullable_struct_fields`
  discarded the deps with `_` and so converted both. Measured: `reference<Leaf>?` and `Leaf?` laid
  out byte-identically (`__nullable<Leaf>[16/8]`) where `reference<Leaf>` is `dbref[12/4]`.

  Three faces, one mechanism, and the loudest hid the others:
  * **the layout error.** A struct stored inline cannot contain itself, so on a reference graph
    that returns to its own struct the field has no finite size: `struct Node { next:
    reference<Node>? }` failed with *"field 'next' has no position (u16::MAX)"*. The terminator of
    a linked list is the one slot that MUST hold null, and the only spelling that compiled was the
    non-null one — which is why loft#1313 had to suppress `(N-Store)` for exactly this shape
    rather than name a cure that does not compile.
  * **the refusal.** `&pool[i]` in a literal is admitted by the FIELD'S TYPE (`B-Ref-StoredRef`),
    and the gate matched `Type::Reference` unpeeled, so writing `?` withdrew the `&` the pointer
    field exists for.
  * **the quiet one.** `h.l = &pool[i]` fell past the `#328` repoint arm, also unpeeled, to
    `copy_ref` — a deep copy through the field's CURRENT value. On an acyclic type that compiles
    and answers plausibly: the control prints `11` where a pointer prints `22`, so declaring `?`
    silently replaced sharing with a copy, with no diagnostic anywhere.

  **Status — CLOSED.** All three read the marker or peel with `base()`. loft#1313's suppression
  (`field_has_no_nullable_spelling` + `Data::reference_cycle_back_to`) is deleted with it, and its
  guard cells in `tests/heap_nstore.rs` flip from silent to warning. The notice they now emit had
  its own defect of the same kind: `Type::name` renders a pointer field as the bare struct name,
  so the cure read `Node?` — the INLINE form, which on this shape does not compile, and on an
  acyclic one compiles while swapping the pointer for a copy. `Parser::cure_spelling` names the
  field's own type. Guard: `tests/scripts/1316-a-nullable-reference-field-is-still-a-pointer.loft`,
  whose controls are the `?`-less pointer field (still shares) and the embedded `T?` (still
  copies — `L-Null-Tag` still governs it).

  ⚠ **The third entry in a row for one mechanism, and each closed on "all three sites".**
  D-layout-2 was a nullable shape reaching a bare `Type` match, D-layout-3 was the same from the
  write side, and this is the same again with the added twist that the two halves of the split
  are BOTH reachable through one IR spelling. What repeats is not a site but a habit: a `Type`
  matched without `base()` and without the marker, in a position a field type reaches. A census
  of the parser plus `typedef.rs` finds **103** bare `Type::Reference` matches, of which **21**
  read a field's declared type. That number is a QUEUE, not a defect count — most read the
  synthetic `__nullable<S>`'s own payload attribute, where no `Optional` can arrive — but it is
  the population any next instance of this class will be drawn from, and it is the reason the
  rule-led walk over `@FR-L-Null` is not finished by this entry.

## Carried by layout.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [layout.md](layout.md) now states only what is open.

### D-Null-Local — OPENED AND CLOSED (2026-09-05, loft#1367, @PLN153 phase 3c): a tagged projection bound to a local kept the slot's spelling

`(L-Null)` gives a binding the pointer spelling of `S?`; `(L-Null-Tag)` reserves the tagged
`__nullable<S>` for INLINE storage.  A projection of a tagged slot bound to a local (`x = o.opt`,
`x = nv[i]`, a destructured member) was bound AS THE SLOT — the local's type became the synthetic
— and the binding then carried whichever spelling its LAST assignment parsed: after `x = y; x =
o.opt` the pointer parameter `y` was read as a tagged record and the owner witness freed its
store (a use-after-free in the caller, silent without `LOFT_STRICT_STORES`); `x = o.opt ?? y` was
refused naming the synthetic (*cannot change type from `S?` to `__nullable<S>?`*); `x = o.opt; if
c { x = null }` was refused; and `d: S = o.opt` was silent and read a present record of zeroes
for an absent slot.  The reverse order (`x = o.opt; x = y`) was right all along: a bind off a
parameter copies (`@FR-B-Copy`), and the write through `x` lands in the copy.

Closed at one point: `Parser::read_through_tag` — a tagged value reaching a non-slot position
(the assignment seam's plain-local target, the tuple destructure, the `??` subject, the postfix
`?` subject) is read through its tag there, and the value and its type become the pointer on
both passes.  The read is `if <present> { <payload projection> } else { nullref }`, and three
ownership predicates each read that `if` wrong in turn — the witness saw neither mint nor view,
the oracle joined a view with an owned null, the view marking saw no projection — until one
predicate answered for all: `use_analysis::through_null_arm` (a two-arm `if` whose other arm
holds no store delivers its present arm; `holds_no_store` is the join's identity).  Guard:
`tests/scripts/1367-a-tagged-projection-bound-to-a-local-is-the-pointer.loft` (21 cells, both
backends under strict stores); the corpus moved on fifteen files, every one an emission and none
a diagnostic, each green on both backends.  `Contract: settled` — the rules named the local's
spelling; the code failed to convert at the boundary.

- **D-layout-5 — an absent pointer left its slot in the SLOT's spelling** (2026-09-06,
  loft#1374).  `L-Null` gives a reference that is a VALUE one spelling of absence, `nullref`,
  and `L-Null-Which` says a local, a parameter and a return are values.  An element read
  past the end, a keyed miss and a zero child pointer answered the container's live
  `store_nr` with `rec == 0` instead — the shape a reference has when its HOLDER has no
  record — and only the `rec`-testing readers saw it.  The handle test (`OpRefIsNull`) read
  it present, a `S?` parameter's `!= null` passed and its field read answered the integer
  null, a `-> S?` return delivered it present, and the nullable call-result bind
  deep-copied a record that was not there into a fresh store of garbage: `b = re(v, 9); b
  == null` was `false` and `b.n` was `5695106865`, both backends, strict stores silent.  A
  local bound from the read directly was right by accident — that spelling lowers to
  `OpEqRef`, which tests `rec`.  The `get_vector` doc already said the two OOB answers
  "read as the same absent value"; the code answered two.

  Ten cells (`~/workspace/pln153-scratch/stage5/cells`): the vector past the end and before
  the front, `hash`, `sorted` and `index` misses, each across nine sinks, both backends —
  every `S?` sink wrong before, the pointer-field control right.  **Status — CLOSED.**  One
  predicate, `DbRef::or_null`, at the exits where a read mints a value: `vector::get_vector`,
  `State::vec_get_or_raise` and `Stores::vec_get_or_raise_runtime`, `Stores::get_ref`,
  `State::get_record` and `codegen_runtime::OpGetRecord`; no emission moved (a runtime
  change), every consumer that tested `rec == 0` still does (`nullref` has `rec == 0`).
  The same walk found the record bind choosing its null-aware form by the SOURCE's type
  alone, on both backends (`gen_set_first_ref_var_copy`, the native record bind): `x =
  t.h[k]; y: S? = x` on a miss copied nothing into a record allocated for `y`, and `y ==
  null` read false where `x == null` read true.  One predicate for both emitters,
  `Variables::bind_admits_absence`, asked of both sides.  And a consumer that resolves a
  holder's store BEFORE testing its record now panics where it used to read the store's
  header word: the interpreter's `iterate` and `step` did, where their native twins test
  first (loft#691's shared cursor derivation had left the entry test unshared) — both now
  test the record first, and the iteration scratch builders (a `hash`, `radix`, `index` or
  `trie` walk collects its records into a scratch before the iterate) answer `nullref` for
  an absent holder, so a `for` over a collection field reached through `nullref` iterates
  nothing on every kind, as C80 asks.  Guard:
  `tests/scripts/1374-an-absent-pointer-leaves-its-slot-as-nullref.loft`.
  `Contract: settled` — the rule named the value's spelling; the reads did not mint it.

- **D-layout-6 — a tagged element read by a variable index was `τ??`, and its field read
  skipped the tag** (2026-09-06).  Two readers of a `vector<S?>` element, found by the same
  matrix.  `parse_index` wrapped the element type `Optional` for an index not trusted by
  contract, and the element type is the tagged `__nullable<S>` — already `S?` in the slot's
  spelling — so the read typed `Optional(__nullable<S>)`, the `τ??` `N-Idem` forbids; the
  local bound from it took `S?` on one pass and `__nullable<S>?` on the other and the program
  was refused as a type change, while the same read by a constant index compiled.  The
  phase-0 census read 0 over the corpus because it counted `Optional(Optional)` and not an
  `Optional` over the synthetic; it now counts both, with a hand-built cell as its control.
  And the field read `v[i].n` (E2 in `fields.rs`) projected the payload's sub-ref without
  consulting the discriminant, so an absent element's field answered the payload's zero —
  `v[1].n ?? -1` was `0` where `x = v[1]; x.n ?? -1` beside it was `-1` — by a constant
  index as much as a variable one, both backends.  **Status — CLOSED.**  `parse_index` asks
  `tagged_pointer_type` before wrapping; the E2 receiver goes through `read_through_tag`, so
  the field read and the method call proceed on the pointer `S?` exactly as for a local (and
  a method with a dense `self` reports `(N-Store)` for the undischarged receiver, as it does
  for a local).  Guards: `153-a-tagged-element-read-by-a-variable-index-is-one-null.loft`,
  `153-a-method-call-on-a-tagged-element-reports-the-undischarged-receiver.loft`.
  `Contract: settled`.

### the status line formal/README.md's area table carried until 2026-09-04

**rules written (2026-07-07), 1 open** — the FORMAT counterpart to heap.md's steps (it defines the `field_offset` heap.md reads at); one format (RAM = disk); nullability is a sentinel, not a layout (`L-Null`); **D-layout-1** (no version guard on persisted bytes, #477) is **mechanism-shipped** — the golden test + the `.dschema` sidecar — pending a durable-store consumer to auto-invoke it (@PLN97)

