<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/binding.md — reference types & `&` (strict)

**Catalogue:** @F21 (references `&T`). Roadmap: @PLN87.

> **Rules then deviations** (see [README](README.md)). The governing rule: **`&` is a
> TYPE ANNOTATION, not an operator.** `&τ` is a *reference type* (a live link to a
> τ-lvalue); the `&` belongs to the **variable's type**, fixed at its binding — it is
> not something the expression grammar applies per use. The @PLN87 ladder (built in the
> `loft2` worktree, branch `tuxedo-work2`) **realises this model** and landed via PR#436
> (merged into this branch); D-bind-7, its last residual, was fixed that cycle. **D-bind: 0
> open** — `B-Ref-Reshape` (2026-08-05) declines a container disturbance under a live `&`, and
> all three of its disturbances are enforced. This doc's SECOND axis,
> `const` (@PLN40, shipped), completes the binding table alongside `&`/copy/view — its
> deviation list is now **closed (D-const: 0 open)**; D-const-1 (enum-variant enforcement
> scope) was fixed via @PLN102 K1 (see § Deviations), unrelated to the `&`-ladder.
>
> The model here is now also the one in [OWNERSHIP_MODEL.md § The law](../OWNERSHIP_MODEL.md) —
> @PLN87 rewrote it to the bind-site-link framing (the old "`&` = reassignment write-back"
> framing is gone). Design home: [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md),
> [DESIGN_DECISIONS.md C77](../DESIGN_DECISIONS.md).
> This doc is the **binding surface**; the `deps`/borrow *checker* (lifetimes) stays in
> the deferred `ownership.md`.

## Notation

- **lvalue** — an assignable location: a variable `x`, a struct field `s.x`, a vector
  element `v[i]`.
- **`&τ`** — the **reference type**: the type of a variable that is a live link to a
  τ-lvalue (read- **and** write-through). A type constructor, like `vector<τ>`.
- **alias** — a binding that SHARES the source (field/element mutation writes through).
  A plain bind is NOT an alias (it copies, `B-Copy`); you get one with `&` (`d = &v`,
  `B-Ref-Alias`), or for free when reading a struct-typed projection (`B-View`).
- **binding-const** / **value-const** — the two `const` positions (@PLN40): `const`
  before the NAME freezes the *slot* (no re-bind, contents stay mutable); `const` before
  the TYPE freezes the *value* (no through-write, whole-value rebind stays allowed). See
  `Const-Bind` / `Const-Value` below — orthogonal to `&`/alias, not a replacement for it.

---

## Rules

### `&τ` is a type; `&` is its annotation (NOT an operator)

```
  (B-RefType)        `&τ` is a TYPE — a live link to a τ-lvalue.  It is part of the
                     type system (a type constructor), written with a leading `&`.
                     There is NO `&` operator in the expression grammar.
  (B-RefType-OfVar)  a variable or parameter HAS type `&τ` for its whole lifetime; the
                     reference-ness is a property of the VARIABLE (its type), fixed at
                     the binding — never a per-expression operation.
```

**In words.** `&integer` is a *type* — the type of a variable that is a live link to some
integer, the same way `vector<integer>` is the type of a vector. The `&` belongs to the
variable's type and stays there for the variable's whole life; it is never an action you
perform on a value. There is no `&` operator.

### Introducing a reference (at a binding, not by evaluating an expression)

```
  (B-Ref-Intro)      a `&`-annotated binding gives the bound variable type `&τ` LINKED
                     to an lvalue:  fn f(b: &integer)  ·  b = &a  (b : &(typeof a)).
                     It records "b's type is a link to a" — it does NOT evaluate `a`
                     into a fresh value.
  (B-Ref-Lvalue)     the linked source is an lvalue (variable / field / element).
  (B-Ref-AnnotationOnly)  ⚑ VITAL.  `&` occurs ONLY as a reference-type annotation — in a
                     type (`&τ`) or at the bind site that gives a variable that type.  A
                     unary `&` in ANY other position is a PARSE ERROR: an operand
                     (`x + &y`, and `1 + &y` — the position does not matter), a collection
                     element (`[&a]`), a call/format argument (`f(&a)` passes a value, not
                     a `&`-prefixed expression), a condition (`if &a > 0`), a bare
                     statement (`&a;`), a `return &a;`, a COMPOUND-assignment right-hand
                     side (`b += &a` — it mutates `b`, it does not give `b` a reference
                     type, so there is no binding to annotate), an assignment TARGET
                     (`&x = 3`).  There is NO `&` operator; permitting `&` as a
                     value-level prefix is precisely what lets the reference leak into
                     contexts that then mis-elaborate.
  (B-Ref-StoredRef)  the ONE position outside a `&τ` binding where a prefix `&` is legal
                     is a struct-literal field whose declared type is `reference<τ>`:
                     `Linked { link: &pool[i] }`.  That is a DIFFERENT type former from
                     `&τ` — `Type::Reference`, a stored cross-store pointer, versus
                     `Type::RefVar`, the stack link the rest of this doc is about — and
                     the field's type is what admits the `&`, so a `&` in a field of any
                     other type is still B-Ref-AnnotationOnly's parse error.
  (B-Ref-NotTarget)  (instance of B-Ref-AnnotationOnly) `&x = 3` is an error — a type
                     annotation lives at a binding, not on an lvalue being written.
```

**In words.** You make a reference by writing `&` at a binding — `b = &a`, or a
`&integer` parameter — which gives `b` a link to `a`; it does *not* read a value out of
`a`. Because `&` is a type annotation and not an operator, it is allowed *only* there:
writing `&` anywhere else (`1 + &a`, `[&a]`, `f(&a)`, `&x = 3`, …) is a parse error. The
single other place the token is legal is a field declared `reference<τ>`, where it is a
different type former doing a different job (`B-Ref-StoredRef`).

### Using a `&τ` variable — the link is carried by the type

```
  (B-Ref-Read)    reading a `&τ` variable yields the source's CURRENT value (live):
                      a = 3; b = &a; a = 5;  b == 5
  (B-Ref-Write)   writing a `&τ` variable writes the SOURCE — the NORTH STAR:
                      a = 3; b = &a; b = 4;  a == 4
                  At a HEAP τ the write REPLACES the source's contents; it does not
                  re-point the link, and the value it displaces is released:
                      c = "a"; pc = &c; pc = "z";        c == "z"
                      d = S{n:1}; pd = &d; pd = S{n:2};  d.n == 2, the first record freed
                      e = [1]; pe = &e; pe = [2, 2];     len(e) == 2, in e's OWN store
                  That is this rule read at τ = a heap value rather than a second rule,
                  and it is what the `&τ` PARAMETER write-back has always done
                  ([calls.md](calls.md) F-ParamRef).  It went unstated long enough for the
                  LOCAL bind to answer three different ways — a silent copy for a text, a
                  re-point for a vector, an orphaned record for a struct (loft#1371).
  (B-Ref-Uniform) a `&τ` variable is used EXACTLY like a τ variable — read, write,
                  field/element mutate — and every operation goes through the link via
                  the EXISTING mutation code.  The TYPE carries the linkage; no
                  operation is special-cased and the mutation code is unchanged.
```

**In words.** A linked variable is a window onto its source: read it and you see the
source's current value; write it and the source changes (`a=3; b=&a; b=4` leaves
`a==4`). You use it like any normal variable — the link is invisible at the use site
because it lives in the type. (In the type system, this read-through is the conversion
rule `C-Ref` in [types.md](types.md): a `&τ` is accepted wherever a `τ` is.)

### `&` vs the default — a plain bind COPIES; `&` links; a struct projection views

