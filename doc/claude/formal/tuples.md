<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/tuples.md — semantics for tuples (strict)

**Catalogue:** @F (tuples), @PLN89 (differential oracle). Reference: [TUPLES.md](../TUPLES.md).

> **Rules then deviations** (see [README](README.md)). This is the relation for **tuples** —
> anonymous positional products: construction, element projection, and destructuring. It extends
> [operational.md](operational.md) (eval order, assignment) and [calls.md](calls.md) (a tuple is
> a first-class argument / return). Every rule is a **user-visible contract** verified on both
> backends.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩`. A tuple value is an ordered
`(v₁, …, vₙ)` with `n ≥ 2`; its type is `(τ₁, …, τₙ)`. `t.i` is the `i`-th element (0-based, a
compile-time index).

---

## Rules

### Construction — positional, left to right, at least two elements

```
  (T-Cons)   ⟨(e₁, …, eₙ), σ⟩   evaluates e₁, …, eₙ LEFT TO RIGHT (operational.md E-Left) into
             the tuple value (v₁, …, vₙ),  n ≥ 2.  A HEAP element is COPIED in, exactly as
             [binding.md](binding.md)'s `B-Copy` copies a plain bind: the tuple's element and
             the source are INDEPENDENT afterwards, and mutating either does not reach the
             other.  Aliasing is admissible only where it cannot be observed — the source is
             dead after the construction — which is the same last-use elision the STRUCT
             constructor already applies (`LOFT_NO_MOVE_ELIDE` restores the copy).  A
             PARAMETER handed to a tuple keeps aliasing its caller: that is `B-Ref-Alias`, and
             it is a property of the parameter rather than of the construction.
  (T-Paren)  a single parenthesised expression `(e)` is NOT a tuple — it is just grouping.  A
             tuple needs ≥ 2 comma-separated elements.
```

**In words.** `(3, 7)` builds a 2-tuple, evaluating the elements in source order. Tuples are
anonymous (no declared type name) and positional — the elements can be of different types
(`(integer, text)`). A lone `(e)` is ordinary parenthesisation, not a 1-tuple; the minimum tuple
width is 2. And a tuple given a heap local takes a COPY of it, so `t = (vl, 9); vl[0] = 41`
leaves `t.0[0]` unchanged — the same answer `S { v: vl }` and `[vl]` give, because a constructor
handing a value to a new name is the same step a plain bind is (D-tup-4).

### Projection — `.i` reads the i-th element

```
  (T-Proj)   ⟨t.i, σ⟩ → ⟨vᵢ, σ⟩         where t = (v₀, …, vₙ₋₁) and 0 ≤ i < n; i is a COMPILE-TIME
             index (a literal), and its type is τᵢ.  An out-of-range i is a STATIC error.
```

**In words.** `t.0` is the first element, `t.1` the second, and so on — the index is a literal
fixed at compile time (not a runtime value), so its element type is known statically and an
out-of-range `.i` is a compile error, never a runtime null. (Verified: `(3,7).0` is `3`,
`.1` is `7`.)

### Destructuring — bind the elements positionally

```
  (T-Destr)  ⟨(x₁, …, xₙ) = e, σ⟩ → bind each xᵢ to the i-th element of the tuple value of e.
             The arities must match (n names for an n-tuple), positionally.
```

**In words.** `(a, b) = (5, 9)` binds `a = 5`, `b = 9` — a positional unpack. It composes with a
tuple-returning call: `(x, y) = pair()` unpacks the returned tuple directly (verified: `2 3`).

### Tuples as call arguments and returns

```
  (T-Ret)    a function may return a tuple type `(τ₁, …, τₙ)`; the returned tuple is an
             INDEPENDENT value (calls.md F-Ret), commonly unpacked at the call site by T-Destr.
```

**In words.** A tuple is a first-class value — you can return one (`fn pair() -> (integer,
integer)`), pass one, and unpack it at the caller. Returning a tuple is the idiomatic
"return two things," and the result is independent like any return (calls.md).

**A tuple type takes no `?`.** `(N-Opt)` licenses `τ?` for every τ and a tuple is the one former
with no representation for absence: it is its members' bytes, so `(L-Null)`'s sentinel has no
value to reserve and `(L-Null-Tag)`'s discriminant is for a struct stored INLINE. So
`(integer, integer)?` is refused at the declaration, in every position, naming the two cures —
nullable MEMBERS (`(integer?, text?)`), or a `struct` wrapper, which can be `?`. `(N-Index)`
still builds the type where the type parser cannot spell it (`v[i]` on a `vector<(τ, τ)>` IS
`(τ, τ)?`), and every USE of that value is a discharge: `t?` gives the members' defaults,
`t ?? (…)` a default of your own, and an undischarged member read or destructure is refused with
the same two cures. The gap between the rule and the model is
[types-history.md](types-history.md) D-Opt-NoNull, and loft#1423 carries the design call.

### Reference tuples — `&(…)` writes the caller's elements in place

```
  (T-Ref)     a BINDING may be declared `&(τ₁, …, τₙ)` — a parameter, or a local written
              either way binding.md B-Ref-Intro allows (`b: &(…) = a`, `b = &a`).  It denotes
              the bound tuple itself: a projection `p.i` reads that tuple's element and an
              assignment `p.i = e` writes it, both through the tuple's stored reference at the
              element's own offset — the same `(ref, offset)` pair an ordinary struct FIELD
              uses (binding.md B-Ref).  For a parameter the tuple is the CALLER's; for a local
              it is the source variable's, so the two positions are one mechanism and not two.
  (T-Ref-Rep) the tuple a `&(…)` names is STACK-backed when every τᵢ is a scalar, and a
              `__tuple<τ₁, …, τₙ>` RECORD otherwise — the same record a heap-tuple RETURN and
              the loop variable over a `vector<(…)>` already are.  A tuple LOCAL that is the
              source of such a link is built as that record; every other tuple local keeps its
              stack form, so a program with no `&(…)` is unchanged by this rule.
  (T-Ref-El)  every τᵢ is a scalar (`integer` of any width, `float`, `single`, `character`,
              `boolean`, a value enum) or a type a struct FIELD can hold — `text`, a struct, a
              vector, a keyed collection, a struct-enum.  What the record cannot spell or lay
              out as a field is a STATIC error naming the element type: a NULLABLE element, a
              fn-ref, a nested tuple.  A `&(…)` containing a type declared LATER in the file is
              refused the way a tuple return is.  Never a runtime fault and never an ICE, and
              asked wherever the `&` is WRITTEN, so a `&(…)` a signature refuses cannot be
              accepted at a local.
  (T-Ref-Src) the source of a `&(…)` local is a tuple VARIABLE.  A tuple ELEMENT or FIELD
              (`b = &v[0]`, `b = &s.pair`) is a STATIC error: a tuple place is read element by
              element into a fresh by-value tuple, so no place survives for the link to name.
              For a record-backed `&(…)` PARAMETER the argument is likewise a tuple LOCAL of the
              caller (a return-bound local, a literal local, a loop variable); a by-value tuple
              parameter passed on, or a field, is a STATIC error saying to bind it first.
              Declining is binding.md B-Ref-Reshape's rule — where the link cannot be honoured
              loft refuses the program rather than downgrading it to a copy.
