<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/heap.md — small-step semantics for the store (strict)

**Catalogue:** @F3 (heap/store), @PLN85 (store-lifetime), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the small-step evaluation
> relation for loft's **heap** — the store operations both backends must implement
> identically: allocation, field/element read + write, the whole-value COPY, and the free.
> It extends [operational.md](operational.md)'s scalar core with the heap `H`, and it is the
> written contract for exactly the part operational.md's D-op-1 named as *"unwritten … the
> interpreter remains their spec."*
>
> **Two docs, one heap, different questions.** [ownership.md](ownership.md) is the lifetime
> **CHECKER** — the static `deps` analysis that decides WHICH frees are sound (a value's
> `owns`/`borrows` fact). This doc is the **STEPS** — what `alloc`/`read`/`write`/`copy`/`free`
> actually DO to `H`, independent of whether a given free was wise. They meet at one theorem
> (`H-Sound` below): a `deps`-sound program never takes a step this semantics faults on (no
> use-after-free, no out-of-LIFO free, no aliased mutation of a copy). ownership.md proves the
> program only emits safe frees; this doc defines what a free is.

## The store model (the ground the rules stand on)

- **`H`** — the **heap**: a finite map `store_nr ⟼ Store`. `store_nr` is a `u16` slot index
  (`Stores.allocations`). Slot `0` is reserved for the interpreter's **evaluation stack**
  (`stack_store_at_zero`); it is never a freeable heap store.
- **`Store`** — a word-addressed byte region (`src/store.rs`): a header (signature,
  free-space index, record size) then content, with an intrusive free-block tree. It holds
  zero or more **records**, each a run of bytes at a record index.
- **`DbRef`** = `(store_nr: u16, rec: u32, pos: u32)` (`src/keys.rs`) — the **universal
  pointer** and a first-class runtime **value**: which store, which record, which byte
  offset within it. A field/element access is pointer arithmetic on `pos` — the byte offsets it
  adds are the [layout.md](layout.md) contract (heap.md gives the STEP, layout.md the FORMAT).