```
  (B-Copy)        a PLAIN bind COPIES the source — a scalar (`d = a`) AND a heap
                  WHOLE-VALUE (`d = v`, `d = self.data`, a struct-enum `c = e`): the
                  bound variable is INDEPENDENT, and mutating it does NOT reach the
                  source.  This is [heap.md](heap.md) H-Copy (`fv = e.items; fv[0]=99`
                  leaves `e.items[0]`).  A struct-enum value is a heap RECORD exactly as
                  a struct is — `Type::heap_def_nr` names both — and a `(C-Var)` widening
                  `c: E = s` from a variant is a copy like any other.  For a type that
                  owns a droppable without `OpCopy`, such a bind is a compile-time error on
                  its own line, whatever follows it ([heap.md](heap.md) H-Copy-Refuse); with
                  `OpCopy` it takes a second lease (H-Copy-Lease).
  (B-Ref-Alias)   the `&τ` annotation makes ANY binding — scalar OR heap — a live LINK
                  to the source instead of a copy.  `d = &v` / `d = &self.data` ALIAS the
                  vector: `d[i] = x` (and `d += …`) write THROUGH to the source, which is
                  NON-OWNING (the source frees the store).  `&` is how you OPT INTO
                  aliasing; without it a heap bind copies.  This is B-Ref-Write for a
                  vector lvalue.
  (B-Ref-Repoint) `p = &q` on a variable that IS a `&τ` link RE-POINTS the link to `q`:
                  the link's own slot takes the new cell, the old source is neither
                  written nor released (a link owns nothing, B-Ref-Alias), and reads
                  and writes through `p` reach `q` from then on.  The `&` on the right
                  is what tells a re-point from B-Ref-Write's write-through:
                      a = S{n:2}; b = S{n:3}; p = &a; p = &b; p.n = 9;  a.n == 2, b.n == 9
                      c = "ab"; d = "cde"; pc = &c; pc = &d;           len(c) == 2, len(pc) == 3
                  Every τ alike — scalar, text, vector, record — and both backends.
  (B-View)        a STRUCT-typed PROJECTION (`s = o.inner`, `e = v[i]` where the element
                  IS a struct) is a VIEW that aliases WITHOUT `&` ([heap.md](heap.md)
                  H-View: `c = o.i; c.v=9` ⇒ `o.i.v==9`) — the one place aliasing is the
                  default, because a struct projection names an interior place, not a
                  fresh whole value.  The alias lasts as long as the PLACE: where the
                  container is DISTURBED (B-Disturb) while the view is still live, the
                  binding MATERIALISES — it is given its own copy, taken at the bind,
                  and the author is told — so writes through it stop reaching the
                  container (@PLN130 F2/F4/F8).  A plain bind already copies, so this
                  is consistent with what it meant; a `&` gets B-Ref-Reshape instead.
                  A `?`-DISCHARGED element read (`e = v[i]?`) is the same projection with
                  its absence discharged and is a VIEW on the same terms: the PRESENT
                  element is viewed, and an ABSENT one discharges to the type's DEFAULT
                  RECORD, built in a per-site buffer of this frame's (LOFT.md § `?`,
                  `points[i]?` ⇒ `Point{}`) — not to null, which is what this clause said
                  until @PLN164 C3 measured every backend answering the default.  There is
                  nothing of the container to view on that arm, so the buffer is what the
                  binding names there, and no copy is taken on either.
                  A view of a member that owns a droppable without `OpCopy` never
                  materialises: disturbing its container while the view is used is an error
                  at the disturbance ([heap.md](heap.md) H-View-Drop).
                  A GENERATOR HANDLE read out of a member (`h = t.g`, `h = tasks[i]`, a
                  `for` over a collection of handles) is a view on the same terms: it names
                  the member's frame, and the member releases it ([coroutines.md](coroutines.md)
                  G-Hold).
  (B-View-Base)   a projection off a BORROWED base is a VIEW at EVERY element type — not only
                  a struct-typed one.  `for b in bv { c = b.vecf; … }` aliases exactly as
                  `c = b.strf` does, and so does a tuple element.  Ownership of the BASE is the
                  axis: off an OWNED base a COLLECTION projection copies (B-Copy, `af = bx.v`,
                  and `af = bx.v ?? d` the same — D-own-35)
                  while a STRUCT projection views (B-View); off a BORROWED base everything
                  views.  `classify_vec_bind`'s `depend().is_empty()` is where the parser asks
                  it, and @PLN25 p379 depends on the write-through (`cells = sc.v;
                  cells[i] = h`).
  (B-View-Depth)  a vector INDEX read (`a = vv[0]`) and a NESTED field read
                  (`c = o.inner.v`) are VIEWS whatever the element type — #426's RESOLUTION,
                  whose FILED premise (*"these must COPY"*) was recorded as the wrong read:
                  under the reference-default model a binding to a heap value aliases the
                  source and in-place mutation writes through.  A view survives the realloc
                  of a container it does NOT name — `a = vv[0]; vv[0] += [9]` grows the INNER
                  vector and `a`, which names the outer element SLOT, reads the repointed
                  handle (`85-store-lifetime-reference-default-views.loft` cell A, and this
                  is the only realloc it measures).  The realloc of the container the view
                  DOES name ends the place instead, and is B-Disturb's fourth event.
                  Also guarded by `294-vector-element-view-semantics.loft`.
  (B-Disturb)     four events END the place a reference names, and they are the same
                  four for every rule below: REMOVING from the container (`v.remove(i)`
                  renumbers every later position — collections.md Col-Remove),
                  GROWING it (an append, an insert or a keyed add: a container that
                  outgrows its allocation is copied into a larger record and every
                  element moves), RE-KEYING an element (writing a key field: the record
                  moves, or becomes reachable by no key), and REASSIGNING the container
                  itself (`bx = T{…}` leaves the place with nothing to point at).
                  Overwriting a place is NOT disturbing it: `o.inner = Box{…}` writes
                  INTO the place `o.inner` already occupies, so a view of it survives.
                  ANY growth disturbs, not only one that provably crosses the capacity:
                  whether it reallocates is an allocator fact the author cannot see, and
                  a rule that answered differently on either side of it would give one
                  program two meanings (measured — `d: S = v[0]` read `1` after two
                  appends and `4294967296` after two hundred, loft#1373).
                  And an event disturbs WHEREVER IT HAPPENS — in this frame, or in
                  anything the frame CALLS, at any depth.  B-Ref-Reshape states this for
                  the refusal and B-View inherits it for the materialise: the same
                  `e = sc.els[0]?; grow(sc); e.a` that answers `3` with the append written
                  inline read `4294967401` with it one frame down, on both backends, until
                  @PLN164 C3 gave the walk the callee's half of the question.  Which side
                  of a call a statement sits on is not one of the four events.
  (B-Ref-Reshape) DISTURBING a container while a `&` reference into it is still LIVE is
                  a COMPILE-TIME ERROR.  These are the shapes where B-Ref-Alias could
                  not hold, and declining them is what makes B-Ref-Alias unconditional
                  with no runtime machinery.  Taking a `&` is the author's OWNERSHIP
                  DECISION, and this is its consequence: loft will not quietly downgrade
                  the reference to a copy, so where it cannot honour the write it
                  declines the program (C79, revisited 2026-08-05).  LIVENESS is the
                  condition, not existence — `c = &v[0]; c.n = 1; v.remove(0);` is fine,
                  because the reference is dead at the disturbance.  The disturbance may
                  be in this frame or in anything the frame CALLS (`f(v[i], v)` where
                  `f` removes from its container parameter, at any depth).
                  A plain LOCAL bind is exempt and keeps compiling — it materialises
                  (B-View).  A plain PARAMETER is NOT exempt: it aliases the caller's
                  element exactly as a `&` one does (calls.md F-ParamHeap), so the rule
                  keys on the aliasing relation, not on the token.
```

**In words.** Binding copies by default — a scalar and a whole vector alike (`d = v`
gives you an independent copy). Writing `&` at the bind turns it into a live link, so
`d = &self.data; d[i] = x` writes through to the source (a game can grab a sub-vector and
mutate it in place).

