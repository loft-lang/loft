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

### Absence — a tuple type is never nullable; a tuple read as null is all-null members (`T-Absent`)

```
  (T-Absent)  a TUPLE TYPE is never nullable — `(τ₁, …, τₙ)?` is refused BY NAME at every
              declaration (types.md N-Opt: a tuple is its members' bytes and has no value to
              spend on absence) — and a tuple that ARRIVES absent is a PRESENT tuple whose
              members are all null:   optional((τ₁, …, τₙ))  ≡  (τ₁?, …, τₙ?).
              Every producer of absence synthesises exactly that, and no `Optional(Tuple)` ever
              exists, not even in flight: an index that misses (`v[i]` on a vector<(τ₁, τ₂)>,
              N-Index), a generic `T?` instantiated at a tuple, a decoded document whose key is
              missing.  The null QUESTION on a tuple is answered by its members, from ONE home:
              `t == null` and `t ?? d` read "every member null"; `t?` reads the members'
              defaults (N-Default, loft#1424); `t.i` on such a tuple is `τᵢ?` (N-Prop) and
              N-Store polices where it lands.  Owner ruling 2026-09-07 (loft#1423): a tuple has
              no faithful document form anyway, so its absence is presented as the tuple that
              exists with nothing in it.  The tag layout a stack `(τ, τ)?` would need is
              declined in DESIGN_DECISIONS C119.
```

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

**OPEN: 1.**

