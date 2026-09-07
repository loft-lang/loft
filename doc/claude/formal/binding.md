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
                  `c: E = s` from a variant is a copy like any other.
  (B-Ref-Alias)   the `&τ` annotation makes ANY binding — scalar OR heap — a live LINK
                  to the source instead of a copy.  `d = &v` / `d = &self.data` ALIAS the
                  vector: `d[i] = x` (and `d += …`) write THROUGH to the source, which is
                  NON-OWNING (the source frees the store).  `&` is how you OPT INTO
                  aliasing; without it a heap bind copies.  This is B-Ref-Write for a
                  vector lvalue.
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

**OPEN: 0** — **D-bind-28 OPENED AND CLOSED 2026-09-07, the collection half of
`(B-Ref-Uniform)`.**  The rule says a `&τ` variable is used *exactly* like a `τ` variable and
that no operation is special-cased.  THREE independent mechanisms broke that for collections;
all three are now closed.

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
* **CLOSED 2026-09-07 (loft#1445) — the keyed PARAMETER.**  `c += […]` on a `&hash` /
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

The closed ones: D-bind-25/26/27 CLOSED 2026-09-07: `(B-Disturb)` ends a placefor a `sorted` removal (`(Col-RemoveDense)` — the INLINE keyed kind), for a removal reached
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
