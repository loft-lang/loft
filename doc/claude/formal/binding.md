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

**OPEN: 3.**

* **D-bind-41** *(opened 2026-09-14)* — `(B-Ref-Repoint)` against `(B-Ref-Write)` when the SOURCE is itself a
  link.  `c = &b` must re-point `c` to what `b` links, and `c = b` must write `b`'s value through `c`;
  both lower to the same IR, `c = b`, so each backend gives one answer to both spellings.
  - Locals, `a = 1; n = 7; b = &a; c = &n;` then `c = &b; c = 9` — `--interpret` writes through (`A1
    N9`), `--native` re-points (`A9 N7`, right).  Then `c = b` instead — `--interpret` writes through
    (`N1`, right), `--native` re-points (`N7`).
  - A `&` parameter, `c = &d` with `d` another `&` parameter or a callee-local link — both backends
    write through (the caller's variable behind `c` takes the value); `c = d` is right on both.
  A first bind (`c = &b` with no earlier link, and the annotated `c: &integer = b`) is right on both
  backends.  Silent on every face; the same on 2cff47dfc.  **Where (measured).**  The IR of `c = &b` and
  `c = b` is identical, `c(1): &integer = b(1)`, so no backend can tell them apart; the interpreter's
  link write-through and native's local ref-to-ref arm each read that one spelling one way.  The cure
  is in the parser, which has to give the repoint to a link its own spelling.  Found while measuring
  D-bind-37's repoint of one `&` parameter to another.
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
* **D-bind-39** *(opened 2026-09-14; silent until the refusal the same day, now loud)* —
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
* **D-bind-38** *(opened 2026-09-14)* — `(B-Ref-Lvalue)`: a link to a TEXT place is refused.  `a:
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