**The exceptions are not one but three, and stating only the first is what made three separate
correct behaviours read as bugs in one week** (D-bind-12's collection half, a nested field read,
a vector index read — all three filed against `B-Copy` and all three correct).  A projection is
a VIEW when *any* of these holds: its type is a STRUCT (`o.inner`, B-View); its base is
BORROWED, at every element type (B-View-Base); or the read is an INDEX or NESTED one
(B-View-Depth, #426).  What is left for `B-Copy` is a whole VALUE, a scalar, and a one-level
COLLECTION projection off an OWNED base — which is exactly `OWNERSHIP_MODEL § The law`'s
`af = bx.v`.

**The whole boundary is pinned in one place:**
`tests/scripts/bind-copies-or-views-the-whole-boundary.loft`, seventeen cells, measured identical
on both backends.  Ask it rather than re-deriving: the cells existed before, scattered across four
files, and no single one said what the rule was.

Both kinds of alias last exactly as long as the place they name, and the four things
that end a place are the same for both (B-Disturb). What differs is the answer. A plain
view gets a **copy** and is told so: it already meant value semantics, so losing
write-through is consistent. A `&` gets an **error**, because it did not — the author
asked for a live link, and silently handing back a copy would make that request a lie.
That is the consequence of writing `&`: it is an ownership decision, so loft declines the
program rather than quietly changing what it means.

**LIVENESS is not lexical, and reading it as lexical is what makes the materialise miss.** A
view lives as long as its VARIABLE, not as long as the block the binding was written in: a
re-bind of an outer local inside an `if` or a loop body still names a place that the code after
the block can read. And a LOOP body's own disturbances precede every use on the NEXT turn, so
the back edge is a second disturbance a single forward reading cannot see. Both halves of that
are what `a = w.inner` inside `for … { w = Outer{inner: a}; a = w.inner }` needs: the container
is reassigned around a live view, so B-View materialises it and says so. Pinned by
`tests/scripts/1184-a-view-assigned-back-onto-its-own-source.loft`, whose last two cells are the
control — an UNDISTURBED view still aliases and still writes through, on both backends.

### `const` — the immutability axis (binding-const vs value-const, @PLN40, shipped)

`&` (above) and `const` are the two orthogonal axes of a binding: `&` opts a binding
*into* write-through aliasing; `const` opts a binding *out of* writes, at one of two
independent positions. The FIELD axis (`const v: T` / `v: const T` on a struct field) is
a struct-attribute property, documented in [LOFT.md § Fields](../LOFT.md) (the
four-quadrant table), not this doc's `&`-binding surface. The PARAM axis (`p: const T`) is
parameter-binding — see [calls.md](calls.md) (parameter binding) and
[LOFT.md § Functions](../LOFT.md) for the const-param prose; today only VALUE-const is
wired for parameters (binding-const params are not yet, see Const-Bind below). Design
source: [../plans/40-const-fields/const-model.md](../plans/40-const-fields/const-model.md).

```
  (Const-Bind)            `const` BEFORE THE NAME is binding-const: the SLOT never
                          re-points.  `const v: T` (field), `const x: T` (local); a
                          binding-const PARAMETER is not yet wired (Phase 1 shipped
                          value-const params + binding-const locals/fields only).
                          `t.v = other` is a compile error ("cannot reassign const
                          field '…' of struct '…' — const fields are
                          write-once-at-construction"); the CONTENTS stay mutable —
                          `t.v += […]` (append) and `t.v[i] = x` (element write) are
                          allowed.  The immutable-binding sibling of B-Copy: B-Copy's
                          copy is independent but still freely re-bindable;
                          Const-Bind additionally forbids re-binding it.
  (Const-Value)           `const` BEFORE THE TYPE is value-const: the VALUE is
                          read-only through this name.  `v: const T` (field),
                          `p: const T` (param), `x: const T` (local).  Every
                          through-write is rejected — a direct mutation
                          ("cannot mutate value-const field '…' of struct '…' — its
                          value is read-only (rebind with '=' to re-point, or drop
                          'const')") and a mutation reached BY DEREFERENCING THROUGH
                          a value-const field, `t.v[i] = …` / `t.r.x = …`
                          ("Cannot modify value-const field '…'; its value is
                          read-only") — but a WHOLE-VALUE rebind `t.v = other` is
                          ALLOWED (it re-points the slot; it does not touch the old
                          value).  The read-only dual of B-Ref-Alias: where `&` opts
                          a binding INTO write-through aliasing, `const` on the type
                          opts it OUT of every through-write.  "Every" includes a
                          write reached through a NULL DISCHARGE — `h.i?.x = …`
                          binds to `h` exactly as `h.i.x = …` does — because the rule
                          is about the write's ROOT, and a discharge changes what a
                          read ANSWERS, not which binding a write travels to
                          ([operational.md](operational.md) `E-Asgn-Discharge` is the
                          separate question of a discharge that IS the target).
                          While the resolver stopped at the discharge and answered
                          "no binding at all" this went unenforced and a `const`
                          parameter was mutated in silence (loft#1211).
                          "Through this name" includes every VIEW of the value —
                          a loop variable over its elements, an element or field
                          bound to a local, a `&` link — because a view names the
                          same place (`B-View`); a COPY out of it (`B-Copy`, a
                          scalar or `text` read) is the reader's own.  And the value
                          may not be handed to a `&` parameter, whose callee writes
                          it (D-bind-44; the plain-parameter half is D-bind-45).
  (Const-ScalarCollapse)  a by-value SCALAR (`integer` / `float` / `single` /
                          `boolean` / `character`) has no interior distinct from its
                          binding, so it freezes FULLY under EITHER axis:
                          `const n: integer` AND `n: const integer` both reject
                          `t.n = …` AND `t.n += …`.  (`text` is compound here, not
                          scalar — `const body: text` still allows `+=` append.)
  (Const-Compose)         `const v: const T` composes BOTH axes — Const-Bind rejects
                          the rebind, Const-Value rejects every through-write — so
                          the field is FULLY immutable: neither `t.v = other` nor
                          any mutation beneath it is accepted.
  (Const-ConstructExempt) construction lowers via a SEPARATE path (`Value::Insert`),
                          not the reassignment guard (`validate_write` /
                          `const_write_blocked`), so write-once is SET at
                          construction, not CHECKED there — `T{ v: 1 }` never
                          reaches the reassignment guard regardless of `v`'s
                          const-ness.
  (Const-VirtualReject)   `const virtual(...)` is a compile error: a
                          `virtual`/computed field is already read-only (no storage
                          to freeze), so `const` on it is redundant and rejected
                          rather than silently accepted.
```

**In words.** `const` is orthogonal to `&`: writing `&` at a bind opts INTO write-through
aliasing (`B-Ref-Alias`); writing `const` opts OUT of writes, at one of two positions.
Put `const` before the NAME (`const v: T`) and the *slot* freezes — the field can never be
re-pointed, but if the slot holds a collection or text you can still grow it in place
(`t.v += …`, `t.v[i] = x`); this is the builder shape (a `Mesh.verts`-style accumulator
grown after construction). Put `const` before the TYPE (`v: const T`) and the *value*
freezes instead — you may swap in a whole new value (`t.v = other`), but you can never
reach in and mutate the one that is there, at any depth (`t.v[i]=`, `t.v.x=`, `t.r.x=` are
all rejected). A plain scalar has no "interior" apart from its binding, so the two
positions collapse to the same fully-frozen behaviour for it (`Const-ScalarCollapse`). The
two axes compose (`const v: const T`) into a genuinely immutable field. Construction is
exempt by construction, not by a special case: a struct literal writes through a
different lowering path (`Value::Insert`) that the reassignment guard never sees, so
"write-once" really means "unchecked during construction, checked on every write after."

### Pattern captures (@PLN35, SHIPPED)

> **@PLN35 · SHIPPED.** These two rules were written spec-first, ahead of the code; the code
> landed with phases 1–7 + PC1–PC5 ([matching.md § Rules — PEG patterns](matching.md)) and obeys
> both — verified on both backends: a `[first, ..rest]` capture of a struct element writes
> THROUGH to the subject (`P-Cap-View`), and mutating the captured `rest` leaves the subject
> untouched (`P-Cap-Fresh`). Design:
> [../plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md).

```
  (P-Cap-View)   a SINGLE structural capture that names an INTERIOR place of the subject (a struct
                 field, a struct-typed element) is a VIEW (B-View / heap.md H-View): it aliases
                 WITHOUT `&`, and carries the subject's borrow-dep (`Deps::frame1(subject)`) so both
                 backends agree on free.
  (P-Cap-Fresh)  a `..rest` sub-slice and a repetition `(a)*` accumulator are FRESH vectors
                 (heap.md H-Alloc), INDEPENDENT of the subject (B-Copy / iteration.md I-Comp).
```

**In words.** Binding a single interior piece of the matched value — a field, a struct element — is
a *view* onto that place, exactly like reading `o.inner` today (`B-View`): no copy, and a mutation
writes through. A `..rest` tail or a repetition's collected vector is instead a *fresh* vector,
independent of the subject — the same "fresh result vector" a comprehension builds (`I-Comp`) — so
mutating a captured `rest` never touches the original. This split keeps the cheap case cheap while
avoiding an interior-sub-slice lifetime that neither backend models cleanly.

---

## Deviations

**OPEN: 2.**

* **D-bind-50** *(opened 2026-09-22, CLOSED 2026-09-22; loft#1612)* — `(B-Copy)` for the
  destination of a `??` CHAIN of three or more operands.  A plain bind copies a heap whole
  value, and the two-operand spelling does: it is lowered per arm, and an arm's bind is that
  copy.  `??` is left-associative, so a longer chain hoisted its subject into a `__ncc_N` temp
  and bound the destination to THAT — and the temp views the operand it chose, by the rule that
  makes it a borrow (loft#723: freeing it would be a use-after-free).  So the destination shared
  the chosen operand's store: on the interpreter for a record, on BOTH backends for a vector and
  for a chain whose first operand is a call.  Silent — the shared store reads right until the
  operand is rebound, and then the destination reads the new value, or freed memory once both
  release it.  `(O-NoDiverge)` was broken with it, since `--native` copies a record's temp.
  **Closed** in the PARSER, by making `??` RIGHT-associative (`grammar.md` (G-Assoc): its RHS
  parses at its own level, the one place associativity is decided).  `??` is associative as a
  value — either grouping answers the first present operand, in the same order, short-circuiting
  the same way — so no program's meaning moves; what moves is the SHAPE.  Right-associated there
  is no chain subject to hoist: every operand is an ARM of a plain `if`, and an arm's bind is the
  copy `(B-Copy)` describes, which is exactly why the two-operand spelling was always right.
  `LOFT_COALESCE_LEFT_ASSOC=1` parses the left-associated form again.
  A chain the AUTHOR parenthesised — `(x ?? y) ?? d` — still hands the parser a subject that is
  not a variable, so that one is right-associated in the scopes pass instead
  (`reassociate_coalesce_chains`, loft#1591's rewrite with its destination gate lifted;
  `LOFT_NO_COALESCE_REASSOC=1` keeps the gate).  Without it the parenthesised spelling kept the
  hoisted form and the destination VIEWED the operand — on the interpreter only, so it was a
  `(O-NoDiverge)` divergence as well (guard cell `c27`).
  A SECOND defect was in the way, pre-existing and reachable by hand as `b = x ?? (c() ?? d())`:
  a chain BLOCK as an arm is not a tail a plain bind may take (its tail is the compiler temp),
  so `sink_set_into_arms` declined the whole branch — and then the chosen VARIABLE's arm beside
  it recorded no hand-off, so at a droppable type `x`'s record was released from the destination
  AND from `x`, the second release on freed memory (`M8,R8,D8,D8`; loft2-21 measured this shape
  on the first cure, and a value cell cannot see it — which is why the guard's droppable cells
  exist).  Closed with it: the chain block is sunk as a UNIT, the arm binding the join exactly as
  a statement of its own would, and a chain block handed to the sink as the VALUE declines, since
  that statement is already what a sunk arm writes.  Guard
  `tests/scripts/1612-a-coalesce-chain-copies-the-operand-it-chooses.loft`.
* **D-bind-49** *(opened 2026-09-21, CLOSED 2026-09-21; loft#1554)* — the CALL-SITE half of
  `(B-Ref-Reshape)` read one spelling of one event.  A call handed both a container and a
  reference into it (`shift(v[2], v)`) was refused only where the callee REMOVES through a bare
  `&vector` parameter — `removed_params_map`, built from `OpRemoveVector`/`OpRemove` over a `Var`
  typed `RefVar`.  The rule exempts no spelling (*"A plain PARAMETER is NOT exempt"*), `(B-Disturb)`
  names four events *"wherever they happen"*, and `calls.md` `F-ParamHeap` states the call-site
  reach in so many words — so every other shape was a deviation, and a silent one: the program
  compiled and read or wrote the element that moved, identically on both backends.
  **Measured**, 14 hazard cells and 9 controls, on the build before the cure: ONE hazard refused
  (`t = &v[2]; shift(t, v)`, and that by the FRAME half).  The other 13 compiled — a plain
  `vector` container (a lost write: `33` for `99`), a struct FIELD as the container, the field
  itself handed in (`shift(b.items[2], b.items)`), a GROWTH of either (`11` for `99`; the issue's
  `0` once the vector reallocates), a growth two frames down, a `&vector` growth, a `&` element
  parameter, an element bound earlier from a field, a reference INTO an element
  (`shift(v[1].inner, v)`), a struct-enum element, a `?`-discharged element, and the `&vector`
  removal itself once the call sat inside a FORMAT STRING, where the element argument is the
  nullable read `OpGetVectorNullable` — a second spelling the element test did not know.
  **Cure:** the call-site half reads `disturbed_params_map` (@PLN164 C3's fact, the one the frame
  half and the materialise already read), unioned with `removed` so `LOFT_NO_CALLEE_DISTURB=1`
  keeps the narrower refusal; the argument's place is composed with the callee's
  (`call_arg_place` · `compose_param_place`), and the element test names places through
  `value_view_places` (`arg_references_element_in`, `names_element_in`) — a local bound earlier
  from a field is resolved by its own bindings, an unresolved one is taken to name the container.
  No new fact and no new predicate.  The message gained the growth form (*"`stash` grows `bag`,
  and a container that outgrows its allocation moves every element…"*); the removal form is
  byte-identical.
  **What keeps it from over-reaching, each a compiling control with a hand-computed answer on
  both backends:** a SIBLING field's growth (`100`), a callee disturbing ANOTHER container
  parameter (`94`), a SCALAR read out of an element (`35`), an element of another container
  (`123`), a callee that only reads, scalar elements, and the index workaround the message names.
  A call through a FN-REF is the one edge the refusal cannot follow; it compiles and loses the
  write (`33`), as before.
  **Blast radius (measured, not predicted):** `loft --check` over the 1628-file corpus names
  **0** files (the harness proven able to fail on a hazard cell first); over the consumer sources
  (crawler, dryopea, moros, zero-trust-shared-files, loft-libs-*) every file that parses names 0,
  and a syntactic scan of all 325 for a call handed an indexed element beside its container or a
  prefix of it finds ONE site, whose element argument is an integer (`hud.itex[i] ?? 0`).
  ⚠ **How it stayed open while reading CLOSED.**  This was cured once, on 2026-09-17, and the
  cure was lost in a JOIN: two streams had each numbered a D-bind-47 the same day, the join kept
  the other stream's `scopes.rs` and merged the PROSE — so D-bind-48's *"Two shapes the call-site
  half also missed"* paragraph below and `calls.md` both described a call-site half that read
  `value_view_places` while the code read `removed`, the six tests were gone, and the issue
  closed on a `Fixes` trailer with `main` still answering `2 2 0` on its own repro.  Nothing
  gated the difference: a register entry is prose, and the merged tree's guards were the other
  stream's.  The check that found it is the cheap one — run the issue's repro on the joined
  tree — and it belongs to every join that squashes a stream carrying a `Fixes`.

* **D-bind-48** *(opened 2026-09-17, CLOSED 2026-09-17)* — `(B-Ref-Reshape)`'s CALLEE clause was
  never implemented for a GROWTH, so a disturbance one frame down did not refuse.  The rule states
  the reach outright — *"The disturbance may be in this frame or in anything the frame CALLS"* —
  and `(B-Disturb)` states it for all four events: *"an event disturbs WHEREVER IT HAPPENS … at any
  depth."*  @PLN164 C3 gave that reach to the MATERIALISE walk and not to the refusal.
  **Measured**, 17 cells, both backends byte-identical.  The purest is all-`&`, with the callee
  disturbing the very parameter it was handed: `fn vgrow(v: &vector<H>, n) { v += [mk(n)] }` under
  a live `e = &v[0]` compiled and released one resource TWICE (`M1 M2 R1 D1 D1 D2`).  The droppable
  population `(H-View-Drop)` owns the same shape without the `&`, and a callee's REMOVAL from a
  FIELD of a parameter was not refused either — `removed_ref_params` keys on
  `OpRemoveVector(arg0)` / `OpRemove(arg1)` over a bare `Var` typed `RefVar`, so producer 1
  reached only a parameter named DIRECTLY (`fn drop_last(all: &vector<Box>) { all.remove(2) }`,
  pinned by `b_ref_reshape_callee_removal_under_local_amp_link_is_error`).  Producer 2 — a call
  handed BOTH a container and a reference into it, `shift(v[2], v)` — is a separate route with its
  own message and was never in question here.
  **Where:** `def_reshape_refusals` ran `ViewWalk::run(..., Some(removed), None, …)` — `removed`
  passed, `disturbed` withheld.
  **Two shapes the call-site half also missed, and the blast radius, measured before landing.**
  Its element test did not count the nullable element read a FORMAT string passes
  (`print("{shift(v[2], v)}")` printed the moved element's stale bytes) nor a `?`-discharged one;
  the test reads the place through `value_view_places` now, and a SIBLING field's growth still
  compiles.  Over the 1629-file corpus and the consumer sources (crawler, dryopea, moros,
  zero-trust-shared-files, loft-libs-*) the wider refusal names ONE program —
  `164-element-place`'s `g27`, written to hand a container and its element in, now called
  through a fn-ref, which is the one edge the refusal cannot follow.
  ⚠ **The lesson, and it cost the first reading of this defect a much larger cure.**
  `ViewWalk::shake_plain_places`'s doc SAID it works *"over PLAIN views only, leaving every `&`
  link alone"*, which described its INTENT for the materialise consumer and not what it does.  The
  code has no `&` test anywhere: `shake_places_keyed` builds its hit list from `same_place &&
  !spared && !names_container_itself`.  So a link INTO a disturbed container was already shaken and
  already MATERIALISED — the silent `&`-to-copy downgrade this rule exists to forbid, emitted with
  the copy-out advice — and what was missing was only a CONSUMER reading that answer.  A comment
  that states intent where a reader will take it for behaviour is worth more care than a wrong one,
  because it is believed.  That doc now states what the function does: the shake is followed by a
  restore keyed by VIEW over `whole_container`, which is `(B-Ref-Alias)`'s in-versus-to distinction
  and not a `&`-versus-plain one.  The quotation is kept in the PAST tense deliberately — nothing
  gates a doc that quotes a code comment, since `check_doc_drift.sh` reads plan links, time
  projections and retired-feature claims, so the next edit of that comment cannot report this line.
  **Cure:** `reshape_refusals` builds `disturbed_params_map` and threads it into
  `def_reshape_refusals` → `ViewWalk::run`'s fifth argument.  Two hunks; no new fact and no new
  predicate.
  **What keeps it from over-reaching, both measured as controls:** `names_container_itself` still
  spares a link TO a container, so `157-view-header`'s `grown_between` keeps reading 11 — D-bind-46's
  in-versus-to distinction does the work — and the gate is unchanged (`amp || drops`), so a
  NON-droppable plain view across a callee's growth still compiles, still materialises and still
  says so.  That last cell is the one that decides the unit, and it holds.
  **Consequence worth stating:** the refusal reads the same `callee_disturb_enabled` switch as the
  scope pass, so `LOFT_NO_CALLEE_DISTURB=1` restores pre-C3 blindness on BOTH sides at once and is
  no longer a clean A/B for the materialise alone.  That is the switch's honest meaning — one
  rule's reach, two consumers — and the halves stay tellable apart at the symptom, a refusal being
  loud where a materialise is quiet.
  **Blast radius (measured, not predicted):** corpus A/B over 1591 files, **0 changed** — no
  program in the tree writes the shape, which is why the corpus could not have caught it and is
  also why it is not evidence of safety for real code.
  The message gained a population-dependent joiner: the callee form names the callee's act before
  the reason, and for a droppable view the growth CAUSES the copy (`so`) where for a `&` link the
  two are parallel facts (`and`).  With one joiner both read *"would grow `b`, and … and …"*.
  Found while re-measuring `heap.md` D-heap-11, whose own **Boundary** paragraph described this
  asymmetry incorrectly and is corrected there.

* **D-bind-47** *(opened 2026-09-17, CLOSED 2026-09-17)* — `(B-Ref-Reshape)`'s refusal could not
  see a GROWTH of a container held in a FIELD, so the rule's answer depended on where the
  container was stored.  Measured on both backends, four cells varying only the container's home
  and the disturbance: `v = [...]; c = &v[0]; v.remove(1)` refused, `v += [x]` refused,
  `b.v.remove(1)` refused — and `c = &b.v[0]; b.v += [x]` **compiled**, materialising the link
  into a copy with the copy-out advice.  So a `&` was silently downgraded exactly where this rule
  says REFUSE: a write through the link is lost, and where the element owns a droppable the
  resource is released TWICE (`M1 M2 R1 D1 D1 D2`, both backends).  `is_amp_link` is not the axis
  — the identical binding refuses under `remove`.
  **Where (measured).**  A growth names its container by field NUMBER (`OpNewRecord(b, tp, 1)`)
  while a view carries a byte OFFSET; `Stores::field_position` is the only converter, and
  `grown_containers` returns early without a store, leaving every field-qualified growth
  UNCOLLECTED.  `def_reshape_refusals` ran `ViewWalk::run` with `database: None`, which
  `binding-history.md` records as deliberate — *"the `&`-refusal path has no store to convert with
  and keeps the conservative answer, which for a REFUSAL is the safe direction"*.  That is an
  AVAILABILITY premise, and it no longer holds: the parser owns `pub database: Stores` at
  `check_reshape_under_reference`, the layouts are registered when each struct is declared, and
  `field_position` answers `u16::MAX` — *cannot say* — for anything it does not know.  A missed
  disturbance is still the safe direction; it was not a reason to leave one whole class invisible.
  **Cure:** the refusal walk is handed the same store the materialise walk has always had.  The
  callee reach (`disturbed`) stays `None` deliberately — that is @PLN164 C3's separable widening.
  **Blast radius (measured, not predicted):** one corpus file, this rule's own guard
  `a-link-to-a-whole-container-survives-that-containers-growth.loft`, whose control cell
  `a_link_into_the_container_is_unchanged` asserted the unrefused answer in so many words
  (*"keeps today's answer"*).  It pinned the compiler, not the rule, and is now
  `parse_errors::b_ref_reshape_growth_of_a_field_container_under_amp_link_is_error` — a cell that
  refuses belongs in the refusal harness, because it takes a whole `.loft` file with it.
  Found while measuring `heap.md` D-heap-11, whose cure reuses this gate.

* **D-bind-46** *(opened 2026-09-16, CLOSED 2026-09-16)* — `(B-Ref-Alias)`'s in-versus-to
  distinction at a container held in a FIELD.  `d = &cv.data; cv.data += [7]; cv.data += [8];
  (d[2] ?? -1) + len(d)` read **0** where 11 is right, on both backends, with the copy-out advice
  rather than silence.  The SAME body with the two appends moved into a CALLEE read 11 — and that
  half is pinned by `157-view-header`'s `grown_between` — so one program had two meanings
  depending on which side of a call the append sat on.  A `remove` under the live link behaved
  the same way, and a write through the link after a growth was lost (`d[0] = 99` left `cv.data[0]`
  at 1).  **Where (measured).**  `record_target` already draws the distinction the rule needs —
  *"`pe = &e` names no container while `pw = &w[0]` names `w`"* — but through `base_container_var`,
  which reaches only a whole VARIABLE.  A link to a container held in a field IS a projection, so
  it named `(cv, off_data)`: the same place `cv.data[0]` names, the place model carrying one
  variable and one field OFFSET.  `(B-Disturb)`'s growth ends only the second of those — it moves
  every ELEMENT, while it merely repoints the field SLOT that a reference TO the container
  re-reads.  **Closed** by reporting, off the walk that already answers the place
  (`projection_place_of`, exposed as `use_analysis::view_source_place_indexed`), whether the chain
  read an element; `ViewWalk` records the place such a binding names DIRECTLY and
  `names_container_itself` spares it from `Grown` and `Reshaped`.  `Reassigned` still shakes it —
  that is the one event which leaves the slot itself with nothing to point at, and sparing it
  would hand out a link to a store the reassignment released.  The DIRECT place is matched, never
  `resolve_view_root`'s: a binding whose own container is a view resolves to the OUTER container,
  and growing THAT does move the record holding this binding's slot.
  Measured on both backends, byte-identical across a 19-cell matrix, clean under `LOFT_POISON`,
  the interpreter leak gate and the native leak check, and identical under `LOFT_HOIST_VERIFY=1`
  and `LOFT_NO_VIEW_HOIST=1` — the native header hoist being the one place the cure could have
  gone wrong on a single backend.
  ⚠ **The `&` has to be asked for, and the first cut did not ask.**  A plain whole-collection bind
  copies at PARSE time into its own `__vdb_N` backing — identically off an owned base and off a
  borrowed PARAMETER — which read as licence to key the mark on the collection TYPE alone.  Off a
  LOOP VARIABLE it does not copy: `for b in bv { c = b.vecf; b.vecf += [9] }` aliases and
  materialises today, and `(B-View)` says it must keep doing so, because a plain bind already meant
  value semantics.  Hence `amp_container_link`, set where `amp_collection_bind` is decided, and NOT
  a widening of `is_amp_link`, whose readers are struct-shaped (the whole-record write route gates
  on `Reference`/`Enum`; the re-key refusal and `(B-Ref-Reshape)`'s refusal would decline different
  programs).
  Guard `tests/scripts/a-link-to-a-whole-container-survives-that-containers-growth.loft`.
  ⚠ Two of its cells passed BEFORE the fix and are controls rather than coverage, which matters
  before either is cited as evidence: a keyed `a = &s.h` across an add, and a nested
  `d = &o.inner.data` across a growth.  The nested one passes only because `grown_containers`
  cannot name a nested place at all (its `OpGetField` arm requires the container to be a bare
  `Var`), so it is a MISSED disturbance rather than a considered answer — it would pass the same
  way with the cure reverted.
  **Left open, and it is the same rule's other half:** a collection link is still quietly
  downgraded where the rules say REFUSE.  `c = &s.h; s = Host{…}` materialises in silence, which
  `(B-Ref-Reshape)` calls a compile-time error, because that refusal reads `is_amp_link` and a
  collection bind is not in its population.  Closing it widens which programs the refusal declines
  and needs its own measurement over the corpus and the published libraries.
  Found via @PLN164 C3's matrix (loft#1543).  ⚠ That issue's body names
  `ViewWalk::shake_plain_places` and `use_analysis::view_source_place_indexed` as code C3 landed.
  Both are live code now — C3 merged in `46aaf2967` — but neither existed when the issue was
  filed: the body describes a design sketch as though it had shipped.  Date what it claims against
  that merge rather than reading it as a record of the tree.
* **D-bind-45** *(opened 2026-09-15, CLOSED 2026-09-15)* — `(Const-Value)` for a
  value-const value handed to a PLAIN heap parameter.  A plain struct or vector parameter names the
  caller's record (`calls.md` F-ParamHeap), so `fn bump(a: Account) { a.balance = 999 }` called as
  `bump(acct)` with `acct: const Account` wrote the caller's balance on both backends with no
  diagnostic.  **Decided (owner, C124): by SIGNATURE.**  A value-const value reaches only a parameter
  declared `const`; whether the callee's body writes it is not asked — a line's meaning is judged by
  the line and the signatures it names (C121), and a proof about a body stays an optimisation's
  (C122).  **Built, as a WARNING first** (`const-to-plain-parameter`, the owner's rollout: 102 call
  sites in consumer libraries reach 17 read-only helpers not yet declared `const` — DESIGN_DECISIONS.md
  C124 § Rollout lists them; it becomes an error once they are): the call gate beside D-bind-44's `&`
  gate, reading the parameter's `const` from
  the definition's attribute (`Attribute.value_const`, now serialised in the IR store so a cached
  stdlib or library keeps it); the standard library declares its read-only heap parameters `const`;
  a generic instance, an interface stub, a bound-method stub, a default-value function and an
  overload dispatcher carry the `const` of what they copy; an `Op*` primitive is exempt, since only
  the standard library's own bodies can call one and `const` there means an immediate operand.
  Guards `tests/scripts/a-const-value-reaches-only-a-const-parameter.loft` and
  `…-passed-to-a-const-parameter-is-legal.loft`.  **Closed with the function-reference half:** a function type spells `const` (`fn(const T)`,
  carried as `ConstParams` beside the parameter types rather than as a wrapper on them), so a call
  through a reference, a builtin's callback (`map` / `filter` / `any` / `all` / `count_if` /
  `reduce`) and a lambda's own `const` parameter are judged by the same signature rule; a
  plain-parameter function is refused where `fn(const T)` is expected, a join of functions keeps the
  `const` every arm declares, and a closure's capture of a value-const value is a value-const field
  of its record.  Guards `tests/scripts/a-const-value-reaches-a-function-reference-only-through-const.loft`,
  `…-is-not-written-through-a-const-lambda-or-a-closure.loft` and
  `…-through-a-const-function-type-is-legal.loft`.  loft#1540.
* **D-bind-44** *(opened 2026-09-15, CLOSED 2026-09-15)* — `(Const-Value)` through a VIEW: a value-const
  value was written, in silence and on both backends, through a loop variable over its elements
  (`for f in ps { f.x = 5 }`), an element or field bound to a local (`p = ps[0]`, `q = w.p`,
  `r = &ps[0]`), a loop over such a view, a loop over a value-const FIELD, and `f#remove`; and it could
  be handed to a `&` parameter, whose callee wrote it.  `ps[0].x = 5` was refused, which made the rest
  look enforced.  **Where (measured).**  The guards resolve a write to its ROOT variable and ask that
  variable's flag; a view is a variable of its own, bound by a projection, and nothing gave it the flag.
  **Closed** by marking the view at its bind (`Parser::mark_const_view`): a projection, a `&` link or a
  loop over a value-const value — the binds `(B-View)` and `(B-Ref-Alias)` make aliases — gives the view
  the value's read-only flag, so every existing guard refuses a write through it, and the refusal names
  the view and what it views.  A bare-variable bind (`(B-Copy)`) and a call's result (`(O-Move)`) are
  not views and stay writable, as does a scalar or `text` copied out.  A value-const value or view
  handed to a `&` parameter is refused at the call (plan 40 rule 4).  Measured on both backends over
  25 cells, and by `--check` over 8566 `.loft` files (this repository and the consumer checkouts):
  no file gained or lost a refusal, 461 of them never reaching pass 2; one over-approximation remains — the mark follows the bind, so a view local rebound to a
  fresh value is still refused.  Guards `tests/scripts/a-view-of-a-const-value-is-read-only.loft` and
  `…-leaves-its-copies-writable.loft`.  loft#1540.
* **D-bind-43** *(opened 2026-09-14, CLOSED 2026-09-15)* — `(B-Ref-Write)` for a VECTOR written through a
  local `&` link from a named vector: `a: vector<integer> = [1]; n: vector<integer> = [7, 8]; c = &n; c = a`
  must make `n` a copy of `a`, and left `n` at `[7, 8]` on both backends while `c` read a copy of `a` — the
  write was lost and the link named a store of its own.  The same from another vector link and in a loop.
  Silent; the same on 2cff47dfc.  A literal build or an append through the link was right.  **Where
  (measured).**  The link is typed a plain vector sharing `n`'s store and registered in
  `amp_vector_locals`; `c = a` lowered to a fresh store filled from `a` (`OpDatabase`, `c = OpGetField(…)`,
  `OpAppendVector(c, a, 0)`).  The clear-and-refill it needs lives in `Parser::assign_refvar_vector`, which
  took only a `&vector` PARAMETER or annotated local (`RefVar(Vector)`).  **Closed** by letting that
  handler also take a plain vector local registered in `amp_vector_locals`, for `=` only, and never for the
  statement that makes the link: that statement registers the local a moment earlier, and claiming it
  left the local without a slot (a compile error on every vector link, caught by the first build).
  Measured on both backends, plain, under LOFT_POISON and with the native leak check; a literal, an append
  and a write-through onto the vector the link names are unchanged, and no existing corpus program's
  emission moved.  Guard `tests/scripts/a-vector-written-through-a-local-link-refills-what-it-names.loft`.
  Found while sizing D-bind-42 over the heap kinds.
* **D-bind-42** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Ref-Write)` and `(B-Copy)` for a RECORD
  written through a LOCAL `&` link from a named record: `a = S{v: 1}; n = S{v: 7}; c = &n; c = a; c.v = 9`
  must leave `a` at 1 and `n` at 9, and left both at 9 on both backends; `…; c = a; a.v = 5` then read 5
  through `n`.  The write-through made `n` and `a` one record.  The same held from another link, in a
  loop, for a struct-enum link, and in the shape `157-link-repoint.loft`'s struct cell uses — that cell
  only reads after the write-through, so it passed on the alias.  The `&S` PARAMETER write-back copied
  and was right.  Silent; the same on 2cff47dfc.  **Where (measured).**  `Parser::assign_refvar_reference`
  materialises the copy (`OpDatabase`, `OpCopyRecord`) for a bare variable only when the target is a
  PARAMETER, because the same function once also saw a `&` local bind that must link.  A local link's
  `c = a` stayed `c = a`, and the interpreter installed `a`'s record reference (`SetStackRef`).
  **Closed** by dropping the parameter restriction: a `&` bind that must link reaches that function
  already lowered (`OpCreateStack`, or `OpVarRef` since D-bind-41), so a bare variable there is always the
  write-through.  Measured on both backends, plain, under LOFT_POISON and with the native leak check,
  across every shape above; a `&` re-point, a text write-through and the parameter write-back are
  unchanged, and `c = n` onto the record `c` already names copies that record onto itself, which gives the
  same values.  The copy is real now, so `advice[avoidable-copy]` reports it where the source is still
  used.  Guard `tests/scripts/a-record-written-through-a-local-link-is-copied-into-it.loft`.  Found while
  measuring D-bind-41's struct cells.
* **D-bind-41** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Ref-Repoint)` against `(B-Ref-Write)` when
  the SOURCE is itself a link.  `c = &b` must re-point `c` to what `b` links, and `c = b` must write `b`'s
  value through `c`; both lowered to the same IR, `c = b`, so each backend gave one answer to both
  spellings, silently.  With `a = 1; n = 7; b = &a; c = &n;`, `c = &b; c = 9` wrote `n` on `--interpret`
  (`A1 N9`) and `a` on `--native`, and `c = b` copied on `--interpret` and re-pointed on `--native`
  (`N7`).  The same split held in a loop, in one arm, for a local link re-pointed to a `&` parameter,
  and for `text` links; a `&` parameter re-pointed to another link wrote through on both backends,
  and a `&text` or `&S` parameter re-pointed to a callee-local link wrote through the same way.  A
  first bind was right on both.  **Where (measured).**  The parser's `&` lowering had no arm for a
  source whose type is a link, so the `&` was dropped; the interpreter's link write-through and
  native's local link-to-link arm each read the resulting `c = b` one way.  **Closed** by spelling
  the re-point: the parser lowers `c = &b` to `c = OpVarRef(b)`, the raw link cell `b` holds — the op
  the interpreter's first-bind link copy already emits.  The interpreter routes it to the link's
  slot like the install op, for every kind of link; native takes `b`'s pointer in the local, record
  and parameter arms, and its local link-to-link arm now serves only a FIRST bind, so a reassignment
  `c = b` writes through.  Measured on both backends, plain, under LOFT_POISON and with the native
  leak check, across every shape above.  The one corpus program with a first bind from a link
  (`434-pln87-scalar-reference.loft`) changes its IR spelling and native's record-link representation
  and passes on both backends.  Guard
  `tests/scripts/a-link-re-pointed-to-another-link-takes-what-that-link-names.loft`.  Found while
  measuring D-bind-37's repoint of one `&` parameter to another.
* **D-bind-40** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Ref-Alias)`, `(B-Ref-Write)` and
  `(F-ParamRef)` for a value ENUM: a `&` to an enum local, element or field was silently a copy, and
  a write through a `&` enum parameter crashed the interpreter.
  - `e = Col.Green; c = &e; c = Col.Blue` left `e` Green on both backends, and so did a link to an
    element (`e = &es[1]`), a field (`e = &o.k`) and a nullable local.
  - `fn f(c: &Col) { c = Col.Blue; }` overflowed the interpreter's call stack; native answered right.
  **Where (measured).**  Four sites, each on one path the enum takes:
  - The parser lowers a `&` only for a scalar, and its own scalar list left the value enum out while
    `data::is_scalar` counts it, so the bind copied.  The same list made a tuple with an enum member
    take the record-backed link (`tuples.md` D-tup-14).
  - On the first pass an enum element read is the bare `OpGetVector` place, before its enum getter
    wraps it, and the lowering knew only the wrapped spelling.  The first pass typed the local as
    the enum and the second as its link: "cannot change type from Col to &Col".
  - The interpreter read and wrote an enum link with `OpGetByte` / `OpSetByte`, which take a range
    minimum the site never wrote.  The next instruction was read as that minimum, the code stream
    lost its alignment, and control fell back into the caller's code, which called again.
  - Native's local-link arms had no enum, so the bind emitted no right-hand side; and a bare variant
    written through the link did not resolve, because the enum context read the type without
    peeling the link.
  Closed at each: the lowering asks `data::is_scalar` and accepts the bare element op, the link ops
  are `OpGetEnum` / `OpSetEnum`, native links an enum as a `*mut u8`, and `enum_context` and the
  variant resolver read through `peel_link`.  Guard
  `tests/scripts/an-enum-link-reads-and-writes-through-its-own-op.loft`.  Found while measuring
  D-bind-39's enum element face.
* **D-bind-39** *(opened 2026-09-14, loft#1567; silent until the refusal the same day, now loud)* —
  `(B-Ref-Lvalue)` for an integer STORE place stored in fewer than 8 bytes — an element or a field
  of `u8`, `i8`, `u16`, `i32`, or a narrow range: `c = &u[1]`, `c = &o.a`.  The rule says such a place
  links, and it is refused on both backends with one error per `&` ("`&` cannot link to an integer
  element or field that is stored in fewer than 8 bytes…"), naming the two ways out: copy into a
  local and write it back, or declare the element or field `integer`.  A narrow LOCAL, a `&` parameter
  and a tuple local are 8-byte frame slots and still link.
  **Before the refusal (measured, silent).**  A `u8`/`i8` element link read 2042 for 250 on
  `--interpret`, and a write through it reached the next element (`1,200,0`); `--native` panicked in
  the store (`addr_mut`: not aligned for `i64`).  A `u8` field link was a plain copy, so its write was
  lost on both backends.  `u16` and `i32` places were already refused, under a message that called
  them temporaries.
  **Why a refusal and not a link.**  Reading and writing through a link picks the op by the kind alone
  (`OpGetInt` / `OpSetInt` for every integer in `state/codegen.rs`, a `*mut i64` natively), and the
  type cannot choose better: a link to a `u8` local (an 8-byte frame slot) and a link to a `u8`
  element (a 1-byte store slot) have the same type, `&integer(0, 255)`, and a link can be re-pointed
  from one to the other, so the width belongs to the target at run time.  `(B-Ref-Reshape)` prefers
  refusing a link it cannot honour to a silent copy.  **Closes when** a link carries its target's
  width — a representation decision — and the refusal is lifted.  One home decides the refused set,
  `Parser::is_narrow_store_place`, asked at the `&` and by the lowering on both passes.  Guards
  `tests/scripts/a-link-to-a-narrow-integer-store-place-is-refused.loft` (one error per `&`) and
  `tests/scripts/a-link-to-a-narrow-integer-local-reads-and-writes-it.loft` (what must still link).
  Found while checking D-bind-36's cells against a middle element.
* **D-bind-38** *(opened 2026-09-14, loft#1566)* — `(B-Ref-Lvalue)`: a link to a TEXT place is refused.  `a:
  vector<text> = ["aa"]; t = &a[0]` and `o = O{s: "aa"}; t = &o.s` stop with "`&` requires an
  addressable operand — a variable, struct field, or vector element", on both backends and already
  on the first bind, while the same spellings over an integer, float, enum or struct place link.  The
  rule names a field and an element as lvalues without an exception for text.  **Where (measured).**
  A text place reads through its own op — `OpGetText(OpGetVector(a, 4, 0), 0)`, `OpGetText(o, 8)` —
  and `Parser::is_amp_place` does not list it.  Lifting the refusal is not the whole cure: a local
  `&text` link is a `*mut String` on `--native`, and a store's text slot is not a `String`, so the
  link needs a representation for a text that lives in a store.  (The `u16` and `i32` places this
  entry first carried are narrow integer places and share D-bind-39's refusal, which now names them
  correctly.)  Found while probing D-bind-36's repoint over every element kind.
* **D-bind-37** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(O-NoDiverge)` for `(B-Ref-Repoint)` on a
  `&τ` PARAMETER: `fn f(c: &integer, v: vector<integer>) { c = &v[1]; … }` re-pointed the parameter's link
  on `--interpret` (after D-bind-36) and did not compile on `--native` — rustc E0308 for an element or
  a field, and an empty right-hand side (`*var_c = ;`, or `u8::from()` for a boolean) for a callee
  local.  A loud divergence, never a wrong value.  **Where (measured).**  The native generator treats
  every assignment to a `&` parameter as the write-back, so a place wrote its reference into the
  caller's variable.  **Closed** by a repoint arm at the top of that branch: for a scalar parameter,
  a place op on the right binds a borrow of the store slot, and `OpCreateStack(m)` a borrow of the
  local; the slot pointer is built by the same helper a local link uses.  Measured on both backends,
  plain, under LOFT_POISON and with the native leak check: an element, an element then a write, a
  field, a loop, a callee local, read-repoint-read-write, and float, enum, boolean and character
  parameters; the write-through control is unchanged.  A `&` parameter re-pointed to ANOTHER link is
  D-bind-41, which this arm does not reach.  Guard
  `tests/scripts/a-reference-parameter-re-points-to-an-element-a-field-or-a-local.loft`.
* **D-bind-36** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Ref-Repoint)` on `--interpret` for
  a link to a SCALAR whose new source is an element or a field: `c = &w[0]; c = &v[0]` and `f =
  &o.x; f = &o.y` panicked (a store access out of bounds; in the allocator for a float element),
  while native re-pointed.  The parser lowers the place to its own op (`OpGetVector`, `OpGetField`),
  not to the install op D-bind-32 routed to the link's slot, so `set_var` took the write-through
  path.  For a link to a scalar the right-hand side's TYPE separates
  the two spellings: a place op is declared to return a reference, a value read out of the place is
  the scalar.  `set_var` routes the former to the link's slot as well, except for an integer stored
  narrower than 8 bytes, whose link reads the wrong width (D-bind-39).  A link to a record or a
  collection reads an element's VALUE as a reference too, so there the type cannot tell them apart
  and only the install op is routed.  (A struct element bound with `&` is typed as a view of its
  vector, `ref(P)`, rather than as a `&P` link; rebinding it already left the first element alone,
  and a cell pins that.)  Guard `tests/scripts/a-scalar-link-re-points-to-an-element-or-a-field.loft`.
* **D-bind-35** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Disturb)` did not hold for a view
  bound inside ONE ARM of an `if` whose other arm assigns the same local: `x = mk(4); if k > 0 { x =
  h.inner } else { x = mk(0) }; h = Hold{…}` left `x` reading the new container on both backends,
  where the unbranched bind and the value join materialise.  The materialisation walk (`ViewWalk`)
  walked the two arms one after the other, so the second arm's assignment ended the view the first
  arm had bound — the same program with the arms swapped materialised.  The arms are now walked as
  alternatives: each from the views open before the `if`, keeping what either leaves open or
  disturbed, and a literal key only where both agree.  Found by D-bind-34's closure, which lowers the
  value join to exactly that statement form.  The walk's imprecision in the other direction is
  unchanged and deliberate: a view bound before the `if` and disturbed on one path only is
  materialised on both, and the author is told.  On `--native` the newly materialised copy then
  LEAKED: a displacement free emptied the slot first, so the copy landed in a fresh store, and the
  generator's owner tracker records a copy only on a first declaration — the reassignment branch
  left it unnamed, and nothing released it.  Falsifying the guard caught it in the leak column; that
  branch now asks `materialises_element` too, the question the first declaration already asked.
  Guard
  `tests/scripts/a-projection-assigned-in-an-arm-views-until-its-container-is-reassigned.loft`.
* **D-bind-34** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-View)` and `(O-NoDiverge)`: a REASSIGNMENT from a
  value join whose taken arm is a struct PROJECTION copies on `--interpret` and views on `--native`.
  `h = Hold{inner: mk(1), …}; x = mk(4); x = if k > 0 { h.inner } else { mk(0) }; x.a = 9` leaves
  `h.inner.a` at 1 on the interpreter and at 9 natively, while the unbranched `x = h.inner` and the
  author's statement form `if k > 0 { x = h.inner } else { x = mk(0) }` view on both backends.  The
  container is not disturbed, so `(B-View)` gives the view and the interpreter is the deviating
  side.  Found while closing D-bind-33: that cure, first built wider, wrote this join out as
  statements too, and native then copied as well — the rule's answer lost on the one side that had
  it.  Narrowed to local arms, the split is back to what it was, and it is recorded here.
  **Where it happens (measured).**  The author's statement form carries an owner witness
  (`(O-Witness)`: `__own_x`, released by store identity at the projection's assignment), so the
  projection arm is a view on both backends.  The value join gets none: `owner_witness_locals` runs
  before the scan and does not read the join's projection arm as a view assignment, so `x` stays an
  owning slot, and the interpreter lowers an owning slot's reassignment from an `Own::Join` value to
  `OpBindOrCopy`, which copies the borrowed arm.  The boundary agrees — a binding whose previous
  assignment was itself a view views on the interpreter too.  It is the third fact a pass before the
  scan reads off the join where the author's spelling has it (family 7's per-path flags and
  D-bind-33's call-arm owner were the other two, `heap.md` D-heap-7).
  **Closed by carrying the scan's own decision.**  The first scan records, by the address of each
  `Set`'s value node, the reassignments it writes out per arm; the caller rewrites exactly those in
  the original code, strips the arm locals' deps the parser gave the binding, and scans again — so
  every analysis before the scan, the owner witness among them, reads the per-arm form.  A structural
  "is this a reassignment" was measured first and rejected: it disagreed with the scan at 52 of 899
  corpus sites.  The lowered family-7 cells now emit exactly what their author-written twins emit,
  and the value join views on both backends.  Closing it exposed D-bind-35.
* **D-bind-33** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Copy)` did not hold for a
  REASSIGNMENT from a join whose other arm is an owning CALL.  `a = mk(1); x = mk(9); x = if c { a }
  else { mk(2) }; x.id = 77` changed `a` on the path that took it, and a later write to `a` showed
  through `x`, on both backends; the first bind from the same join copied.  The releases went wrong
  with it (`heap.md` D-heap-7, `p_j1`/`p_j2`): the displaced `mk(9)` was never released, and neither
  was a record assigned to `x` after the join, because the binding stayed typed as a view of `a`.
  **Between two mechanisms.**  A first bind lifts each arm into a temp of its own (D-bind-16,
  loft#1321); a reassignment cannot borrow those temps (`(O-Latest)`), so it is written out per arm
  instead (`scopes::sink_set_into_arms`).  The parser had already given the call arm an owner for the
  value form's view-typed join (`materialise_owned_call`'s `join-arm-owner` block), whose tail is a
  compiler temp, and the write-out declined every compiler-temp tail — so this reassignment kept the
  value form, which binds the chosen arm's STORE.  The write-out now accepts that block and writes the
  arm out as its call, wherever every other arm is a local or `null`.  Beside a projection arm it
  still declines: written out, a projection arm lost the dep that keeps it a `(B-View)` view, which
  was measured and is D-bind-34.  Guard
  `tests/scripts/a-reassignment-from-a-join-with-a-call-arm-copies-its-local-arm.loft`.