- **`nullref`** — `DbRef::NULL`, the reference null. It is the `E-Null` sentinel of the
  reference types (the same in-band discipline as `integer`'s `i64::MIN`).
- **locks** — a `Store` may be `read_only` (immutable: writes and frees fault) or
  `free_protected` (soft: frees fault, writes allowed) — set by the const store, the fn-call
  deep-copy bracket (a caller's argument is protected-from-free for the call), and worker
  borrows. Locks are part of the contract, not an implementation detail: a backend that
  ignores a lock steps differently.

## Notation

- `σ = ⟨ρ, H⟩` — the store/environment splits into the variable map `ρ` (operational.md's
  `σ(x)`) and the heap `H`. `⟨e, σ⟩ → ⟨e', σ'⟩` is one small step, as in operational.md.
- `r` ranges over `DbRef` values; `r ⊕ n` is `r` with `pos` advanced by byte offset `n`
  (same store + record). `H[r]` is the value stored at `r` (read at its width); `H[r ↦ v]`
  is `H` updated at `r`.
- `store(r)` is `r.store_nr`; `H ⊢ r live` means `store(r) ∈ dom(H)` and that store is not
  freed. `fresh(H)` is a `store_nr` not in `dom(H)`.
- `⊥` is the faulting configuration **only** where noted; per operational.md's C80, most
  "can't" cases are the value `null` + continue, **not** a halt (see `H-ReadNull`).

---

## Rules

### Heap values and the reference null

```
  (H-RefVal)   a DbRef r is a value (a normal form); it does not step.  (Named apart from
               `H-Ref` below, the `&`-bind ALIASING rule, which other docs cite by that name.)
  (H-RefNull)  nullref is the reference null — the per-type SENTINEL (E-Null) of a
               reference type.  Two configs that agree on the abstract reference (a live
               record, or nullref) MUST agree, however a backend encodes the sentinel.
```

**In words.** A heap reference is a finished value, just like an integer. `nullref` is a real
value (a reference that points at nothing), not a separate error state — the reference-typed
analogue of `integer`'s `i64::MIN` null.

### Allocation — a fresh store, zero-initialised

```
  (H-Alloc)    ⟨alloc τ, ⟨ρ, H⟩⟩ → ⟨r, ⟨ρ, H'⟩⟩
                 where s = fresh(H),  r = (s, 0, 0),
                       H' = H[s ↦ zeroed Store for τ]      (every field/element is its type's null/zero)
  (H-NewRec)   ⟨new-record r_v, ⟨ρ, H⟩⟩ → ⟨r_e, ⟨ρ, H'⟩⟩
                 a fresh record inside the vector/collection store r_v; r_e points at it,
                 its fields zero/null-initialised, the container's length grown by one.
```

**In words.** Allocating a struct/vector reserves a **fresh** store slot (`OpDatabase`) whose
content starts fully null/zero — a fresh `store_nr` distinct from every live store, so a new
value can never coincide with an existing one. Appending an element (`OpNewRecord` /
`OpFinishRecord`) claims a record inside the container's store. Construction is a pure
extension of `H`: it frees nothing and aliases nothing (this is why *constructing* a host
value is unrestricted under [capabilities.md](capabilities.md)'s `Cap-Own`).

### Read — through a live ref, or null-continue through nullref

```
  (H-Read)      ⟨read(r ⊕ n), ⟨ρ, H⟩⟩ → ⟨H[r ⊕ n], σ⟩            when H ⊢ r live
  (H-ReadNull)  ⟨read(nullref ⊕ n), σ⟩ → ⟨null, σ⟩               (deref of absent = null, CONTINUE)
  (H-Index)     ⟨read(r[i]), ⟨ρ, H⟩⟩ → ⟨H[r ⊕ stride·i], σ⟩       when 0 ≤ i < len(r)
                ⟨read(r[i]), σ⟩ → ⟨read(r[len(r) + i]), σ⟩         when -len(r) ≤ i < 0: a NEGATIVE
                                                                   index names the element that far
                                                                   from the END (v[-1] is the last)
                ⟨read(r[i]), σ⟩ → ⟨null, σ⟩                        when i out of bounds (OOB = null, CONTINUE)
  (H-Stride)    stride(τ) — the distance between consecutive elements, and the distance a dense
                removal slides by (collections.md `Col-RemoveDense`) — is fixed by the declared
                TYPE τ, NEVER by the DEFINITION τ resolves to.  A definition names a KIND; the
                width lives in the type's own spec, and one definition may serve many widths.
```

**In words.** Reading a field/element is a read at the pointer's offset (`pos + field_offset`,
or `pos + stride·index`). Reading through **nullref**, or **out of bounds**, yields **null**
and execution **continues** — the same spreadsheet discipline as arithmetic (operational.md
`E-Uncomp`): an absent value degrades to null locally, it never halts the run.

**Why `stride` is read off the type.** Every primitive but one carries its width on its own
definition — `boolean` is a byte, `single` and `character` are four — so asking the definition
is right everywhere it is asked, except in the one place it silently is not: `integer` is a
SINGLE definition serving seven widths (`i8`/`u8`/`i16`/`u16`/`i32`/`u32` and the 8-byte
default), and the narrowing lives in the type's `IntegerSpec`, which the definition cannot see.
A site that asks the definition therefore answers **8 bytes for every narrow integer**, and
does so consistently enough to look correct: the wide default and every non-integer element
agree with it.

This has been re-derived wrongly **six** times, in five modules, which is why it is a rule and
not a comment: the vector element WRITE (loft#1378, narrow elements written eight bytes wide
into a one-byte slot), the dense REMOVAL (loft#1412, the tail slid eight bytes per element on
both spellings, `len` staying right throughout), a closure's captured cell (loft#1409), and
three more found together (loft#1420) — the vector STRIDE (`element_store_size`, whose narrow
branch asked `forced_size(type_elm(elm))`, a test that can never pass because `type_elm` maps
every `Type::Integer` to the one `integer` def: a dead branch, so `reverse` on a `vector<u8>`
answered `0,0,0,0,0`), the slice-element READ (`read_slice_elem` reaching `get_val` through
`get_field(_, usize::MAX, _)`, whose `attr_type` answers `def.returned`, so `[a, .., z]`
answered `0x05_04_03_02_01`) and the element WRITE behind `insert` (`set_field_check`, losing
the type the same way and zeroing the successors it wrote over).

The shape is worth naming, because it is what makes this rule so easy to violate: each of these
sites converts a TYPE to a definition and back — `type_elm`, `type_def_nr`, `attr_type(_,
usize::MAX)` — and the round trip is lossy in exactly one place, for exactly one primitive.
Nothing at the call site looks like a width decision.

Two homes hold the answer. `Data::vector_element_type` names the element's storage TYPE, and
`Data::narrow_vector_element` names its `(spec, nullable, width)` — the latter being what
`narrow_vector_content` registers the storage through, so a site that measures with it cannot
disagree with the layout it is walking. A site that instead holds the declared type and needs a
value read or written passes THAT type down (`read_slice_elem`, `Parser::set_element`) rather
than recovering it from a def.

**An index is end-relative when it is negative**, so "out of bounds" is `i ≥ len(r)` or
`i < -len(r)`, not simply `i ∉ [0, len)` — `v[-1]` is the last element and `v[-len]` the first
(LOFT.md § Vectors, @P384; the value-slice bound follows the same rule,
[collections.md](collections.md) `Slice-Value`). The consequence is the one the language
reference names: because a negative index in range yields a **real element**, a computed index
that goes negative does not null-guard — `v[i] ?? d` catches `i ≥ len` and not a `-1`
"not-found" sentinel. Both backends normalise at one pair of twinned sites
(`State::vec_get_or_raise` / `Stores::vec_get_or_raise_runtime`), which is also where a still-out-of-range
index becomes `nullref` plus a recoverable fault rather than a silent read.

### Write — update in place; null/lock faults are values or rejects, never wild writes

```
  (H-Write)      ⟨write(r ⊕ n, v), ⟨ρ, H⟩⟩ → ⟨v, ⟨ρ, H[r ⊕ n ↦ v]⟩⟩   when H ⊢ r live, store(r) writable
  (H-WriteNull)  ⟨write(nullref ⊕ n, v), σ⟩ → ⟨v, σ⟩                    (no store to update; a no-op step)
  (H-WriteOOB)   ⟨write(r[i], v), σ⟩ → ⟨v, σ⟩                           when i out of bounds (no-op, CONTINUE)
                 A NEGATIVE i in [-len(r), -1) is NOT out of bounds — it names the element from
                 the END (H-Index), and the write LANDS there.
  (H-WriteLocked)  a write to a read_only store is a STATIC reject where provable, else a
                   runtime lock fault — never a silent successful write.
```

**In words.** A write updates the byte(s) at the target and yields the written value. A write
**through nullref** or **out of bounds** targets no live cell, so it is a no-op that continues
(it must never scribble on an arbitrary address — the null discipline extends to the write
side). Both are the SAME test at the same place: an out-of-range index produces `nullref`, and
every typed setter refuses to resolve a store for it, so `v[9] = x` on a three-element vector
and `absent.f = x` through a keyed miss take the identical step (measured on both backends —
value, length and the neighbouring records all unchanged). A write to a **locked** store is refused (the `#lock` runtime guard / const store),
never silently applied. Crucially, a write's target ROOT decides whose state it touches: a
write whose root is a **parameter** mutates the caller's value; a write to a **local** touches
only that local's own store (see `H-Copy`) — the exact fact [capabilities.md](capabilities.md)'s
`Cap-Own`/raw-write admission rests on.

### Copy vs view — a whole-value / vector bind COPIES; a struct-typed projection is a VIEW

```
  (H-Copy)   ⟨x = r, ⟨ρ, H⟩⟩ → ⟨r', ⟨ρ[x ↦ r'], H'⟩⟩            when x = r is a PLAIN bind of a
               WHOLE VALUE — a local variable, or a VECTOR-typed projection (`fv = e.items`) —
               where r' = (fresh(H),0,0) and H'[store(r')] := deep-copy of the record graph at r.
               x and the source are then INDEPENDENT (C86, #415).
  (H-Ref)    ⟨x = &r, ⟨ρ, H⟩⟩ → ⟨r, ⟨ρ[x ↦ r], H⟩⟩              an EXPLICIT `&`-bind aliases: x
               shares r's backing (NO fresh store), so `x[0] = 99` mutates r (C77).  This is the
               vector twin of a parameter — both reach the source; only a plain bind copies.
  (H-View)   ⟨x = r, ⟨ρ, H⟩⟩ → ⟨r, ⟨ρ[x ↦ r], H⟩⟩               when r is a STRUCT-TYPED PROJECTION
               — a struct FIELD (`c = o.i`) or a struct ELEMENT of a vector (`s = v[0]`).  x is a
               VIEW: it aliases the place inside the container, so a write through x mutates the
               container (#426, ownership.md).  No store is allocated.
  (H-Materialise)  H-View holds only while the PLACE does.  Where the container is DISTURBED
               ([binding.md](binding.md) B-Disturb — a removal, a re-key, or a reassignment of the
               container) while x is still LIVE, the bind takes the H-Copy step instead: x gets a
               fresh store holding the value at the bind, and the author is told.  Writes through
               x then stop reaching the container.  A view whose last use PRECEDES the disturbance
               is unaffected and keeps aliasing (@PLN130 F2/F4/F8).  This is the plain-bind
               answer; an explicit `&` is DECLINED at compile time instead (B-Ref-Reshape),
               because a copy is not what it asked for.
```

**In words.** Whether a bind copies or aliases depends on **what is bound** — and the two backends
agree exactly (verified):

- **COPY** — a **plain** bind of a whole heap **value**: a local variable (`c = o`, `c = v`), or a
  **vector-typed** projection (`fv = e.items`). A fresh store is allocated and the graph
  duplicated, so the two are independent: `fv = e.items; fv[0] = 99` leaves `e.items[0] == 1`;
  `c = o; c.v = 9` leaves `o.v == 1`.
- **ALIAS** — an **explicit `&`-bind** (`r = &v`) binds a **live reference** ([C77](../DESIGN_DECISIONS.md#c77--binding-ownership-heap-aliases-by-default--binds-a-live-reference)),
  NOT a copy: `r = &v; r[0] = 99` makes `v[0] == 99` (verified both backends). This is why a `&`
  is written — to share the backing, not duplicate it. (A vector **parameter** likewise aliases
  the caller — see the invariant below.)
- **VIEW** — binding a **struct-typed projection**: a struct field (`c = o.i`) or a struct element
  of a vector (`s = v[0]`). `x` aliases the place, so a write through it mutates the container:
  `c = o.i; c.v = 9` makes `o.i.v == 9`; `s = v[0]; s.v = 9` makes `v[0].v == 9`.
- **…and a VIEW falls back to COPY when its place is destroyed under it** (`H-Materialise`). A
  container that is reshaped, re-keyed or reassigned while the view is still in use leaves the
  view pointing at nothing meaningful — a removal renumbers positions, so `c` starts naming a
  DIFFERENT element (measured: a read answered `44/444` where its element held `33/333`). The
  bind takes the copy step instead, and says so, so `c = v[0]; v.remove(2); c.n = 99` leaves
  `v[0].n == 11`. Order matters: `c = v[0]; c.n = 99; v.remove(2)` keeps the alias and lands the
  99, because the view is dead before the container changes.

This is the [DESIGN_DECISIONS.md C86](../DESIGN_DECISIONS.md) (`#415`) copy vs [ownership.md](ownership.md)
`#426` view boundary, and it is **exactly** the struct-vs-vector split
[capabilities.md](capabilities.md)'s raw-write rule (D-cap-3) already encodes: a **vector** local is
owned (copies, so writing it is `Cap-Own`), a **struct-typed** local may be a view of host data (so
its writes are host-gated). The invariant a raw write rests on is therefore *the target ROOT*: a
write reaches another binding's state only when its root is a **parameter**, an **explicit
`&`-reference bind** (`r = &v`, whose dep chain reaches the aliased source — possibly a
parameter), OR a **struct-typed view** of one — never a plain copy. The capability gate
(D-cap-3) enforces this by **following the vector's dep chain**: a local vector that aliases a
parameter (via `&`) is host, a genuinely-copied one is script-owned.

### Free — release the store a reference NAMES, once, never the stack

```
  (H-Free)       ⟨free(r), ⟨ρ, H⟩⟩ → ⟨(), ⟨ρ, H \ store(r)⟩⟩
                   when H ⊢ r live, store(r) ≠ 0 (not the eval stack), and store(r) is not
                   PINNED.  A pinned store (const/global) lives for the whole program and its
                   free is a no-op.
  (H-FreeNull)   ⟨free(nullref), σ⟩ → ⟨(), σ⟩                      (freeing null is a no-op)
  (H-FreeAny)    the store released is the one `r` NAMES, whatever its allocation order — a
                 store may be freed while newer ones are live, and that is ordinary.  The
                 table is a slot vector recycled by a BITMAP (`Stores::free_bits`: allocate
                 takes the lowest free slot below the watermark), not a stack that unwinds.
                 Guard `heap_free_discipline.rs::lifo_order_is_not_a_fault`.
  (H-FreeStack)  freeing store 0 (the evaluation stack) is a FAULT (#306): a stack-record ref
                 is never an owned heap store.  REFUSED loudly at runtime.
  (H-FreeTwice)  freeing an already-freed store is REFUSED: a no-op that leaves the allocation
                 table undisturbed.  A refusal and not a halt, because the corrupting frees are
                 discharged STATICALLY by [ownership.md](ownership.md) — reaching one means
                 that system already failed, so the runtime's job is to not compound it.  The
                 danger it averts is slot REUSE: a freed slot is handed out again, so a second
                 free would release someone else's store.
  (H-FreeAll)    every store is freed exactly ONCE, by program exit.  The residue is reported
                 (`N stores not freed at program exit`) — a warning ordinarily and an ERROR
                 under `LOFT_STRICT_STORES`, which asks for exactly-once and so has to fail
                 both halves: a store used after its free, and a store never freed at all.
  (H-RootExtent) a store whose ROOT record is the one-field collection wrapper
                 `OpDatabase` mints holds NOTHING ELSE: every record in it was
                 claimed for that collection — its elements, and whatever they own.
                 The collection's extent IS the store's extent, so a point at which
                 the collection is wholly dead is a point at which the store is, and
                 releasing it there is one store reset rather than a free per
                 element.  What would break the rule is a record placed into the
                 store from outside; the one mechanism that places (R-MoveAppend)
                 places an ELEMENT of that very collection, so it holds.  A field
                 vector, a user-struct root and a placed buffer are not store roots
                 and carry no such claim.
  (H-RootExtent) a store whose ROOT record is the one-field collection wrapper
                 `OpDatabase` mints holds NOTHING ELSE: every record in it was
                 claimed for that collection — its elements, and whatever they own.
                 The collection's extent IS the store's extent, so a point at which
                 the collection is wholly dead is a point at which the store is, and
                 releasing it there is one store reset rather than a free per
                 element.  What would break the rule is a record placed into the
                 store from outside; the one mechanism that places (R-MoveAppend)
                 places an ELEMENT of that very collection, so it holds.  A field
                 vector, a user-struct root and a placed buffer are not store roots
                 and carry no such claim.
  (H-ClearRelease) CLEARING a vector that OUTLIVES the clear releases what its
                 elements own.  `clear_vector` is a length reset — sound wherever
                 the vector's store dies straight after (every use it was written
                 for) and unsound for a vector the ABI REUSES: a shape-A hidden
                 return buffer, or any store-root vector cleared per call.  There
                 each clear strands the previous elements' owned heap in a store
                 that never dies, unbounded in the call count.  The release arm is
                 gated on the SHAPE and the ELEMENT TYPE, both read from the
                 layout: the store's root record is a one-field `main_vector<T>`
                 wrapper (what `OpDatabase` mints — asking the shape, never a byte
                 offset, is what keeps the two backends together: the field sits at
                 8 on `--native` and 12 on the interpreter) and the element type
                 OWNS HEAP.  A no-heap element pays exactly the old reset; a field
                 vector, a user-struct root and a placed buffer (never record 1)
                 keep it too.  Values are unaffected either way — this is a leak
                 rule, not a semantics rule.  The release is ONE STORE RESET, not a
                 walk: the shape above says the vector owns the store's whole
                 extent, so the store is re-initialised and its two records
                 re-established — the wrapper, and the vector's own record at
                 length zero — instead of deleting each element's owned blocks
                 into the free tree.  The cleared vector must stay PRESENT, since
                 an absent heap value is falsy where an empty one is true.
                 A RECORD the ABI refills is the same case: a hidden return
                 buffer REUSED across calls (R-Reuse hands one record to every
                 call of a site) is written by the callee's literal field by
                 field, and the literal overwrites every handle it writes, so
                 what the previous occupant owned is released before each call
                 after the first.  The release is the reuse's, at the call site
                 that reuses: a buffer offered once (a placed record, a return
                 arena) holds nothing yet and pays nothing.  A record buffer is
                 not a store root, so the release is the walk of its fields, not
                 a reset; it walks the record's own type, or a struct-enum's
                 PARENT type, so it follows the variant the buffer HOLDS rather
                 than the one about to be written.  A record of scalars has
                 nothing to release and emits nothing.

  (H-FreeFooter) inside one store, a FREE block of n words carries −n at BOTH ends: its
                 header word and the HIGH half of its LAST word (the tree node's color
                 rides bit 31 of its RIGHT link, so the half-word is clear at every
                 tracked size; a one-word block's footer shares its header's word).  A
                 record delete then coalesces BACKWARD in O(1): the footer at the freed
                 block's left edge NAMES a candidate predecessor, its header must agree,
                 and the free TREE must hold that exact block — claimed data can spell a
                 false footer and header, and the tree cannot lie (falsified: without the
                 tree confirmation, a fabricated footer+header pair merges a freed block
                 into the middle of a live claim).  A one-word predecessor is untracked
                 and unconfirmable: it is left, and the lazy O(blocks) sweep stays armed
                 for exactly that case.  Footers live in FREE space only — a persisted
                 image is unchanged, and an image written before footers existed is
                 re-footed by the open walk.
  (H-Wilderness) inside one store, the free block that ENDS the store (the wilderness)
                 is held beside the free tree rather than in it, and every tree
                 operation treats it as the node it would have been: a block of at
                 least the tree's minimum size that ends the store is recorded as the
                 wilderness instead of inserted, removing it clears the record, and a
                 best-fit take weighs it against the smallest fitting node by the
                 tree's own (size, position) order — the wilderness has the highest
                 position of any free block, so a node of equal size precedes it.
                 The block every claim takes, and so the store's layout, is therefore
                 the one the tree alone would give; what changes is that a claim from
                 the tail and a delete into it cost no tree delete, insert or
                 rebalance.  At most one wilderness exists, it never overlaps a claim,
                 and the open walk (and every re-tiling) re-derives it.
```

**`H-RootExtent` is what makes `H-ClearRelease`'s release affordable.** The release has to
reach everything the cleared elements own, and it can do that two ways: walk the elements and
return each owned block to the free tree, or — knowing the collection owns the store's whole
extent — reset the store and re-establish the two records the walk would have left. The
second is O(1) where the first is a delete per element, and it also restores the allocator's
bump path, which a fragmented store never reaches again. The rule is what licenses it: without
"the store holds nothing else", a reset would drop a record someone still reaches. It is
ASSERTED by construction rather than proved — `OpDatabase` mints the wrapper and every later
claim in that store is made for the collection — so the gate that reads it also reads the
shape (`clear_vector_release`), and a shape that is not a store root keeps the walk.
The same extent is what lets the reset re-establish the vector at the capacity the previous
fill reached rather than at the fresh minimum (`vector::reached_capacity`, read off the
record before the reset): the space is the store's already, the buffer is reused across
calls (R-Reuse), and a ladder re-run per call would free a rung into the store each step
and take every later claim off the bump path (@PLN157 § V-ai).

**In words.** `free` releases a store slot and everything in it. It is disciplined: (1) **LIFO** —
you free stores in reverse allocation order, because a store's lifetime is nested inside the
stores allocated before it; (2) never the **stack** store (a `#306` bug is exactly a
stack-record ref mistaken for an owned heap store); (3) never **twice** (a double-free), and
never a store still reachable from a live binding (a use-after-free); (4) never a
`free_protected` store (a caller's argument during a call). `free(nullref)` is a harmless no-op.
Unlike a read/write, a bad free is a genuine **fault**, not a null-continue — it corrupts the
heap, so the discipline is a hard invariant, not a degradable value.

**The `free_protected` side condition is the one a CALLEE cannot decide for itself, and it is
enforced where the decision is made.** A free releases a store the *frame doing the freeing*
believes it owns, and one construct hands that belief across a frame boundary: a `&`
parameter's whole-value write-back installs a store the callee minted and displaces whatever
the caller's binding named. The callee sees a `DbRef` and nothing else — not whether the
caller OWNS that store. Where the caller's binding is a plain heap PARAMETER it does not
(`calls.md` F-ParamHeap: the parameter aliases *its* caller's argument), so the store belongs
to a frame two below and the free is an `H-FreeTwice` waiting for the real owner's own
release, with every read in between a use-after-free. The call site is the only place that
knows, so it MARKS the store for the duration of the call and `Stores::free_displaced`
refuses it — the marker is how a static ownership fact reaches a callee compiled once for
every caller (loft#1287, `scopes::scan_args`). Nothing widens: the mark names one store, the
parameter's ENTRY store, so a REPEATED call still releases the fresh store the previous one
installed, which the frame does own.

> **D-heap-LIFO — OPENED 2026-09-09, CLOSED 2026-09-12.**  `(H-FreeLIFO)` stated a fault the implementation
> deliberately stopped requiring, and nothing enforces it.  `Stores::free_bits` (S29) says so
> in its own doc — a bitmap of free slots, so `database_named` reuses the lowest free slot
> below `max`, which *"eliminates the LIFO-order requirement on `free()` that the old
> cascade-based scan imposed"*.  Measured from the other side: `scripts/rule_tags.py sites
> H-FreeLIFO` reports **zero** citations, alone among heap.md's five free rules, and there is no
> LIFO check anywhere in the free path.
>
> Freeing an older store while a newer one is live succeeds and leaves the newer one untouched
> (`tests/heap_free_discipline.rs::lifo_order_is_not_a_fault`), and slot REUSE — the thing a
> LIFO discipline exists to avoid needing — is what makes a double free dangerous in the first
> place, which is the rule that replaced it.
>
> It stayed open because the wording was a DESIGN call, not a transcription, and @PLN155 phase 4
> measured it without taking that call.  **The owner took it on 2026-09-12: the rules are
> rewritten to how the mechanism functions.**  `(H-FreeLIFO)` is gone; `(H-FreeAny)` states the
> positive fact in its place, and `(H-FreeAll)` states the completeness obligation that was
> never written down at all — every store freed exactly once by program exit, which the runtime
> has always reported and `LOFT_STRICT_STORES` has always made an error.
>
> Neither the deletion nor the "weaker nesting invariant" of the two candidates was taken
> whole.  Nesting is not a HEAP rule: the mechanism has two free-lists (a slot vector recycled
> by `free_bits`, and `claim` over an LLRB tree inside each store's buffer) and neither asks
> which store was newest, while single-ownership is discharged by
> [ownership.md](ownership.md) — `free_named` carries no ref-count at all (@PLN57 phase C:
> *"every non-pinned store is single-owner … so `free_named` always frees"*).  Writing nesting
> here would have put the obligation in the doc that does not enforce it.
>
> ⚠ **Two neighbours were found stale by the same reading, and both are corrected above.**
> `(H-Free)`'s premise carried *"store(r) is the MOST-RECENTLY-ALLOCATED live heap store"* — the
> same retired requirement, in the rule that HAS citations, so the tagged rule was not the one
> constraining code.  And it required `store(r) not free_protected`, which `free_named` never
> checks: `is_free_protected()` is read at the deep-copy CALL SITES that decide whether to
> release a source, so it is a caller's gate and not a precondition of the primitive.

### Drop — a structure's lease is released once, at that structure's death

```
  (H-Drop)   a type that declares `OpDrop` releases a RESOURCE the record holds (a handle,
             a lock, a file), and the hook runs ONCE per STRUCTURE that holds a lease on
             it (H-Lease), at that structure's death:
               - the owner's scope end (@PLN125 arc B), in reverse declaration order;
               - a REASSIGNMENT of the owner that displaces the record — a rebuild in
                 place, a call result, a rebind to null — after the new value has been
                 computed and before anything after the statement runs (loft#1362);
               - a CONTAINER's death for what it holds, its own hook first and then its
                 members, through the synthesized cascade (C111).
             ⚠ **A DROP HAS NO SAFE DIRECTION, and a cure discussion that assumes one is
             already wrong.**  For a FREE there is one: never free what might still be held,
             because a leak costs memory and a double free corrupts.  A drop's two failures
             are not ordered that way — a hook that does not run leaves the resource open,
             and a hook that runs twice closes a handle the author already closed, which is
             the author's own use-after-free.  So a change that converts one into the other
             is not progress and must not be landed as an improvement (measured 2026-09-10 on
             loft#1515's shared-destination shape: giving `drop_transferred` a per-path merge
             turns every lost hook in that family into a doubled one).  The consequence for
             the cures below is that "err toward the safe side while the real fix is designed"
             is not available here: a drop's fix has to be RIGHT on every path, which is why
             `(O-Complete)`'s per-path clause and this rule meet so often.
  (H-Lease)  a structure whose type OWNS a droppable — the type declares `OpDrop`, or holds
             a member that does, at any depth — holds its OWN lease on the resource, and
             two structures are two drops.  A copy makes a second structure, so it takes a
             second lease (H-Copy-Lease) or is refused (H-Copy-Refuse); only a MOVE (H-Move)
             hands a value to a new owner without one.  Moving bytes without making a
             structure — a vector's growth, a keyed collection's rebalance, a store's
             compaction — is not a copy.
  (H-Move)   a MOVE is WRITTEN, never inferred: whether a position moves is read off the
             line and the declared types alone, whatever the program does after that line.
             Four positions move a value:
               - a FRESH value — a call result, a constructor, a literal — placed where it
                 is produced: bound, written into a field, element or tuple member,
                 appended, or returned;
               - a value this function OWNS — a local it bound to a fresh value, and never a
                 parameter, a member of a container, or a captured variable — placed into a
                 new structure: bound, written into a field, element or tuple member,
                 appended, or returned.  Its lifetime ENDS there: the new structure releases
                 it, and the name is SPENT (H-Spent);
               - a `return` whose every possible value is a variable this function OWNS
                 (not a parameter, not a view) or a fresh value — `return a`,
                 `return a ?? mk()` — since the function's own variables end with it;
               - a block whose value is a variable declared in that block
                 (`if c { a = mk(); a } else { … }`).
             Every other position that places an existing value into a new structure is a
             COPY.
  (H-Spent)  a name whose value has MOVED to a new owner is SPENT from the END of the
             statement that moved it: reading it is a compile-time error, and the error names
             where the value went.  Reads inside that same statement are not late — the
             members of `Named { h: c, label: c.tag }` are read before the structure is
             built, so both are reads of a value the name still holds.  A reassignment
             REFILLS the name, which is spent again only if it moves again.  On a path where
             the move did not run the name is NOT spent and still owes its release, so a move
             written under a branch is decided PER PATH — one name may be spent on one path
             and live on another, and the release must run on exactly the paths that did not
             move it.
  (H-Copy-Lease) a copy of a type that declares `fn OpCopy(self: τ)` copies the bytes and
             then runs `OpCopy` on the NEW structure, which takes its own lease there.  A
             struct holding such a member gets a synthesized copy cascade, the mirror of the
             drop cascade: its own `OpCopy` first, then its members'.
  (H-Copy-Refuse) a COPY of a type that owns a droppable without `OpCopy` — the type itself,
             or a member at any depth — is a COMPILE-TIME ERROR on the line that writes it,
             whatever the program does after that line.  A copy places a value the function
             does NOT own — a parameter, or a local holding the caller's record, since the
             caller releases it; a member of a container (`s.h`, `v[i]`, `t.0`, `mk().h`),
             since the container releases it; a loop variable or a `match` binding over one;
             a captured variable — or a name already SPENT (H-Spent), into a new structure:
               - bound to a variable as a whole value (binding.md B-Copy: `x = p`);
               - written into a field, enum payload, element or tuple member of a literal,
                 or appended (`S { h: p }`, `v += [s.h]`, `(p, 1)`);
               - returned where (H-Move) does not allow it (`return p`, `return s.h`);
               - as an operand of `??` or an arm of a join in any of those positions
                 (`x = p ?? mk()`, `S { h: s.h ?? mk() }`).  A MEMBER chosen by an arm
                 and bound to a variable is not a copy: the arm is read on its own
                 (binding.md D-bind-16), and there it is a view (B-View, `x = s.h`).
             Legal, because no second structure is made: a fresh value anywhere, a value the
             function OWNS placed ONCE (H-Move — its lifetime ends there), passing a value as
             an argument (calls.md F-ParamHeap binds without copying), a `&` bind
             (binding.md B-Ref-Alias), and a view of a member of a variable (B-View,
             `x = s.h`).  The error names the copy and what to write instead: use the value
             where it is, pass it, build it where it belongs, or return the owner — and for a
             spent name, read it through the owner it moved to.
  (H-View-Drop) a VIEW of a member that owns a droppable without `OpCopy` stays a view: the
             compiler never turns it into a copy.  Disturbing its container while the view
             is still used (binding.md B-Disturb) is a COMPILE-TIME ERROR reported at the
             disturbance and naming the view — (B-Ref-Reshape), for a view the author did
             not spell with `&`.
  (H-Elide)  the compiler may elide a copy of a type with `OpCopy` together with the drop
             of the structure the copy would have made, where C86's transparent-link
             conditions hold.  Neither hook may rely on running for an elided copy.
  (H-Drop-Not) the OLD value of an overwritten FIELD or ELEMENT (`o.s = other`,
             `v[i] = other`), an element taken OUT (`v.remove(i)`), and a keyed
             collection's records are NOT released by the language — the author releases
             them (INTERFACES.md § `OpDrop`, the documented boundary).
```

**In words.** A drop is not a destructor of loft-side data — the ownership model pays for
that — it is the release of something outside the program. So the question every site asks is
*"which structures hold a lease on this resource?"*, and each of them is released once, at its
own death. Copying a droppable makes a second structure, and a second structure needs a lease
of its own. A type that can make one says so with `OpCopy`. A type that cannot — an outward
connection, a writable file, a database cursor, whose duplicates would share one stream and one
position — has its copies refused at compile time. Whether a line copies is read off that line
and the declared types: no later line changes it. What makes no second structure stays legal: a
fresh value placed where it is made, passing a value to a function, a `&` link, a view, and
returning what the function owns.

A call such as `a = f(a)` is decided inside `f`.  `return p` of a parameter copies a structure
the caller still holds, so for a refusing type `f` is refused there.  The loft spelling of that
idiom does not return the value: `f` writes through its parameter, which aliases the caller's.

**Why the verdict is read off the line.**  The first version of these rules, the same day, made a
copy of a value that is not used afterwards a move.  Then `x = a; send(x)` compiled and
`x = a; send(x); send(a)` failed at its FIRST line: a later line decided what an earlier one
meant.  Correct and incorrect code looked the same to anyone who does not know how the compiler
reasons, and the owner rules that out — a programmer must be able to judge from the code whether
it is valid.  Liveness may still decide an optimisation (`(H-Elide)`), never validity.  A
droppable is a corner of the language with a clean workaround in nearly every case — build the
value where it belongs, pass it, borrow it — so the strict rule costs little.  It also keeps a
library that holds a connection readable: every use of `self.db` is a borrow in plain sight, and
the one line that would duplicate it is the line the error names.

**Why a refusal, and not a moved release, a warning or a mark.**  The earlier rule moved the
release along with a copy and warned where the owner was unclear.  Wherever both structures
lived on, that left a gotcha: the release went with one of them, and the program closed a
handle the other was still using, or closed it twice.  A mark inside a structure recording
which member was moved out, or a static fact deciding which copy owns, would each leave the
programmer guessing which structure still works.  Refusing the copy leaves no guess, because a
program that compiles has one structure per lease.  Decided with the owner 2026-09-15 from the
use cases in `plans/163-copy-leases.md` (DESIGN_DECISIONS.md C121).

**Conformance.** `tests/scripts/139-drop-cascade.loft` (the cascade),
`a-whole-value-copy-of-a-droppable-releases-once.loft` (a whole-value copy), and
`1362-a-rebind-releases-the-droppable-it-displaces.loft` (the reassignment), each measured
identical on both backends.  The copy rules have no conformance yet: `(H-Copy-Refuse)` is
`D-heap-8`, `(H-Copy-Lease)` is `D-heap-9`, and `(H-View-Drop)` is `D-heap-11`.
`tests/ownership_drop_gate.rs` classifies every generated cell under these rules and ties each
cell that disagrees to its deviation.  Sites: the deaths are `scopes::displaced_drop` and
`scopes::scope_end_drop`; `scopes::copy_moves_drop_from` and `scopes::copy_hands_off` move the
release across copies these rules refuse, until `D-heap-8` closes.

### The soundness bridge — a well-typed program never faults a free

```
  (H-Sound)   if a program is `deps`-SOUND (ownership.md O-Derived + O-Complete: every free
              it emits is on an OWNED store at its last use), then no execution reaches a
              refused free — H-FreeStack / H-FreeTwice never fire, no read observes a freed
              store, and H-FreeAll's residue is empty.  The static checker discharges the
              dynamic invariant, which is why the runtime REFUSES rather than halts.
```

**In words.** These free rules describe what a free *does* and when it *would* corrupt the heap.
The promise that a real loft program never hits a corrupting free is **not** re-checked at
runtime — it is discharged statically by [ownership.md](ownership.md)'s `deps` checker: it only
emits a free on a store the value provably OWNS, at its last use, so LIFO holds, the stack is
never freed, and nothing freed is later read.

⚠ **That discharge is only as strong as the checker's register, so READ THAT REGISTER — do not
read a number restated here.** This paragraph named `D-own-8` and a path-completeness gap for a
cycle after `D-own-8` closed, while `ownership.md` itself read `OPEN: 0`; the two disagreed and
the gated side was the other document's. As of 2026-09-11 ownership.md is at `OPEN: 1` with
`D-own-40` — `(O-Witness)` armed where its premise is false, which is an ARMING defect rather
than the path-completeness one this paragraph used to describe, so it does not bear on `H-Sound`
the same way. Open [ownership.md](ownership.md)'s own `OPEN` line and its entries before treating
a free fault here as impossible; a count copied into this file is the way that reading goes
stale. This doc
defines the cliff; ownership.md proves the program walks the path beside it. The
`LOFT_POISON` harness is the empirical cross-check: it overwrites freed stores with a poison
pattern so any surviving `H-FreeTwice` / use-after-free surfaces as a corrupted read.

---

## Deviations

OPEN: **4** — `D-heap-8`, `D-heap-9`, `D-heap-36` and `D-heap-38` (`D-heap-15` CLOSED as
reclassified 2026-09-23: its last cell, `p_v2`, reads a name `(H-Spent)` makes an error, so it is
`D-heap-8`'s.  `D-heap-42`,
loft#1628, opened and CLOSED 2026-09-23: a returned witnessed local released the record it handed
out, and a rebind released what it displaced before the new value and at the wrong owner's scope
end — the half `D-heap-41` left open.  `D-heap-41`,
loft#1623, opened and CLOSED 2026-09-23: a returned join binding released what it handed out —
the half `D-heap-39` filed beside itself rather than closing.  `D-heap-39`,
loft#1617, and `D-heap-40`, loft#1622, both opened and CLOSED 2026-09-22: a lifted arm's hook
handed the SOURCE's record, and the cascade key that ICEd on two droppable types behind nullable
fields — the second found while building the first's matrix.  `D-heap-39` leaves loft#1623 beside
it, the same lift form RETURNED releasing in the callee and again in the caller, which is filed
rather than closed because its cure wants a design; `D-heap-36`,
loft#1600, opened 2026-09-22 as a design question; `D-heap-37`, the hoist order found beside it,
opened and closed that day; `D-heap-38`, what an arm-scope rule would leave for a vector bound in
both arms, opened that day; `D-heap-34` and `D-heap-35` opened and closed
2026-09-22, loft#1597 and the struct-enum unit literal found beside it; `D-heap-26` and `D-heap-28` closed 2026-09-22;
`D-heap-30`, `D-heap-31` and `D-heap-32` opened and closed 2026-09-22, loft#1594, loft#1596 and
loft#1598; `D-heap-28` and
`D-heap-29` found 2026-09-22 while closing `D-heap-15`'s `p_i2`, the second closed that day; `D-heap-27`, the tuple twin of
`D-heap-21` and `D-heap-22`, opened and closed 2026-09-22; `D-heap-22` and `D-heap-24`
closed 2026-09-21, `D-heap-25`, found in D-heap-24's controls, and `D-heap-26`, found in
D-heap-22's, both opened that day and the first closed; `D-heap-13`
closed 2026-09-20; `D-heap-16` closed 2026-09-21 together with the three it uncovered,
`D-heap-18`, `D-heap-19` and `D-heap-20`, each opened and closed that day; `D-heap-14` closed the
same day, and `D-heap-21` and `D-heap-23` opened and closed with it; `D-heap-22`, `D-heap-21`'s
loop-body face, and `D-heap-24`, found beside `D-heap-23`, still open).  The
first two are the copy-lease rules `(H-Copy-Refuse)` and `(H-Copy-Lease)`, written 2026-09-15
before their implementation (@PLN163); `D-heap-8` was NARROWED 2026-09-17 when the owner ruled that
a value the function owns MOVES, which makes 156 of its 227 measured sites legal and leaves the 71
that are not the function's.  `D-heap-13` and `D-heap-14` were both found while measuring that
population — a collection a CALL answers, bound to a local, never releases its elements
(loft#1551, CLOSED 2026-09-20: a cascade of the collection's own, run over the binding the
buffer delivered to), and a hand-over written under a BRANCH leaks the source on the path that
does not run, for every destination except a bind to a local.  `D-heap-15` and `D-heap-16` were EXPOSED
by the ruling rather than found beside it: narrowing the verdict to what the rules move turned 23
gate cells from `Refused`, which asks nothing of a release, into `Once`, which asks for exactly
one — 21 of them run two and 2 run none.  A cell the rules refuse is a cell whose releases nothing
measures, so a register that refuses widely hides what it has not yet judged.  The third of the copy-lease
set, `D-heap-11` for `(H-View-Drop)`, CLOSED 2026-09-17: a view of a droppable member is no longer
turned into a copy — the disturbance is refused, which is what the rule asked for.  The rules were revised the same day to judge a copy by its own line (§ Drop, *Why the
verdict is read off the line*), which reclassified the older entries: every shape of `D-heap-1` and
`D-heap-7` that writes a copy of an existing value belongs to `D-heap-8`, so `D-heap-1` and
`D-heap-10` are CLOSED as reclassified.  `D-heap-7` kept `return a ?? b` over two locals, the one
cell those rules still required to release once, and CLOSED 2026-09-15 with the two neighbours
measuring it found (§ D-heap-7).
`tests/ownership_drop_gate.rs` gives every generated cell a lease verdict and ties each cell that
must release once, and does not, to exactly one open entry — none, since that close.
`D-heap-LIFO`
CLOSED 2026-09-12 by rewriting the rules to how the mechanism functions.  The count read **1** while `D-heap-LIFO` was
already written and marked OPEN in the rules section — an `OPEN: n` is a claim to re-measure,
and a deviation placed beside its rule rather than under this heading is the way it goes
stale.  `D-heap-2` (a cascade that reached three of the member kinds it owned) opened and
CLOSED 2026-09-10, below; it is why D-heap-1's own list never counted it.  `D-heap-3` (a
struct field projected off a CALL result) closed the same day, in three parts and by three
hands: the dense family, then the `?`/`??` join and the returned projection, then the
MEMORY-only residual those left behind — a binding and its materialising work-ref freeing one
store twice, in the order their declarations happened to fall, which is `D-heap-5`
(loft#1513).  Its count read **3** while D-heap-3's own heading already said OPENED AND
CLOSED — this line and the entry below it are two readers of one fact, and a three-handed
close updated only the entry.  `D-heap-6` (a literal built through a work-ref read as a VIEW,
so a local with two OWNING assignments was witnessed as a mixed one and its records were released
by nobody) opened and CLOSED 2026-09-11 — it is D-heap-4's neighbour rather than
a re-open of it: that entry made the construction hand-off conditional on an ownership fact,
and this one rides the fact being decided wrong.  It needed TWO mechanisms, because the fabricated mix was
MASKING an `(O-Detach)` defect that made both arming cures regress a value until it was fixed
first (`D-own-41`); the entry carries the cure that landed and the one that was not taken.  D-heap-1's list has
been re-cut twice as it was measured — a shape closed, a shape that turned out to be the
opposite fault, and a shape found by widening one cell — so the three named there are what is
open TODAY and not the original filing.  `D-heap-4` (a mixed own/view local's owned record
freed without its hook) opened and CLOSED 2026-09-10, below.  `D-heap-12` (a refilled record
buffer stranding what its previous occupant owned) opened and CLOSED 2026-09-17, below.  `D-heap-17` (a self-append's claims walk
read the source record through a number captured before the growth relocated it) opened and
CLOSED 2026-09-17, below.

### D-heap-42 — OPENED AND CLOSED (2026-09-23, loft#1628): a returned witnessed local released the record it handed out, and a rebind released what it displaced in the wrong place

- **Violates:** (H-Move) — a `return` moves what it answers — and (H-Drop)'s reassignment
  clause, *"after the new value has been computed and before anything after the statement runs"*.
- **Where:** a local bound from a join that VIEWS on one arm and takes a local's record over on
  the other (`x: H = s.h ?? b`), then rebound to a mint, carries an owner witness
  (`(O-Witness)`).  The join declined the per-arm write-out, so the local ALIASES `b` on the path
  that took it.  So which structure holds the record `x` answers is a per-RUN fact: the witness
  after a rebind, `b` on a path that never rebinds, `s` on the path that viewed the member.
  - At a `materialized_view_return` the return copied `x` out and then ran the holder's hook as
    well — the witness's in `x = mk(9); return x` (`m2,m9,d9,d7,G9,d9`), `b`'s where the rebind
    did not run (`m2,d7,G7,d7`).
  - A minting rebind (`WitnessSet::Mint`) released the witness's record in the statement's
    PREFIX, before the call (`R9,d9,m11`).  And the record `x` took over from `b` was released
    only at `b`'s scope end, not at the rebind that displaced it.
- **Effect:** one lease, two releases — the second on memory the first had freed — plus two
  releases in the wrong place.  Both backends identical, no diagnostic, on `main` and on every
  branch since `(O-Witness)` landed.
- **Status:** CLOSED 2026-09-23.  Filed from loft#1623 as a design question, because one
  suppression rule could not name both holders from a fact that tells them apart.  The rules
  settle it.  `(H-Move)` makes the join's local arm a MOVE, so `x` holds `b`'s lease on that
  path.  `(B-View)`, read arm by arm (binding.md D-bind-16), makes the member arm a view.  And a
  return's move is of the RECORD `x` names, so the holder that names that same record is the
  one whose hook stands down.  That is a runtime fact, and record identity reads it with no new
  channel.
- **Closed:** `return_moved_holders` names the witness and the plain locals the witnessed local
  was bound to as they are (`Scopes::witness_aliases`, recorded at each `Set`).  The return
  sweep guards each one's hook on `OpNeRef(holder, x)`.  RECORD identity, not store identity: on
  the member path `s` shares `x`'s store and still owes its own release (loft#1623's `c4`).
  A rebind snapshots `OpEqRef(x, b)` before the new value and, after it lands, runs `b`'s hook
  and sets `b`'s `__hoff_` flag, so its scope-end hook stands down.  Its STORE is still freed
  there, with the call buffer it shares.  A minting rebind releases the witness through the same
  identity-guarded path a reading mint always used.  `Uses::is_hook_release` reads a guarded hook
  as release machinery, so the copy lint does not count the guard's operand as a later use.
  Guard `tests/scripts/1628-a-returned-witnessed-local-hands-out-the-record-it-holds.loft`: 11
  cells, 3 controls, every trace exact, falsified at `4af6d4c41` on both backends.

### D-heap-41 — OPENED AND CLOSED (2026-09-23, loft#1623): a returned join binding released what it handed out

- **Violates:** (H-Move) — a `return` moves what it answers to the caller, and the function's own
  variables end with it — with (H-Drop)'s *"a hook that runs twice closes a handle the author
  already closed"*.
- **Where:** a binding whose value branch declined the per-arm write-out BORROWS a temp per arm
  and releases nothing itself (`Scopes::lift_join_arm_tails`), so at a `materialized_view_return`
  the value the return moves is the TEMPS', not the binding's.  `return_copies_whole_local`
  answers the owned-local spelling and declines a view — correctly, since a view owes no release
  — and nothing answered for the temps behind it, so they released at the callee's exit what the
  caller had just adopted.
- **Effect:** one lease, two releases — the second on memory the first had freed.  Measured
  2026-09-23, both backends identical, no diagnostic: `fn f() -> H { s = SN { h: null };
  b = mk(2); x: H = s.h ?? b; x.id = 7; return x; }` traces `m2,d7,G7,d7` where the owned-local
  spelling `return b` traces `m2,G7,d7`.  A caller that rebinds the result separates the two
  records and shows the doubled hook firing on the callee's id.
- **Status:** CLOSED 2026-09-23.  Found while measuring loft#1617's matrix (cell A15) and filed
  rather than fixed there, because the cure needed a design the identity fix did not.
- **What made it more than a suppression.**  The temps come from THREE places and the third is
  why the fix records them where they are made rather than reading the binding's deps: the arm
  lifts, the minting-call arms the scope pass gives a temp, and the arms the PARSER already owns
  — whose `join-arm-owner` `__ref_N` the dep list never names and which hooks all the same.
  Two cells then say what a holder is NOT:
  - a container the frame KEEPS is not one.  On the path where `x = s.h ?? b` chooses `s.h`, the
    binding views `s.h`, the caller gets a COPY of a member, and `s` still owes its own release
    — so the callee's hook there is correct.  Guard cell `c4`, which carries a release in the
    callee that every failing cell does not.
  - a temp whose value nobody took is not one either, which makes the fact PER EXIT rather than
    a hand-off recorded once: a second exit returning something else leaves the temps holding a
    value the caller never saw, and there their hook is the only release it gets.  Guard cell
    `c7`.
- **Closed:** `Scopes::join_holders` records the temps against the binding that borrows them, and
  `return_copies_view_holders` reads them at the return into `arm_dropped` — the per-exit
  mechanism loft#1515 shape 2 already established, so the suppression lasts exactly one exit.
  Guard `tests/scripts/1623-a-returned-join-binding-hands-out-its-arms-temps.loft`, 9 cells and
  2 controls, clean under `LOFT_STRICT_STORES=1 LOFT_POISON=1` before and after.
- **Left open beside it:** a join binding REBOUND to a fresh value and then returned releases
  that fresh value in the callee AND at the caller (`x: H = s.h ?? b; x.id = 7; x = mk(9);
  return x`).  The lift's own release is correct there — nobody took it — and the double is on
  the REBOUND value, so it is the rebind axis rather than this one.  Pre-existing by
  construction: this fix only ever REMOVES a hook at a return and cannot produce a second.
  Filed separately as loft#1628 — `D-heap-42`, closed the same day.

### D-heap-39 — OPENED AND CLOSED (2026-09-22, loft#1617): a lifted arm's hook is handed the SOURCE's record, not the one the binding holds

- **Violates:** (H-Move), and (H-Drop)'s *"act on what the value holds"*.
- **Where:** `Scopes::lift_join_arm_tails`.  Where a value branch cannot be written out per arm —
  an arm that VIEWS, or a `??` chain block as the whole value — the binding BORROWS a per-arm
  `__lift_N` temp instead of owning its copy, and each `__lift_N = a` is a `(H-Move)`: the temp
  holds the structure from there on.  The hand-off ran the other way round.  `handoff_target`'s
  per-path form answered the DESTINATION, so the temp was put in `drop_transferred` — statically
  silent — and the source kept its own scope-end release.
- **Effect:** the one release the resource earns is handed the SOURCE's record.  The COUNT is
  right in every shape, which is why `ownership_drop_gate`'s "releases each resource once" is
  green on all of it: with no write between the bind and the release the two records carry equal
  bytes, and only a write to one of them separates them.  Measured 2026-09-22, both backends
  byte-identical, no diagnostic — `b = mk(2); x: H = s.h ?? b; x.id = 7` traces `m2,R7|B2,d2`
  where the plain bind `x: H = b` traces `m2,R7|B2,d7`, so the two spellings of one value
  disagree about which record the hook acts on.  A hook exists to close THAT handle or free THAT
  slot; handed another record it acts on the wrong resource.
  The filed scope was the `if` ARM and the matrix says otherwise: a chain at STATEMENT level is
  the same defect and the `if` is incidental.  What decides it is whether the per-arm write-out
  applies — a branch whose arms are all owned was right throughout.
- **Status:** CLOSED 2026-09-22.  Found by an edge sweep over `??` spellings while closing
  `D-bind-50`; the guard that issue shipped says so in its own header, because a value cell
  cannot see a release identity and its droppable cells recorded the release alone.
- **Closed:** the direction is the rule's.  The lift releases what it holds, and the SOURCE's
  release is guarded on a per-path `__hoff_` flag — `D-heap-14`'s machinery, one axis over: the
  arm may not run, and on the path that did not copy the source still owes its release.  A copy
  off a PARAMETER still stops itself, since that record is the caller's.  Measured on a 16-cell
  matrix moving the statement context (statement level, `if` arm, `match` arm, loop body,
  returned), the arm kind (a chosen field, a chosen local, a call beside either), the operand
  count, the route to the release (a direct hook, a cascade through a member) and the source's
  later life (rebound after the chain): 10 of 16 named the wrong record before, none does after,
  and the six that were right are unmoved.  Guard
  `tests/scripts/1617-a-lifted-arm-releases-the-record-the-binding-holds.loft`.
- **Left open beside it:** loft#1623 — the same lift form RETURNED releases in the callee and
  again in the caller.  One lease, two hooks; pre-existing, and this entry moves which record the
  callee's spurious hook names without changing the count.  It is not closed here because the
  binding's holders come in two kinds — a `__lift_N` its dep list names and a parser-made
  `join-arm-owner` `__ref_N` it does not — so the cure wants a per-binding record of them
  written where they are made, which is a design and not a filter over the dep list.

### D-heap-40 — OPENED AND CLOSED (2026-09-22, loft#1622): two droppable types behind nullable fields mint ONE drop-cascade key

- **Violates:** (H-Drop) — a container's death releases what it holds, through the cascade — and
  [interfaces.md](interfaces.md) `(G-Key)`, which asks that a definition key identify its
  definition.
- **Where:** `synth_drop_cascades` gives an enum VARIANT a cascade of its own, keyed by
  `mangle_method(def.name(), "OpDropAll")` — the BARE variant name.  A variant name is unique only
  WITHIN its enum, which is exactly what `Data::variant_of` exists to say (*"two enums may share a
  variant name and a variant is never found without a contextual enum"*).  Every `τ?` lowers to a
  `__nullable<τ>` whose payload variant is named `Some`.
- **Effect:** two distinct droppable types each reached through a NULLABLE FIELD both mint
  `t_4Some_OpDropAll`, and the second trips `Data::add_def`'s dual-definition assert — an ICE
  before the program runs, on both backends.  Loud, not silent, and it takes a shape no reader
  would expect to be rare: a pair of structs holding optionals.  Measured 2026-09-22 —
  `struct SA { a: A? }` beside `struct SB { b: B? }` over two droppable payloads aborts; two
  nullable LOCALS do not, nor does one nullable field beside a non-nullable one, nor two nullable
  fields where only one payload is droppable.
- **Status:** CLOSED 2026-09-22.  Found while building `D-heap-39`'s matrix, whose cascade cell
  needs a second droppable type; until then that cell did not compile.
- **Closed:** the key spells a variant as `<enum>::<variant>`, so the two are
  `__nullable<A>::Some` and `__nullable<B>::Some`.  It has ONE home, `Data::drop_cascade_key`,
  read by the parser that MINTS the cascade and by every lookup that finds it — the name had two
  homes before (the parser's `drop_cascade_name` and three inline rebuilds in `Data`), and a
  cascade minted under one spelling and looked up under another is a release that silently never
  runs.  Guard `tests/scripts/1622-a-drop-cascade-key-names-the-variants-enum.loft`.

### D-heap-23 — OPENED AND CLOSED (2026-09-21): a whole-collection copy of a collection the function owns released its elements twice

- **Violates:** (H-Move) — a collection the function owns moves wherever it is placed, and its
  elements go with it: `d = v`, `w += v`, `d = a + b`, and a `return v` from a nested block.
- **Where:** the parser writes each as a whole-collection COPY — `OpAppendVector(d, v)`, or
  `OpReplaceVector(buffer, v)` into the caller's buffer where `v`'s backing is not that buffer,
  which is every `return v` below a function's top level — and no site handed the release over,
  so the source's backing ran its cascade over the elements the copy had moved.
- **Effect:** measured on both backends, identical: `d = v` released `80` twice; `w += v`
  `70` twice; `d = a + b` both operands twice; and `if c { v = [mk(20)]; return v }` released
  `20` in the callee BEFORE the caller read it and again in the caller — a use after release in
  a shape as ordinary as building a result inside an `if`.  The same through a `match` arm and
  from a loop body.  A top-level `return v` was clean: there the backing IS the caller's buffer,
  so nothing is copied.  The stores were right throughout (poison, strict stores and native's
  leak check silent); only the hooks doubled.
- **Closed:** the source's backing takes loft#1515's per-path flag, set right after the copy
  (`collection_copy_handoff`, one home, read by the pre-scan registration and the flag write); a
  re-mint of the backing resets it (`in_place_rebuild`), because what the rebuilt store holds is
  its own again.  A collection GROWN after its elements moved gives the release back at the
  growth: `(H-Spent)` refuses that program, and until the error exists it keeps the answer it had
  (`p_v2`).  `p_v1` retired from `D-heap-15`.  Guard
  `tests/scripts/a-moved-collection-releases-its-elements-once.loft`.

### D-heap-25 — OPENED AND CLOSED (2026-09-21, loft#1573): a literal rebuilt into a local released the record it displaced before the new value was computed

- **Violates:** (H-Drop), its reassignment clause — the release runs *"after the new value has
  been computed"*.
- **Where:** a literal into a live record local is REBUILT in place: the re-init
  (`OpDatabase(s, tp)`) is a statement of its own and the field writes are the statements after
  it, and `Scopes::in_place_rebuild` places the release right after the re-init.  A literal
  whose fields read `s` itself had its initialisers lifted above the re-init (#330), which is
  why it was right.
- **Effect:** measured on both backends, identical: `s = Hold { h: mk(20) }; s = Hold { h:
  mk(21) }` made 20, released 20, then made 21.  The rule's order is make 20, make 21, release
  20.  The same for a bare droppable (`s = H { id: tick(21) }` released before `tick` ran), for
  a local renamed onto the return buffer, inside a loop, for an enum variant, a nested literal
  and a rebuild in an `if` arm.  A rebind from a call, a literal that reads `s`, and a nullable
  local were in the rule's order.
- **Closed:** where the rebuilt type owns a droppable (`Data::owns_droppable` — its cascade is
  synthesized only after the parse) and an initialiser calls a user function
  (`ir_has_user_call`), the literal is built apart and bound (`D-rw-5`'s road), so the release
  follows the `Set`.  Lifting just that initialiser above the re-init, as #330 lifts a
  self-reading one, was built first and measured wrong: a `??` join with a call, copied out of
  its temp, released twice (the drop gate's `q_default_field_*_field`).  On a first bind the
  road costs nothing, since the binding adopts the work-ref's store.  Guard
  `tests/scripts/1573-a-rebuilt-local-releases-what-it-displaces-after-the-new-value.loft`.

### D-heap-24 — OPENED AND CLOSED (2026-09-21, loft#1564): a local lost a record it held when its binding ran more than once, or when it was renamed onto the return buffer

- **Violates:** (H-Drop), its reassignment and scope-end clauses.
- **Where:** three facts the scope pass reads to release a displaced record were missing, and
  one rename should not have happened.
  - A local renamed onto the return buffer is first BUILT, not bound, when its first value is a
    literal: the guarded in-place write (`parse_object`, @PLN157 § V-d).  `in_place_rebuild`
    never recorded that the local then owns what it built, so the next reassignment released
    nothing.
  - A local first bound in a loop body and read after the loop has its scope moved out to the
    function, with a null before the loop (loft#1156).  That null recorded no ownership, so the
    binding in the loop released nothing on the passes after the first.
  - A local owned on entry to a loop and rebound in it lost its ownership record at the loop's
    end, because the merge kept only entries the body left UNCHANGED.  A rebind after the loop
    then released nothing for the last pass's record.
  - A local DECLARED in a loop body and returned was renamed onto the return buffer.  As a
    parameter by slot, it was never released at a pass end, and every pass refilled the same
    buffer.
- **Effect:** measured on both backends, identical: `s = Hold { h: mk(20) }; s = Hold { h:
  mk(21) }; return s` never released 20, with or without an early return between the two, and
  for a record owning a vector or a nullable member too.  `for i in 0..3 { s = mk(20 + i); if i
  == k { return s; } } return mk(99)` lost every pass it did not return on, all three when it
  never returned.  `for … { s = mk(20 + i) } use(s)` lost 20 and 21 in a function returning
  nothing: no rename is needed for that shape.  The entry as first written named only the
  returned-local shape.
- **Closed:** the in-place rebuild records the local as owning what it built, after its snapshot,
  so the caller's record at entry stays untouched.  The loop pre-init records the null as owned;
  the release is guarded on the record being live, so the first pass releases nothing.  The loop
  merge keeps an entry that is owned both on entry and at the body's end, at its entry depth; a
  local whose body mixes owning and viewing releases through its owner witness instead
  (loft#1336).  The promotion ladder declines the rename for a local the program declared in a
  loop (`Function::created_in_loop`), so it is an ordinary loop-body local and its `return` copies
  into the buffer.  The copy advice then claimed that copy was avoidable because `s` was "still
  used after this point".  The uses it counted were the pass-end release and a guarded free,
  which no path from the return reaches.  The collector now skips every free
  (`OpSets::frees`) and every drop hook (`OpSets::drop_functions`) as scope machinery.  Before,
  it skipped only `OpFreeRef`, and the notice was false on the tree before this entry too, for
  a second loop local.  Guard
  `tests/scripts/1564-a-local-releases-every-record-it-held-across-passes-and-returns.loft`.

### D-heap-21 — OPENED AND CLOSED (2026-09-21): a collection local was released after every other local at its scope's end

- **Violates:** (H-Drop), its scope-end clause: *"the owner's scope end, in reverse declaration
  order"*.  The order is part of the contract because a later structure may hold a lease on an
  earlier one's resource: cursors in a vector must close before the connection declared ahead of
  them.
- **Where:** `get_free_vars` sweeps a scope in reverse `var_order`, which is the order variables
  are REGISTERED.  A collection local is a view of the store holding its elements — its
  `__vdb_N` backing, or the `__ref_N` buffer a call delivered it through — and that store is
  registered by its null-init at the head of the function, so it was swept last whatever the
  declaration order.
- **Effect:** measured on both backends, identical: `b = mk(5); v: vector<H> = [mk(4)]` released
  `5` before `4`; two vectors released in declaration order; three locals `b, v, w` released
  `5 4 6` for the rule's `6 4 5`; a collection a call answers (`d = mkv(4)`) the same.  A struct
  holding a vector, and a vector declared first, were right — the controls.
- **Closed:** when a local is first registered, a `__vdb_N` it views — or, for a collection, a
  `__ref_N` — moves to its place in `var_order`, which is where the store is minted.  Its scope
  and its ownership are untouched; only its turn in the sweep moves.  A record local releases
  through itself and its buffer's free is identity-guarded, so a record buffer keeps its place.
  Guard `tests/scripts/a-collection-local-releases-in-declaration-order.loft`.

### D-heap-26 — OPENED 2026-09-21, CLOSED 2026-09-22 (loft#1582): a vector local reassigned released the elements it displaces at scope end, or before the new ones were built

- **Violates:** (H-Drop), its reassignment clause — the displaced record is released *"after the
  new value has been computed and before anything after the statement runs"*.
- **Where:** outside a loop, a rebind of a vector local mints a new `__vdb_N` backing, and the
  displaced one is released only by its own scope-end sweep.  Inside a loop, on a vector declared
  before it or read after it, the same backing is re-minted each pass, and
  `Scopes::in_place_rebuild` releases its snapshot right after the re-init — the vector twin of
  `D-heap-25`.
- **Effect:** measured on both backends, identical: `v = [mk(20)]; v = [mk(21)]` makes 20 and 21,
  reads, and releases 21 and then 20 at the scope's end.  The rule's order is make 21, release 20,
  read.  The same for a call (`v = mkv(21)`), a copy (`v = w`) and `v = []` followed by an append.
  In a loop the displaced elements go before the new ones are made.  The count is right.
- **Status:** CLOSED 2026-09-22 — found 2026-09-21 in `D-heap-22`'s guard, whose hoisted-vector
  control it is.
- **Closed:** the scope pass releases the displaced backing at the statement's END.  At a vector
  rebind, `Scopes::vector_rebind_release` releases every literal backing the local is bound to
  anywhere except the new one (`vector_literal_backings`).  Each is released through its hook,
  freed, and set to the sentinel; the one it held is live and the others are already the
  sentinel.  A literal's `Set` heads the statements that fill its backing, so that release,
  and a re-minted backing's snapshot (`in_place_rebuild`), wait for the statement's end: a
  `Line` marker, or the block's end in front of its value.  The parser puts that marker after a
  statement it spliced flat (`parse_block`).  The spliced statements alone could not say where
  the statement ended: `v = [mk(2)]` and `v = []; v += [mk(2)]` on one line lowered to the same
  IR, and the rule orders them differently.  Measured on both backends: a literal, a call, a
  copy, `[]` then an append, a loop, an `if` arm, two displaced elements, a block's value, and
  a droppable member.  An element view held across the rebind is refused, as before
  (`(H-View-Drop)`), so the earlier free cannot be observed through one.  Guard
  `tests/scripts/1582-a-vector-rebind-releases-what-it-displaces-after-the-new-value.loft`.
  Three neighbours were found with the matrix and are their own issues: a vector that held a
  CALL's result loses its release when rebound (loft#1596), a vector of vectors never releases
  its inner elements (loft#1597), and a vector moved out and then refilled releases the moved
  elements twice (loft#1598).

### D-heap-38 — OPEN (2026-09-22, loft#1607): under the arm-scope rule, a vector local bound in both arms of an `if` is released at the end of the scope around it

- **Violates:** (H-Drop), its scope-end clause — an arm's owner dies at the arm's end.
- **Where:** `D-heap-36`'s opt-in rule (`LOFT_ARM_SCOPE=1`) makes a local bound in BOTH arms
  each arm's own for a record or a text, and not for a vector or a tuple.  Each arm's bind lands in a backing of its own
  (`__vdb_2`, `__vdb_3`), and the variable's type names one of them, the LAST bind's
  (`@FR-O-Latest`).  The arm-end release reads that one dep (`scopes::outer_collection_backing`),
  so the other arm would release the wrong backing.  Measured when the case was allowed: the
  true arm released the false arm's empty `__vdb_3`, and `D50` never ran.  So the case keeps the
  pre-init in front of the `if`.
- **Effect:** measured on both backends, identical: `if c { w = v; … } else { w = v; … }` released
  `v`'s element after the `if`'s successor ran.  The count is right.
- **Status:** OPEN.  The cure is a per-path fact, the backing the local's latest bind on THIS
  path names, snapshotted per arm as `construction_backing` is for records; or both arms' binds
  sharing one backing.

### D-heap-37 — OPENED AND CLOSED (2026-09-22, found with loft#1600): a local hoisted out of an `if`, a loop or a block was released after the locals declared before it

- **Violates:** (H-Drop), its scope-end clause — reverse declaration order.
- **Where:** a local bound inside an `if` arm, a loop body or a block and READ after it is
  registered at the enclosing scope by a hoist (`Scopes::scan_if`'s pre-init, and the loop and
  block hoist `locals_read_after` feeds).  The three registered it with a bare `var_order` push.
  `D-heap-21`'s move of a collection local's backing to the local's turn was made only by the
  plain first bind, so the backing kept its turn at the function's head.
- **Effect:** measured on both backends, identical: `v: vector<H> = [mk(50)]; if c { w = [mk(52)] }
  len(w)` released `50 52` for the rule's `52 50`; the same bound in a loop body, in a block, and
  a vector a call returned.
- **Status:** CLOSED 2026-09-22.
- **Closed:** one home for a local's registration, `Scopes::register_binding`, which the first
  bind and the three hoists call.  Guard
  `tests/scripts/1600-a-local-bound-inside-an-if-arm-is-released-at-the-arms-end.loft` `a9`–`a11`.

### D-heap-36 — OPEN (2026-09-22, loft#1600): which scope does a local every mention of which lies in one `if` arm die at?

- **The question.** (H-Drop) releases at *"the owner's scope end (@PLAN125 arc B)"*, and arc B
  HOISTS a local written inside an `if` block to the function's scope: `pln125-b-drop.loft`'s
  `if_block_local` pins its drop at the function's end, *"the drop is wherever the free is"*.
  A block local stays readable after its block (`{ n = 5 } n` is legal), so the function IS its
  owner's scope by that reading.  loft#1600 asks for the arm's end instead, and `D-heap-27`'s
  paraphrase of the clause ("for a loop body's or a block's owner, THAT scope's end") reads the
  same way.  The implementation answers both: a vector LITERAL bound in an arm is released at
  the arm's end (its backing is not registered when `Scopes::scan_if` asks), every other heap
  local at the end of the scope around the `if`.
- **Status:** OPEN, a design call for the owner — may such a local be released at the arm's end,
  for memory only, or for drops too?  The arm-scope rule is built and OPT-IN:
  `LOFT_ARM_SCOPE=1` makes a local the program declared, every mention of which lies inside the
  `if`'s arms, the arm's (`Scopes::confined_to_one_arm`, counted by `var_mentions_in`).  Any
  mention elsewhere keeps the pre-init, and so do these: a local the arm hands out as its value
  or through a `return`; a value-branch bind the scan wrote out into the arms (`sunk`); and a
  compiler temp.  Bound in both arms, a record or a text is each arm's own; a vector is
  `D-heap-38`.  Default-on, it regressed `1495-a-diverging-arm-beside-a-value-arm-still-yields`
  (a value `if` whose return the parser rewrote into a buffer copy: value-wrong and an
  interpreter panic) and `a-copy-of-a-local-that-may-not-own-its-record-takes-the-per-path-answer`
  (release order), and it contradicts `pln125-b-drop`.  The arm cells of
  `tests/scripts/1600-a-local-bound-inside-an-if-arm-is-released-at-the-arms-end.loft` assert
  it when the switch is set.

### D-heap-35 — OPENED AND CLOSED (2026-09-22, found with loft#1597): a struct-enum vector released a unit variant's literal twice

- **Violates:** (H-Move) — a fresh value placed into an element moves there, and the element's
  container releases it once.
- **Where:** a unit variant (`B`) is built in a work-ref and copied into the element; a variant
  with a payload is written in place.  The copy hands the release over when the destination is
  an appended element whose type owns a droppable (`scopes::appends_to_element`), and the
  element variable of a struct-enum vector was typed off a PLACEHOLDER def whenever the
  container's content did not resolve (`Parser::unique_elm_var`), so the test said no.
- **Effect:** measured on both backends, identical: `v: vector<E> = [A { id: 1 }, B, A { id: 2 }]`
  ran B's hook twice, from the vector's cascade and from the work-ref; appended (`v += [B]`)
  and in a struct field's literal the same.
- **Status:** CLOSED 2026-09-22.
- **Closed:** a struct-enum element variable is typed as a record of its enum.  A literal of one
  of its variants is still built INTO that element, as it was against the placeholder: a
  variant literal matches an inline element slot typed as its enum (`Parser`'s object-literal
  `type_matches`).  Otherwise every `v += [A { … }]` goes through a work-ref and a copy and
  loses its push header — measured on the `enum_record`, `mint_window` and `copy_in_place`
  emission pins.  Guard
  `tests/scripts/1597-a-vector-of-vectors-releases-its-inner-elements.loft` `c10`.

### D-heap-34 — OPENED AND CLOSED (2026-09-22, loft#1597): a vector of vectors never released its inner elements

- **Violates:** (H-Drop) — a container's death releases what it holds.
- **Where:** a vector's synthesized cascade walks its elements when the element type owns a
  droppable (`Parser::cascade_element`), and it named only RECORD elements.  A vector element
  had no cascade to call, so `vector<vector<H>>`, a struct field of that type and every deeper
  nesting walked nothing.
- **Effect:** measured on both backends, identical, with no diagnostic:
  `v: vector<vector<H>> = [[mk(50)], [mk(52)]]` traced `M50 M52 R2` — neither released.  A
  rebind, a return and a loop the same.
- **Status:** CLOSED 2026-09-22.
- **Closed:** a vector element is released through its own collection's cascade
  (`collection_def_nr`), read as `v[i]` reads it; a struct's vector field of vectors takes the
  same walk.  A local vector appended as an element (`v += [inner]`) is a collection copy, and
  hands the release of the store its elements live in to the element on the paths the copy
  ran (`scopes::collection_copy_ends`, beside `D-heap-23`'s whole-collection copies).  A removed
  inner vector keeps its hooks unrun (`(H-Drop-Not)`).  Guard
  `tests/scripts/1597-a-vector-of-vectors-releases-its-inner-elements.loft`.

### D-heap-32 — OPENED AND CLOSED (2026-09-22, loft#1598): a vector moved out and then refilled released the moved elements twice

- **Violates:** (H-Move) — a collection the function owns moves wherever it is placed, and each
  element is released once.
- **Where:** a whole-collection copy hands the elements' release over by flagging the backing
  store they live in (`collection_copy_handoff`, `D-heap-23`).  It named the backing the
  source's TYPE names, which is the one of its LAST binding.  At the copy, a source rebound
  later still held an earlier one.
- **Effect:** measured on both backends, identical: `v = [mk(20)]; w = v; v = [mk(21)]` released
  20 twice, from `v`'s first backing and from `w`.  A move with no refill was clean.
- **Status:** CLOSED 2026-09-22 — found the same day with `D-heap-26`'s matrix.
- **Closed:** the copy flags every literal backing its source is bound to
  (`collection_copy_backings`, over `vector_literal_backings`).  Only the one it holds is live:
  a rebind releases the others and sets them to the sentinel (`D-heap-26`), and a refill's re-mint
  gives its own backing its release back.  Guard
  `tests/scripts/1598-a-vector-moved-out-and-refilled-releases-the-moved-elements-once.loft`.
  Found beside it: a vector moved into a local inside an `if` arm is released at the function's
  end rather than the arm's (loft#1600).

### D-heap-31 — OPENED AND CLOSED (2026-09-22, loft#1596): a vector local rebound away from a call's result never released it

- **Violates:** (H-Drop), its reassignment clause.
- **Where:** a call delivers a vector into a return buffer of the caller's (`__ref_N`), and the
  local that names the delivered value is what releases its elements (`drop_hook`'s delivered
  binding).  After a rebind that local names the new value, so no release was ever emitted for
  the old one.  In a loop the call is handed the SAME buffer again and cleared the old elements
  in place, with no hook (`(H-Drop-Not)`).  A local promoted onto the return buffer is that
  buffer, so a call, a literal or `[]` refilled it in place the same way.
- **Effect:** measured on both backends, identical: `v = mkv(1); v = mkv(2)` traced
  `M1 M2 R1 D2`, with 1 never released.  The same for a literal, a copy, `[]`, an `if` arm, and
  a function returning the local, and every pass of a loop but the last lost its elements.
- **Status:** CLOSED 2026-09-22 — found the same day with `D-heap-26`'s matrix.
- **Closed:** `Scopes::vector_call_rebind` takes the displaced value aside before a rebind of a
  local that may hold a call's result: an alias, where the local's store is one of the call
  buffers of its type (store identity, since a call that reads its own destination rotates two
  buffers).  A buffer the rebinding call is handed is detached first, so the callee fills a fresh
  one, and is handed that store back after the call.  After the new value, the aside's elements
  are released, every buffer sharing its store is set to the sentinel, and the store is freed.
  For a local promoted onto the return buffer, `Scopes::promoted_vector_refill` copies the old
  elements out before a refill (`OpAppendVector` takes their heap along) and releases the copy
  after.  The first fill displaces nothing: the buffer a caller hands in is emptied at entry and
  may hold what the caller already released.  Guard
  `tests/scripts/1596-a-vector-rebound-from-a-call-result-releases-it.loft`.  Found beside it: a
  function returning such a local rebound in a LOOP re-points it at a buffer of its own, and the
  caller frees only the one it handed in (loft#1599, a leak).

### D-heap-28 — OPENED AND CLOSED (2026-09-22, loft#1591): a variable rebound to a `??` chain that holds itself never released the kept record on `--native`

- **Violates:** (H-Move), and `@FR-O-NoDiverge` — the two backends disagree.
- **Where:** `a = a ?? b ?? mk()` is `(a ?? b) ?? mk()`, so the subject `(a ?? b)` is hoisted into a
  `__ncc_N` temp.  The oracle calls the chain `Own::Join`, which makes the rebind a view, so `a`
  carries an owner witness (`@FR-O-Witness`).  The chain's one copy sits in a different place on
  each backend.  The interpreter aliases the hoist (`VarRef a; PutRef __ncc_1`), and native
  deep-copies it (`OpDatabase` + `OpCopyRecord`, `generation/dispatch.rs`'s record-bind arm).  On
  native the witness's store-identity test sees a new store and releases the witness without its
  hook (the hand-off flag is set).  The copy `a` now holds has no owner.
- **Effect:** measured: `a: H? = mk(17); b: H? = null; a = a ?? b ?? mk(18)` traces `M17 R17 D17`
  on the interpreter and `M17 R17` on `--native`.  `a = x ?? a ?? mk(3)` behaves the same.  Clean
  on both backends: the same chain bound to another variable, every chain without the
  destination, the destination absent, and a single `??` over itself (`D-heap-15`, `p_i2`).
- **Status:** CLOSED 2026-09-22 — found the same day while closing `p_i2`.
- **Closed:** the chain is rewritten before any analysis reads the function
  (`scopes::reassociate_coalesce_chains`), so neither the hoist nor the witness is involved.  In
  `if present(p) { p } else { q }` only the `q` arm can be absent, so `(p ?? q) ?? d` equals
  `if present(p) { p } else { q ?? d }`, with the same value and the same evaluation order.
  With the destination as the HEAD the rebind becomes `if present(a) { } else { a = rest }`, where
  `rest` is the chain without its head and no longer names `a`.  With the destination further
  along, and every operand before the last a variable, the chain is right-associated all the
  way down, so each arm is a variable or the last default and the per-arm write-out takes it,
  its `a` arm the identity (`D-heap-15`, `p_i2`).  A chain that names no destination keeps its
  form.  Measured on both backends: the destination first, second, third and last, a call in
  the middle, the head absent, and in a loop.  Guard
  `tests/scripts/1591-a-coalesce-chain-that-holds-its-destination-releases-once.loft`.

### D-heap-29 — OPENED AND CLOSED (2026-09-22, loft#1592): in a loop, a join over a source declared outside it, moved on, released twice

- **Violates:** (H-Move), and `@FR-O-Complete`.
- **Where:** in a loop, `Scopes::arm_source_outlives_loop` declines to write a first-bind join out
  per arm and lifts it instead (`lift_join_arm_tails`).  The binding then borrows the per-arm
  temps, and the minting call's temp owns its store.  The decline stands in for the `(H-Spent)`
  refusal of a name moved on every pass, and the stand-in does not hold when the body REFILLS
  the source before the next pass.  `x = a ?? mk(); a = x` copies the borrow into `a`, so both
  `a` and the temp release the record, and on native both free its store.
- **Effect:** measured: `a: H? = null; for i in 0..1 { x = a ?? mk(11 + i); a = x }` traces
  `M11 R11 D11 D11` on both backends.  Two passes trace `M11 R11 D11 R11 D11 D11` on the
  interpreter and panic on `--native` (`allocation.rs:1622`, store index 65535).  Clean on both
  backends: the unrolled form, `a` declared in the loop, the join without `a = x`, and a plain
  `x = mk(); a = x` in the same loop.
- **Status:** CLOSED 2026-09-22 — found the same day while closing `p_i2`.
- **Closed:** the scan records, for each loop it enters, the variables the body assigns on
  EVERY pass (`loop_body_refills`): a `Set` among the loop's own statements or those of its body
  block, in a body with no `continue` that could skip it.  `arm_source_outlives_loop` no longer
  counts such a source, so the join is written out per arm, and the move into `x` and back into
  `a` is the one-pass move the per-path flags already decide.  Measured clean on both backends:
  one and two passes, the refill before the join, the refill from another value, a nested loop
  refilled inside, a `break` after the refill, `while`, and the `if` spelling.  A source spent on
  every pass (a program `(H-Spent)` refuses) keeps the lift.  Guard
  `tests/scripts/1592-a-join-in-a-loop-over-a-refilled-source-releases-once.loft`.

### D-heap-30 — OPENED AND CLOSED (2026-09-22, loft#1594): a tuple member's hook ran twice — a returned tuple local, and a destructuring `for`

- **Violates:** (H-Drop) — one hook per structure holding a lease — and (H-Move)'s return
  clause.
- **Where:** two shapes, one member release (`scopes::tuple_owned_elem_frees`).
  - A returned tuple local.  `a = (1, mk(1)); a` copies each member into the return record
    (`synthetic_tuple_return`), and `a`'s scope end released the member again.  The copy read
    the member through a stash (`emit_tuple_set_ops`), so the hand-off the scan records stopped
    the stash and not `a`.  A nullable member was copied through a stash of its own, under its
    own null test.  A member with no claimant to disarm (a bufferless call, a generator's
    advance) had no buffer to mark at all.
  - A destructuring `for (i, hh) in v` over a `vector<(integer, H)>`.  Each binder was created
    as an OWNER and released its member as its iteration ended, and `v` released it again.
- **Effect:** measured identically on both backends, before loft#1589 as after: a returned
  local traced `m1 d1 B b1:1 … d1`, and a destructuring loop `d5 d6` per pass and again at `v`'s
  end.
- **Status:** CLOSED 2026-09-22 — found while closing loft#1589, whose generator tuple `for`
  variable reached the same release.
- **Closed:** a tuple VARIABLE is read in place by `emit_tuple_set_ops`, so each member copy
  names it.  The scan honours a returned tuple's copies as it does a whole-tuple bind's
  (`synthetic_tuple_return` beside `tuple_member_move`).  A copy into a place whose container
  releases it (`copy_hands_off`) hands a member off like one into a buffer.  A stash names the
  member it holds, and a nullable member's own null test is entered (`walk_member_writes`).  A
  member with no claimant drops its pairing, keeping its free and losing its hook.  A binder
  read off a loop variable's member is a view of that variable (`(B-View)`).  Measured clean on
  both backends: one and two heap members, a nullable member present and absent, a bind then a
  return, the destructuring loop, and the controls, a returned literal and a plain `for`.  Guard
  `tests/scripts/1594-a-tuple-member-is-released-once.loft`.

### D-heap-27 — OPENED AND CLOSED (2026-09-22, loft#1588): a tuple's member backings were released at the function's head turn, not the tuple's

- **Violates:** (H-Drop), its scope-end clause — *"the owner's scope end, in reverse declaration
  order"*, and for a loop body's or a block's owner, THAT scope's end.
- **Where:** a tuple's vector member lives in a `__vdb_N` backing, and a record member a
  whole-tuple bind (`u = t`) copies lives in a `__ref_p2_N` one (loft#1361).  Both are registered
  by their null-init at the function's head, so `get_free_vars` swept them after every other
  local, and a tuple declared in a block or a loop body left them to the function's end.  The
  `D-heap-21` and `D-heap-22` cures read a VECTOR local's one dep, and a tuple has no dep list of
  its own.
- **Effect:** measured on both backends, identical: `b = mk(11); t = (mk(12), 1); u = t` released
  `11 12` for the rule's `12 11`; `t = (mk(21), 1); u = t; t = (mk(22), 2)` released `22 21` for
  `21 22`; in a loop body the last pass's member was released after the loop, and in a block after
  the block's successor ran — for a vector member even with no move (`if … { t = ([mk(1)], 1) }`).
  The counts were right.  Along the way a SECOND defect showed, hidden by the first: a whole-tuple
  bind of a vector member copied the elements into a backing minted as `main_vector<vector<T>>`
  (`vector_db` takes the ELEMENT type and was handed the vector's), which carried no hook, so the
  source's backing released them at the SOURCE's turn and the copy never did — a double release
  wearing a single one, `(H-Move)` unmet.
- **Closed:** each of a tuple's member backings takes the tuple's turn in the sweep, read after the
  literal is scanned so a refill's backing lands at the tuple and not at the refill; a tuple
  leaving an inner scope releases a member backing registered outside it there — a vector's
  emptied, a record's freed and reset to the null sentinel — unless a variable of a scope that
  stays open views it; the tuple member copy's backing is typed by its element, and a whole-tuple
  bind (`tuple_member_move`) moves a vector member's release to the copy, paired through
  `Scopes::tuple_member_now` because the tuple's type carries only its LATEST assignment's deps.
  Order within one tuple is reverse member order, as a struct's fields are: seven cells of
  `a-tuple-member-releases-once-beside-any-neighbour.loft` had pinned whatever the head
  registration gave.  A refused copy of a vector member (`t2 = (tt.0, 2)`) now releases twice with
  the rest of `D-heap-8`'s `c_tuple_*` family, where the hook-less backing gave one.
  Guard `tests/scripts/1588-a-tuple-releases-at-its-own-turn.loft`.

### D-heap-22 — OPENED AND CLOSED (2026-09-21, loft#1565): a vector declared in a loop body released its elements during the NEXT pass, not at the end of its own

- **Violates:** (H-Drop), its scope-end clause — the owner of a loop-body local dies at the end
  of each pass, in reverse declaration order.
- **Where:** the vector's backing — its `__vdb_N`, or the `__ref_N` buffer a call delivered it
  through — is registered at FUNCTION scope so the store is reused across passes, and nothing
  released the elements at the pass end.  A literal's were released by the next pass's re-mint
  (its displaced snapshot), the last pass's at the function's end.  A call-delivered vector's were
  never released: D-heap-13's release runs over the BINDING, which is out of scope by the time the
  backing is swept.
- **Effect:** measured on both backends, identical: `for i in 0..2 { b = mk(10 + i); v: vector<H>
  = [mk(20 + i)]; }` released `10 20 11 21` for the rule's `20 10 21 11`.  A `break` released the
  vector after the loop's successor ran.  `v = mkv(20 + i)` in a loop never released 20 or 21.
  Where the vector was the pass's first statement the order came out right by accident.
- **Closed:** `get_free_vars` releases a vector local whose one dep is a backing registered in an
  outer scope (`outer_collection_backing`) at the local's own scope exit: normal end, `break` and
  `continue` alike, in its declaration turn.  Only a local the program declared, and never an
  argument backing — measured wrong both ways: a compiler temp with that dep is an INNER vector
  of a nested literal, owned by the outer one, and clearing it emptied `t[0..2][0]`; a literal
  backing renamed onto the return buffer is the caller's, and releasing it emptied the vector
  being returned.  And only where the release runs a hook: for any other element type nothing
  observable is late.
  The release is the backing's `scope_end_drop`, so a hand-off that stopped it still stops it.
  The vector is then emptied (`OpClearVector` runs no hooks, `H-Drop-Not`), so the re-mint and the
  backing's own release find nothing to release a second time; the store is still reused.  Guard
  `tests/scripts/1565-a-loop-body-vector-releases-its-elements-at-the-end-of-its-own-pass.loft`.

### D-heap-18 — OPENED AND CLOSED (2026-09-21): a local bound from a join, placed again, released twice

- **Violates:** (H-Move) — every arm of `x = a ?? b` (or `x = if c { a } else { b }`) is a value
  the function owns, so `x` owns what the join hands it, and placing `x` again moves it on.
  One release, by whatever holds it last.
- **Where:** the arm lift (`Scopes::lift_join_arm_tails`).  It gives each arm a `__lift_N` temp,
  keeps each arm's release with the arm's SOURCE and makes `x` a BORROW of the temps.  That is
  right for `x` read in place, and wrong the moment `x` is placed: `y = x`, `S { h: x }`,
  `v += [x]` and `return x` each stopped `x`, which released nothing, while the source still
  released.  The author's own statement form, `if c { x = a } else { x = b }`, was clean
  throughout: there `x` owns, and loft#1515's per-path flags stop the sources.
- **Effect:** measured 2026-09-21 on the 144-cell `bound` family of `ownership_drop_gate`
  (`b_*`: the `??`, the value `if` and the statement form; first bind and reassignment; a
  call, a variable and a literal default; both paths; four placements): **63 cells released
  twice** on the tree before, some also before a read, on both backends and with no diagnostic.
  None of the gate's earlier families placed a join-bound local a second time.
- **Closed:** a FIRST bind of a type that owns a droppable is written out to the statement
  form, as a reassignment already was (`sink_set_into_arms`, then a rescan so the analyses
  before the scan read that form).  A literal default arm is written out whole, and the
  binding adopts its construction as a single bind of the literal does.  Not where an arm's
  source outlives the loop the bind runs in: that places one name on every iteration, which
  `(H-Spent)` refuses; until that error is built, the lift's keep-with-the-source is the answer
  that releases once (`p_l1`, which the rewrite otherwise turned into a use after release).
  After: 144 of 144 clean on both backends.

### D-heap-19 — OPENED AND CLOSED (2026-09-21): a copy into a local promoted onto the return buffer moved nothing

- **Violates:** (H-Move), (H-Drop).
- **Where:** `copy_moves_drop_from` refuses a hand-off into an ARGUMENT, and exempts the return
  buffer only when it is named `__ref…`.  A local promoted onto the buffer
  (`x = …; return x` becomes `fn f(…, x: H)`) is that buffer under the local's name.
- **Effect:** `a = mk(5); x = mk(1); x = a; return x` released `5` twice, the first time before
  the caller read it, on both backends; the same under a branch (`if c { x = a }`).
- **Closed:** `copy_moves_drop_from` asks `promoted_ret_buffer`, the one home `displaced_drop`
  already used for the same question (D-heap-7's `a = mk(1); a = mk(5); return a`), so every
  reader of a copy agrees that the caller adopts what the promoted local holds.

### D-heap-20 — OPENED AND CLOSED (2026-09-21): a literal rebuilt into a local promoted onto the return buffer lost what it displaced

- **Violates:** (H-Drop), its reassignment clause.
- **Where:** a literal into a promoted return buffer is guarded — a record the buffer already
  holds is written in place, an absent one is minted (`parse_object`, @PLN157 § V-d) — and
  `Scopes::in_place_rebuild` recognised only the unguarded `OpDatabase`.  The guard's premise
  holds at entry, where a record in the buffer is the caller's; after `x = mk(1)` it is this
  frame's.
- **Effect:** `x = mk(1); x = H { id: 7 }; return x` never released `1`, on both backends; a
  caller rebinding from such a callee (`r = f(1); r = f(2)`) lost both.
- **Closed:** `in_place_rebuild` looks through the guard.  `displaced_drop` then releases only
  a record this frame bound, so the caller's offered record at entry stays untouched.

### D-heap-17 — OPENED AND CLOSED (2026-09-17): a self-append read its source through the record number captured before the growth moved it

- **Violates:** (H-Move) — the elements of a vector appended to itself are the vector's own
  values; `Stores::vector_add` answered them through a freed block.
- **Where:** `Stores::vector_add` captured the source record's number BEFORE growing the
  destination, and for a self-append (`v += v`) the source IS the record the growth
  relocates.  The byte copy took a pre-growth snapshot (a `Vec<u8>` filled and written back a
  byte at a time — 8.6 % of the drawing lane's `render_marks` row), but the CLAIMS walk that
  follows it for heap-owning elements read the source through the stale number: for a `text`
  vector of 200 the run stopped with `Store access out of bounds … the reference is corrupt`
  on both backends, and under `LOFT_POISON=1` the copied texts mismatched.  A no-heap
  self-append was right by luck — the snapshot had already left the freed block.
- **Closed:** the source record is re-read from its field slot after the growth whenever it
  shares the destination's store, and the self-append is the same-store block copy every
  other append already took (source range at the record's front, destination at its new tail,
  no overlap).  `LOFT_NO_SELF_APPEND_BLOCK=1` keeps the snapshot form of the copy — with the
  re-read, since the switch is a bisect step for the copy, not a way back to the fault.
  Guard `tests/scripts/a-self-append-is-one-block-copy.loft` (twelve cells: scalars, a
  relocating growth, text, records, nested vectors, a field, the doubling fill), falsified
  against 48d49e24 (both backends panicked → clean).

### D-heap-13 — CLOSED (2026-09-20): a collection returned from a call and bound to a local never releases its elements (loft#1551)

- **Violates:** (H-Drop), reached through (H-Move) and (H-Lease) — `d = mkv()` is a legal move of
  a fresh call result, so `d` is the owner, the owner has its own lease, and the hook runs at the
  owner's scope end.
- **Where:** the local's backing is the callee's return buffer, a bare `vec<ref(T)>`, where a
  local vector's is the wrapper record that carries the generated `OpDropAll` cascade; scope exit
  emits `OpFreeRef` with no cascade call.  WHICH site decides to omit it is not established —
  the candidates all read `Data::drop_cascade_nr` — and establishing it is the first step of a
  fix, not an assumption to build on.
- **Effect:** the resource stays open for the life of the process, both backends, byte-identical,
  with no diagnostic.  `d = mkv(); d += […]` and `take(d)` leak too, and so does a
  `vector<vector<H>>`; the same call into a FIELD (`b.v = mkv()`), a struct from a call, a struct
  CONTAINING a vector and a plain local vector are all clean, which is what bounds it to a bare
  collection from a call into a local.
- **Why nothing caught it:** the MEMORY is freed correctly and only the hook is skipped, so
  `LOFT_STRICT_STORES` and `LOFT_POISON` are structurally blind; `ownership_drop_gate`'s CROSS
  family has no collection source and no collection destination, so no generated cell writes the
  shape.
- **Status:** OPEN — the drop-cascade family (@PLN163).  `d = mkv(); e = d` releases exactly once
  today because `D-heap-8`'s second structure supplies the release this entry loses, so that
  shape is not a control and curing either entry alone changes its answer.
- **Removal:** give the bound local the cascade its wrapper-backed twin has.  `(H-Drop)`'s own
  warning applies — a drop has no safe direction, and turning a lost hook into a doubled one must
  not land as an improvement.

### D-heap-12 — OPENED AND CLOSED (2026-09-17): a refilled record buffer stranded its previous occupant's heap (loft#1549)

- **Violates:** (H-ClearRelease) — its record clause, which this entry added: the rule named the
  vector a buffer ABI reuses and said nothing of the record (R-Reuse) reuses.
- **Where:** `scopes::reuse_record_buffers` handed the same record to every call of a site,
  and the callee's literal — behind the "caller offered a record" guard
  `Parser::build_into_return_buffer` puts around its `OpDatabase` — overwrote the record's
  vector, text and nested-record handles with nothing releasing what they named.
- **Effect:** a loop binding a call whose record owns heap (`s = mk(i)`, `mk` returning
  `S { n: i, v: [i, i + 1, i + 2] }`) kept every previous turn's heap claimed in the buffer's
  store: about 100 bytes a turn for that shape, 600 for a vector of heap-owning records, on both
  backends, unbounded in the loop length.  Values were right, and the store itself is freed on
  time, so no store-count gate saw it.
- **Closed by:** the pool's lazy guard releases on reuse —
  `if OpRefIsNull(b) { OpDatabase(b, T) } else OpClear(b, T)`, `OpClear` being `remove_claims`
  (the eager form releases in front of each use) — emitted only when a field is not a scalar
  (`scopes::releases_what_it_held`), with a struct-enum released through its parent type.
  The first cut released in the CALLEE's offered arm instead: equally correct, and measured
  +1 % instructions on the drawing parse row, because B2 offers a freshly placed `Paint` on
  every `read_paint` and the walk there finds nothing.  Only the pool ever re-offers a record,
  so the release is the pool's.  Guard
  `tests/scripts/1549-a-pooled-buffer-releases-its-previous-occupant.loft` (resident memory
  across 900 000 refills, and twelve value cells); plan cells
  `plans/164-activation-arena/bytecode-comparisons/1549-pooled-buffer-release-cells.loft`.

### D-heap-8 — OPEN (2026-09-15, loft#1568): nothing refuses a copy — a written copy of a droppable without `OpCopy` compiles

- **Violates:** (H-Copy-Refuse).
- ⚠ **NARROWED 2026-09-17, when the owner ruled on the copy rules.**  `(H-Move)` now moves a value
  the function OWNS, so most of what this entry covered is not a copy at all.  Measured over the
  227 refused lines in the corpus: **156 are a local the function owns, and every one already
  releases exactly once on both backends** — they were never unsound, and the rules now say so.
  What stays open here is the 71 that are NOT the function's — a parameter (45) and a member of a
  container (26) — each of which really does release twice (`L1 D2 A2 D2` and `L1 D3 D3`,
  measured, both backends).  A SECOND half is open with no implementation at all: `(H-Spent)`'s
  error for reading a name after its value moved.  Today that is silent — `c = open(1);
  if c { v += [c]; } … c.id` reads a value whose hook has already run, and no store instrument can
  see it because the memory is intact and only the resource is gone.  The drop gate's `p_v2`
  (`d = v; v += […]`) is that error's cell, `CENSUS_BLIND` until it is built (reclassified from
  `D-heap-15` 2026-09-23).  ⚠ The refusal built behind
  `LOFT_LEASE_REFUSE` still implements the PRE-ruling population, so it is now WIDER than the
  rules: it refuses the 156 owned-local placements the rules permit.  Narrowing it to the
  not-yours cases, and adding the spent-name error, is what closes this entry.
- **Where:** no site refuses a copy.  The copy sites are the ones `LOFT_DROP_COPY_CENSUS` lists,
  and that enumeration is now the whole population — it was not until 2026-09-17.  The bind arm
  matches `Value::Set(v, Var(src))`, a node the parser never produces for a whole COLLECTION:
  `d = v` mints a fresh `__vdb_N`, reads `d = OpGetField(__vdb_N, 0, …)` out of it, and fills it
  with **`OpAppendVector(d, v)`** — `vectors.rs` calls that fill a *"deep-COPY of a's elements into
  v's own store"* in its own comment.  So the census saw only the backing's `snapshot` rows, judged
  neither (`lease=-`), and an absent row was indistinguishable from a clean one.
  **The spelling is not one shape but four, and three of them are not binds at all**, which is why
  an arm keyed on the bind would still have missed them (measured 2026-09-17, both backends):
  `d = v` and `d = b.v`, the concat `v += w` (`M70 M71 R2 D70 D71 D71`), the self-append `v += v`
  (`M52 R2 D52 D52`), and `d = a + b`, which copies BOTH operands and doubles both
  (`D42 D43 D42 D43`).  The census now judges the append and answers `refuse:copy:<var>` for a
  local source and `refuse:container:<root>` for a member one.
  **The bound is measured, not asserted:** a concat whose right-hand side is FRESH (`v += mkv()`,
  `d = mkv() + mkv()`) is outside the population and answers `lease=move`, because the arm judges
  the source EXPRESSION rather than the op — one parts loop (`parser/vectors.rs`) emits this node
  for a copy and for a fresh literal alike, and `lease::leaves` reads a call as `Leaf::Fresh`.  An
  append into a FIELD (`b.v += w`, `OpAppendVector(OpGetField(rec, fld), src)`) is a different
  site, recorded by `Uses::construct_copy`, and releases once today.
  A refusal built from the census now covers the spelling; `type_owns_droppable_anywhere` answers
  true for `vector<H>` and `(H-Copy-Refuse)` names `x = a` a copy, so the rule always did.
  ⚠ **No program in the tree writes any of these shapes** — the census over all 37 corpus files
  that declare `OpDrop` reports 1299 rows before the arm and 1299 after, none of them an append.
  So the corpus cannot validate this arm, and could never have caught the defect; only the gate's
  `p_v1`/`p_v2`/`p_v3` cells and the probes they were cut from measure it.
  In place of a refusal, `scopes::copy_moves_drop_from`, `scopes::copy_hands_off`,
  `scopes::appends_to_element` and the per-path hand-off flags move the release to ONE of the two
  structures, and `use_analysis::warn_double_move` warns on some of the shapes inside a structure.
- **Effect:** a program that writes a copy of a droppable compiles.  Where the moved release picks
  the structure that lives longer, it releases once and is still outside the rules; where it
  cannot, it releases twice, or while the other structure is in use, with no diagnostic.  The drop
  gate's cells with a `Refused` verdict are this entry's: every baseline line but
  `q_present_local_var_ret`, and every CLEAN cell that writes a copy (`p_k1`, `p_h2`–`p_h7`,
  `c_local_local`, `c_param_local`, …).  The sqldb fixtures' `cs = sq.db_select(sql);
  out = RowsSqlite { rs: cs }` is one, rewritten as `out = RowsSqlite { rs: sq.db_select(sql) }`.
  **Measured 2026-09-17, five cells, both backends byte-identical.**  A whole-collection bind of a
  droppable collection releases one resource TWICE with no diagnostic, and needs no disturbance to
  do it: `v: vector<H> = []; v += [mk(1)]; d = v; len(d)` prints `M1 L1 D1 D1`.  A growth after the
  bind (`d = v; v += [mk(2)]`) and the FIELD spelling (`d = b.v`) both read `M1 M2 L1 D1 D2 D1`, so
  the growth is incidental rather than the cause.  Every neighbour is correct and releases once:
  the record local bind `d = s` (which the census judges, `bind … lease=refuse:copy:s`), the record
  member view `d = b.s`, and the vector PARAMETER bind `u = p` — `calls.md` F-ParamHeap binds
  without copying, pinned by `a-copy-off-a-tuple-parameter-member-leaves-the-caller-owning.loft`'s
  `c_vector_param`.  So the axis is collection-versus-record, not where the container is stored.
  No file in the corpus writes the local spelling — the only whole-value bind of a droppable
  collection in the 37 hook-declaring files is that parameter control — which is why nothing had
  reported it.  The release itself runs through `OpFreeRef` and the type's generated `OpDropAll`
  cascade — at scope end BOTH backings take `OpDropAll` + `OpFreeRef`, so two structures run two
  cascades over one resource.  ⚠ `OpDropAll` is a generated per-type METHOD rather than an
  operator, so the operator census cannot see it: `OpFreeRef` being among the most emitted ops in
  the tree says nothing about how well that cascade is covered, and no instrument here measures it.
- **Status:** OPEN — @PLN163 P2 (the refusal as a report, reworked to this rule) is done, and P3's
  refusal is BUILT behind `LOFT_LEASE_REFUSE` (opt-in, 2026-09-17).  With the switch on, every
  verdict above is raised as `error[copy-of-droppable]` naming the copy and what to write instead,
  identically on both backends — it is decided after the scope pass, before either generates, so
  the two cannot disagree.  The entry stays OPEN because the default is off: what the rules refuse
  still compiles on an ordinary build.  The switch is opt-in while this repository's own corpus is
  converted, which is 227 lines across 29 of the 38 files that declare `OpDrop`, plus 8 sites in
  the `registry` sqldb fixture; no published library, consumer or registry package declares one
  (P0), so nothing outside this tree waits on it.  Measured with the switch OFF: the census is
  byte-identical over all 38 files, so an ordinary build is unchanged.
- **Removal:** the corpus and the fixtures converted — the refused cells become refusal cells —
  and then the refusal on by default, with the switch left as the bisect step for a program the
  rules refuse.

### D-heap-9 — OPEN (2026-09-15, loft#1569): `OpCopy` is not a hook

- **Violates:** (H-Copy-Lease), (H-Elide).
- **Where:** `parser/definitions.rs` checks `OpDrop`'s signature (`check_drop_signature`) and
  synthesizes its cascade (`synth_drop_cascades`), and does neither for `OpCopy`; no copy site on
  either backend calls one.
- **Effect:** `fn OpCopy(self: H)` compiles as an ordinary function and never runs.  Measured on
  both backends: `a = mk(1); b = a` prints no line from the hook, and the release moves to one of
  the two structures (`M1 R1 1 D1`).  A type that means to lease its copies gets neither the lease
  nor an error saying it has none.
- **Status:** OPEN — @PLN163 P4; P5 then removes the release-moving machinery the lease replaces.
- **Removal:** the signature check, the synthesized copy cascade, and a call at every copy site
  on both backends; the moved release removed wherever a copy leases.

### D-heap-13 — CLOSED (2026-09-20): a collection returned from a call and bound to a local never releases its elements

- **Violates:** (H-Drop), and through it (H-Move) and (H-Lease).
- **Where:** the release a local's scope end runs is the cascade of the WRAPPER RECORD its backing
  names.  A vector built locally is backed by a `main_vector<τ>` record — the variable table reads
  `v … deps=[__vdb_1]` with `__vdb_1` typed `ref(723)` — and that type has a generated
  `OpDropAll`, which the scope exit calls.  A vector a CALL answers is backed by the callee's
  return buffer instead, typed as the bare collection (`d … deps=[__ref_1]`, `__ref_1` typed
  `vec<ref(718)>`), and the scope exit emits `OpFreeRef(__ref_1)` with **no cascade call at all**.
  The cascade itself is correct wherever it runs — read on both backends, it walks the elements
  and calls the hook.
  **The site is `scopes::drop_hook`, and it was MEASURED rather than read.**  The buffer IS
  offered to the scope-exit drop gate — twice — and the gate's own type pattern
  `Type::Reference(d, _) | Type::Enum(d, true, _)` turns it away, so it answers `None` and no
  cascade is emitted.  An env-gated probe printing BEFORE that pattern gives, for the leaking
  program, exactly `2 × var=__ref_1 kind=Vector` and, for the clean one, ZERO Vector lines; the
  full multiset of variables reaching the gate differs between the two programs by three lines,
  and that is the only new one.  ⚠ Reading alone had got this wrong twice in one sitting — first
  by naming the guard without testing it, then by concluding from a TRUNCATED frequency list
  (`sort -rn | head -12`, which hides a line seen twice) that no collection reached the gate at
  all.  The probe is what settled it.
  ⚠ **The cure is not to widen that pattern where it stands.**  `Type::heap_def_nr` carries the
  identical pattern and has 44 call sites across 11 files, and the `Reference | Enum` spelling of
  *"this is a heap owner"* is written out 148 times across 12 files (`scopes.rs` 44,
  `parser/control.rs` 20, `state/codegen.rs` 14).  Widening the shared predicate moves the parser,
  the scope pass, both backends and the hoist at once.  What the drop sites need is a narrow
  question of their own.  The cascade to run belongs to the WRAPPER record `main_vector<T>`, which
  is derivable from the element type (`compile.rs`, `native_lib.rs`, `data.rs` all build that
  name) — but not by naive name lookup: `data.rs`'s own regression
  `vector_wrapper_is_per_element_def_not_per_spelling` records a collision where the second asker
  got the first's wrapper.
- **Effect:** the resource stays open for the life of the process, with no diagnostic.  `(H-Move)`
  makes `d = mkv()` a MOVE — a fresh call result placed where it is produced — so `d` is the owner,
  `(H-Lease)` gives that owner its own lease, and `(H-Drop)` releases it at the owner's scope end.
  Measured 2026-09-17, both backends byte-identical, markers around each cell:
  `d = mkv(); d[0].id` prints `Cstart M75 R75 Cend` — minted, READ BACK, never released.
  Leaks too: grown after the bind (`d += [mk()]`, both ids), passed on to another function, a
  nested `vector<vector<H>>`, and a callee that fills by push rather than by literal.
  **Four neighbours are clean and bound it to the bare collection crossing a return**: a STRUCT
  from a call (`M60 R60 D60`), a struct CONTAINING a vector from a call (`M61 R1 D61`), the same
  call assigned into a FIELD (`b.v = mkv()`, `M63 R1 D63` — @PLN164 C2's buffer-is-the-place path),
  and a plain local vector (`M64 R1 D64`).
- ⚠ **`d = mkv(); e = d` releases exactly once (`M34 R1 D34`), and that is two defects cancelling,
  not a clean shape.**  The whole-collection bind mints a second structure whose backing IS a
  wrapper record, so `D-heap-8`'s extra copy supplies the release this entry lost.  It is therefore
  not a control, and curing either entry alone changes its answer — `D-heap-13` alone makes it
  release twice, `D-heap-8`'s refusal alone makes it an error.
- **Why nothing reported it.**  Every free-side instrument is structurally blind here: the MEMORY
  is freed correctly and only the hook is skipped, so `LOFT_STRICT_STORES=1` and `LOFT_POISON=1`
  are both silent.  `tests/ownership_drop_gate.rs`'s CROSS family has no collection SOURCE and no
  collection DESTINATION — its `push` and `veclit` destinations put a droppable INTO a collection
  and never bind one — so no generated cell produces the shape, and none of the 37 corpus files
  that declare `OpDrop` writes it.  Found only by probing the axis by hand while measuring
  `D-heap-8`'s census population.
- **The population is WIDER than the four rows above** — measured 2026-09-17, both backends
  byte-identical, markers around each cell.  Every one of these binds a collection a call
  answered and releases nothing: `d = mkv() ?? []` (`M21 R1`), a TUPLE member
  `t = (mkv(), 1)` (`M23 R1`), an ANNOTATED bind `d: vector<H> = mkv()` (`M24 R1`), a bind in
  a LOOP body (`M13 R1 M14 R1`), a branch of calls `d = if c { mkv() } else { mkv() }`
  (`M11 R1`), and a call that returns a local IT bound from another call (`M14 R1`).  The
  entry's own table reads as four shapes; the axis is *any* bind of a call-answered
  collection, which `scripts/matrix_axes.py` names as the container-provenance axis.
- ⚠ **The KEYED neighbour is not this defect and must not be chased as one.**  `d = mkh()`
  and a plain local `h: hash<E[k]> = []` both release nothing (`M31 R1`, `M32 R1`, both
  backends) — but `(H-Drop-Not)` says a keyed collection's records are not released by the
  language at all, and INTERFACES.md § *Three things a drop does NOT do* documents it as a
  boundary.  It is by DESIGN, so no vector-side cure should reach it, and a cascade that did
  would be a defect rather than a fix.
- **The cure is TWO changes, and the narrow one alone is measurably wrong.**  Built and
  measured 2026-09-17, then reverted:
  - Teaching `scopes::drop_hook` a `Type::Vector` arm that resolves `main_vector<τ>` from the
    element (collision-checked against the wrapper's own `vector` attribute, the
    `vector_wrapper_is_per_element_def_not_per_spelling` hazard) DOES emit the cascade, and the
    site is confirmed: the buffer reaches the sweep's final leg, which calls `scope_end_hook`
    (`is_buffer` is false — the `paired_witness` leg would have emitted
    `OpFreeRefIfDistinct(__ref_1, d)` and the emission is a plain `OpFreeRef`).  So the gate,
    not the sweep, is what turned the collection away.
  - On `--native` that alone fixed six of the families — and **doubled** `d = mkv(); e = d`
    (`M5 R1 D5 D5`, the `D-heap-8` interaction this entry already predicts) and
    `b.v = mkv()` (`M7 R1 D7 D7`), released EARLY in the nested `vector<vector<H>>`
    (`M4 D4 R1`, the inner buffer released while the outer element holds a copy), and never
    reached the loop bind.  `(H-Drop)`'s ⚠ makes that inadmissible: three lost hooks became
    doubled ones.  The missing half is `(H-Drop)`'s responsibility clause — *the copy owns,
    the source stops dropping* — which wants the buffer registered in `drop_transferred`
    wherever its elements are copied out (`OpAppendVector` into a field, `OpCopyRecord` into
    an element), the machinery `copy_hands_off` / `appends_to_element` already spell for
    other shapes.  ⚠ Whether `OpAppendVector` COPIES the element records, which decides
    whether `b.v = mkv()` holds one structure or two, was NOT established — and it decides
    whether that cell's pre-cure `D7` is a correct single release or itself a lost hook.
  - The two backends then DISAGREED on one IR, which is the second reason it cannot land: the
    interpreter ran no cascade at all.  A vector-typed var loads through `OpVarVector` — the
    4-byte record pointer `state/codegen.rs` gives vectors and keyed locals — while the
    cascade's `self` and `OpConvBoolFromRef` want the full `DbRef` that `OpVarRef` pushes, so
    the liveness guard read `rec == 0` and declined silently; `--native`, where a `DbRef` is a
    `DbRef` either way, ran them.  Wrapping the subject in `OpRefAlias` does NOT fix it — the
    argument is still loaded by `OpVarVector` (measured).
- **Status: CLOSED 2026-09-20.**  The cure is two facts, and the attempt above had the first
  one wrong in a way its own ⚠ predicted.
  - **The cascade is the COLLECTION's own, not the wrapper record's.**  `vector<τ>` already has
    a def (`DefType::Vector`, minted beside the wrapper by `Data::vector_def`), so it can carry a
    cascade whose `self` IS the collection: one walk of `self`, no own hook, no fields
    (`synth_drop_cascades`' `DefType::Vector` target, `cascade_self_type`, and
    `drop_elements_loop` now taking the collection VALUE so a record's cascade and a
    collection's share one body).  That is what makes the two backends agree, and it is why the
    wrapper route could not: handing the WRAPPER's cascade a collection binding releases on
    `--native` and silently does nothing on `--interpret`.
  - **It runs over the BINDING the buffer delivered to, and only where there is one**
    (`delivered_binding`: the single local whose type borrows the buffer and is itself a
    collection).  `(H-Drop)` names the owner and `(H-Move)` says that is the author's local.  It
    is also the only subject that works on the interpreter, measured: `OpDatabase`'s reuse arm
    claims a FRESH record in the cleared store there while native re-establishes record 1, so
    after the callee fills it the caller's buffer still names the record it held before the
    call — empty.  Requiring a binding is what keeps `b.v = mkv()` single: that buffer has no
    binding, its value was handed to a field that already releases it, and cascading over the
    buffer as well is exactly the `D7 D7` the attempt above measured.
- ⚠ **The `d = mkv(); e = d` "double" was a misreading, and its ORACLE settles it.**  The bind
  COPIES — measured: `e += [mk()]` leaves `d` at length 1 — so there are two structures and
  `(H-Drop)` owes two releases.  The no-call twin `f: vector<H> = [mk(30)]; g = f` releases
  TWICE on both backends, before this fix and after; it is the shape the language already
  answers and this entry's cell now matches it.  The entry above read the second release as a
  defect because the pre-cure shape released once; what cancelled was a lost hook against a
  correct one.
- **What this does NOT reach, measured against the same oracle, and both are pre-existing:**
  a local REASSIGNED in a loop through a call releases only the last (`d = mkv(11); for … { d
  = mkv(12+i) }` gives one where the no-call twin gives three), and a NESTED
  `vector<vector<τ>>` releases nothing — but its no-call twin releases nothing either, so that
  is a separate defect of the nested cascade and not of this one.  `collection_elem_cascade`
  admits only a `Reference`/`Enum` element for that reason, which is also what keeps the
  attempt's EARLY release in the nested cell from happening at all.
- **Guard** `tests/scripts/1551-a-collection-a-call-answers-releases-its-elements.loft`: eleven
  cells asserting the exact drop TRACE (a file, for the reason `139-drop-cascade.loft` gives),
  each with its no-call oracle beside it, byte-identical on both backends.  Falsified by
  sabotage — the collection arm made to answer `u32::MAX` fails `a1` first on both backends
  (`r1` where `r1,1` is owed) and the drop gate reports `p_v5`–`p_v7` `clean -> LOST`
  (`released 0x, want 1`).  The gate's own verdict on the cure: exactly those three lines GONE,
  **no NEW line on either baseline**, and the controls `p_v8`/`p_v9` unmoved.
- **Removal:** the fact belongs in the TYPE, not in the gate.  The callee already retypes its
  buffer — `n_mkv`'s `__vdb_1` loads as `ref(main_vector<H>)` under `OpVarRef` while the
  caller's `__ref_1` for the same store keeps `vector<τ>` and loads under `OpVarVector` — so
  the caller-side return buffer carrying the wrapper's record type is what makes the existing
  `Reference` arm fire, both backends agree, and no new spelling is added at the 148-site
  predicate's expense.  *Superseded by the closure above*: the fact went into a cascade of the
  COLLECTION's own rather than into the buffer's type, which needed no retyping and no new
  spelling of the 148-site predicate, and the hand-off half turned out to be unnecessary once
  the subject is the BINDING — a buffer whose value was handed to a field simply has no binding
  to cascade over.  `p_v5`–`p_v7` are clean on both baselines and `p_v8`/`p_v9` still read one
  release each.

### D-heap-14 — OPENED 2026-09-17, CLOSED 2026-09-21: a hand-over written under a branch leaks the source on the path that does not run

- **Violates:** (H-Drop), and (H-Spent)'s per-path clause.
- **Where:** `Scopes::mint_handoff_flag` arms a per-path `__hoff_` flag only for a conditional bind
  to a LOCAL — loft#1515's shape, and its own doc says so (*"a copy off a PARAMETER written in a
  branch arm"*).  Every other destination a hand-over can have suppresses the source's scope-end
  release STATICALLY, with nothing to restore it on the path that did not run.
- **Effect:** the source is never released.  Measured 2026-09-17 with the branch never taken,
  both backends byte-identical, no diagnostic:
  `if c { x = cc }` (a bind to a local) traces `V9 D9 D1` — correct, and it is the only form that
  mints a flag (2 of them); `if c { s = S { h: cc } }` traces `E` — nothing released;
  `if c { s.h = cc }` traces `F2 D2` — the source leaked; `if c { v += [cc] }` traces `L0` —
  nothing released.  The control with the same append and the branch TAKEN traces `L1 D1`, so the
  hand-over itself is correct; what is missing is the other path.  The variable tables of the
  taken and not-taken forms are byte-identical and neither carries a `__hoff_` variable, so the
  decision is static — measured, not read off the source.
  ⚠ **No leak instrument can see this.**  `LOFT_POISON`, `LOFT_POISON_CLAIM`, `LOFT_STRICT_STORES`
  and `LOFT_NATIVE_LEAK_CHECK` report nothing on the leaking cell — and nothing on the clean
  control either, so their silence is not a verdict.  What is skipped is the HOOK while the
  record's memory is still freed, which is the blindness `D-heap-13` already records.  The hook
  trace is the only channel that shows it.
- **Status:** CLOSED 2026-09-21 — found while measuring `D-heap-8`'s population.  No corpus guard
  exercised the shape: the only conditional appends among the 38 hook-declaring files are
  `bytes += [c as u8 ?? 0]` in the trace helper, which appends bytes rather than a droppable.
- **Closed:** the per-path flag is armed for every destination a hand-over can have.  A user
  variable whose type owns a droppable, handed to a field, an element, a literal or a return
  buffer by a copy inside a branch arm (`arm_container_handoffs`), gets loft#1515's flag; the
  collector keeps it out of the static set (the pair `(u16::MAX, source)`), and the scan sets the
  flag right after the copy runs.  Which copies stop what is one question with one home,
  `copy_record_handoff`, read by the collector and by the flag write alike.  Measured on the new
  `handover` family of `ownership_drop_gate` (`k_*`: eight destinations × `if`, `if`/`else` and a
  `match` arm × taken and skipped, a `return` from an arm, a loop that moves on some passes): 24
  of its 56 cells lost the source on the tree before, on both backends, and none does after.
  Guard `tests/scripts/a-move-in-one-arm-leaves-the-other-path-releasing.loft`.

### D-heap-15 — CLOSED (2026-09-17, NARROWED 2026-09-21, CLOSED as reclassified 2026-09-23, loft#1563): a value the rules MOVE is still copied, and both structures release it

- **Violates:** (H-Move), and through it (H-Lease).
- **Where:** not established.  What is measured is the population, below; the shapes share that
  the value reaches its destination through a LIFT or a backing rather than a direct bind, but
  that is an observation about the cells and not a site, and naming a site without probing one is
  the error this chapter has already paid for twice.
- **Effect:** the owner's 2026-09-17 ruling makes a value the function OWNS a MOVE wherever it is
  placed, so each of these owes exactly ONE release.  21 of the drop gate's 285 cells run two,
  identically on both backends: `p_v1` and `p_v2` (a whole-collection bind), `p_j3` and `p_j4` (a
  join arm placed into a container), `c_coalesce_field`, `c_coalesce_enum`, `c_coalesce_push`,
  `c_coalesce_veclit`, and every `q_*_local_*_field` / `q_*_local_*_push`.  Measured directly:
  `a: H? = mk(1); v += [a ?? mk(9)]` traces `M1 L1 D1 D1` and
  `a: H? = mk(2); c = Hold { h: a ?? mk(9) }` traces `M2 F2 D2 D2` — the present arm's local is
  copied into the destination and both copies release.  `p_o2` (a tuple) and `p_i2` (a self-bind)
  add the scorer's EARLY channel on top, so those two release twice AND release before a read,
  which is a use-after-release as well as a double one.
  ⚠ **The plain bind is CLEAN and that is the boundary**: `a = open(1); v: vector<H> = [a]`
  traces `L1 D1`, one release, and `c_local_field` / `c_local_push` / `c_local_veclit` /
  `c_local_enum` are all absent from this set.  So this is not "an owned local placed into a
  container" in general — it is the shapes that reach the destination some other way.
- **Status:** OPEN — exposed 2026-09-17 by narrowing the refusal to the rules.  Before the
  ruling every one of these was verdict `Refused`, so the gate asked nothing of their releases
  and the disagreement could not show.
  ⚠ **That is worth stating as a property and not as an anecdote: a `Refused` verdict is an
  ABSENT MEASUREMENT wearing a verdict's clothes.**  A refused cell is never asked how many times
  it releases, so a register that refuses too widely does not merely forbid correct programs — it
  HIDES the cells nobody has judged, and it hides them behind something that reads like an
  answer.  This is the same failure as an instrument that answers a narrower question than the one
  it appears to, where silence reads as a pass; it is worse only because a verdict does not even
  look like silence.  The 23 cells here and in `D-heap-16` were the measure of it: they appeared
  the moment the verdict narrowed, having been there all along.
- ⚠ **NARROWED 2026-09-21 — the JOIN half is closed.**  17 of the 21 cells placed a join into a
  container: `Hold { h: a ?? b }`, `v += [if c { a } else { mk() }]`.  The parser copies such a
  join as ONE value (`OpCopyRecord(<join>, dest, tp)`), whose source names no variable, so no
  release was handed over on any path.  The author's own statement spelling,
  `if c { v += [a] } else { v += [b] }`, was measured clean on all twelve cells of the same
  matrix, so the join is now written out to it before any analysis reads the function
  (`scopes::write_out_joined_copies`), and each arm is the plain copy that `D-heap-14`'s per-path
  flag already decides.  A bare minting call arm is given the owner a view-typed join gives it
  (`{ __ref_N = call; __ref_N }`), which also closed the STORE that `v += [a ?? mk()]` leaked on
  the path that made it.  All 17 are clean on both backends, and so are 11 of `D-heap-8`'s cells
  on the path where the value the join chose was the function's own (`q_default_field_*`,
  `q_default_param_*`).  **Open:** `p_v2` (`d = v`, then `v` grown — `(H-Spent)` refuses it;
  `p_v1`, the same bind without the growth, closed with `D-heap-23`), `p_o2` (`u = t` of a tuple)
  and `p_i2` (`a = a ?? mk()`, a variable rebound to a `??` over itself).  None of them is a
  join.
- ⚠ **NARROWED again 2026-09-22 — the tuple bind is closed (`p_o2`, loft#1563).**  `u = t` of a
  tuple is lowered onto one copy per member (loft#1361), and a member names its work-ref in the
  tuple's type — except a member its own CALL minted (`t = (mk(11), 1)`), whose pairing lives only
  in the scan's `tuple_call_mint`.  So that copy moved nothing: both sides ran the hook, and a
  refill released the moved member again before the copy was read (`p_o2`; a literal member
  measured the same).  `scopes::call_minted_member_handoff` makes the member's buffer the source
  the copy moves the release from, `tuple_owned_elem_frees` skips a handed-off buffer's hook, and
  an unconditional refill retires the hand-off for the members it mints.  Only the whole-tuple
  bind moves: `(t.0, 2)` lowers onto the same copy but spells a copy of a container's member,
  which `(H-Copy-Refuse)` refuses, so the bind's builder names its blocks `tuple_member_move` and
  the hand-off reads that name (`c_tuple_tuplem` stays with its `c_tuple_*` siblings).  And only
  where the copy is CERTAIN to run (`walk_unconditional`, in the tuple's own scope): a move
  written in an `if` arm keeps both releases on the path that runs it, because moving it there
  lost the release on the path that skips the arm.  Guard
  `tests/scripts/1563-a-moved-tuple-releases-a-call-minted-member-once.loft`.  **Open:** `p_v2`
  and `p_i2` (programs `(H-Spent)` and `(H-Copy-Refuse)` refuse, `D-heap-8`'s errors), and that
  tuple move written in an arm.
- ⚠ **NARROWED again 2026-09-22 — the variable rebound to a join over ITSELF is closed (`p_i2`,
  loft#1563).**  `p_i2` is not a copy: `a` is a local the function owns, so `(H-Move)` moves it
  and it owes one release (`PILOT_ONCE`).  A rebind from a value branch is written out per arm
  (`@FR-O-Complete`), and `sink_set_into_arms` declined an arm whose tail is the binding itself.
  The value form then ran instead, with two outcomes and both wrong.  For `a = a ?? mk()`, the
  rebind snapshotted `a` as displaced and released it on the path where the new value IS that
  record: early, and again at scope end.  For `a = if c { a } else { mk() }`, the join's type
  named `a`, so `a` read as a borrow of itself and released nothing at all.  Written out, that
  arm is `a = a`, the identity `(B-Copy)` gives no second structure (#330's elision).  It moves,
  displaces and releases nothing, and each other arm is the plain rebind the author's own
  `if c { } else { a = mk() }` makes.  The binding's dep on itself is dropped with the arm.
  Measured clean on both backends: every arm side of `??` and `if`, each operand present or
  absent, a local on the other arm (`a = a ?? b`, `a = b ?? a`), in a loop, in an `if` arm, and
  a record holding a droppable member.  Guard
  `tests/scripts/1563-a-variable-rebound-to-a-join-over-itself-releases-once.loft`.
  **Open:** `p_v2` (`(H-Spent)`) and the tuple move written in an arm.  A `??` CHAIN whose
  hoisted subject names the destination (`a = a ?? b ?? mk()`) is a different defect, a backend
  split at the hoist, and has its own entry: `D-heap-28`.
- **Removal:** the copy that makes the second structure, removed wherever the rules move the
  value; `scopes::copy_moves_drop_from` and the hand-off flags beside it are @PLN163 P5's
  subject and this entry is the measurement P5 is verified against.

- **CLOSED as reclassified 2026-09-23.**  The one cell left was `p_v2`,
  `v: vector<H> = [mk(81)]; d = v; v += [mk(82)]`.  It was re-read against the rules rather than
  against the gate's own verdict.  `d = v` places a value the function OWNS, so `(H-Move)` moves
  it and `v` is SPENT from the end of that statement, and `v += …` reads it, which `(H-Spent)`
  makes a compile-time error.  So `p_v2` is a program the rules REFUSE, and its releases are not
  this entry's to judge: the gate's `PILOT_ONCE` verdict was the oracle's error, and the cell
  moves to `PILOT_REFUSED` under `D-heap-8`, whose unbuilt second half is exactly that
  spent-name error (loft#1568, open).  `rule_tags.py registers --issues` flagged the entry
  because it named the closed loft#1563 while still reading OPEN.  That was right to re-measure,
  and the measurement is this reclassification, not a fix.
### D-heap-16 — OPENED 2026-09-17, CLOSED 2026-09-21: the fresh value of a `??` DEFAULT arm is never released

- **Violates:** (H-Drop).
- **Where:** the arm lift (`Scopes::lift_join_arm_tails`).  `x = a ?? mk(7)` over a LOCAL `a` is
  typed as `a`'s own type, so the parser reads the join as owned and leaves the call arm for `x`
  to own.  The lift then rewrites `x` into a borrow of the per-arm temps and gives a temp only
  to the `a` arm.  The call's answer lives only in the value the join hands over: its `__ref_N`
  buffer starts as the null sentinel (loft#1085), and a callee handed null mints a store of its
  own.  So nothing owned it, and the buffer's scope-end free was paired with `x` besides.  The
  `if` spelling of the same join was clean, because the parser types its `a` arm as a view and
  gives the call arm an owner (`join-arm-owner`) before the lift runs.
- **Effect:** the value the default arm builds owes one release and runs none.  Measured on both
  backends: `a: H? = null; x = a ?? mk(7)` traces `M7 R7` — minted, read, never released — and
  the loop form `for i in 0..2 { x = a ?? mk(27 + i) }` leaks once per iteration
  (`M27 R27 M28 R28`).  The PRESENT twin is correct and is the control: `a: H? = mk(5);
  x = a ?? mk(7)` traces `M5 R5 D5`, one release, with `mk(7)` never evaluated at all.  Two gate
  cells carry it, `p_l2` and `q_default_local_call_local`, scored `LOST`.
  ⚠ **Not the same as D-heap-14** despite both involving a branch.  There is no hand-over here:
  nothing suppresses a source's release, because the default arm's value has no source — it is
  minted in the arm.  The per-path `__hoff_` flag is absent from these cells and would not apply.
  That distinction was measured rather than read off the shape.
  The neighbouring record is `closures-history.md`'s `??`-default STORE leak
  (`g = fn(q: P?) -> P { q ?? P{} }`, argument witness closed by loft#1248, capture witness
  tracked there).  That is a lambda leaking a store per call; this is a plain function leaving a
  HOOK unrun.  Whether one cure reaches both is not established.
- **Status:** CLOSED 2026-09-21 — exposed 2026-09-17 by narrowing the refusal, the same way as
  `D-heap-15`.
- **Closed:** when the lift turns the binding into a borrow, it gives each bare minting call arm a
  temp of its own as well, which owns the store on the path that made it and is null on every
  other (`Scopes::lift_owned_call_tails`).  `p_l2`, `q_default_local_call_local` and
  `q_default_param_call_local` moved LOST → clean on both backends, with the present-arm control
  at one release.  ⚠ The missing owner was never the hook's alone: the STORE leaked too, for
  EVERY record type — `a: P? = null; x = a ?? mkp(n)` in a function called 1000 times left
  `P×1000` unfreed on both backends — and that is closed with it.  The fix also exposed
  `D-heap-18` in four cells, which had released once only because their store had no owner.

### D-heap-11 — OPENED 2026-09-15, CLOSED 2026-09-17: a view of a droppable member is turned into a copy when its container is disturbed

- **Violates:** (H-View-Drop).
- **Where:** `(B-View)`'s materialisation — a bind is given its own copy when its container is
  disturbed while the view is still used (@PLN130 F2/F4/F8, reported through
  `copy_manifest::note_materialised_view` and its siblings).
- **Effect:** for a type that owns a droppable without `OpCopy`, the materialised copy is a second
  structure on one resource, made by the compiler on a line the author did not write as a copy,
  with only the advice that writes no longer reach the container.  No gate cell covers it yet.
  **Measured 2026-09-17**, both backends byte-identical, one resource released TWICE on each of
  `(B-Disturb)`'s three reachable events — a growth (`M1 M2 R1 D1 D1 D2`), a removal
  (`M1 M2 R2 D2 D2`, the removed element correctly released by nobody per `(H-Drop-Not)`), and a
  reassignment of the base (`M1 D1 R1 D1`, the second release after the read) — against controls
  with no disturbance, which release once (`M1 R1 D1`).
- **Narrowed 2026-09-17 to the view the author did NOT spell `&`.**  The `&`-spelled half was
  measured releasing twice the same way, and is now refused rather than copied: `binding.md`
  D-bind-47 gave `(B-Ref-Reshape)`'s refusal the store it needs to see a growth of a container
  held in a FIELD, which is the answer this entry's **Removal** already points at.  What is left
  open here is the plain view, which `(B-View)` materialises on purpose for every other type.
- **Status:** CLOSED 2026-09-17 (@PLN163 P3's `(H-View-Drop)` half).
- **Removal, as taken:** the disturbance is a compile-time error naming the view — the answer
  `(B-Ref-Reshape)` already gives a `&` reference — raised from the walk that already refuses for
  that family (`scopes::def_reshape_refusals`), with one condition added: the view's type owns a
  droppable.  Nothing new decides WHICH bindings are views: `record_target` already admits only a
  binding that is a view at all and whose right-hand side names a container, so the walk's answer
  and the copy-out advice agree cell for cell (measured), and the type question is the only one
  this rule adds.  The message is its own, because the `&` family's — *"a write through `c` would
  no longer reach the element it names"* — is beside the point for a plain view, which never wrote
  through: this population is told that the copy itself is the fault, and is offered reading the
  member where it lives rather than "bind without `&` to work on a copy", which names exactly the
  copy `(H-Copy-Refuse)` rejects.
- **Boundary — stated wrongly here on 2026-09-17, RE-MEASURED and CLOSED the same day.**  This
  entry read *"a callee's REMOVAL from a droppable container refuses while a callee's GROWTH does
  not — the same program, one frame apart, gets two answers."*  The asymmetry was real; the
  description of it was not.  Measured over 17 cells on both backends, **neither refused**: the
  removal refuses only when the container is the `&` PARAMETER ITSELF, because
  `scopes::removed_ref_params` keys on `OpRemoveVector(arg0)` / `OpRemove(arg1)` over a bare
  `Var` typed `RefVar`, so a removal from a FIELD of a parameter
  (`fn shrink(b: &Bag) { b.v.remove(0) }`) was not refused either, and a GROWTH had no callee
  producer on the refusal side at all.  The claim was written from the code's shape rather than
  from a cell, which is the error it records.
  **Closed** by giving the refusal the callee reach the rules already state — see `binding.md`
  D-bind-48, which is one change serving both populations, this rule's and `(B-Ref-Reshape)`'s.
- **Verified:** 13 cells, both backends byte-identical, each predicted before it was run — the
  three disturbance events and a nested droppable refuse; a non-droppable view still materialises
  and still says so; a droppable TUPLE member, a view with no disturbance, a view dead before the
  growth, a sibling field's growth and a whole-container bind all keep compiling.  Corpus: 1590
  files compiled with a before and an after binary, **0 changed** — no program in the tree writes
  this shape, which is why the corpus could not have caught it.  Guards:
  `tests/scripts/a-view-of-a-droppable-member-stays-a-view.loft` (the over-reach controls) and
  `parse_errors::h_view_drop_*` (the four refusals, with their exact prose).
- **Found while measuring this entry, and NOT it:** `d = b.v; b.v += [mk(2)]` — a whole-container
  bind of droppables — releases one resource twice in silence.  Nothing is materialised there, so
  it is not `(H-View-Drop)`; it is a written COPY, which `D-heap-8` already owns, and it now has a
  measured cell waiting for that pass.

### D-heap-10 — CLOSED (2026-09-15, reclassified): a variable rebound to a `??` over itself released its record before the read, and again

Opened the same day against `(H-Rebind-Self)`, which said `a = a ?? d` makes no new structure.
The revised rules judge a copy by its own line and have no such rule: `a = a ?? mk(122)` writes a
copy of the existing `a`, so it is refused and belongs to `D-heap-8`.  Measured while open, on both
backends: `a: H? = mk(121); a = a ?? mk(122); println(a.id)` released 121 before the read and again
at scope end (gate cell `p_i2`), and `a = a` released once (`p_i1`).

### D-heap-1 — CLOSED (2026-09-15, reclassified): a copy of a tuple member releases its resource twice

**Reclassified 2026-09-15 by the copy-lease rules (@PLN163).**  Every shape below writes a copy of
an existing value — a parameter's member that escapes, a copy off a loop variable, a bound
projection returned, a `??`-guarded returned member, and the tuple assigned twice (`u = t`,
`p_o2`) — so the rules refuse each, and they belong to `D-heap-8`.  The rest of this entry records
how the shapes were measured.

`(H-Drop)` runs the hook once per resource and moves the responsibility with a copy.  A
tuple is a container like any other (`layout.md (L-Tuple)`), so `t = (s, 5); u = t` must
release once — and since loft#1361 made the whole-tuple bind COPY its heap member, the copy
is a real second record, which is what puts the question here at all.  Before that bind
copied, one record wore both names and "released once" was true by accident.

Most of the family holds: the whole-tuple bind, two droppable members, a member at index 1,
a chain of copies, a struct-enum member, a return buffer, a destructure after the copy and
the literal itself are each measured at one release on both backends
(`tests/scripts/a-copy-of-a-tuple-with-a-droppable-member-releases-once.loft`), and so are
every dep-carrying NEIGHBOUR of a droppable member and every ANNOTATED spelling of the same
tuple (`a-tuple-member-releases-once-beside-any-neighbour.loft`).  FIVE shapes do not, both
backends, silently — re-measured 2026-09-10.  The list has been re-cut four times as it was
measured, and FOUR of the shapes it has carried were found by a guard or a matrix reaching
past its own subject — which is what an over-wide cell is for:

- a copy off a parameter member that ESCAPES the callee — into a CONTAINER field
  (`fn f(p: (S, integer)) { c = H { s: p.0 }; }`), or through the return (`u = p; return u.0`,
  and `return p.0` directly): twice.  They share the boundary the parameter cure cannot cross:
  the release that has to be suppressed is the CALLER's, and no analysis of the callee's frame
  can reach it — the container case needs a cascade that skips one field, and the returning
  cases hand the caller a second record of its own resource, which is a fact about the
  SIGNATURE (does the return share the parameter's resource?) rather than about either body.
  ⚠ `return p.0` releases a store already RECYCLED — its hook read an id the program never set
  (`4` where the resource was `131`) — so this shape has a use-after-free face and a cell for it
  must score the VALUE the hook sees and not only the count.  The CONTAINER spelling and
  `return p.0` now carry `warning[double-move]` (D-heap-7 family 2); `u = p; return u.0` does
  not, and the releases of all three are unchanged;
- a copy off a LOOP VARIABLE over a `vector<(τ, τ)>` — `for e in v { u = e; }`: twice;
- a tuple variable ASSIGNED TWICE — `t = (s, w); u = t; t = (s2, w2); z = t;`: both twice.
  What the resolver has is the VARIABLE, and each assignment brought its own backings, so
  there is no single pairing to read; the carried table (below) joins them and DECLINES rather
  than answer with the latest one, which is measured to lose `s2`'s release outright instead
  of duplicating `s`'s.  The cure is per-ASSIGNMENT resolution — the copy's own position, which
  `drop_bearing_source` does not see — and it is the same missing fact the bound-projection
  shape needs, so the two are one plan and not two;
- a bound projection RETURNED — `x = t.0; return x`, where the materialised return copies the
  view into the return buffer and `copy_moves_drop_from` suppresses `x`, which owns nothing:
  the member's backing still drops beside the buffer.  The same question as the loop-variable
  shape (a copy whose SOURCE is a view, discussed in its own paragraph below), with the
  difference that a tuple member's backing is a nameable work-ref, so a cure exists here
  where a vector element has none.  Measured 2026-09-10 — see the mechanism paragraph after
  this list, which separates it from the `??` shape it was briefly conflated with;
- a returned member guarded by `??` — `return t.0.0 ?? d`, and `return t.0 ?? d` at FLAT depth
  too, so the axis is the `??` and not the nesting: twice.  Measured on the pristine control and
  unchanged by either read-site fix below, so it is neither of them.  **Read off the IR
  2026-09-10** — the source is the coalesce `If`, which the resolver declines outright; the
  mechanism paragraph below has it, and it is NOT the bound shape's.

**The `??` shape and the bound-projection shape share a SYMPTOM, not a root.**  Measured
2026-09-10 over an 11-cell matrix, byte-identical on both backends (`(O-NoDiverge)`), with a
droppable `TdH` whose `OpDrop` appends to a trace file.  Both leave the same trace — the source
tuple released before the value is used, the returned copy released again after — and it was that
shared signature that made them look like one defect:

| cell | shape | trace | releases |
|---|---|---|---|
| b1 | `return t.0` | `alive11,11` | once ✓ |
| q1 | `return t.0 ?? d` | `21,alive21,21` | **twice** |
| q2 | `return t.0.0 ?? d` | `22,alive22,22` | **twice** |
| x5 | `m = t.0; return m` | `35,alive35,35` | **twice** |
| x3 | `m = t.0; return m ?? d` | `33,alive33,33` | **twice** |
| x6 | `m = t.0 ?? d`, NO return | `alive36,36` | once ✓ |
| x7 | `m = t.0`, NO return | `alive37,37` | once ✓ |
| x1 | `return s ?? d` (a local, no tuple) | `alive31,31` | once ✓ |
| x2 | `return bx.s ?? d` (a field, no tuple) | `alive32,32` | once ✓ |
| x4 | `return t.0` with a `??` ELSEWHERE | `alive340,34` | once ✓ |
| x8 | two members, `return t.0 ?? d` | `81,82,alive81,81` | **whole tuple early** |

x5 doubles with NO `??`, and x1/x2 keep a plain `??` at one release, so the `??` is not the axis
either shape turns on: **x3 was the bound shape wearing a `??`**.  What both need is a RETURN
(x6/x7 hold at one) and a tuple MEMBER (x1/x2 hold at one).  x8 says what the early release
actually is — the whole SOURCE TUPLE drops (both `81` and `82`) before the alive marker, and the
returned member then drops again in the caller.

Both fail in `scopes::drop_bearing_source`, which resolves a copy's source by IR SHAPE — but they
fail at DIFFERENT arms of it, and an env-gated probe inside that function is what separates them.
The variable table alone is not enough to tell them apart — it prints `__ncc_1` as an ordinary row
naming the tuple, which reads exactly like the bound shape's carrier — so the arm each source
actually reaches has to be observed in the resolver, not inferred from the row.

**The bound shape is a carrier.**  `m = t.0; return m` arrives as `Value::Var(m)`, and that arm
answers *that variable* and stops.  `m` is not the drop-bearing source — the table prints it
`deps=[t(3)]`, naming the tuple VARIABLE, while the resource sits behind the member's backing
work-ref `__ref_p2_1(6)` — so `copy_moves_drop_from` suppresses something that owns nothing and
the backing drops beside the copy that took its resource.

**The `??` shape is not a carrier at all.**  `return t.0 ?? d` arrives as the coalesce lowering
`If(<null-check Var(__ncc_1)>, Var(__ncc_1), <default block>)`.  `drop_bearing_source` has arms
for `Var`, `TupleGet`, `Block` and `Insert` only, so an `If` falls to `_ => None` and the hand-off
is declined outright.  That is also why it cannot be a resolver tweak: the member is handed off on
the PRESENT arm and not on the default one, so the answer is per-path — the conditional-ownership
question `ownership_cfg.rs` already carries `__ncc_*` machinery for, not a lookup.

Measured negative: recording a `tuphold_origin` entry at the `_ncc` mint AND resolving the `Var`
arm through `tuphold_origin` moved **no cell** — as the `If` reading predicts, since that source
never reaches the `Var` arm.  The change was reverted rather than shipped as a no-op.

So the two want different cures.  The bound shape wants the carrier's origin recorded, and
`Function::tuphold_origin` is the fact for it — `tuple_copy_source_path` already walks it — but it
is recorded only for the `_tuphold` and `tuple_tmp` work-refs (`parser/operators.rs`,
`parser/vectors.rs`), never for a user bind.  What makes that more than a one-liner is
REASSIGNMENT: an origin recorded at `m = t.0` is stale after `m = <anything else>`, so the record
needs invalidation, which is why this is plan-sized rather than a bind-site hook.  Its prediction
is narrow and falsifiable: **x5 alone** falls to one release, while q1, q2 and x3 — all three
`If`-sourced — do not move, and x1, x2, x4, x6, x7 and the existing guard's nine cells stay put.

✓ **A dep-carrying NEIGHBOUR that is not a heap leaf — `t = (s, w); u = t` — CLOSED
2026-09-10.**  The pairing was INFERRED by counting — the k-th dep backs the k-th heap leaf —
and counting is wrong in both directions, with neither predicate a superset of the other.  A
`text` or a value-ENUM member bound from a variable contributes a dep without being a heap
leaf, so the list is LONGER than the walk; a text LITERAL member has a dep SLOT that stays
empty, so widening the walk to *"has a dep slot"* makes the list SHORTER than it.  (The
earlier entry named the value enum as the other direction on the strength of its EMPTY dep
list; measured, `(s, ve)` contributes `ve` and doubles like the text, and the cell that a
widened predicate actually breaks is the text literal.  Both directions are real; which
member kind sits on which side was not.)

So the pairing is CARRIED.  `Vars::tuple_backings` records each heap leaf's own backing at
`Vars::depend_all` — the one site that writes the union over the members — and
`scopes::tuple_member_backing` reads it there first.  The count read stays beneath it: it is
still the only answer for a tuple that already carried the union by the time its variable was
typed, measured live in three corpus guards, where the table declines.  ⚠ Recorded as the
JOIN over the variable's assignments, per PASS: pass 1 spells a heap member with the LOCAL it
was built from and only pass 2 copies it into a backing of its own, so a join across the
passes reads every entry as two assignments disagreeing and the fix vanishes — measured, on a
build that was otherwise complete.

⚠ **And the same question had a SECOND site, which this guard's over-wide cell found and the
report did not name.**  Writing the tuple's type out routes the binding through the conversion
that adopts an ANNOTATION over the literal's own type, and that site carried the literal's deps
across as the union for the same reason the variable does — a tuple has no dep list of its own,
so `Type::with_deps_of` cannot reach one and was a silent no-op.  `Type::with_member_deps_of`
is the tuple-aware form; it pairs the members up.  Without it every ANNOTATED spelling still
released twice while every inferred one had been closed — including the nullable member that
site was fixed for once already, as soon as a `text` stood beside it.  Guard
`a-tuple-member-releases-once-beside-any-neighbour.loft`, 24 cells scoring the release COUNT:
17 move and 7 hold.

✓ **A copy off a tuple PARAMETER's member — `fn f(p: (S, integer)) { u = p; }` — CLOSED
2026-09-10** for every copy that DIES in the callee.  The rule was already implemented for the
sibling spelling: a plain struct parameter is answered by `copy_moves_drop_from`, whose
`is_argument(src)` arm suppresses the DESTINATION rather than the source, because the callee's
own copy is what stops dropping and the caller's record is the one that stays.  A tuple
parameter reaches the same rule as a MEMBER read, and that is what fell through — the member's
backing is in the CALLER's dep space, so no variable of this frame names it and
`tuple_member_backing` declines rather than guess a local wearing that number.  Declining was
right and insufficient: the rule does not need the caller's backing.  Cure:
`tuple_argument_member` answers the member's TYPE when the chain ends at an argument, and the
destination is suppressed on that alone, using the type test `copy_moves_drop_from` already
used — extracted as `copy_carries_drop` so the two cannot drift.  Guard
`a-copy-off-a-tuple-parameter-member-leaves-the-caller-owning.loft`, 14 cells, including the
copy in a LOOP (three records before the fix), the parameter passed ON, and the callee's OWN
local tuple, which wants the member pairing instead and must not be swallowed.

✓ **A nested member RETURNED — `t = ((s, 1), 2); return t.0.0` — CLOSED 2026-09-10.**  One
question, two read sites.  A nested COPY reads each leaf through a `_tuphold`; a nested READ
materialises through a `tuple_tmp` work-ref instead, which `Function::tuphold_origin` had no
entry for, so the walk back to the tuple carrying the leaf/work-ref pairing stopped at a temp
whose dep names the tuple LOCAL.  The second site now records its projection the way the first
does.  At depth THREE the projection arrives wrapped in the lowered block of the level below
it, so the record is taken through `data::tuple_projection_of` — matching the bare `TupleGet`
alone left `t.0.0.0` releasing twice while the two shallower cells passed, which is the shape a
one-cell probe would have called closed.  Nullable or dense makes no difference (`@FR-N-Shape`).
Guard `a-nested-tuple-member-returned-releases-once.loft`, 12 cells.

⚠ **And it took a second chokepoint, which the OVER-REACH cell found.**
`scopes::construction_work_ref` asks *does this block hand its target a record it BUILT*, and
it asked by resolving the block's tail to a work-ref — a question a member read now answers.
So `u = t.0.0` read as a construction, the member's backing was marked handed-off, and the
binding it was handed to is a VIEW that drops nothing: the resource was released by NOBODY,
which is the opposite fault and one no cell of the returning family could see.  The predicate
reads the tail by NAME now (`block_tail_var`), which is the honest statement of what a
construction is: a block that ends IN the work-ref it filled, where a projection ends in a
member read.  The general form is worth keeping in view — a hand-off is only sound when the
destination will actually run the hook, and `(H-Drop)` says a copy moves the release, not that
something else will make it.

✓ **A NESTED tuple — `t = ((s, 1), 2); u = t` — CLOSED 2026-09-10.**  `(L-Tuple)` makes a
tuple a synthetic struct, so a tuple inside a tuple is a container inside a container and the
depth is not a semantic axis.  Two things had to be read to find a nested leaf's work-ref in
the dep list and neither was.  The count and the index were taken over ONE LEVEL's members
while the list runs over every heap leaf THROUGH the nesting, so a tuple with an inner tuple
either declined (a level count of 1 against a list of 2) or indexed the list at the wrong
place.  And a nested copy reads each leaf through a HOLD (`_tuphold_N`,
`Parser::tuple_member_owned_copy`) whose own dep list names its base variable and nothing
else, so the tuple whose type carries the pairing is one or more holds further out and the
source named none of them; the hold's member index is the one fact its type cannot carry, so
the parser records it (`Function::tuphold_origin`) and `scopes::tuple_member_backing` walks
that chain to the root.  Cure: `tuple_copy_source_path` + `tuple_leaf_at` + the recursive
`tuple_heap_leaves`, all in `scopes.rs`, with the length check unchanged as the safety valve.
Guard `a-nested-tuple-copy-releases-each-member-once.loft`, 14 cells, scoring the release
COUNT.

⚠ **This entry's earlier scope did not survive being measured.**  It read *"not nesting as
such but a droppable reached THROUGH an inner tuple"*, on the strength of `t = (s, (1, 2))`
being correct.  It is — but only because that tuple has ONE heap leaf, so the level count and
the leaf count agree by accident.  Put a droppable on each side (`t = (a, (b, 2))`) and BOTH
release twice, the outer one included.  That cell is also what pins the index to member order
rather than to the numbering: `(a, (b, 2))` parses the inner literal first, so the inner
member's work-ref carries the LOWER number while the list still names the outer member's
first (measured `deps=[__ref_4(8), __ref_3(7)]`).  A read that sorted, or that trusted the
mint order, passes every other cell and fails that one.

✓ **A member declared NULLABLE — `t: (S?, integer) = (s, 5); u = t` — CLOSED 2026-09-10.**
The site is the one that adopts an ANNOTATION over a literal's own type
(`parser/expressions.rs`, loft#1034's tuple conversion): a declared type says the SHAPE and
cannot say what the value in hand owns, because it was written before that value existed, and
adopting it whole dropped the literal's backing dep.  `Type::with_deps_of` states exactly that
rule but does not reach a TUPLE — a tuple carries no dep list of its own and `deps_ref`
answers `None` for it — so the deps are read with `depend()` (which unions over the members)
and re-applied with `with_deps` (which gives every member the union), the shape a tuple's dep
lists already have.  Only the `?` reaches that site: a declared `(S, integer)` is `is_equal`
to the literal's type, since `is_equal` collapses deps, so it is never converted and keeps its
backing by not taking the path at all.  Guard
`a-tuple-member-declared-nullable-releases-once.loft`, which carries the dense and inferred
CONTROLS for that reason.  Measured on the control, BOTH members of a two-nullable-member
tuple released twice — not one twice and one once — which is what says the backing was lost
for the whole tuple rather than paired to the wrong work-ref.

⚠ **A tuple release bug that is NOT this entry, and the direction is how to tell.**  Every
shape here releases TWICE; loft#1511 released NEVER — a call result placed directly in a tuple
literal member (`u = (mk(1), 9)`) ran no hook at all, while the same call bound to a local
first, or placed in a struct FIELD or a vector ELEMENT, releases once.  The store IS freed, so
nothing warned.  `Parser::tuple_member_owned_copy` takes its `_ => return None` arm for a call
source, which is right for the aliasing question it was written for — `(T-Cons)` holds, a
`vector` member built from a call reads its own length — and leaves the OWNERSHIP half
unanswered: the free analysis found an owner for that record and the drop analysis did not.
**CLOSED 2026-09-10**: the element free (`scopes::tuple_owned_elem_frees`) concluded
ownership from the member type's empty dep list and released the record with a bare
`OpFreeRef` — the hook was owed exactly there.  The scan now records which elements a
tuple-literal RHS minted by their own call (`Scopes::tuple_call_mint`, path-sensitive with
`owned_refs`'s intersect-merge; a call is a mint when its return delivers through the hidden
buffer minted for that slot, or adopts a fresh store — a nullable return, which has no
buffer), and the element free runs the type's cascade first, then writes the SENTINEL into
the call's buffer: the buffer's own scope-end drop+free otherwise claims the same record on a
callee that delivers into it, and the sentinel is also what makes a loop mint per pass
instead of rebuilding the freed slot.  Guard
`tests/scripts/1511-a-call-result-in-a-tuple-member-releases-once.loft`, 13 cells, hook
sequences plus the five sibling-container controls and the return path unchanged.
The same literal in ARGUMENT position (`show((mk(1), 9))`, loft#1512) binds no variable, so
no element-free site exists and the record leaked WHOLE — no hook, no free, both backends.
**CLOSED 2026-09-10**, one step earlier than the element free: `scan_args` lifts each
call-minted member of a tuple-literal argument into a `__lift_N` (the same lift a bare call
argument gets, same `inline_struct_return` gate), which reduces the tuple to the
local-member spelling that always released once — no value→type derivation needed, because
the lift's type is the CALLEE's return type.  Guard
`tests/scripts/1512-a-tuple-argument-call-member-releases-once.loft`; its leak channel is
armed by `tests/store_lifetime_1512_1513.rs`.
⚠ And the entry's own guard carried a second finding in its CONTROL: `literal_member`
(`u = (S { h: H { id: 21 } }, 9)`) asserted the count and passed while the hook read STALE
bytes — the element free was BARE and the record's hook ran through the construction
work-ref's scope-end cascade, after the free.  A literal member is the same mint as a call
member (the record delivered through `__ref_p2_N`), so `tuple_call_mints` now records it via
`construction_work_ref` and the element free runs the cascade live, then disarms the
work-ref.  The poison face of that cell is armed with the D-heap-5 guard's harness below.

⚠ **A fifth shape was here and did not belong to this entry.**  `h = Holder { t: (s, 5) };
u = h.t` released the resource NEVER — the OPPOSITE fault, which is why it is worth stating
that an entry saying "releases twice" cannot cover it and that a closing guard must assert the
release COUNT rather than its absence.  Isolating it showed the copy was not involved at all:
`h = Holder { t: (s, 5) }` on its own, with nothing copied out of it, already leaked.  It was
`D-heap-2` below — a cascade that never walked a tuple FIELD — and closing that closed this
cell.  The lesson is the scoping one: the cell was filed under the copy question because the
repro that found it contained a copy, and one probe with the copy removed said otherwise.

**A shape scoped wider than it measures**, and the control says so: a declared but NOT
nullable `t: (S, integer) = (s, 5); u = t` releases ONCE (correct), so the axis is the `?` and
not the author's annotation.  The nesting shape carried the same kind of over-wide scope and
is corrected above — its supposed control was a one-leaf tuple, which is quiet for a reason
that does not generalise.

⚠ **And the LOOP-VARIABLE shape is not a tuple question either** — measured 2026-09-10, the
same double release comes off a `match` payload binding with no tuple anywhere
(`match w { WH{s} => { u = s; } }`).  What the two share is that the copy's SOURCE is a VIEW
of a container's member: `(B-Copy)` makes `u = e` a copy, `(H-Drop)` says the copy owns and the
source stops dropping, and the source's owner is a CONTAINER whose cascade releases every
element it holds — there is no per-element suppression to reach for.  A direct projection is
quiet because it does not copy at all (`u = v[0]` and `u = h.s` are `(B-View-Depth)` and
`(B-View)` views, measured at one release).  It is NOT the member-pairing cure the other
shapes want.

⚠ **Asked of the rules 2026-09-10, and one of the two branches is closed.**  The reading that
`(B-Copy)` owes a VIEW here does not survive: `(B-View-Base)` makes a PROJECTION off a
borrowed base a view, and `u = e` is a bind of a VARIABLE, not a projection — and loft#1361
decided the whole-tuple bind COPIES on purpose, which is the same call in the same place.  So
the copy is right and `(H-Drop)`'s own clause is what applies: *the copy owns, the source stops
dropping* — where the source is a container's element, which means the CASCADE must skip that
element.  There is no per-element suppression to skip it with, and building one is the hard
part rather than an oversight: which element was copied out is a RUNTIME fact, so a static
approximation is wrong in both directions (a suppression that fires too often loses a release,
one that fires too rarely keeps this defect).  What is left for the design call is therefore
narrow and stated: either the cascade learns a per-element mark, or this shape is `(H-Drop)`'s
`warning[double-move]` clause and the defect is a missing warning.

**One cause, read off the IR.**  The hand-off is recognised by resolving a copy's tuple-MEMBER
source to the work-ref backing it, and the tuple's TYPE is where that pairing lives.  The
shapes still open have no pairing this frame can read.  A PARAMETER's deps are the CALLER's,
in another dep space, so its member's backing is not a variable of this function at all — and
the rule does not want one: `(H-Drop)`'s closing clause suppresses the CALLEE's copy, which is
what `copy_moves_drop_from` already does for a plain struct parameter (`fn f(p: S) { u = p; }`
releases once, measured) and what the tuple spelling has yet to reach.  The `text` neighbour
has a list that counts one dep more than the walk counts leaves.  So
`scopes::tuple_member_backing` DECLINES rather than naming a work-ref it guessed — declining
costs the hand-off and leaves the pre-loft#1361 double release, where guessing would suppress
the release of whatever local wore that number.

**No longer branch-internal.**  This entry landed in the SAME commit as loft#1361 (`2808e1833`,
2026-09-06), so `main` has carried the copy — and therefore every shape here — since that merge;
re-measured there 2026-09-10 on both backends.  The claim that none of them reproduces on `main`
was true only for the hours before that commit merged, which is how a "branch-internal" note
goes stale: it is a statement about two trees, and one of them moved.  Per the bug policy that keeps them here rather than in the tracker.

**Closes when** the five shapes above read exactly one release on both backends and the
guard's `@falsified-at` line covers them, scoring the COUNT rather than its absence.  The
ESCAPING copies want a caller-side fact — a signature that says the return shares the
parameter's resource, or a cascade that skips one field.  The `??`-guarded return wants the
per-path answer `ownership_cfg.rs` already carries the machinery for.  The bound projection,
the loop variable and the REASSIGNED variable are one question — the resolver is given a
variable where the answer belongs to an ASSIGNMENT — and want the design call above answered
before a cure is chosen for any of them.


### D-heap-7 — CLOSED (2026-09-15): the drop gate's first sweep — eight more families release wrongly, and silently

**Reclassified 2026-09-15 by the copy-lease rules (@PLN163), which judge a copy by its own line.**
Every family below that writes a copy of an existing value — a parameter copied out of its callee
(family 2), a projection copied into a container (family 4, whose warning is the refusal's first
form and not its closure), a join or `??` over a local placed anywhere (family 1), family 7's
residual — is refused by the rules and belongs to `D-heap-8`.  One cell stayed open here:
`return a ?? b` over two locals (`q_present_local_var_ret`), which `(H-Move)` makes a move because
a function's own variables end with it.  It lost a release on the path that returns `a`: `b` was
never released.  The families below are the record of the sweep.

**CLOSED 2026-09-15 — and the cell was one of three mechanisms, all silent, identical on both
backends.**  The matrix varied the return's SHAPE (a `??`, an `if`, a `match`, a nested `if`, an
arm that is a call, an arm that reads the local, separate `return` statements), which local is
handed out, and whether the returned local is reassigned first — each cell's trace computed by hand
before it ran.  The filed scope was the first row of the first mechanism:

1. **A join over locals released the local it did not return with no hook.**  Its sources are kept
   out of the scope-exit sweep as return sources, and the join leg frees each at run time with
   `OpFreeRefIfDistinct(src, __ret_N)`, which ran no hook — `return if c { a } else { b }`, a
   `match`, a nested `if` and `return if c { a } else { mk() }` lost it the same way.  The hook now
   rides that leg, guarded by `OpDistinctStore` on the same pair, so the release and the free cannot
   disagree about the path, and it runs after the value is computed.  (Moving the hooks INTO the
   arms, as loft#1515 does for a copied join, was built first and measured wrong: it released the
   local before a call arm that reads it — `mk(a.id + 2)` saw a released `a`.)
2. **A later `return b` released `b` twice.**  When an earlier `return a` had renamed `a` onto the
   return buffer, `return b` copies `b` onto it, and `b` kept its own hook besides the one the copy
   carries to the caller.  `return_copies_whole_local` is `return_copy_out`'s whole-record twin: the
   copy owns, the source's hook is skipped at that return.
3. **The promoted local's own record was displaced with no release.**  The renamed local is an
   ARGUMENT by slot, and `displaced_drop` refused every argument, so `a = mk(1); a = mk(5); return
   a` — in a branch or a loop too — never released the first, and the sibling return's re-mint of
   the buffer (`OpDatabase`) discarded `a`'s record the same way.  The refusal now asks
   `is_promoted_ret_buffer`: this frame minted what that slot holds.  The re-mint takes the same
   null-safe snapshot, so a path that never assigned the local releases nothing — measured with the
   result reassigned into a reused caller buffer, where a release would be the caller's record twice.

The drop gate moved exactly one cell (`q_present_local_var_ret` LOST → clean, both backends), and
nothing else among its 276.  Guard:
`tests/scripts/a-return-that-hands-out-one-local-releases-the-others-once.loft` (29 assertions,
three of them controls that were right before and must stay right).

`(H-Drop)` releases a resource once, at its owner's death, and moves the release with a copy.
`tests/ownership_drop_gate.rs` generates 223 cells and scores the release itself (TESTING.md
§ The drop gate).  On `origin/main` `ec4a95226` its baselines pin **94** cells that do not
hold, the same on both backends except where noted below, and **none carries a diagnostic** —
`warning[double-move]` fires on none of them.  The cell names below are the baselines' names,
so a family is re-measured by one run of the gate.

D-heap-1's five shapes are among them (`p_o1`–`p_o5`).  The rest fall outside that entry:

1. **A `??` result as a copy source** — `x = a ?? d` (`q_*`, `c_coalesce_*`).  Not one
   mechanism: a tuple-member or branch-arm destination is clean in all 18 rows of the coalesce
   family, a struct-field destination releases twice in all 18 — including the absent path,
   where the default is a fresh call — and a local destination releases twice on the present
   path and LOSES a fresh-call default on the absent one.  The same join spelled as an `if`
   (`if a != null { a } else { mk(2) }` — `a ?? d` lowers to exactly that) releases once on
   both paths and both backends, so the `if` spelling's IR is the working form, and the two
   mechanisms are the differences from it:
   - ✓ **The present path — CLOSED 2026-09-14.**  The `??` arm lifts `__lift_1 = a` and records
     the per-path hand-off, and the statement scan then RETIRED that hand-off as if it were an
     unconditional reassignment: the retirement reads scope depth as "certain to run", and a
     `??` arm is a bare `Insert` that opens no scope, so the Set arrived at the temp's own
     depth.  (Measured with a probe at the retirement: it fired on both `??` paths and never on
     the `if` spelling, whose arms are blocks.)  The retirement now skips `arm_lift_temps`,
     the per-path fact the hand-off was recorded under.  A `match` with bare arms and a boolean
     `if` never reached the misfire.
   - **The absent path — OPEN.**  `x` holds the default's record while its deps name only
     `__lift_1`, so nothing releases it.  The `??` builder types its result from the subject's
     DECLARED type, which carries no dep on the subject, so it decides the join OWNS and gives a
     call default no owner — and the scopes lift then makes `x` a borrow of the lift temp.  The
     `if` spelling's join type carries `a`, so its call arm is given an owner
     (`own_joined_call_arms`) before the lift runs.  The missing piece is that TYPE fact, not a
     second owner decision: giving the default an owner under the owned type would release it
     twice wherever the lift declines.

     **Where the fact is lost, and why restoring it alone is not landable (measured
     2026-09-14).**  The `@FR-B-Copy` arm of a variable read in `parser/objects.rs` decides
     `x = a …` is a whole-value bind while reading the NAME `a`, makes `x` independent of `a`
     and returns `a`'s type without its dep.  Its lookahead excludes `.`, `[` and `#` — none of
     them a whole-value bind — but not `??`, and `x = a ?? d` is the join, not the bind.
     Adding `|| self.lexer.peek_token("??")` to that lookahead makes the `??` lower exactly as
     the `if` spelling does (`x ["__lift_1", "a"]`, the default arm a `#join-arm-owner`), and
     closes the LOST on both backends — `coal_absent`, the loop's `p_l2` and both
     `q_default_*_call_local`.  It was measured and taken back out, because by `(H-Drop)`'s ⚠
     a change that converts one failure into another is not an improvement, and this one
     converts two, on both backends: `v += [a ?? mk()]` with `a` absent goes from right (by
     accident) to releasing twice, and `x = mk(); x = a ?? d` goes from a loud native compile
     refusal to a silent LOST.  (The postfix `a?` does not share the hole: its default is
     built into a work-ref this frame already releases, both backends.)
   - **Why it waits: the same join's other consumers are broken for BOTH spellings.**  The
     `if` spelling — the "working" form above — releases twice when its join is copied into a
     container (`v += [if a != null { a } else { mk(2) }]`, the gate's `p_j3`/`p_j4`) and loses
     the record a local's reassignment displaces (`x = mk(); x = if … { a } else { … }`,
     `p_j1`/`p_j2`).  A join bound to a local first does not help: `t = a ?? mk(2); v += [t]`
     releases twice in both spellings, while `t = mk(3); v += [t]` releases once.  The
     join-bound local is a CARRIER — its type borrows the per-path temps, so the element
     hand-off stops `t`, which never released anything, and the per-path source keeps its
     release.  That is D-heap-1's open question (the resolver is given a VARIABLE where the
     answer belongs to an ASSIGNMENT); the type fact above can land once that is answered.
2. **A parameter copied out of its callee** — `p` or `p.h` into a field, an enum payload, a
   vector or the return (`c_param_*`, `c_pfield_*`): twice, and the callee's release runs BEFORE
   the caller's own later read.  The rule gives the answer — a copy off a PARAMETER moves
   nothing, the caller owns — so the callee's copy must not release.  D-heap-1's first shape is
   the tuple-member spelling of this family, and the cure it names (a caller-side fact) is the
   same one.
   - ✓ **A parameter copy the owner witness names — CLOSED 2026-09-15.**  `x = p; x = s.h`
     released the caller's resource inside the callee, and the caller released it again: twice,
     on both backends, `LOFT_POISON` clean.  Not the mechanism of the rows above, read off the IR.
     A local that owns on one assignment and views on another carries the loft#1336 owner
     witness, and a whole-value copy is a store of the local's own (`@FR-B-Copy`), so the
     parameter copy pointed the witness at its record (`OpRefAlias`).  The witness then released
     that store WITH the type's hook.  The witness was right about the store, which is the
     local's to free, and wrong about the release, which is the caller's.  Measured at all four
     places a witness hooks:
     - a later field or element view;
     - an in-place rebuild by a literal;
     - scope exit;
     - per iteration inside a loop.

     Each held with the copy unconditional, after a record of the local's own, or in a branch
     arm, and for a record that holds the resource.  A null or a fresh call displacing the copy
     was right.

     The cure carries the fact the p_h6 close introduced: a witnessed local assigned a copy off
     a parameter gets the `__hoff_` flag, the copy sets it, and every other assignment retires it.
     The in-place rebuild, which is no `Set`, retires it too.  The witness frees the store either
     way and skips the hook while the flag is set, one home for all four sites
     (`Scopes::witness_hook`).  A release that runs where the local stops owning is emitted
     before the statement's flag writes, so its hook reads the flag of the record it releases.
     Guard: `tests/scripts/a-parameter-copy-the-owner-witness-names-is-released-by-the-caller-only.loft`.
   - ✓ **A parameter copy handed on — CLOSED 2026-09-15 for a carrier that holds the caller's
     record on every path.**  `x = p; y = x` released the caller's resource at `y`'s scope exit
     and again in the caller, which then read a record already released: twice, both backends,
     `LOFT_POISON` clean.  The same happened through a witnessed `y` later given a view.  The one
     decider every copy asks (`copy_moves_drop_from`) read `x` as an ordinary local, so it moved
     the release to `y`.

     A local is now marked before the scan (`Function::holds_caller_record`) when:
     - every assignment that binds a store is a copy off a parameter or off another marked local;
     - no write place is rooted at it;
     - it is never passed bare to a call.

     The decider answers a marked local as it answers a parameter.  Measured beside it, and
     unchanged:
     - a local written through after the copy (`x = p; x.h = mk(); y = x`) keeps its own release,
       because its record then holds a resource the frame made;
     - `x = p; return x` releases twice exactly as `return p` does — the caller-side shape above;
     - a destination rebound to a fresh record, a carrier that kept its own record, and all-local
       copies are clean.

     Guard:
     `tests/scripts/a-parameter-copy-handed-on-to-another-local-is-released-by-the-caller-only.loft`.
   - ✓ **Handed on from a branch arm — CLOSED 2026-09-15.**  `x = mk(); if c { x = p; } y = x` with
     `c` true released the caller's resource through `y`, then in the caller: twice, both backends,
     `LOFT_POISON` clean.  The carrier holds the caller's record on one path only, so the static
     mark above cannot apply.  The per-path flag that records the answer (loft#1515) belonged to the
     source alone, so the copy took a release on every path.  The same was measured with the
     hand-on inside another arm, into a witnessed local given a view, and through a chain
     (`z = y`).  It also happened with NO parameter: `x = mk(); if c { z = x; } y = x` released the
     record through `z` and again through `y`.

     A copy of a flagged local now inherits the flag at the copy.  The inherited value is read
     before the statement writes the source's own flag, so the destination releases on exactly the
     paths the source would have.  A fixpoint before the scan mints the flag for every such
     destination.  Unchanged: a carrier that kept its own record, a carrier copied from a local in
     the arm, and the arm not taken.  Guard:
     `tests/scripts/a-copy-of-a-local-that-may-not-own-its-record-takes-the-per-path-answer.loft`.
   - ✓ **Placed in a structure — CLOSED 2026-09-15 as a WARNING, by § Standalone right,
     encapsulated warned.**  `p`, or a member of it, copied into a field, an enum payload or a
     vector (`c_param_{field,enum,push,veclit}`, `c_pfield_*`, D-heap-1's `p_o5`), and a member
     returned (`return p.h`, `return p.0`, `c_pfield_ret`): `warning[double-move]` at the copy or
     at the `return`, and the releases are unchanged.  The family-4 lint already kept a member
     copy pending on its root, and left a parameter root out only while this family was
     undecided.  A parameter is now a root, and so is a whole parameter placed in a container.  A
     copy inside an arm or a loop reports once; one parameter placed in two containers reports
     twice.

     Outside the warning:
     - a WHOLE parameter returned or bound (`c_param_ret`, `q_present_param_*_ret`) — a plain
       value, whose release is still to be made right;
     - a member overwritten after the copy, which retires the copy;
     - a miss: the parameter handed on to a call after the copy;
     - a miss: a copy or a view of the parameter whose member is returned (`u = p; return u.0`,
       `e = p.h; return e`).

     A member returned as the body's tail without `return` reports like `return p.h`: after
     `scopes::check` the tail is a `return`.  The misses are pinned in `tests/double_move.rs`, and
     the corpus reaches the shape once:
     `1506b`'s `param_member` control now carries the warning.
3. **A hand-off that suppressed a release it does not belong to** — filed from
   `x = mk(K); x = p` (`c_param_reassign`: the displaced record never released, and the parameter's
   copy released inside the callee), and wider than filed: `b = mk(1); b = mk(2); y = b` lost
   `mk(1)` with no parameter anywhere.  The set of variables that stop dropping
   (`drop_transferred`) was seeded from the WHOLE body before the scan, so a hand-off made by a
   LATER statement was already in force at an EARLIER reassignment; and each statement re-armed
   its own hand-off BEFORE its scan, so `x = p` hid `mk(K)` behind the parameter copy's fact and
   then retired that fact and dropped the copy.  `@FR-O-Latest` is the rule broken — a hand-off
   belongs to the assignment it follows.
   - ✓ **CLOSED 2026-09-14 for sequential code and a taken branch.**  The set starts empty and each
     statement arms its hand-offs after it is scanned.  Gate cells `p_h1`–`p_h5` and
     `c_param_reassign`, both backends.
   - ✓ **A loop — CLOSED 2026-09-15.**  `for … { x = mk(); x = p; }` released none of the displaced
     records (`p_h7`).  A loop body arms its hand-offs before its statements, because on the next
     iteration an earlier statement displaces what a later one handed off, and a static fact
     cannot say whether that record is the caller's.  So a copy off a parameter anywhere in the
     body stopped `x` for the whole function.  Measured wider than the gate's cell:
     `x = mk(); for … { x = p; x = mk(); }` also lost the last fresh record at scope exit, and a
     body holding only the copy lost the record it first displaced.

     The answer is per ITERATION, and the destination-keyed runtime flag above gives exactly that.
     A copy that stops its destination, written in a loop body, is now a per-path hand-off like an
     arm copy.  The early seed skips it, the copy sets the flag, every other assignment retires it,
     and each displaced release and the scope-exit drop read it when they run.  A copy that moves
     its source's release keeps the early seed.  The fresh-records-only loop is unchanged.  Guard:
     `tests/scripts/a-parameter-copy-in-a-loop-body-keeps-the-release-of-every-record-it-displaces.loft`.
   - ✓ **The branch not taken — CLOSED 2026-09-15.**  `x = mk(); if c { x = p; }` with `c` false
     lost `mk()` (`p_h6`): the copy off the parameter stopped `x` on EVERY path, so on the path
     that never copied nothing released `x`'s own record.  Wider than filed, measured on both
     backends and under `LOFT_POISON`:
     - the else arm: `if c { x = mk(2) } else { x = p }` lost `mk(2)` on the then path;
     - every path copying a parameter: `if c { x = p } else { x = q }` lost the record the else
       arm displaced;
     - a nested branch, a `match` arm, and a record that holds the resource;
     - a later unconditional rebind, which lost the displaced record on the path not taken;
     - the written-out join `x = if c { a } else { p }` on `c` true, which lost `x`'s copy of `a`;
     - a loop whose body copies the parameter in one arm, which lost every displaced record.

     Two facts had to become per path.  loft#1515's flag was given only to a copy that stops its
     SOURCE; a copy that stops its DESTINATION now gets the same flag, keyed on the side it stops
     (`scopes::per_path_stops`).  That alone left the rebind and the loop: the ownership memo the
     displaced release reads (`owned_refs`) records the parameter copy as a VIEW, so the join of
     an arm that copied with one that did not read "not owned", and inside a loop the
     own-assignment test read false.  Measured with a probe on the memo before the second change.
     A copy the flag guards now reads owned in both places, and the flag decides the release
     (`copy_flagged_on_target`).  Guard:
     `tests/scripts/a-copy-off-a-parameter-in-a-branch-arm-keeps-the-release-of-the-path-not-taken.loft`.
4. ✓ **A projection copied into a container — CLOSED 2026-09-15 as a WARNING, by the owner's
   call.**  `s.h`, `vs[0]` or `tt.0` into a field, an enum payload, a vector or a tuple member
   (`c_field_*`, `c_elem_*`, `c_tuple_*`): twice.  The same projection bound to a LOCAL is a
   `(B-View)` view and releases once.  Placed in a container it is a copy, and the source's own
   container still cascades the member.  This was the question D-heap-1 left as a design call: a
   per-element mark, or `(H-Drop)`'s `warning[double-move]` clause.  It was decided for the
   WARNING.  There is no runtime dependency and no extra bookkeeping inside structures, because
   `OpDrop` is meant for a clear lifetime of variable use and works there.  So a member shared by
   two containers is the author's to restructure, and the release behaviour stays as it is: the
   gate's cells stay pinned as the shape the warning names.

   **Corrected 2026-09-15: the copy has to carry a release.**  The first version warned whenever
   the DESTINATION container owned a droppable somewhere.  So a plain member copied into such a
   container (`Mix { h: mk(), n: s.n }`, and the same through `ns[0]` and `t.0`) was reported
   while every record released once.  The lint now reads the copied record's type off the copy
   (`keys::COPY_TP_MASK`) and requires a cascade or a hook.

   **One stage on every path.**  The post-scope lints read facts only `scopes::check` produces:
   the caller-record mark, the releases it places, the copies it materialises.  So `loft <file>`
   runs the scope pass before them exactly as `loft test` does, and one program answers the same
   under both.  That IR carries the scope pass's own work, which the lints read as follows:
   - a RELEASE (a free, or a drop hook or cascade) is neither a write nor a read
     (`use_analysis::releases_first_arg`).  Read as a write, the root's own drop retired this
     warning's pending copy; read as a read, a free hid a lost write;
   - the allocation `scopes::value_struct_copy` makes for its copy (its source temp is
     `__vs_src_<N>`, named after the destination) defines the copy and does not read it;
   - an inline call argument arrives as the `__lift_N` the scope pass bound to it.

   Measured over every `tests/scripts` program on both paths: the program path's `double-move`
   and `lost-write` findings are unchanged, and `loft test` now reports the 14 lost writes it
   never did.  `x = p; t = x; u = x` no longer warns.

   A member copy is reported only when something besides the container releases the member:
   - a caller-owned root: a parameter, or a local that holds the caller's record;
   - the root promoted to the caller's return buffer;
   - a drop the scope pass emitted for the root;
   - for a view, its base by the same test.

   `x = p; x.h = mk(); c = Hold { h: x.h }` is therefore silent, and it releases each record once:
   a copy off a parameter stops its destination, so the container is the member's only releaser
   (`g6`).

   The lint keeps such a copy pending on its ROOT and reports it where the root's release is
   certain:
   - its scope end;
   - a rebind of a root that owns its record, which releases the record it displaces, member
     included;
   - a `return`, including a root promoted to the caller's return buffer.

   It stays silent where the member is overwritten first (an overwritten member is not released,
   `(H-Drop-Not)`), even on one path only, and where the rebind's value reads the root, because a
   warning gates.  A FIELD placed in a tuple literal is not a copy and releases once.  Measured
   outside it, released twice and pinned in `tests/double_move.rs` as its boundary:
   - a parameter's member (family 2: the caller owns it);
   - a loop variable and a `match` payload binding, each a plain variable in the IR rather than a
     projection;
   - a tuple MEMBER copied into a tuple, which lands in the new tuple's backing work-ref and not
     in a container place.
5. **An element appended to another vector** — `v += [vs[0]]` and `[vs[0]]` (`c_elem_push`,
   `c_elem_veclit`): the interpreter PANICKED reading the element's id as a record number
   (`Store access out of bounds: rec=39001`), and native releases twice.  The double release is
   family 4 (a projection copied into a container) and is what remains.
   - ✓ **The interpreter's crash — CLOSED 2026-09-14, and it was the loud face of a silent
     defect.**  With a small element id the same program exited 0 and ran the hook about ninety
     times over garbage records (`p_e1`, `p_e2`).  Two deciders disagreed about where a droppable
     vector's declaration is.  The scan saw `parse_code`'s body-0 `__vdb = null`, treated the
     first build as a rebuild and placed a null-safe release snapshot above it; Plan-57's
     last-use reclaim then moved that null-init down to the build, below the snapshot, so the
     snapshot read a slot whose declaration had not run, and the interpreter handed it whatever
     an earlier variable left there.  (`LASTUSE_RECLAIM_OFF=1` removed it; confinement's
     `LOFT_NO_CONF_RECOVER=1` did not.)
   - ✓ **Found by the same cure, and closed with it: reclaim released the wrong store.**  Vectors
     of droppable elements built one after another, each dead before the next (`p_r1`): reclaim
     frees a dead store early and removes its scope-exit FREE, but not its scope-exit DROP.  At
     scope exit the slot names a store a later build reused, so the interpreter released the last
     vector's elements three times and native lost the earlier vectors' releases.  Measured
     identical before any change today — the interpreter's one correct release of the first
     vector had come only from the stray snapshot above.
   - **The cure is one exclusion in the reclaim plan**, which `lastuse_reclaim` and its Phase-4
     guard both read: a store whose record type has a drop cascade is not eligible
     (`scopes::drop_bearing_stores`).  Reclaim moves a store's DEATH and a drop belongs to that
     death, so such a store keeps its body-0 null-init and releases through its hook and its free
     together.  Its only cost is that these stores are not reclaimed early; the corpus-wide
     emission diff differs in the seven files that declare a hook and nowhere else.
   - **Measured and not landed:** placing a relocated null-init before the first op that READS
     its store.  It removed the crash, but `reads_var` counts a free on an early-return path as
     a read, so it pulled null-inits back to body 0 in drop-free code
     (`177-reclaim-early-return.loft`, `872-vector-return-into-struct-literal-field.loft`) and
     undid their reclaim — and it left the released-wrong-store defect in place.
6. ✓ **A construction delivered through a branch arm — CLOSED 2026-09-14.**  Filed as a call
   result's field in an arm (`c_callproj_arm`), and wider than filed: ANY construction handed to
   a local through a join arm released twice — a plain struct literal (`x: H = if c { H {…} }
   else { mk() }`, `c_literal_arm`), both arms constructions, a `match`, a loop.  A rebind from
   such a join (`x: H = mk(); x = if … { mk_s().h } …`) was a use-after-free: under
   `LOFT_POISON=1` the second hook read the poison pattern, both backends.  A call arm was always
   clean — its result is adopted through its own buffer — and so was every untaken arm (`c_*_arm0`).
   - **One predicate, two deciders.**  `construction_work_ref` answers the work-ref a
     construction block delivers, and `None` for a join.  Both halves of the single-construction
     hand-off read it: `drop_handoff_node` stops the work-ref's drop, and `scan_set` disarms the
     work-ref with the sentinel (`D-heap-5`, loft#1513).  For a join both were skipped together,
     so the arm's work-ref kept its name and its drop beside the binding's.
   - **The cure** (`construction_work_refs`): a join lists every arm's construction, and both
     deciders read the list.  Static per path — on the path that ran the binding adopts that
     arm's store; on the others that arm's construction never ran, so its work-ref holds nothing
     and the disarm is a no-op.  `is_null_sentinel_detach` keys only on the work-ref's name and
     the bare sentinel, so each arm's disarm is read as a disarm, not a displacement.
   - ⚠ **The join disarm is decided AFTER the arm lift.**  Decided before it, `x = a ?? H {…}` lost
     its default's release: the join parses as owned, the literal arm was disarmed, and the lift
     then made `x` a borrow of `a`'s temp, so nothing released the literal.  After the lift the
     disarm reads the same ownership fact the drop hand-off reads.  Gate cells `p_g1`–`p_g5`,
     `c_literal_arm`, `c_callproj_arm`; the corpus-wide emission diff is identical in all 1495
     files, so no existing program used the shape.
7. ✓ **A reassignment from a branch join — CLOSED 2026-09-14.**  `x = mk(9); x = if c { a } else
   { b }` with `a` and `b` owned locals lost the displaced `mk(9)` and the untaken `b` on `c` true,
   and `a` on `c` false (`p_s1`, `p_s2`), both backends, plain and under `LOFT_POISON=1`; the same
   join written by the author as a statement, `if c { x = a } else { x = b }`, was clean (`p_s3`).
   A reassignment from a value branch is lowered to exactly that statement form
   (`scopes::sink_set_into_arms`, `@FR-O-Complete` / `@FR-B-Copy`), but inside the scan, and the
   written-out arms lacked three things the author's arms have:
   - **the per-path flags** — loft#1515's `__hoff_a` / `__hoff_b` were minted from the pre-scan
     IR, where only `Set(x, <branch>)` exists, so a static hand-off stopped both sources and the
     untaken one was never released.  The site that writes the arms out now registers its own
     pairs and mints their flags before the first arm is scanned: it is the one site that knows
     those copies exist.  Family 8's cure is what made that safe — before it, a flag replaced every
     static stop of its source.
   - **the first arm's displaced release** — the parser typed the binding with the join's deps
     (`[a, b]`), and the scan cleared them at the first arm's own copy, AFTER that arm asked whether
     the binding owns the record it displaces (`proxy_says_owned`), so only the second arm took a
     snapshot (`LOFT_LOG=type_timeline:x` shows the two writes).  The same site now strips the arm
     tails' deps first, through the predicate the bind itself uses (`var_copy_owns`).
   - **a block per arm** — the `??` and scalar-`match` lowerings write their arms without one, so a
     written-out arm's snapshot temp was registered at the ENCLOSING scope and released at that
     scope's exit on every path, reading an uninitialised slot where the arm never ran: an
     interpreter crash under poison, a native compile refusal (`E0425: cannot find value
     var___disp_2`, `c_coalesce_reassign`), and on the plain interpreter whatever the slot held — a
     second release of `a` on the `??` present path.  This entry first attributed that release to
     the missing flag; that was inferred, not measured, and wrong.  Such an arm is now given a block
     (`tests/scripts/a-reassignment-from-a-bare-arm-releases-its-snapshot-in-that-arm.loft`).
     Landed alone, that change CONVERTED one failure rather than fixing it: `x = a ?? b` with a
     local default and `a` present went from releasing `a` twice to losing `b` (`p_s6`) — `b`'s old
     release had been the uninitialised slot — and the flags above closed it.
   - Measured: every local-arm spelling — an `if`, an `if` chain, a scalar `match`, an `if` nested
     in a `match` arm, a nullable binding, inside a loop, with the arm locals read afterwards —
     releases each resource once, on both backends, plain and under poison.  The gate retires
     `p_s1`, `p_s2`, `p_s6` and `c_coalesce_reassign`.  The block change's emission diff differs in
     28 files by the added block alone (the interpreter's bytecode identical in all 28); the flags'
     differs in one, the new guard.
   - ✓ **Beside it, CLOSED 2026-09-14: a join with an owning CALL arm.**  `x = mk(9); x = if c { a }
     else { mk(2) }` kept the value form — the parser's owner for the call arm is a compiler temp,
     which the write-out declined — so the binding was typed as a view of `a` for its whole life: its
     earlier owned record, and any record assigned after the join, were freed with no hook
     (`p_j1`/`p_j2`), and the binding ALIASED `a` on the path that took it, a `(B-Copy)` violation
     recorded as `binding.md` D-bind-33.  Such an arm is now written out as its call beside local
     arms, and the gate retires `p_j1` and `p_j2`.
   - **The mechanism since (2026-09-14):** the first scan records each reassignment it writes out;
     that reassignment is rewritten in the original code and the function is scanned again, so every
     analysis before the scan reads the per-arm form (`binding.md` D-bind-34).  The fixes at the
     write-out site above stay: a reassignment nested inside another's arm is reached only through
     the first scan's own copy, is not recorded, and is still written out there.
   - **Still open.**  An arm that copies a PARAMETER (`x = if c { a } else { p }`) stops the
     DESTINATION with no flag, so on the path that took `a` its copy is never released — family 3's
     branch-not-taken residual, reached through the written-out form.
8. ✓ **A per-path hand-off hid a later hand-off of the same source — CLOSED 2026-09-14.**
   loft#1515 guards a source's release on a flag that records whether its per-path copy ran, and
   the flag REPLACED the static suppression rather than joining it.  So a later unconditional
   hand-off was ignored: `if c { x = a } else { x = b }; y = a` on `c` false released `a`'s
   resource twice, by `a` and by `y` (`p_s4`), both backends.  The other reader of the same set
   erred the other way: a later `a = mk(3)` on `c` false never released the record it displaces
   (`p_s5`), because `displaced_drop` read only the static set, where the arm's copy had put `a`.
   One set carried two facts — "stopped on every path" and "stopped on the path where the copy
   ran" — and neither reader could take them apart.
   - **The cure keeps the two facts apart.**  A copy in a branch arm never enters
     `drop_transferred`; it is the flag's fact.  Both readers read both: the scope-end release
     returns nothing for a source stopped on every path and otherwise guards on the flag, and a
     displaced release takes its snapshot only where the flag says the copy did not run.  The
     snapshot is guarded rather than the release, because the statement that rebinds the source
     resets its flag before that release runs.
   - Gate cells `p_s4` and `p_s5`, both backends, plain and under poison.  The corpus-wide
     emission diff differs in one file of 1495, loft#1515's own guard: its rebind on the path
     where the copy did not run now releases the record it displaces — the guard asserts only the
     other path.
   - Family 7's cure mints the same flags for the lowered form, which is why this one went first:
     before it, the lowered form of `p_s4` was clean only because both its sources were stopped
     statically.

✓ The NATIVE-only refusal of `x = mk(K); x = a ?? mk(J)` (`c_coalesce_reassign`) — CLOSED
2026-09-14 with family 7's `??` spelling, above: the same misplaced snapshot temp, and the cell
now releases once on both backends.

⚠ **A double release is not only a handle closed twice.**  In one process, a cell that is clean
on its own lost its release when it ran after a doubling one — `t = (a ?? mk(4), 1)` after
family 1's local cell printed its mint and its read and never released.  Whatever the second
release does to the store table reaches later frames.  That is why the gate runs every cell in
a process of its own, and why a cell's verdict inside a batch is not a measurement of that cell.

**Closes when** every line of `tests/ownership_drop_gate.baseline` and its native twin is gone,
each retired in the commit of the fix that moved it — except family 4's, which the owner closed as
`warning[double-move]` and which stay pinned as the shape that warning names, within the boundary
written under family 4.  Closed so far: families 3, 5, 6, 7 and 8 whole, family 1's present path,
and family 2's three carrier shapes (a copy the owner witness names, a copy handed on, and a copy
handed on from a branch arm).  Still open, each with its answer in the rules: family 1's absent
path, which waits on the carrier question — a resolver given a VARIABLE where the answer belongs to
an ASSIGNMENT — and family 2's head, a parameter copied out into a container or the return, whose
answer needs a fact about the caller.  The shape recorded beside family 7, a reassignment from a
join with a CALL arm (`p_j1`/`p_j2`), is closed with `binding.md` D-bind-33.

### D-heap-3 — OPENED AND CLOSED (2026-09-10): a struct field projected off a CALL result releases twice

`(H-Drop)`'s responsibility clause and `(B-View)`'s materialisation clause together settle this
one completely, and in opposite directions — which is why it is worth writing out rather than
leaving as loft#1506's prose.

`(B-View)` makes `mk_dense().h` a VIEW: a projection whose type is a STRUCT is a view, and a
view whose base is disturbed materialises into a copy ("a plain view gets a **copy** and is told
so"; the C86 escape rule the implementation cites at `parser/operators.rs`). The base here is a
call TEMP whose store dies inside the statement, so **the copy is required by rule** — a cure
that aliased into the temp and deferred its death instead would contradict `(B-View)`.

`(H-Drop)` then says what must follow: *"RESPONSIBILITY moves with a copy … the copy owns, the
source stops dropping."* The source is the temp's FIELD, so the source must stop dropping THAT
resource — and only that one. Measured on both backends, identical:

- `x = mk_dense().h.id` — one record, **2** releases;
- `r = mk_dense().h` — same, **2**;
- `x = mk_two().h.id` where `struct Two { h: CfH, g: CfH }` — `h` twice, `g` **once**;
- `d = mk_dense(); x = d.h.id` — **1**, correct, and the reference answer a cure must land ON;
- a `?` on the same receiver adds an independent **+1** (loft#1506's second strength).

The multi-field cell is what constrains the fix: the cascade is doing NECESSARY work for `g` in
the same call in which it duplicates `h`.

**Three candidate cures are excluded by measurement, not by argument.**

1. **`drop_transferred` on the temp** (`scopes::mark_lift_handoff`, reached when
   `copy_hands_off` accepts the destination). Suppresses the WHOLE cascade, so `g` is never
   released: a leak in place of a double release.
2. **The `0x8000` move bit** on `OpCopyRecord`, which frees the source store inside the op. The
   store is the whole parent record, so `g` dies unreleased. Worse than (1).
3. **Adopt the BASE instead of copying the field** — attempted 2026-09-10 and REVERTED. The
   most promising of the four, because it needs no new mechanism: where the projection's base is
   a direct call, let the work-ref adopt the callee's fresh store and read the projection off it,
   so one record has one owner and its own cascade releases each resource once. It measured
   RIGHT on every count cell — `x = mk_dense().h.id` 2 -> 1, and the two-droppable-field record
   at one release each, which is what excluded (1) and (2). ⚠ And it reintroduced @PLN85: the
   curated suite failed `store_lifetime_890_889::call_field_cells_poison_{interpret,native}`, and
   `tests/scripts/889-collection-through-a-call-s-field.loft` read
   `a field of a field: -2401053088876216593, expected 7` under `LOFT_POISON=1` on BOTH backends.
   A deeper chain reads through the adopted store after something frees it, which is exactly the
   dangling read the copy exists to prevent — the count was right and the memory was not. **A
   cure here cannot be scored on release counts alone; the poison sweep is part of its
   boundary.**
4. **"Null the slot so the cascade finds nothing"** (the loft#1476 precedent). Does not reach:
   the generated cascade guards each field on the PARENT's `rec` — `if (self + off).rec != 0` on
   a DbRef derived from `self` — so the guard is the same expression for every field and zeroing
   a field's bytes changes none of it.

**What is missing is a code representation, not a decision.** `(H-Drop)` is per-RESOURCE, and the
three mechanisms the implementation has are per-VARIABLE (`drop_transferred`, `free_transferred`)
or destination-side (`copy_hands_off`). There is no way to say *"this FIELD of this record has
been copied out, so the cascade must not release it"* — and the cascade is a generated
whole-record function (`t_<LEN><Type>_OpDropAll`), so expressing it means either a cascade variant
that takes a skip-offset, derived from `cascade_fields` in its existing one home, or an emitted
per-field sequence at the site, which would restate that list.

⚠ **And the escape axis is not decidable where the copy is made.** `parser/operators.rs`'s
`inline ref copy` runs mid-expression, before it knows what consumes the result; the temp it
would have to name does not exist yet, being created later by `scopes::scan_args`. That is
loft#1496's shape — the answer lives at the point that has seen what FOLLOWS — and it is why a
condition written at the copy site is expected to be *nearly* right, whose failure direction is
not copying where `(B-View)` requires it: a stale read on both backends rather than a doubled
hook.

**Conformance when closed** must assert the COUNT in four cells with four different answers — the
escaping projection unchanged at 2, the non-escaping one at 1, the bound form still 1, and the
two-droppable-field record at one release EACH — because every wrong cure above passes a boundary
that omits one of them, and a cure that overshoots to zero is a leak reading as a fix.

**The dense family CLOSED (2026-09-10)** — the fifth candidate, the one the exclusions left:
the cascade got the skip. `synth_drop_cascades` generates `t_<LEN><T>_OpDropAllExcept(self,
skip)` beside the full cascade, derived from `cascade_fields` in its one home — each FIELD
release additionally guarded by `skip != off`, the own hook and collection fields
unconditional (a copy-out never takes those over). `scan_args` records the (lift, offset)
pairing where the materialising copy is made (`OpCopyRecord(OpGetField(__lift_N, off),
__ref_M)` — only a `__lift_` source, minted fresh per statement, because a pooled work-ref's
scope-end drop covers whatever record it holds LAST), and `scope_end_drop` — the one home for
the scope-end hook — emits the Except call. A type without the variant falls back to the full
cascade: the pre-transfer double release, never a leak or a faulting free.
`LOFT_NO_FIELD_HANDOFF=1` is the bisect switch. The BIND shape (`r = mk().h`) rides the same
mechanism: the projection now materialises TERMINALLY too (gated on the projected type owning
a droppable, so a plain struct keeps the alias lowering), and the terminal block is typed
WITHOUT deps — the binding ADOPTS the copy the way a struct literal's block is adopted, so
the existing construction hand-off and buffer pairing release it exactly once. Typing it as a
frame-dep view instead was built and measured: the construction hand-off gave the work-ref's
drop to a binding that never drops, and the count read ZERO — the overshoot-to-a-leak this
entry's own table predicted. Measured on both backends, identical: the chained projection
2 → 1, the bound projection 2 → 1, the two-droppable-field record at one release each, depth
three 2 → 1, a two-iteration loop 4 → 2, a rebound binding releasing the displaced copy at
the rebind and the literal at scope end, the bound-base reference and the borrowed-argument
control unmoved at 1, and `LOFT_POISON=1` over the 889 guard clean.
`tests/scripts/1506-a-call-result-projection-releases-once.loft` is the conformance guard.

**Both remaining shapes CLOSED (2026-09-10)**, each at the level the rules put it.

1. **The `?`/`??` join.** The reading above was right and the axis it named was too narrow.
   The join's value is `if __ncc_N { __ncc_N } else { <default> }`, both arms vars THIS FRAME
   owns and frees, and the block was typed as a bare OWNER — so whatever consumed it adopted
   the store a SECOND time. That is not a projection question at all: `r = mk_present() ??
   mk_other()` and `r = mk_present()?` each released twice with no `.field` anywhere, and
   each was a use-after-free (`LOFT_POISON=1` delivered the poison pattern into the author's
   `OpDrop`). The projection cells only stacked one more owner on top of it. The cure is the
   type-level fact: `build_null_coalesce_default` gives an OWNED RECORD subject the same
   dep-backed VIEW its OWNED VECTOR arm has had since @PLN102 — `Type::Reference(d,
   Deps::frame([__ncc_N]))` — so the consumer BORROWS what the frame still owns.

   A borrowing result then means the DEFAULT arm needs an owner of its own, which is
   loft#1013's question one gate wider. `materialise_owned_call` answered it only for a
   callee with a hidden return buffer, and a callee that can answer NULL is not
   NRVO-promoted and carries none (`fn pick(n) -> Cell?`) — so a chained `??` leaked one
   record where the borrowing result no longer adopted it. It now mints a `__ref_p2_N`
   work-ref where there is no buffer, which is the general form of the same cure and the
   `join-arm-owner` block's one home either way.

2. **The return-of-view copy.** `materialize_return_into` publishes an owned copy because
   `@FR-F-Ret` wants a fresh value, and `(H-Drop)` then moves the member's release with it.
   The transfer is read at the RETURN SITE and never keyed on the variable, exactly as this
   entry predicted: `two_exits` releases the member on the path that does NOT hand it out,
   and a var-keyed skip would have leaked it there. A copy off a PARAMETER is declined —
   `(H-Drop)`'s own clause, *"a copy off a PARAMETER moves nothing: the caller owns"* — so
   `return p.h` is unmoved at two releases, which is D-heap-1's shape and not this one.
   A source that is a VIEW LOCAL (`e = d.h; return e`) is carried too: `Scopes::view_backing`
   holds the path from the bind that established it to the return that hands it out, retired
   from BOTH ends (@FR-O-Latest — writing the view replaces what it names, and rebuilding the
   BASE leaves every view of it naming a record that no longer holds what the path said).

   ⚠ **A JOIN tail is declined and that is not an oversight.** The sweep at a return runs for
   every path through that ONE return, so a materialised copy sitting in an `if` arm belongs
   to one path: taking its skip would lose the member's release on the arm that did not copy —
   the same defect one shape over. Two separate `return` statements are two sweeps and work;
   `return if c { d.h } else { L }` does not, and closing it wants loft#1476's shape (put the
   SOURCE's release inside the arms) rather than a wider reader. It is live today as the
   opposite fault — the hook is LOST, not doubled (loft#1514, whose root is
   `wrong_shape_for_buffer` opening on `Type::Vector` while the rule it cites carries no such
   qualifier).

**The skip-capable cascade names the member by its PATH, not by a number.** `(skip, depth)`:
byte offset from the record, and how many levels below it. Both, because a member at offset 0
of a nested record shares its owner's address — `d.p` and `d.p.a` are the SAME byte — so an
offset alone skipped the whole subtree and LOST every sibling under it (measured: `return
d.p.a` dropped `p.b` entirely, a lost release where the defect was a double one). With the
depth, one pair reaches any depth with no new machinery: each level hands its member `skip -
off` and `depth - 1`, and a skip that is not under a field lands outside that field's own
offsets at every level below it, so it can match nothing there.

Measured on both backends, identical, and clean under `LOFT_POISON=1`: the bare `?`, the
`??` present and default arms, a chained `??`, the chained / absent / bound / argument /
loop / two-droppable-field / depth-three `?` projections, the returned projection, the
view-bound return, the rebound view, a member two levels down releasing `q` AND `p.b` inside,
a member beside the nested record leaving both of ITS members, and the whole nested member
taking its own with it. Guard:
`tests/scripts/1506b-a-join-and-a-returned-projection-release-once.loft`.

**The last residual CLOSED as D-heap-5 (loft#1513), below** — the invariant cure this entry
predicted. The dense family's terminal materialisation gave a binding and its work-ref ONE
store with TWO frees, and which ran first was decided by DECLARATION ORDER:

```
r = mk_dense().h;                       // r declared after __ref_N — r drops, then __ref_N
r = CfH { id: 4 }; a = r.id; r = mk_dense().h;   // r declared BEFORE — __ref_N frees, then
                                                 //   r's hook reads the freed record
```

Value and count were green in both (`H4 H9`, the ids the program set); under `LOFT_POISON=1`
the second read `H-2401053088876216593`. It is the `1506` guard's own `own_then_copy` cell —
*"a cure here cannot be scored on release counts alone"*, now measured against the cure. The
first of the two cures named here was the right one — disarm the adopted work-ref's free — and
the blast radius this paragraph flagged (`construction_work_ref` matches every struct literal,
so a naive disarm reaches every `r = S { … }`) is exactly what D-heap-5 gates on: the disarm
fires only where the binding's type carries a HOOK, leaving the hookless `value struct`
comprehension's in-place reuse untouched. See D-heap-5 for the mechanism and its guard.

### D-heap-4 — OPENED AND CLOSED (2026-09-10): a mixed own/view local's owned record is freed without its hook (loft#1510)

`(H-Drop)` runs the hook at the record's death; `(O-Latest)` says a static deps list cannot
carry per-assignment ownership, which is what the owner witness (`@FR-O-Witness`) exists for —
release by STORE IDENTITY.  Measured: the identity release frees the store and runs **no
hook**, on both backends, silently, in every spelling of the mix:

- `r = mk_h(); r = d.h` — the witness's own canonical shape (a minting call, then a view):
  ONE hook (the container's), the minted record's never;
- `r = CfH { id: 4 }; r = d.h` — the literal spelling: same, via the `#316`
  ownership-transition free, which emits `OpFreeRef` with no hook;
- `d = mk(); r = d.h; r = CfH { … }` — view then own: the construction hand-off gives the
  literal's work-ref drop to `r`, whose view-typed scope end runs no hook — the record is
  freed by the work-ref's own `OpFreeRef` with the hook lost between the two mechanisms.

Found while closing D-heap-3's dense family (its rebound-binding cell walked straight into
this), and kept apart from it because no call-result projection is involved: the three
spellings above reproduce with a plain bound base.  The shared cause is that every "release
by identity" site — the witness's, `#316`'s transition free, the construction hand-off's
premise — releases the STORE and not the RESOURCE.  A cure belongs at that shared clause,
not per spelling: whichever mechanism releases a store the frame minted must run the type's
cascade first, exactly as `scope_end_drop` does.  ⚠ Two of the mechanisms can claim the same
record (the work-ref's scope end and the binding's rebind), so a cure that adds the hook to
both is the double release coming back — the conformance cells must assert the count on ALL
THREE spellings plus the D-heap-3 guard's rebound cells, which pin the fixed neighbours.

**CLOSED 2026-09-10**, at that shared clause, in four pieces — and it took FOUR mechanisms,
one more than the entry's own list, which the boundary matrix found:

- **the witness releases hook-first**: `scopes::release_witness` — the ONE home for all
  three witness frees (the rebind-distinct release, the `(O-Detach)` release, the scope-end
  release) — runs `drop_hook` before the `OpFreeRef`, guarded on the witness holding a
  record, so the sentinel hooks nothing.  Exactly one mechanism claims a witnessed store:
  the witnessed local never drops and the call's hidden buffer keeps its bare free.
- **the hand-off is made only to a binding that will run it**: `drop_handoff_node`'s Set
  arm transfers the work-ref's drop only where `proxy_says_owned(binding)` — a view-typed
  binding runs no scope-end drop, so the old transfer lost the hook between the two
  mechanisms.  A vetoed transfer leaves the work-ref's own scope-end drop+free as the
  releaser (the view-then-own spelling).
- **the owned→view transition free hooks and disarms**: the loft#1202 arm, releasing a
  literal-minted store the binding is about to stop naming, runs the cascade first and
  writes the SENTINEL into the backing work-ref (`Scopes::construction_backing`, a
  path-sensitive map with `owned_refs`'s intersect-merge) — without the sentinel the
  now-un-transferred scope-end drop re-runs on the freed store, and WITH it a loop's next
  `OpDatabase` mints fresh instead of rebuilding the freed slot.  A backing a join dropped
  keeps the bare free: the hook is lost on that path, never doubled.
- **the in-place literal rebuild hooks through the witness**: `parse_object`'s in-place arm
  reaches the scan as a bare `OpDatabase` on the local — no `Set` — so the witness kept
  naming the store being cleared and the record's hook was lost at the overwrite (the
  three-assignment cell `r = mk_h(); r = CfH{…}; r = d.h` found it: 65 and 9 hooked, 4
  silently overwritten).  The scan now runs the hook through the witness before the clear
  and re-points the witness after; nothing is freed, the store is reused as before.

Guard `tests/scripts/1510-a-mixed-own-view-locals-record-hooks-once.loft` — 11 cells
asserting exact hook SEQUENCES (both loop orders, the conditional construction on both
paths, the three-mechanism frame, and the uniform-owner/uniform-view controls unchanged);
the D-heap-3 guard's rebound cells pin the fixed neighbours.  A residual stated while
closing: a heap PARAMETER rebuilt in place keeps its hook-less overwrite (the in-place arm's
argument path frees through `OpFreeRefIfDistinct` against the entry witness and hooks
nothing) — out of this entry's mixed-LOCAL scope, noted for the walk that owns parameters.

### D-heap-5 — OPENED AND CLOSED (2026-09-10): the adopted construction store had TWO claimants, and the rebind order let the wrong one go first (loft#1513)

`(H-Drop)` runs the hook at the record's death — which requires the hook to run BEFORE any
free of that store.  When a construction delivers through a work-ref and the binding adopts
the store (`drop_handoff_node`'s Set arm: the drop is transferred because "only the binding
owns it"), the work-ref kept NAMING it, and its scope-end bare `OpFreeRef` was a second
claimant.  For a FRESH binding the emission order happened to run the binding's hook+free
first, so the bare free no-opped on an already-freed store and nothing showed; for a REBIND
(`r = CfH{…}; r = mk_dense().h` — the loft#1506 guard's own `own_then_copy` cell) the
binding is declared before the work-ref, scope-end runs in reverse declaration order, and
the bare free went FIRST: the hook then read freed memory.  Counts and values right off
stale bytes in a plain run, the poison pattern under `LOFT_POISON=1`, both backends, in
every adopting spelling (the rebind, a loop rebind, a conditional rebind's taken arm, the
two-droppable-field record).

**Closed at the adoption, not at the ordering**: the moment the binding takes the store, the
Set arm writes the SENTINEL into the work-ref (`handoff_disarm`), riding the SAME predicate
as the drop hand-off (`construction_work_ref` + `proxy_says_owned`) so the two mechanisms
cannot drift — exactly one claimant, whatever order scope-end walks.  A path that skips the
Set leaves the work-ref holding its record, and its own scope-end release still covers it.
Gated on the binding's type carrying a HOOK (`drop_hook`), because that is what the disarm
protects: for a hookless type the double claim is a benign no-op free, while the sentinel
DEFEATS the work-ref's in-place reuse — a comprehension's `OpFreeRefIfDistinct` reads
"distinct" against a sentinel and frees, so it minted per PASS instead of rebuilding one
store, and `value_struct_alloc`'s O(1) promise measured it at N cycles before the gate.
What made the sentinel write possible is the second half: `is_null_sentinel_detach` now
answers true for a `__ref_`/`__rref_` work-ref assigned the bare sentinel — the scan writes
that ONLY as a disarm, after the store is freed (loft#1202, loft#1511's buffer) or adopted
(here) — because the displacement pre-`Set` free read the disarm itself as a displacement
and freed the store the binding had just taken (measured: the first fix attempt poisoned
even the fresh-binding control through exactly that free).  One home, shared by both
backends through `owns_displaced_store`.

This is the residual D-heap-3 kept its entry OPEN for, and it is BRANCH-INTERNAL: the
two-claimant situation exists only because the dense family's terminal materialisation
(this branch's D-heap-3 close, not yet on `main`) makes the rebind materialise where `main`
still aliases — so the poison face reproduces on this branch and not there.  loft#1513 was
filed anyway and is closed here; the fix is correct regardless of provenance.

Guard `tests/scripts/1513-a-rebound-adoption-hooks-live-memory.loft` — the rebind matrix
plus the copy-then-own and untaken-arm controls, and the tuple-literal-member cell (the
loft#1511 entry's second finding, same one-claimant rule).  ⚠ Its own channel is
`LOFT_POISON=1`: a plain run passes on the broken build, so
`tests/store_lifetime_1512_1513.rs` arms the poison runs per test on both backends — and
poison-arms the neighbouring 1506 and 1511 guards, which the corpus only runs plain.

### D-heap-2 — OPENED AND CLOSED (2026-09-10): a cascade released only the members it could reach through ONE field kind

`(H-Drop)` releases what a container holds at the container's death, *"its own hook first and
then its members, through the synthesized cascade"*.  It reached a `Reference` field and a
`Vector` field.  A TUPLE field, a STRUCT-ENUM field and a NULLABLE record field were walked by
nothing, on both backends, silently: `struct H { t: (S, integer) }` built, read and dropped
never ran `S`'s hook at all.

**One question with two decoders, and they disagreed.**  `Data::type_owns_droppable` decides
WHETHER a type owns a droppable and follows a tuple member, a struct-enum payload and a
collection element to say so; `Parser::cascade_fields` decides HOW to reach one and reported
only `Type::Reference`.  A type the first said yes about and the second found nothing in gets
no cascade *declared* — so it answers "I own a resource" and then releases none of it.  The
divergence is dated: `cascade_fields` carried *"an enum-payload or collection field is left for
stages D/E"*, and stages D (an enum's own cascade) and E (a collection field) both landed
without the FIELD walk being widened to call what they built.  The `__nullable<S>` a `S?` field
is rewritten to is a struct-enum, so one missing arm cost two field kinds.

**Cure** — `Parser::cascade_field_target`, one home for *"which definition does this field
release through, and in what spelling do I read it?"*, cited by `cascade_fields`.  A tuple
resolves to its `__tuple<…>` def and is read as a reference to the inline record, which is what
`layout.md (L-Tuple)` already says it is; a struct-enum keeps its enum spelling so its cascade
can test the discriminator before reaching a payload.  A `&τ` field stays out: it is a LINK and
the source frees the store (`binding.md (B-Ref-Alias)`), so releasing through it would release
what another owner still holds.

**Guard** `tests/scripts/a-drop-cascade-reaches-every-field-kind-it-owns.loft`, twelve cells
over the field kinds plus an absent nullable, a unit variant and a two-resource CONTROL,
measured identical on both backends.  It scores the release COUNT: the fault is a hook that
does not run, so a guard scoring only for a DOUBLE release reads every broken cell as a pass.

⚠ **The missing cascade was also a CRASH, and that is the part worth carrying forward.**  A
declared-but-undeclarable cascade does not merely skip a release: `scopes::copy_moves_drop_from`
declines a hand-off whenever `data.drop_cascade_nr(d)` is `u32::MAX`, so a type `owns_droppable`
says yes about and no cascade exists for gets the ownership answer for a type that owns
NOTHING.  `o.inner.f?.h.id` over a holder with a nullable record field then freed a reference
nobody owned, which `--interpret` refused as BUG #306 (*"a stack-record ref was treated as an
owned heap store"*) with a `rec=65535` panic behind it — reproducing back to the shipped
2026.8.0 and not on `--native`, which is why it read as an interpreter fault rather than as a
missing cascade.  So the two decoders disagreeing was not a quiet omission: every downstream
site that keys on the cascade's EXISTENCE inherited the disagreement, and this is the one that
crashed.  Guarded apart in
`a-nested-read-through-an-absent-nullable-field-does-not-free-the-stack-store.loft`, because it
moves the panic channel where the release guard moves assertions and a receipt names one.

⚠ **The over-reach cell is what found the neighbours.**  "An absent nullable field releases
nothing" was written to prove the fix had not started dropping tag bytes; it failed, and the
cause was not the cascade.  Reading an absent nullable through `?.` answers its type's ZERO
and mints a record whose scope end runs the hook — and the ZERO half is not a defect at all:
`types.md (N-Chain)` says a per-link `?` is `(N-Default)`, *"replace THIS structure with an
empty one"*, and the compiler already names the trailing `??` as `advice[redundant-coalesce]`.
The rules settled that one before any of it was worth measuring.  What IS a defect is
loft#1505: `--native` emits that `(N-Default)` construction TWICE for a projected nullable
FIELD — once into a `let _pre_N` binding nothing references — so the hook runs two or three
times where `--interpret` runs it once.  Filed rather than fixed here: the cure is in the
native generator's pre-eval substitution, which matches by generated TEXT across two passes.
The cell reads with `==` so it scores the cascade and not loft#1505.

### D-heap-6 — OPENED AND CLOSED (2026-09-11): a literal built through a work-ref read as a VIEW, so a local whose every assignment OWNS was witnessed as a mixed one (loft#1517)

`(H-Drop)` releases a record through its hook when its owner dies, and the reassignment clause
names a displaced record explicitly.  A heap-record local built from a LITERAL and then
reassigned runs NO hook — neither for the record it displaces nor, where the literal is its
LAST assignment, for the one it still holds at scope exit.  Both backends, no diagnostic, no
leak warning.  The plain-struct twin runs both, which is what makes the defect read as a
struct-ENUM spelling question.

It is not a spelling question: it is the WORK-REF.  A struct-enum literal builds into a
`__ref_p2_N` and hands the local the work-ref as the block's tail; so does a NULLABLE STRUCT
literal, which `@FR-N-Shape` makes the synthetic struct-enum `__nullable<S>` — a cell with no
user enum in it.  A plain struct builds IN PLACE, has no work-ref, and is correct throughout.
Measured across a 19-cell matrix: the displaced record's hook is lost across variants, through
a payload CASCADE, under a nullable wrapper, with a minting CALL as the second assignment, and
once per loop for the literal's own record; a local whose LAST assignment is a literal loses
that record's hook at scope exit as well, which is the same root's second face (a work-ref's
scope-end free is bare).  The filed table says BOTH hooks are lost; on a tree carrying
loft#1510 only the DISPLACED one is, which two hands measured independently.

**The literal build is NECESSARY and not sufficient.**  The local also needs an assignment
`witness_set_kind` calls a MINT — a whole-value copy of another LOCAL, or a call.  A second
assignment that is a PROJECTION is classified `Other` like the literal, the intersection stays
empty, no witness is minted and the local is correct (`x: H? = H { id: 94 }; x = mk().inner`
releases both records on both backends).

**The mix `(O-Witness)` is about does not exist.**  Its premise is *"one assignment hands the
local a store of its own and another hands it a VIEW"*, and for `x: SE = A { k: 3 }; x = a`
both assignments OWN.  Two predicates fabricate the mix between them, each answering its own
question correctly:

- `witness_set_kind` answers `Other` for a construction into a work-ref, and its comment says
  why — while the work-ref still names the store it is not SOLELY the local's.  That is the
  right answer to *"may the witness POINT here?"*
- `is_view_of_storage` then answers `true` for any `Block`, reading that `Other` as the VIEW
  half.  That is the wrong answer to *"does some other BINDING own what this names?"* — the
  work-ref is the compiler's own temp for this literal, and that predicate's own comment
  already says a store nobody names is not a view.

With the mix fabricated, three mechanisms decline in turn and the record is left with no
releaser: `displaced_drop` declines on `@FR-O-Override` (a witnessed local releases through its
witness only); the witness is pointed only at the assignment the classifier calls a mint, so
the literal's record is never its subject; and because a witnessed local is not
`proxy_says_owned`, `scan_set`'s hand-off disarm takes its view-typed arm, leaving the work-ref
holding the store.

This is **D-heap-4's neighbour and not D-heap-4**.  That entry closed the cases where the mix
is REAL, and its cure made the construction hand-off conditional on `proxy_says_owned` — the
predicate this defect then rides, because a local that should never have been witnessed answers
it the way a genuine view does.  A mechanism made conditional on an ownership fact inherits
every defect in how that fact is decided.

#### It took TWO mechanisms, because the arming population was MASKING a second defect

Both cures for the arming predicate were built first and **both regressed**, which is how the
second mechanism was found: the fabricated mix was masking an `(O-Detach)` defect in the
reassignment path for nullable locals, so correcting the arming exposed it as a wrong VALUE.
Fixing that one FIRST is what made the arming fix land, and the order was not a judgement call
— `(O-Witness)` and `(O-Move)` decide it (see below).  Probes (`--interpret`, and the arming read
off `introspect`):

| probe | shape | arming on `main` | value |
|---|---|---|---|
| `q1517` | `x: SE = A{3}; x = a` | `__own_x` | right, hooks LOST |
| `q1085b` | `c: KeepOnly? = KeepOnly{7}; loop { c = keep_k(c ?? …) }` | `__own_c` | 7, right |
| `qview` | `x: H = H{47}; x = bx.inner` | none | right |

- **Cure A — `is_view_of_storage` answers `false` for a work-ref construction.**  THIS IS THE
  CURE THAT LANDED, and it fixes every cell of the matrix on both backends and leaves `qview`
  alone.  Applied on its own it REGRESSED `q1085b` to `0` where `7` is right: that local's `viewed` membership came ONLY from the fabricated reading,
  so the correction disarms its witness — measured off `introspect`: `__own_c` on `main`, none
  after.  `1085b-a-nullable-local-frees-what-it-displaces.loft` loses two cells.
  `nullable_locals_that_displace` excludes a never-free local, so one release mechanism serves
  each local and `c` falls from the witness to the `__lbo_` flag.  Which of the two is wrong was
  left open here and the RULES answer it — see the paragraph below: that local must not be
  witnessed, so the witness is the deviation and the other path is what has to be fixed first.
  Attributing the wrong answer to `__lbo_`'s internals is still an inference this entry does not
  make; which mechanism must OWN the release is what the rules settle.
- **Cure B — separate the two questions** (a fourth `WitnessSet::MintIntoWorkRef`: owning for
  the intersection test, not-pointable for the maintenance site).  NOT TAKEN, and recorded so it
  is not retried: the decomposition is right and the variant is the honest shape, but it ARMS
  witnesses that `main` does not, which is a strictly wider change than the defect needs.
  `q1085b` still breaks (both of its assignments then read as mints, so `viewed` is empty) and
  `qview`
  gains one, moving its displaced hook from the rebind to scope exit — the right COUNT at the
  wrong time, which `(H-Drop)`'s reassignment clause does not allow.

⚠ **So `q1085b` passes on `main` through a witness armed for a reason that is false** — and the
rules SETTLE which half is wrong, where this entry first recorded an open question.
`(O-Witness)` conditions the witness on the assignments MIXING; `q1085b`'s are a CONSTRUCTION
(classification arm (e): a record construction is `Owned`, fresh, `(O-Owner)`) and a call whose
return borrows its parameter, which `(O-Move)` covers explicitly — *"if the return borrows a
parameter, the return type records it and the caller COPIES to obtain its own store"*.  Both
OWN, so there is no mix.  Measured rather than read off the rule: `c.x = 99` after
`c: K? = keep_k(src)` leaves `src` at `7` on both backends, where a `(B-View)` projection writes
through.

So the local must NOT be witnessed, the witness on it was itself a deviation
([ownership.md](ownership.md) `D-own-40`), and the other path's `0` was the defect to fix FIRST.
The order was derived from the rules rather than chosen, and *"keep the witness because removing
it breaks something"* was never an available answer.

**And the second defect turned out to be LIVE ON `main` on its own**, which the mask hid from the
first reading: it needs no witness suppression and no literal, only a NULLABLE local reassigned
from a call whose return borrows a parameter that reads it.  `c: K? = mk(); c = keep(c ?? K { x:
9 })` answered `0` on `--interpret` and `7` on `--native` — a two-line program, both facts
measured on `main` before any of this landed.  It is not the `__lbo_` path at all, which is what
this entry's first reading guessed and deliberately declined to assert: it is the in-place
re-allocation in the interpreter's own reassignment lowering, `(O-Detach)`, and it has its own
entry at [ownership.md](ownership.md) `D-own-41` with its own guard.  The first reading reached
for `__lbo_` because that mechanism is what `nullable_locals_that_displace` hands such a local —
a plausible neighbour, named in the same paragraph of `scopes.rs`, and the wrong one.

⚠ **Cure B's late release is inadmissible by RULE, not by taste.**  `(H-Drop)` states the
reassignment clause with its timing: the displaced record's hook runs *"after the new value has
been computed and before anything after the statement runs"*.  Moving it to scope exit — what
arming a witness on `qview` does — keeps the COUNT and breaks the WHEN, so it is a deviation and
not a cheaper cure.  ⚠ Note also that the clause's illustrative list names *"a rebuild in place,
a call result, a rebind to null"* and NOT a whole-value copy, which is exactly loft#1517's
spelling.  The governing phrase covers it (*"a REASSIGNMENT of the owner that displaces the
record"*), so this is a rule whose EXAMPLE list wants extending, not a rule that cannot express
the case.

⚠ **No gate saw this defect, and that is unchanged by closing it.**  `(O-Override)`'s gate is
`ownership_cfg`'s Check D (`LOFT_OWN_ORACLE=check`), and over both guards it reported
`clean — 0 RED` while twelve cells were wrong: nothing frees illicitly, what was missing was a
DROP.  `(H-Drop)`'s own ⚠ says a drop's two failures are not
ordered the way a free's are, and this is that asymmetry showing up in the instruments — the free
side has a checker, the drop side has only per-guard traces.  A gate for `(H-Drop)`'s three
deaths is the gap, and it would have caught both populations.

⚠ **And the reading that nearly shipped cure A was a control firing on both trees.**  `q1085b`'s
two cells fail under `LOFT_NO_OWNER_WITNESS=1` on `main` as well — they are that guard's own
positive control for the witness doing something — so "it fails identically with the switch on
and off" carries NO information about the edit, while reading exactly like a shared cause.  The
comparison that answers is *changed tree, switch ON* against *`main`, switch ON*.  One level
below it, the first run of that guard as a plain script reported no failures at all, because the
file has no `main` and `fn test_*` only runs under `--tests`: a corpus guard run the wrong way
passes having tested nothing.

**CLOSED** by Cure A, on top of `D-own-41`.  The guard is IN the corpus, as
`tests/scripts/a-record-local-reassigned-after-a-literal-build-releases-what-it-displaces.loft`
— 20 cells, one `fn test_*` each so they report individually.  Eight pass on every tree and are
the boundary a cure must not move (the in-place struct twin; a literal then a genuine VIEW, by a
bound base and by a call's field; a genuinely MIXED local; the two arm-literal cells).  The
twelve broken ones landed as expected-failures, each pinning the trace it read while the defect
was open beside the assertion for the trace it reads now — two-sided, so a partial fix would have
moved a line instead of passing as "still broken".  The cure turned twelve FAILs into twelve
UNEXPECTED PASSES, which is how the file reported that its annotations were due for retirement;
they are gone.  ⚠ The prose describing them may not spell the token either — a comment containing
it above the first `fn` binds at FILE level and would excuse the whole guard silently.  `test_mixed` doubles as the witness control with a release-COUNT
channel (`m55,55,V56,56` here, `m55,55,V56,56,56` with the witness disabled), which is what to
bisect this family on: the loft#1336 guard only reports as a hang, and a control that can only
time out cannot say which tree moved.

Writing these rules **shrinks** [operational.md](operational.md)'s D-op-1 — the heap/store
steps it named as *"unwritten … the interpreter remains their spec"* now have a written
contract (this file). What remains is the SAME meta-deviation, not a heap-specific one:

- **Conformance is differential, not definitional** — the heap steps here are enforced across
  the two backends by the @PLN89 **differential oracle** (D-op-1), whose corpus deliberately
  exercises the heap-heavy areas (collections, text, keyed collections, coroutines) where the
  interpreter's store and the native generator's `DbRef` ABI use the most different mechanisms.
  A program whose heap steps diverge is caught there. This doc does not add a new open row; it
  supplies the contract the oracle's heap-touching cases are read against.
- **The lifetime side has the strongest standing proof, and it is exactly as complete as
  ownership.md's register.** The free discipline's soundness (`H-Sound`) rests on
  [ownership.md](ownership.md): its `OPEN` line is the discharge, and a path-completeness gap
  there (the shape `D-own-8` had — a Join's ownership fact true on one path only) is what
  `H-Sound` consumes. A claim about another doc's register goes stale silently, so read that
  register rather than a restatement of it here.

## Conformance

The rules are checkable directly, and every check is a program both backends must agree on:

- **Copy vs alias vs view (`H-Copy` / `H-Ref` / `H-View`)** — proven both backends: a PLAIN
  whole-value / vector bind COPIES — `c = o; c.v = 9` ⇒ `o.v == 1`; `fv = e.items; fv[0] = 99` ⇒
  `e.items[0] == 1`. An EXPLICIT `&`-bind ALIASES — `r = &v; r[0] = 99` ⇒ `v[0] == 99` (a live
  reference, C77; the vector twin of a parameter). A STRUCT-typed projection is a VIEW —
  `c = o.i; c.v = 9` ⇒ `o.i.v == 9`; `s = v[0]; s.v = 9` ⇒ `v[0].v == 9` (a struct element of a
  vector). capabilities.md's raw-write rule encodes this: a plain-copied vector is owned, a
  `&`-aliased or parameter-rooted vector is host (D-cap-3 follows the dep chain), a struct is a
  possible host view.
- **Null/OOB continue (`H-ReadNull` / `H-Index`)** — reading a field of `nullref`, or `v[i]`
  with `i ≥ len(v)`, is **null** and the program continues; it never halts (operational.md
  C80, extended to the heap).
- **Parameter-root write escapes, local-root write does not (`H-Write` / `H-Copy`)** —
  `fn f(v: vector<integer>) { v[0] = 99 }` mutates the caller's vector (`orig[0] == 99`);
  binding first, `fn f(v) { c = v; c[0] = 99 }`, does not (`orig[0] == 1`). This IS the
  capabilities raw-write boundary.
- **Free discipline (`H-Free*`)** — the `LOFT_POISON` suite + the ownership fuzz gate are the
  standing falsifiers: any `H-FreeTwice` / use-after-free / out-of-LIFO free surfaces as a
  poisoned read or a leak-count mismatch. The register that guarantees they never fire is
  [ownership.md](ownership.md)'s — read its own `OPEN` line, which is not zero (this row said
  "0 open" while that register carried an open entry).

D-op-1's falsifier applies here too: any program where the interpreter and `--native` diverge
on a heap step is the definitional error, and this doc is the definition it fails against.