- **D-tup-10** *(open, loft#1423 / loft#1451)* — `(T-Absent)` says no `Optional(Tuple)` exists,
  and the code still mints one wherever absence is synthesised: `Type::optional` wraps a
  `Tuple` like any other type.

  ⚠ **RE-MEASURED 2026-09-09, both backends: three of the four cells this entry called REFUSED
  now work, and only one still does.**  The entry read as a four-cell refusal and is a one-cell
  one.  What still fails is the LANDING type: `w: (integer?, integer?) = v[i]` is refused as
  *"cannot change type from `(integer?, integer?)` to `(integer, integer)?`"* — the two spellings
  of one notion meeting, which is the deviation itself.  What now WORKS: `v[i].0` by a variable
  index answers `null(oob)` where the entry says it is refused by name; `t == null` answers
  `true` on BOTH spellings, the member-nullable and the in-flight one, where the entry says
  *"No matching operator '=='"*.  `v[i] ?? d` and `v[i]?` still answer right (`1`, `0`).  The
  three were carried along by loft#1450's `(N-Chain)` work and @PLN25's null model rather than
  closed deliberately, which is exactly why a deviation's measured cells are a claim to
  re-measure and not a record to cite.

  ⚠ **A `text` MEMBER does not compile at all on `--native`, and no all-`integer` cell can see
  it.**  `t: vector<(integer, text)>; i = 0; b = t[i]; b.1` is rustc **E0308** — `expected
  `String`, found `&str`` — where `--interpret` answers `seven`.  A CONSTANT index compiles
  (`(N-Index)` trusts it, so the element types plain `(integer, text)` and never takes this
  path), and an all-`integer` tuple compiles by either index.  `OpGetText`'s `#rust` body wraps
  its result in `Str::new(…)`, which `generation/calls.rs` strips for native, and this consumer
  does not coerce what is left.  Pre-existing at `9720dfd06`; filed as loft#1478.  It is the
  shape a tuple ORACLE keeps missing — this doc's counts have been read over a corpus of
  `(integer, integer)` cells, which cannot reach it — so any cell added for this entry should
  carry a non-scalar member.

  ⚠ **And a cure that suggests itself is measured WRONG.**  `(N-Opt)`'s side condition got its
  home in `data::has_null` on 2026-09-09 (loft#1478), and the obvious next step — have the `τ?`
  constructors ask it, so the index stops minting `(integer, text)?` — makes this worse, not
  better: the read then types `(integer, text)`, non-null members holding nulls, with no
  diagnostic.  It trades a type the language cannot spell for one that LIES about what it holds.
  "No `Optional(Tuple)`" is not "no absence"; the tuple's absence has a form and the cure is to
  BUILD it.  `data::constructs_optional` is that carve-out, and it exists to be deleted when this
  entry closes — the gap between the two predicates IS this deviation, in code.

  **Removal:** the identity lives in
  `Type::optional` (one home — a `Tuple` maps to its member-nullable form); the null question
  then needs its one tuple home (`== null` / `??` agreeing on "every member null", as `1120-…`
  made them agree for a collection), and the typed decoder's tuple arm, when it grows one,
  yields the same value for a missing key.

  ⚠ **This entry claimed the member read, the store and the monomorph's return type would
  "follow with no per-site work".  Two of those three are FALSE, measured 2026-09-08 by making
  the arm and running the cells.**  The store/landing half does follow — `w: (integer?,
  integer?) = v[i]` starts being accepted.  The MONOMORPH does not: loft#1451's reproducer is
  byte-identical before and after on both backends, and it was closed separately as
  `D-call-16` in the return-promotion path, not here.  And the consumers REGRESS rather than
  follow — `?` on a tuple becomes *"cannot build a default for `(integer?, integer?)`"* (guard
  `1424`), `??` starts emitting a spurious *"stored into a slot of the non-null type
  `boolean`"*, and `== null` still has no home.  So closing this is six or seven pieces with
  `1423b` and `1424` as rewrites rather than passes, and `build_default` already refuses
  `(integer?, integer)` independently.  **The one-arm change must not be landed alone**: it
  half-migrates the representation and takes `?` on a tuple down with it.  The written `(τ, τ)?` stays
  refused — that half is `1419-a-nullable-tuple-type-is-refused-by-name.loft` and does not move.

D-tup-11 closed 2026-09-08 (loft#1423): `(T-Absent)` says a tuple is absent when EVERY member
is null, and the null question has ONE home shared by `t == null` and `t ?? d`.  Neither held.
`coalesce_not_null` carried its own convention — *"a tuple is null when its FIRST FIELD is its
type's null sentinel"* — which is the SAME test as the rule's for an index miss, where
`OpGetVectorNullable` nulls every member at once, and a different one for every tuple a program
builds partly present: `t: (integer?, integer?) = (null, 2)` was discharged WHOLLY, so
`(t ?? (9, 9)).1` answered `9` and the present `2` was gone, silently, on both backends.  And
`t == null` had no answer at all — it fell past every gate in the comparison lowering and was
refused *"No matching operator '=='"* for a type whose `??` beside it worked, so the language
had a null question a tuple could be asked one way and not the other.  Closed as an OR-fold over
the members in `coalesce_not_null`, which `null_test`'s new tuple arm negates rather than
restating: two spellings each carrying their own member walk is what let the first-member
convention live in one of them unseen.  Guards `1477` (the `??` half: 4 of 10 cells fail on the
pre-fix build, and the six that pass are every cell whose FIRST member is present) and `1477b`
(the `==` half, held apart because it does not COMPILE there).  A third defect fell out of the
same site: the null TEST asked the nullable→non-null STORE face, so `??` reported *"a nullable
`integer?` is stored into a slot of the non-null type `boolean`"* — a slot the author never
wrote, at the site of their own discharge — and a tuple made the count its ARITY.  @FR-N-Store
admits a test, which `null_test` already knew and the coalesce did not; they now admit alike.

⚠ A fourth surfaced only once the walk reached every member, and it is the one worth carrying
forward: `coalesce_not_null`'s arms match some types in their **BARE spelling only**
(`matches!(tp, Type::Boolean)`), because every previous caller peeled `Optional` before reaching
them.  A member type arrives UNPEELED, so a `boolean?` member missed its arm, fell to the generic
truthiness convert, and `false` read as ABSENT: `(integer?, boolean?) = (null, false)` took the
default while the same tuple with a bare `boolean` member kept its value, and a direct
`b: boolean? = false; b ?? true` was right all along.  Peeled at the recursion.  The reason this
needed its own guard cell is that its three neighbours — `0`, `""`, `0.0` — were right without
it, so a zero-valued-member family that omitted the boolean would have read as covered.

D-tup-9 closed 2026-09-05: a tuple literal member typed by a generic's type
variable is copied for every binding — a record or a scalar by @PLN153 phase 1, a vector or a
keyed collection by the @FR-F-Ret walk's boxed monomorph return (loft#1365).  The non-generic
shape is D-tup-8, closed.

The full register — every entry, open and closed, with its dates and issue numbers — is
the companion [tuples-history.md](tuples-history.md).

## Conformance

- **Absence (`T-Absent`)** — a tuple is absent only when EVERY member is null, and both
  spellings of the question agree: `(null, 2) ?? (9, 9)` keeps the `2` and `(null, 2) == null`
  is `false`, while `(null, null)` takes the default and answers `true`.  Checked with the
  absent member at the FRONT, at the BACK and in the MIDDLE, at arity 2 and 3, with a `text`
  and a nested-tuple member, and for both spellings of the type — the written `(τ?, τ?)` and
  the in-flight `(τ, τ)?` an index miss produces.  Both backends.  A conformance entry naming
  one member of a family is a claim about that member (see the history file's note on the
  keyed half), so the front/back/middle split is the point of the list rather than its length.
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
