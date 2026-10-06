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

### Allocation — a fresh store, every field written

```
  (H-Alloc)    ⟨alloc τ, ⟨ρ, H⟩⟩ → ⟨r, ⟨ρ, H'⟩⟩
                 where s = fresh(H),  r = (s, 0, 0),
                       H' = H[s ↦ Store for τ, every field/element WRITTEN to its type's default]
  (H-NewRec)   ⟨new-record r_v, ⟨ρ, H⟩⟩ → ⟨r_e, ⟨ρ, H'⟩⟩
                 a fresh record inside the vector/collection store r_v; r_e points at it,
                 its fields written before any read, the container's length grown by one.
  (H-Claim)    the bytes of a claimed block are UNDEFINED until something writes them: a
                 claim promises no value, zero included.  Every field a construction does not
                 write from its source is written to the type's default (H-Alloc); a slot
                 read before it is written is a defect of the code that reads it, never of
                 the claim.  A zeroed claim is legitimate only as the implementation of a
                 write the program makes anyway — `[0; n]`, and a record whose every field's
                 default is zero built in place — and is requested at that site.  (@C137)
```

**In words.** Allocating a struct/vector reserves a **fresh** store slot (`OpDatabase`) whose
fields are WRITTEN to their defaults — a fresh `store_nr` distinct from every live store, so a
new value can never coincide with an existing one.  The defaults are a write, not the memory's
state: a type, a constructor or a release walk that reads a slot its own code has not written
relies on bytes nothing promised (H-Claim), and `LOFT_POISON_CLAIM=1` — which fills every claim
with 0xDEADBEEF — is the instrument that makes such a read fail; the nightly runs the suite
under it on both backends. Appending an element (`OpNewRecord` /
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
                And where the container is LINKED — its slots hold 4-byte record ids rather
                than the elements — the stride is FOUR, whatever τ's own record measures.
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

**And the LINKED layout is the same trap with a different handle.** A `vector<T>` holds its
elements inline until any keyed collection over `T` exists ANYWHERE in the program, at which
point every `vector<T>` becomes a vector of 4-byte record ids — a `vector`/`sorted` turning
into an `array`/`ordered`, `Stores::is_linked`'s answer. So the stride is not a property of the
declaration in front of the reader, and a site that reads the element's own width is wrong by a
factor for every program that happens to key that type. loft#903 closed it for the by-index
removal; loft#1670 found `reverse` and `insert` had never been reached, both sliding a span
twice as wide, both silent on both backends. `Parser::element_store_size` is where the two
questions meet, and it now asks `is_linked` before it measures the record.

Two homes hold the answer. `Data::vector_element_type` names the element's storage TYPE, and
`Data::narrow_vector_element` names its `(spec, nullable, width)` — the latter being what
`narrow_vector_content` registers the storage through, so a site that measures with it cannot
disagree with the layout it is walking. A site that instead holds the declared type and needs a
value read or written passes THAT type down (`read_slice_elem`, `Parser::set_element`) rather
than recovering it from a def.

**An index is end-relative when it is negative**, so "out of bounds" is `i ≥ len(r)` or
`i < -len(r)`, not simply `i ∉ [0, len)` — `v[-1]` is the last element and `v[-len]` the first
(LOFT_DATA.md § Vectors, @P384; the value-slice bound follows the same rule,
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
  (H-WriteLocked)  a write to a read_only store is a STATIC reject where provable; else a
                   DEVELOPMENT run halts with the lock report and a PRODUCTION run logs the
                   write and DISCARDS it — the store keeps its bytes (C80).  Never a silent
                   successful write.
  (H-TextReplace)  an ASSIGNMENT to a text field (`o.s = t`, `v[i].s = t`, `o.s += t`)
                   releases the text the slot held, or writes `t` over its block when `t`
                   fits, once no borrow of that text is live.  A record LITERAL's write is
                   an initialisation: it never reads the slot it fills.
```

**In words.** A write updates the byte(s) at the target and yields the written value. A write
**through nullref** or **out of bounds** targets no live cell, so it is a no-op that continues
(it must never scribble on an arbitrary address — the null discipline extends to the write
side). Both are the SAME test at the same place: an out-of-range index produces `nullref`, and
every typed setter refuses to resolve a store for it, so `v[9] = x` on a three-element vector
and `absent.f = x` through a keyed miss take the identical step (measured on both backends —
value, length and the neighbouring records all unchanged; the standing guard is
`tests/scripts/a-write-to-an-absent-element-lands-nowhere.loft`, which also pins that a
heap-owning literal written there is released and its source untouched, and that every
native rewrite writing an element without a `DbRef` — the hoisted header and base, the slice
fill, the record-view address, the guarded index chain — answers the same). A write to a **locked** store — the author's `#lock`, a constant, bytes the program does
not own (`file_map`) — is refused, never silently applied: a development run halts with the
report, a production run logs it and discards it, so the program continues on the old bytes.
Discarding is a property of every write route, not of the op that started it: the value
writes, the span writes, the allocator (a production claim hands out a fresh record nothing
can reach, and never grows, moves or frees one the store holds) and the `--native` hoisted
writers all consult the same lock (`tests/locked_writes.rs`). The hoisted writers consult it
ONCE per loop, not once per element: no loop the hoist admits can lock or unlock a store (every
op that does is a writer it refuses), so the lock state is read with the vector header, the push
window or the record address the loop holds.  A locked store then has no room on any fast path —
a window over it has no capacity, a header over it no writable element, a record address in it
leaves the fast path by the same test as a null one — so every write to it takes the runtime's
refusing path, and the fast path carries no lock test of its own. An assignment to a text field frees what it replaces (`H-TextReplace`): the text a field held
belongs to that field alone, so nothing else is left pointing at it — except a BORROW.  A
`text` parameter is handed the field's bytes rather than a copy, and so is a walk over the
field's characters, so `g(r.a, r)` whose body writes `q.a` would read its own parameter
overwritten.  The release therefore waits for the borrows: it is taken where no borrow can be
live in this frame — no text variable of the frame borrows from the record's owner, and the
write is not inside another call's arguments — and in no frame above it.  The owner is then a
local of this function, or a parameter whose every caller is a visible direct call that keeps
the same property: every `text` parameter of the writer is only the value written, and at each
call the argument is not inside another call's arguments and is owned there by a local no text
variable borrows, or by a parameter of a caller that holds the same (a greatest fixpoint over
the program's calls, so recursion is covered).  A text VARIABLE is no borrow: a local `text`
is an owned copy (`x = r.a`, a `??` discharge, a `for c in r.a` walk temporary), and a native
slice of a store's text (`(R-TextBorrow)`) is declined over any block a release can run in.  So
an argument already evaluated is a borrow only when it can answer a text — a field read, a
slice, a call answering text, a `text` parameter.  The initialisation is a
different write because the slot it fills can hold bytes that are not a block: a record minted
without zero-filling (`(R-CompleteWrite)`) or an element slot handed to a callee
(`(R-Place)`).  The guard is
`tests/scripts/1873-an-assigned-text-field-releases-the-text-it-replaces.loft`.
Crucially, a write's target ROOT decides whose state it touches: a
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
  (H-CopySelf)  ⟨write(p, read(p)), σ⟩ → ⟨(), σ⟩    a whole value delivered onto the PLACE that
               already holds it — a record copied onto itself, a vector delivered into the slot
               whose handle it is — is a no-op: the heap is unchanged, nothing is released and
               nothing is claimed.  The runtime decides it by IDENTITY at the delivery
               (`Stores::vector_replace`: same store and same record → return; the record copy's
               in-place arm the same), never by the type or by a compile-time bet, so a rewrite
               that hands a value's own home as its destination (rewrites.md R-Rebind, R-Place's
               "the buffer IS the place") writes exactly what a fresh copy would have left and
               pays for none of it.  The no-op holds ONLY for the whole place: a delivery whose
               source is a DIFFERENT place in the same store clears the destination first and
               copies (`vector_add` snapshots a same-store source), which is H-Copy.
```

**In words.** Whether a bind copies or aliases depends on **what is bound** — and the two backends
agree exactly (verified):

- **COPY** — a **plain** bind of a whole heap **value**: a local variable (`c = o`, `c = v`), or a
  **vector-typed** projection (`fv = e.items`). A fresh store is allocated and the graph
  duplicated, so the two are independent: `fv = e.items; fv[0] = 99` leaves `e.items[0] == 1`;
  `c = o; c.v = 9` leaves `o.v == 1`.
- **ALIAS** — an **explicit `&`-bind** (`r = &v`) binds a **live reference** ([C77](../DESIGN_DECISIONS_OWNERSHIP.md#c77--binding-ownership-heap-aliases-by-default--binds-a-live-reference)),
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
                 nothing to release and emits nothing.  A record whose every
                 heap slot is EMPTY at the release — each text and collection
                 slot zero, each struct-enum tag absent or naming a variant that
                 owns no heap, read from the type's cached slot list — releases
                 nothing and skips the walk; any slot the list cannot read takes
                 the walk.

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
  (H-Carve)      inside one store, a best-fit claim of n words from a tree node of b
                 words with b − n ≥ n puts the (b − n)-word remainder IN THE NODE'S
                 PLACE — the node's links, its color, its parent's pointer — instead of
                 deleting the node and inserting the remainder.  The node is the
                 SMALLEST of at least n words by the tree's (size, position) order, so
                 every node before it is smaller than n: a remainder of at least n
                 words still follows that predecessor, and being smaller than b it
                 still precedes the successor.  The tree's key set is the one the
                 delete and insert leave, so every later claim takes the same block
                 and the layout is unchanged; what changes is that the claim costs no
                 tree delete, insert or rebalance.  A remainder under n words, or
                 under the tree's minimum, takes the delete and the insert.
  (H-LazyFree)   inside one store, a delete of a SMALL block (at most 64 words) during
                 the store's LAZY phase merges with its free neighbours in O(1) as the
                 exact delete does — forward by header, backward by a footer CONFIRMED
                 by the phase's record of lazily freed block starts (claimed data can
                 spell a footer, and that record cannot lie) — and inserts the merged
                 block into no tree; a block that ends the store becomes the wilderness.
                 A larger block (a vector rung, a hash table) is tracked at its delete
                 as ever, the tree's insert being nothing beside the block's own copy.
                 So a claim in the phase finds what the tree holds (the wilderness, the
                 large blocks, the remainders claims leave) or an untracked block the
                 chain walk meets, and a scratch freed each round leaves the layout as
                 it found it.  The count the phase is judged by is the untracked words
                 held NOW: a claim, a resize or a merge that consumes such a block takes
                 it off, and a tree rebuild clears the record whole.  The phase ends at the
                 first claim once the untracked words reach a floor (256) AND a fifth of
                 the extent written so far, whatever the wilderness holds: ONE sweep
                 merges every free block and rebuilds the tree, and from then on each
                 delete is tracked at its site as before; an explicit reclaim ends it
                 the same way.  A fresh or reset store starts the phase again; a store
                 bound to a file never enters it, because a file is read, reclaimed and
                 paged by its layout.  Every block is still free or claimed by its
                 header, so the usage walk, an image and the open walk read the same
                 store; what changes is WHEN the tree learns of a free.  A store built
                 once and released whole never pays for its deletes, a tree whose dead
                 words stay a small share of its live ones stays lazy however big it
                 grows, and a vector ladder — dead about half of what it wrote — sweeps
                 within its first rungs, so the working set never doubles.  Below the
                 bound a claim the wilderness cannot hold grows the store, or takes an
                 untracked block the chain walk meets on its way to growing.
  (H-ReadSurface) a store opened from a file only to be READ — a cached stdlib or program
                 image — writes nothing into the file: its free-block tree and footers
                 (`H-FreeFooter`, `H-Wilderness`) are left unbuilt, since a read surface
                 claims and frees nothing.  The file's mapping is shared by every process
                 that opens it, so a rebuild in place is a write another start reads.
  (H-SwapIn)     the deep copy of a store's ROOT record into the root of another store
                 that holds nothing else, the source store given up after it (the copy's
                 free-source form — the rebind `x = f(…, x, …)` from a callee that minted
                 its result), is an EXCHANGE of the two stores' contents followed by the
                 release of the slot that ends up holding the destination's empty root.
                 A root is a record at `1@8` OF THE STORE'S OWN ROOT TYPE — a record's
                 first field shares its root's address, and only the type says the copy is
                 of the whole record.  Each slot keeps its number, so every reference to
                 the destination reads the source's records as it would read their copy,
                 and the released slot holds what the copy would have released.  Exact
                 only when the copied type keeps every pointer as a record number inside
                 its own store (scalars, texts, enums, inline structs and enum values,
                 vectors of those): a stored reference names a slot and keeps the copy, as
                 do the keyed collections, arrays and child records.  A store pinned to its
                 slot — a file, foreign bytes, a recording, a lock, a borrow, a constant,
                 a lazy binding, the interpreter's stack — keeps the copy; a free
                 protection refuses only the side the exchange releases.
  (H-SwapRebind) on `--native`, the rebind `x = f(…)` whose copy would follow a reset of
                 `x`'s store is the exchange of `(H-SwapIn)` WITHOUT the reset: `x`'s
                 previous tree moves to the released slot with the rest of `x`'s store
                 and is freed with it, which is what the reset releases.  Every other
                 condition is the exchange's; where one fails, the reset and the copy run.
  (H-SwapKeyed)  a keyed LOCAL rebound from a call that gave its collection up (`h =
                 build(…)`, `OpReplaceKeyed`'s free-source form) is `(H-SwapRebind)`'s
                 exchange on both backends: a keyed local is a dedicated store with its
                 header at `1@8`, so the source's whole store is the value the copy would
                 rebuild.  A keyed collection's own pointers are record numbers inside its
                 store, so they survive; what keeps the copy is a stored reference in the
                 ELEMENT tree, which could name the source slot.  The released slot holds
                 the old collection and is freed, never parked for a buffer refill.
```

**`H-SwapIn` is the copy's own answer, delivered without the copy.** The copy it replaces
resets the destination to an empty root, rebuilds the source's tree inside it, and releases
the source store; the exchange ends in the same two states — the destination's slot holding
that tree, the released slot gone — and skips the rebuild.  Its conditions are the copy's
facts made checkable: "the destination holds nothing else" is `holds_only_root`, "the source
is given up" is the free-source flag the caller set, "nothing outside the tree points into it"
is the type walk.  `LOFT_NO_STORE_SWAP=1` keeps the copy; `LOFT_TRACE_STORE_SWAP=1` names
each exchange.  Site: `Stores::try_swap_in`, called by both backends' `OpCopyRecord`.  Guard
`tests/scripts/a-rebind-from-a-fresh-result-exchanges-the-stores.loft`, pin
`tests/store_swap.rs`.

**`H-ReadSurface` is why a cache can be shared.** The tree's links live in the free blocks'
own bodies, so building it is a write to the store's bytes, and for a mapped file those bytes
are the file's.  One start alone rewrites the same links each time, which is why the image
reads byte-identical after a run; two starts at once each read a link the other is halfway
through writing, as a record number.  The rule is a property of the OPEN, not of the caller:
`Store::open_read_surface` builds the in-memory claims set from the headers and nothing else,
and every cache loader goes through it (`ir_read::adopt_read_surface`).  A writable file store
opens with `Store::open` and rebuilds as before.  Guard `tests/concurrent_warm_start.rs`.

**`H-SwapRebind` is the same exchange, one statement earlier.** A reset clears the destination
store and claims a fresh root, and the exchange that follows moves that root out again: the
only thing the reset changed is what the released slot holds when it is freed — an empty root
instead of `x`'s previous tree.  Both are freed, so skipping the reset changes no reachable
value.  The source must still be given up and must not be a store the last fn-ref call
borrowed; a witnessed destination is reset as before, since its old store has to survive.
`LOFT_NO_SWAP_REBIND=1` keeps the reset.  Site: `codegen_runtime::OpRebindRecord` over
`Stores::try_swap_rebind`, emitted for a call-return rebind's copy arm
(`generation/dispatch.rs`).  Guard
`tests/scripts/a-rebind-exchanges-into-a-destination-it-does-not-reset.loft`, pin
`tests/swap_rebind.rs`.

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

> `(H-FreeLIFO)` is retired: `(H-FreeAny)` states that a free needs no nesting order and `(H-FreeAll)`
> the completeness obligation.  Nesting is not a HEAP rule — single ownership is discharged by
> [ownership.md](ownership.md).  The measurement and the ruling: [heap-history.md](heap-history.md)
> § D-heap-LIFO.

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
             is not progress and must not be landed as an improvement (measured on
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
             struct, an enum variant or a collection holding such a member gets a synthesized
             copy cascade, the mirror of the drop cascade: its own `OpCopy` first, then its
             members'.  The copy is then a structure the function OWNS — a local it fills is
             not the caller's record, and placing or returning it is a move.
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
identical on both backends.  The copy rules conform: `(H-Copy-Refuse)` and
`(H-Spent)` are compile-time errors (`D-heap-8`, closed), `(H-Copy-Lease)` runs `OpCopy` on every
copy a leasing type makes (`D-heap-9`, closed), and `(H-View-Drop)` closed as `D-heap-11`.
`tests/ownership_drop_gate.rs` classifies every generated cell under these rules and ties each
cell that disagrees to its deviation; `tests/copy_lease.rs` holds the leasing cells.  Sites: the
deaths are `scopes::displaced_drop` and `scopes::scope_end_drop`; a release moves across a copy
only where `(H-Move)` moves the value — `scopes::copy_moves_drop_from` and `copy_hands_off` for a
source the function owns, per path through the hand-off flags.  The reverse hand-off for a copy
of what the caller holds and the field hand-off (`OpDropAllExcept`) were removed by @PLN163 P5 and
P6: a copy the rules accept either leases or does not exist, and a read through a member of a call
result is a view of the call's record.

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
read a number restated here.**  Open [ownership.md](ownership.md)'s own `OPEN` line and its entries
before treating a free fault here as impossible; a count copied into this file goes stale.  This doc
defines the cliff; ownership.md proves the program walks the path beside it.  The
`LOFT_POISON` harness is the empirical cross-check: it overwrites freed stores with a poison
pattern so any surviving `H-FreeTwice` / use-after-free surfaces as a corrupted read.

---

## Deviations

OPEN: **0**.  The entries the register closed, and how each closed, are in
[heap-history.md](heap-history.md).  `tests/ownership_drop_gate.rs` gives every generated cell
a lease verdict and ties each cell that must release once, and does not, to exactly one open
entry.

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
  possible host view.  The guard: `tests/scripts/201-bind-copies-projection-views.loft`, which
  reads every SOURCE back after the write (an unread source cannot tell a copy from an elided
  one), on both backends.
- **An elided copy is unobservable (`H-Elide`)** — a copy off a parameter is elided only where
  no write can reach a store the caller holds while the copy lives.  An aliasing parameter, a
  view of one, a closure capture, a function reference and a generator's suspension each keep
  the copy (`use_analysis::caller_stores_stable`), and an elided copy skips its `OpCopy` and its
  drop together.  Guards: `tests/copy_lease.rs`
  `an_elided_copy_reads_its_own_value_when_the_callers_store_is_written` (the value, with
  controls that still elide) and `an_elided_copy_skips_its_hook_and_its_drop_together` (the
  hooks), on both backends.
- **The soundness bridge, on both backends (`H-Sound`, `H-FreeAll`)** — `tests/heap_sound.rs`
  runs each cell on both backends under `LOFT_POISON`, with either `LOFT_STRICT_STORES` (no slot
  is reused, so a read of a freed store is named and a store left at exit fails the run) or with
  slots reused and the exit leak check (the only way to see a free decided by a slot NUMBER a
  newer store now carries).  Its cells are the shapes that read or leaked a store on `--native`:
  a caller's buffer released by a rebind or an adopt, a loop buffer kept while a binder frees
  its store, a store handed up through a generic instance, a vector buffer bound to a forwarding
  call, a lifted result beside a value record, a copied result later rebound, and a minted store
  in a reused slot.  The nightly `native-poison` job sweeps the `--native` corpus the same way.
- **A disturbed view materialises (`H-Materialise`)** — a removal, a re-key, a reassignment and
  a GROWTH of the container while the view is live take the copy step, in this frame
  (`tests/scripts/1373-growing-a-container-ends-the-places-inside-it.loft`) and one frame
  down (`tests/scripts/164-callee-disturb.loft`, every cell paired with its inline twin).
- **Null/OOB continue (`H-ReadNull` / `H-Index`)** — reading a field of `nullref`, or `v[i]`
  with `i ≥ len(v)`, is **null** and the program continues; it never halts (operational.md
  C80, extended to the heap).  The negative half of `H-Index` — an index counts from the END
  at EITHER layout — is `tests/scripts/1669-a-negative-index-removes-from-the-end-at-either-layout.loft`;
  no guard is dedicated to the read's null answer itself, which every `??` cell in the corpus
  exercises without naming it.
- **Null/OOB write is a no-op (`H-WriteNull` / `H-WriteOOB`)** — `v[9] = x`, `v[i] = x` with
  `i < -len` or `i` the integer null, `v[9].f = x`, `e = v[9]; e.f = x` and a keyed miss
  `h[7].f = x` change nothing (value, length, neighbours), a literal written there is
  released, and a negative `i ∈ [-len, -1]` LANDS at the element from the end; identical on
  both backends and on every native fast path —
  `tests/scripts/a-write-to-an-absent-element-lands-nowhere.loft`.
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
