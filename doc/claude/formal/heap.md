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

⚠ **That discharge is only as strong as the checker's register, and the register is not at
zero.** `ownership.md` is at `OPEN: 1` — `D-own-8` (*"a Join's ownership fact is true on one
path only"*); `D-own-16` and `D-own-26` both closed 2026-09-03. What is left is a
PATH-COMPLETENESS gap, precisely the property `H-Sound` leans on. So
the free rules below are currently discharged by a checker with an open hole in the relevant
direction. Re-read that entry before treating a free fault here as impossible. This doc
defines the cliff; ownership.md proves the program walks the path beside it. The
`LOFT_POISON` harness is the empirical cross-check: it overwrites freed stores with a poison
pattern so any surviving `H-FreeTwice` / use-after-free surfaces as a corrupted read.

---

## Deviations

OPEN: **2** — `D-heap-1`, below: four shapes release a tuple member's resource TWICE; and
`D-heap-LIFO`, stated with `(H-FreeLIFO)` above, where the rule names a fault the
implementation deliberately stopped requiring.  The count read **1** while `D-heap-LIFO` was
already written and marked OPEN in the rules section — an `OPEN: n` is a claim to re-measure,
and a deviation placed beside its rule rather than under this heading is the way it goes
stale.  `D-heap-2` (a cascade that reached three of the member kinds it owned) opened and
CLOSED 2026-09-10, below; it is why D-heap-1 counts four shapes and not five.

### D-heap-1 — OPEN (2026-09-05): a copy of a tuple member releases its resource twice

`(H-Drop)` runs the hook once per resource and moves the responsibility with a copy.  A
tuple is a container like any other (`layout.md (L-Tuple)`), so `t = (s, 5); u = t` must
release once — and since loft#1361 made the whole-tuple bind COPY its heap member, the copy
is a real second record, which is what puts the question here at all.  Before that bind
copied, one record wore both names and "released once" was true by accident.

Most of the family holds: the whole-tuple bind, two droppable members, a member at index 1,
a chain of copies, a struct-enum member, a return buffer, a destructure after the copy and
the literal itself are each measured at one release on both backends
(`tests/scripts/a-copy-of-a-tuple-with-a-droppable-member-releases-once.loft`).  FOUR
shapes do not, both backends, silently — re-measured 2026-09-10, where the entry had named
three:

- a nested tuple whose droppable sits in the INNER tuple — `t = ((s, 1), 2); u = t`: twice;
- a member declared NULLABLE — `t: (S?, integer) = (s, 5); u = t`: twice;
- a copy off a tuple PARAMETER — `fn f(p: (S, integer)) { u = p; }`, where `(H-Drop)`'s
  closing clause says the CALLER owns and nothing in the callee should release: twice;
- a copy off a LOOP VARIABLE over a `vector<(τ, τ)>` — `for e in v { u = e; }`: twice.

⚠ **A fifth shape was here and did not belong to this entry.**  `h = Holder { t: (s, 5) };
u = h.t` released the resource NEVER — the OPPOSITE fault, which is why it is worth stating
that an entry saying "releases twice" cannot cover it and that a closing guard must assert the
release COUNT rather than its absence.  Isolating it showed the copy was not involved at all:
`h = Holder { t: (s, 5) }` on its own, with nothing copied out of it, already leaked.  It was
`D-heap-2` below — a cascade that never walked a tuple FIELD — and closing that closed this
cell.  The lesson is the scoping one: the cell was filed under the copy question because the
repro that found it contained a copy, and one probe with the copy removed said otherwise.

**Two of the three shapes were scoped wider than they measure**, and the controls say so:
a declared but NOT nullable `t: (S, integer) = (s, 5); u = t` releases ONCE (correct), so the
axis is the `?` and not the author's annotation; and a nested tuple whose droppable sits in the
OUTER position — `t = (s, (1, 2)); u = t` — is also correct, so it is not nesting as such but a
droppable reached THROUGH an inner tuple.

⚠ **And the LOOP-VARIABLE shape is not a tuple question either** — measured 2026-09-10, the
same double release comes off a `match` payload binding with no tuple anywhere
(`match w { WH{s} => { u = s; } }`).  What the two share is that the copy's SOURCE is a VIEW
of a container's member: `(B-Copy)` makes `u = e` a copy, `(H-Drop)` says the copy owns and the
source stops dropping, and the source's owner is a CONTAINER whose cascade releases every
element it holds — there is no per-element suppression to reach for.  A direct projection is
quiet because it does not copy at all (`u = v[0]` and `u = h.s` are `(B-View-Depth)` and
`(B-View)` views, measured at one release).  So this cell wants the rules asked first: either
the copy-out-of-a-container case is `(H-Drop)`'s `warning[double-move]` clause and the defect
is the missing warning, or `(B-Copy)` owes a view here.  It is NOT the member-pairing cure the
other three want.

**One cause, read off the IR.**  The hand-off is recognised by resolving a copy's tuple-MEMBER
source to the work-ref backing it, and the tuple's TYPE is where that pairing lives.  The
first two shapes have no pairing to read: a nested tuple's element is a `Type::Tuple` whose
backing sits on the INNER type, and a declared `(S?, integer)` reaches the binding as
`(ref(S)?, integer)` where the inferred form is `(ref(S)["__ref_1"]?, integer)`.  Note it is
the `?` that loses the dep and not the annotation — a declared `(S, integer)` keeps its
backing and releases once, which is the control above.  The third is not this frame's to read: a parameter's
deps are the CALLER's, in another dep space.  So `scopes::tuple_member_backing` DECLINES all
three rather than naming a work-ref it guessed — declining costs the hand-off and leaves the
pre-loft#1361 double release, where guessing would suppress the release of whatever local
wore that number.

**No longer branch-internal.**  This entry landed in the SAME commit as loft#1361 (`2808e1833`,
2026-09-06), so `main` has carried the copy — and therefore all five shapes — since that merge;
re-measured there 2026-09-10 on both backends.  The claim that none of them reproduces on `main`
was true only for the hours before that commit merged, which is how a "branch-internal" note
goes stale: it is a statement about two trees, and one of them moved.  Per the bug policy that keeps them here rather than in the tracker.

**Closes when** the four shapes above read exactly one release on both backends and the
guard's `@falsified-at` line covers them, scoring the COUNT rather than its absence.  The cure
for the first two is to give the tuple's element type its backing dep in the places that lose
it — the declared-type path in the binding conversion, and the nested tuple's outer element —
after which the existing `TupleGet` arm reaches them unchanged; the parameter shape wants the
argument rule instead, not a member pairing; and the loop-variable shape wants the design
question above answered before a cure is chosen for it.


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

⚠ **The over-reach cell is what found the neighbours.**  "An absent nullable field releases
nothing" was written to prove the fix had not started dropping tag bytes; it failed, and the
cause was not the cascade — reading an absent nullable through `?.` answers its type's ZERO
instead of null and mints a record whose scope end runs the hook.  Both reproduce on a bare
LOCAL (`g: S? = null; g?.h`), where no field walk exists to blame.  Those are their own
defects, tracked apart from this entry, and the cell reads with `==` so it scores the cascade.

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
  ownership.md (0 open).

D-op-1's falsifier applies here too: any program where the interpreter and `--native` diverge
on a heap step is the definitional error, and this doc is the definition it fails against.