```

**In words.** `fn sw(p: &(integer, integer)) { t = p.0; p.0 = p.1; p.1 = t }` swaps the caller's
tuple in place — that is what a reference tuple is for. The same annotation on a LOCAL means the
same thing (`a = (1, 2); b: &(integer, integer) = a; b.0 = 5` leaves `a.0 == 5`), because both
name a tuple sitting in a frame and reach it the same way. `fn sw(p: &(text, text))` swaps a
`text` pair the same way; what differs is where the tuple lives — a scalar tuple sits on the
stack, a tuple with a heap element is the `__tuple<…>` record a return of that shape already is —
and the boundary is enforced wherever the `&` is written, so a program either compiles and
behaves identically on both backends or is refused
where it is written.

The restriction belongs to the STACK-backed reference tuple this annotation builds. The
record-backed one a `for` loop binds over a `vector<(…)>` is a different construction reaching a
real record, and it admits any element type — `for t in [("a", "b")] { t.0 }` is correct on both
backends, and writing `t.0` there reaches the vector. Reading `T-Ref-El` as a fact about tuples
rather than about this binding is the mistake that boundary invites.

A `text`, collection, struct or function-reference element is refused. This is a layout
limitation, not a missing opcode: `OpGetText` / `OpSetText` exist and take the same
`(ref, offset)`, but a reference tuple's storage is not a record with a text slot the way a
struct is. Use a **struct** instead — its fields of any type write through a `&` parameter —
or take the tuple by value and return a new one. The refusal message says both.

---

## Deviations

**OPEN: 0** — D-tup-9 closed 2026-09-05: a tuple literal member typed by a generic's type
variable is copied for every binding — a record or a scalar by @PLN153 phase 1, a vector or a
keyed collection by the @FR-F-Ret walk's boxed monomorph return (loft#1365).  The non-generic
shape is D-tup-8, closed.

The full register — every entry, open and closed, with its dates and issue numbers — is
the companion [tuples-history.md](tuples-history.md).

## Conformance

- **Construct + project (`T-Cons` / `T-Proj`)** — `t = (3, 7); t.0` is `3`, `t.1` is `7`.
- **Destructure (`T-Destr`)** — `(a, b) = (5, 9)` binds `a=5, b=9`.
- **Tuple return + unpack (`T-Ret` + `T-Destr`)** — `fn pair() -> (integer,integer) { (2,3) }`,
  `(x, y) = pair()` binds `x=2, y=3`.
- **Reference tuple (`T-Ref`)** — `fn sw(p: &(integer, integer)) { t = p.0; p.0 = p.1; p.1 = t }`
  swaps the CALLER's tuple: `(1,2)` reads back `2,1`. Verified on both backends for every
  admitted element type — `integer`, `float`, `single`, `character`, `boolean` — uniform and
  mixed (`&(integer, boolean, character)`), and at width 3 so the last element is reached
  (`tests/scripts/1006-reference-tuple-element-types.loft`).
- **Refused element types (`T-Ref-El`)** — `&(text, …)`, `&(fn() -> τ, …)` and a struct element
  are STATIC errors naming the element type, never an ICE
  (`tests/scripts/102-expected-errors.loft`).
- **Static index (`T-Proj`)** — `t.5` on a 2-tuple is a compile error, not a runtime null.
- **Heap element is COPIED (`T-Cons`)** — `t = (h, 9); h[2] = …` leaves `t.0` at its old length
  for EVERY heap element type, not just the vector the paragraph above names: `hash`, `hash<τ>?`,
  `sorted`, `index`, `trie`, `spatial`, and a DEEP case with a nested `vector<text>` inside the
  element. Both backends
  (`tests/scripts/1230-a-keyed-tuple-element-owns-a-copy.loft`).

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on a
tuple's element order, values, or a projection is the definitional error this doc names.
