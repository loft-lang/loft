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

### Free — release a store, in LIFO order, never the stack, never twice

```
  (H-Free)       ⟨free(r), ⟨ρ, H⟩⟩ → ⟨(), ⟨ρ, H \ store(r)⟩⟩
                   when H ⊢ r live, store(r) is the MOST-RECENTLY-ALLOCATED live heap store,
                   store(r) ≠ 0 (not the eval stack), and store(r) not free_protected.
  (H-FreeNull)   ⟨free(nullref), σ⟩ → ⟨(), σ⟩                      (freeing null is a no-op)
  (H-FreeLIFO)   freeing a store that is NOT the current top of the allocation order is a
                 FAULT — the LIFO discipline (a store's lifetime nests within those allocated
                 before it).  ⚠ RETIRED IN THE IMPLEMENTATION — see D-heap-LIFO below.
  (H-FreeStack)  freeing store 0 (the evaluation stack) is a FAULT (#306): a stack-record ref
                 is never an owned heap store.
  (H-FreeTwice)  freeing an already-freed store is a FAULT (use-after-free / double-free).
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
                 rule, not a semantics rule.

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
```

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

> **D-heap-LIFO — OPEN (2026-09-09).**  `(H-FreeLIFO)` states a fault the implementation
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
> Open rather than closed because the wording is a DESIGN call, not a transcription: either the
> rule is deleted, or it is rewritten as the weaker invariant that actually holds (a store's
> lifetime nests within its OWNER's, which is `(H-Free)`'s liveness condition and not an
> ordering).  @PLN155 phase 4 measured it and did not take that call.

### Drop — the hook a type declares runs once per resource, when the record that owns it dies

```
  (H-Drop)   a type that declares `OpDrop` releases a RESOURCE the record holds (a handle,
             a lock, a file), and the hook runs ONCE per resource, at the death of the
             record that OWNS it:
               - the owner's scope end (@PLN125 arc B), in reverse declaration order;
               - a REASSIGNMENT of the owner that displaces the record — a rebuild in
                 place, a call result, a rebind to null — after the new value has been
                 computed and before anything after the statement runs (loft#1362);
               - a CONTAINER's death for what it holds, its own hook first and then its
                 members, through the synthesized cascade (C111).
             RESPONSIBILITY moves with a copy (binding.md B-Copy): a plain whole-value copy
             `t = s`, a copy into a struct field, an enum payload or a vector element, a
             branch arm's temp, a return buffer — the copy owns, the source stops
             dropping.  A copy off a PARAMETER moves nothing: the caller owns.
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
  (H-Drop-Not) the OLD value of an overwritten FIELD or ELEMENT (`o.s = other`,
             `v[i] = other`), an element taken OUT (`v.remove(i)`), and a keyed
             collection's records are NOT released by the language — the author releases
             them (INTERFACES.md § `OpDrop`, the documented boundary).
```

**In words.** A drop is not a destructor of loft-side data — the ownership model pays for
that — it is the release of something outside the program, and such a thing must be
released exactly once. So the question every site asks is *"which record owns the resource
now?"*: the owner's death runs the hook, a copy moves the ownership to the copy, and a
reassignment is a death for the record it displaces. Where two records hold one resource
by the author's doing — two containers built from one droppable, two copies of one value,
one hand-off inside a loop body that runs twice — `warning[double-move]` names what it can
see and the loop shape stays on the author.

**Conformance.** `tests/scripts/139-drop-cascade.loft` (the cascade),
`a-whole-value-copy-of-a-droppable-releases-once.loft` (the copy moves), and
`1362-a-rebind-releases-the-droppable-it-displaces.loft` (the reassignment), each measured
identical on both backends.  Sites: `scopes::displaced_drop`, `scopes::copy_moves_drop_from`,
`scopes::scope_end_drop`, `scopes::copy_hands_off`.

### The soundness bridge — a well-typed program never faults a free

```
  (H-Sound)   if a program is `deps`-SOUND (ownership.md O-Derived + O-Complete: every free
              it emits is on an OWNED store at its last use), then no execution reaches a
              faulting free — H-FreeLIFO / H-FreeStack / H-FreeTwice never fire, and no read
              observes a freed store.  The static checker discharges the dynamic invariant.
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

OPEN: **2** — `D-heap-1`, below: five shapes release a tuple member's resource TWICE (the
list has been re-cut as each was measured; the count is what is open TODAY); and
`D-heap-LIFO`, stated with `(H-FreeLIFO)` above, where the rule names a fault the
implementation deliberately stopped requiring.  The count read **1** while `D-heap-LIFO` was
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
freed without its hook) opened and CLOSED 2026-09-10, below.

### D-heap-1 — OPEN (2026-09-05): a copy of a tuple member releases its resource twice

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
  must score the VALUE the hook sees and not only the count;
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