* **D-bind-32** *(opened 2026-09-09, CLOSED 2026-09-09; numbered 30 on its own branch, where
  `D-bind-30` was already spent by the `(B-Ref-Reshape)` entry closed a day earlier — the join is
  what showed the collision)* — `(B-Ref-Repoint)` on `--interpret`,
  at every τ but a vector.  The rule was not written: B-Ref-Write said what a heap write
  through a link does ("it does not re-point the link") and nothing said what `p = &q` on an
  existing link does, while the parser had long lowered it to the link's install op
  (`Set(p, OpCreateStack(q))`) and native ran it as a re-point (`var_p = addr_of_mut!(var_q)`).
  The interpreter's `set_var` link branch recognised the install value only to keep a fn-ref
  off the write-through, and every other kind fell into the write-through: the cell went
  THROUGH the link into the old source's slot and the displaced-store free ran on a stack ref
  (`BUG (#306)`), so a struct read a garbage word and lost the second record's field, a text
  was cleared through the link and read empty, an integer kept its first source.  Found by
  @PLN157 § V-p's cell c18 (a plain-record `&` view rebound in a loop), whose interpreter
  oracle answered 34359738373 for 13.  Closed with the rule written and the install routed
  to the link's own slot FIRST, before any kind's write-through — `state/codegen.rs::set_var`
  cites it; `tests/scripts/157-link-repoint.loft` carries the six kinds and the control.
* **D-bind-31** *(opened 2026-09-09, CLOSED 2026-09-09, loft#1489)* — `(B-Copy)` did not hold for
  a bind out of a CLOSURE CAPTURE.  `e = q` inside a lambda ALIASED the captured collection or
  record: `e += […]` grew the outer `q`, `e[0].a = 99` and `e.a = 99` reached it, on both
  backends, with nothing saying so.  The same bind out of a PARAMETER — the identical shared heap
  value, `(F-ParamHeap)` and `(L-CapHeap)` being one sentence — copied throughout, which is what
  named the difference.
  **One op, two notions.**  A closure reaches its capture through the closure record, so the
  source arrives as `OpGetDbRef(__closure, off)`: a PROJECTION's spelling for a whole value the
  author bound by name.  Every reader that had to tell `(B-Copy)`'s whole value from
  `(B-View)`'s interior place tested for a bare `Var` or an `OpGetField` and answered "not a
  bind" — the FIFTH time this family's selector has been the narrow part while its lowering was
  already right (P261, loft#917, loft#1279, loft#1326).  `reads_a_capture_whole` is the one home
  now, and it asks about the BASE, because the loose spelling also admits an auto-`Reference`
  POINTER FIELD read (`h.link`), which really is the projection this is not.
  The two passes did not agree either: on pass 1 a capture is still a placeholder `Var`, so the
  vector selector answered `CopyVar` there and `NotABind` on pass 2 for one body.
  Guard `1489-a-capture-is-bound-and-returned-as-a-whole-value.loft`, whose `(B-View)` and
  `(B-Ref-Alias)` cells are what say the copy did not swallow the aliasing that is meant to
  stay.
* **D-bind-29** *(opened 2026-09-08, CLOSED 2026-09-08, loft#1463)* — the FUNCTION half of
  `(B-Ref-Uniform)`, on `--native` only.  A write through a `&fn(…) -> τ` link did not land when
  the caller's slot already held a CAPTURING closure: the interpreter wrote it, native left the
  old value in place and said nothing.
  **The special case was in the DISPATCH, not in the write.**  A fn-ref call passes the closure
  environment beside the tag, and the emitter took it from the caller's own `___clos_N` local
  whenever one existed — re-deriving *which environment does this slot hold* from the MINT SITE,
  which is right only while nothing else can write the slot.  A `&fn(…)` link is what can.  The
  environment now always comes from the slot's own `.1`: where the mint put it, and where a
  rebind puts the next one.
  The entry as opened guessed the 20-byte stack form was the axis — 8 B `d_nr` plus a 12 B
  closure `DbRef` against a `d_nr` alone — and the LAYOUT was a symptom rather than the cause.
  Capturing is what makes a `___clos_N` local EXIST for the emitter to prefer; the widths never
  entered the decision.  Worth keeping, because the layout reading is the one a reader arrives
  at from the report and it costs a session: the two `loft introspect` dumps differ in the
  dispatch line, not in any width.
  Guarded by `1463-a-closure-written-through-a-link-lands-on-a-capturing-slot.loft`, whose
  CONTROLS are loft#1443's non-capturing cells — the shape that always worked — so the file says
  both are closed rather than one traded for the other.

**D-bind-28 CLOSED 2026-09-07, the collection half of `(B-Ref-Uniform)`.**
The rule says a `&τ` variable is used *exactly* like a `τ` variable and that no operation is
special-cased.  THREE independent mechanisms broke that for collections and all three are now
closed; the keyed PARAMETER took two attempts, and what closed it was splitting the overloaded
predicate into `keyed_kind` (peels) and `owns_keyed_store` (does not) rather than widening it.

* **CLOSED 2026-09-07 — the VECTOR surface.**  Four compiler special-cases (`insert`,
  `reverse`, `sort`, `reserve`) matched `Type::Vector` against the argument type with the `&`
  still on it, and `Parser::resolve_type_var` bound a type variable to the argument's shape
  without stripping the link — so every generic over `vector<T>` (`sum`, `min_of`, `max_of`,
  and any a USER writes) was unreachable through a reference.  Both now ask `Type::peel_link`,
  the one home for *what a value IS* as opposed to *how it is reached*.  `(C-Ref)` settled it:
  a reference reads through to its referent, so the refusals were deviations rather than design
  calls.  The ordinary operations (`.remove`, `v[i] = x`, `+=`, `len`, `.clear`, `for..in`,
  `v[a..b]`) went through the normal call path and were correct throughout, which is what
  localised the fault to the sites that re-derived the shape.
* **CLOSED 2026-09-07 (loft#1433) — the keyed BIND.**  `a = &h` on any of the five keyed kinds
  was a COPY rather than a link: the `&`-bind's source set was `matches!(source,
  Type::Vector(_, _))`, a set of ONE, so a keyed source took the deep-copy path and got its own
  store with `OpReplaceKeyed` filling it from the source.  The two collections were then
  independent — measured, after one write through each name `h` held `{1,3}` and `a` held
  `{1,2}`, *both reporting length 2*.  The set now comes from `vectors::is_collection`, which
  `(Col-Store)` already defines as the `is_keyed` set plus `Vector`.
  ⚠ **This was NOT the broken emission path below**, though loft#1433 was filed as if it were
  ("the alias silently drops every append", which is how an EMPTY source presents).  The append
  was never dropped: it landed in the alias's own store.  The bind and the append are two
  faults, and both are now closed.
* **OPEN — the keyed PARAMETER (loft#1445).**  ⚠ A first fix was landed and REVERTED the same
  day: it peeled the link in the SHARED `is_keyed` / `is_collection`, which are asked at 78
  sites and answer both *which collection kind* and *does this variable own a store*; a
  `&hash` parameter's `Set(v, Null)` then reached `gen_keyed_null` (which allocates a keyed
  LOCAL's store, resolving with the unpeeled `base()`) and ICEd, taking loft#1291's guard from
  8/8 green to 8/8 `unreachable!`.  The diagnosis below is unaffected and is what the narrow
  rework implements — peel at the `+=` ROUTE, not in the shared predicates.  `c += […]` on a `&hash` /
  `&sorted` / `&index` / `&trie` / `&spatial` PARAMETER was refused, naming a `vector<τ>` the
  program never wrote.  Closed by peeling the link at all THREE predicate sites — `is_keyed`,
  `is_collection` and `keyed_known_type` — plus the `+=` route's DESTINATION.
  ⚠ **The fourth is not optional and its omission fails in the CONTROL:** peeling the
  predicates alone hands `append_source` a `RefVar` that matches no arm, and the
  `&vector<Row>` twin that always worked starts refusing.  Separately, two of the five kinds
  (`trie`, `spatial`) were an ICE through `&` at all — a deref allow-list listing kinds instead
  of deriving them from `vectors::is_collection`.
  ⚠ **This entry previously said the refusal was the SAFER state and that "the fix belongs in
  the keyed emission path, not in the predicates".  That conclusion was exactly backwards, and
  the correction is worth more than the entry.**  The MEASUREMENTS behind it were real — the
  surface peel alone gives `len` 0 with no diagnostic, and on another build `len=1` with empty
  payload fields.  The ATTRIBUTION was invented: the emission path resolves a keyed store
  through a `&` parameter and always did, which a keyed INSERT one operator over
  (`c[7] = Row{…}`) demonstrates on both backends with the pre-existing key still readable.
  The real cause was a THIRD instance of the SAME predicate miss — `keyed_known_type` also
  opening with `base()` — so the fallback handed `OpNewRecord` the `vector<τ>` id and
  `record_finish` dispatched through `Parts::Vector`.  It presents as a wrong type NUMBER
  (`parent_tp` reading the `&vector` twin's id), not as a reachability failure, which is why a
  mechanism sentence could not tell the two apart and a `parent_tp` comparison could.  The peel
  was never the wrong move — it was half a move, and calling the remaining half "the emission
  path" sent the next reader to rebuild something that was not broken.

The closed ones: D-bind-25/26/27 CLOSED 2026-09-07: `(B-Disturb)` ends a place
for a `sorted` removal (`(Col-RemoveDense)` — the INLINE keyed kind), for a removal reached
through a FIELD, and for every place a branch's arms can name rather than only an agreed one;
D-bind-24 CLOSED 2026-09-06 (loft#1401): a projection
discharged with `??` is the view its plain spelling is, so it materialises where that one does;
D-bind-20 CLOSED 2026-09-06 (loft#1393): a view whose container is itself a
view is a place inside the OUTER container, so a disturbance of that one ends it; D-bind-19
CLOSED 2026-09-06 (the `@FR-O-Owner` walk): a struct-ENUM PAYLOAD view is a view like any
other, and `(B-View)` materialises it; D-bind-18 CLOSED 2026-09-06 (loft#1392): a VECTOR link
follows a rebind of its SOURCE, as `(B-Ref-Alias)` says a live link must.  D-bind-17 CLOSED
2026-09-06 (loft#1372): a `&` link now carries a NULLABLE
slot, so `(B-Ref-Intro)`'s *`&τ` for every τ* holds with no τ excluded.  `Optional(τ)` shares
`τ`'s storage, so a `&τ?` has the same representation as its `&τ` twin and the absence rides
the slot's own sentinel; what was missing was not a mechanism but one spelling of the SLOT
behind a link, at the nine sites that each asked the link's inner type bare.  The entry read:
*`&τ?` is declined — a link to a NULLABLE slot (`q = &x` with `x: integer?`,
`fn f(p: &integer?)`) is refused where its type is built, until the read and write lowerings
carry the wrapper on both backends; before the refusal the local bind was a silent copy.*

The record of the closed ones is in
[binding-history.md](binding-history.md).

> **A zero here is a claim to re-measure, and this is what the oracle covers.** The `&`
> ladder (`pln87_link_l*`), the const quadrants (`40-const-fields`), the copy-vs-view boundary
> (`bind-copies-or-views-the-whole-boundary`, whose subjects are all NON-nullable — loft#1319
> is the row it cannot see), the reference-tuple guards (`reference-tuple-local-binding`,
> `1006-…`, `reference-tuple-heap-elements-link`).  Held FIXED: every `&(…)` source is a tuple
> local or a loop variable, and a `&(…)` element is never nullable, a fn-ref or a nested
> tuple — those three are refused, not unmeasured.

The full register — every entry, open and closed, with its dates and issue numbers — is
the companion [binding-history.md](binding-history.md).

## Conformance

The rules' falsifying programs are the ladder lock-ins (`pln87_link_l*`); the north star
`a=3; b=&a; b=4; a==4` is `B-Ref-Write` (D-bind-2). As `loft2` lands a rung its lock-in
flips to PASS and the matching deviation is **deleted** here. D-bind-0 is the deepest:
closing it (a real `&τ` reference type) makes the others fall out of the type rather than
out of per-site flags. When OPEN reaches 0, `&`-binding is formal and feeds the deferred
`deps`/borrow `ownership.md`.

The `const` rules' falsifying programs are `tests/scripts/40-const-fields.loft` (positive
cells: construct/read/contents-mutation for every quadrant, struct **and** enum-variant)
plus the `pln40_const_*` / `pln40_vc_*` / `pln40_enum_variant_*` negatives in
`tests/issues.rs` — all graduated from the boundary matrix in
[../plans/40-const-fields/const-model.md](../plans/40-const-fields/const-model.md) and
[const-model-phase2.md](../plans/40-const-fields/const-model-phase2.md). D-const-1's
falsifier — the enum-variant write `s.radius = 9` after a `Circle` match — is now a pinned
regression (`pln40_enum_variant_const_reassign_rejected`), so a further regression fails
the suite.

⚠ **That oracle crosses `const` with the four quadrants and with struct-vs-enum, and with
nothing else** — in particular it contains no `&` cell and no keyed collection, so it read
green while `Const-Value` went unenforced on two whole append routes. The check used to sit
INSIDE each lowering route, one copy per route, which makes it exactly as complete as each
route's own target-shape test: `p: & const vector<T>` failed the vector route's
`Type::Vector` destructure (`Type::base()` peels `Optional`, not `RefVar`) and
`p: const hash<R[k]>` / `sorted` / `index` reached keyed append routes that carried no
check at all. Both appended into the CALLER on both backends while the parameter said
`const`. It is asked once now, ahead of the route dispatch, because whether a write is
allowed is a property of the BINDING and never of the route that lowers it —
`Parser::guard_const_write`, called from `parse_assign_op_inner`. The crossing the oracle
was missing is `tests/scripts/const-binds-through-every-append-route.loft`.
