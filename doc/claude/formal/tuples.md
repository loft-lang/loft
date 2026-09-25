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

⚠ **A tuple YIELDED by a generator is the one position with a decided edge, and it lives in
another chapter.** `--native` refuses a `yield` whose tuple has a `text` member or a nested one,
naming the type and the cure (wrap it in a struct), where `--interpret` runs it —
[coroutines-history.md](coroutines-history.md) D-cor-2, closed 2026-08-28 with the channel ladder
that decided it. It is written down there and was not written here, so a reader of this chapter
met it as an ICE-shaped surprise (measured 2026-09-25, during the `(T-Destr)` walk). A decided
edge a reader cannot reach from the rule it bounds is, for that reader, undecided.

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
              spend on absence).  ABSENCE BELONGS TO THE `?`, NOT TO THE MEMBERS: exactly one
              tuple can be absent, the IN-FLIGHT `(τ₁, …, τₙ)?` that a read which MISSES
              produces — `v[121134133]` on a vector<(τ₁, τ₂)> (N-Index), a keyed miss, a generic
              `T?` instantiated at a tuple, a decoded document whose key is missing.  That type
              lives in a local VARIABLE and nowhere else: no author can DECLARE it, and the
              compiler carries it with the `?` on the outside (`Optional` around whichever home
              the value lives in — the stack tuple, or the `__tuple<…>` record a heap-carrying
              tuple is boxed into).  Every OTHER tuple EXISTS, whatever its members hold: a
              written `(τ₁?, …, τₙ?)`, a declared member-nullable return, a field read.
              `(null, null)` written down is a tuple that is THERE, holding two nulls.

              So the operators split, and the split is decided by the TYPE ON THE LINE even
              though both spellings hold the same all-null bytes at run time:
                `t?` / `t ?? d`   discharge an ABSENCE — legal on the in-flight tuple, and a
                                  REFUSAL on a tuple that exists, which has none.  The refusal
                                  is on the SUBJECT: `(N-Index)` trusts a CONSTANT index, so
                                  `v[0]` is typed without the `?` yet is still an element read
                                  that can miss, and it discharges like `v[i]`.
                `t == null`       asks the in-flight tuple's MEMBERS — all null is how that
                                  absence is represented — and is constantly FALSE for a tuple
                                  that exists (`!=` constantly true).
                `a == b`          compares tuples that EXIST, element by element; a side that is
                                  absent makes `==` false and `!=` true.
                `t.i`             is `τᵢ?` on an in-flight tuple (N-Prop); N-Store polices where
                                  it lands, and discharging THAT member is the cure the refusal
                                  above names.
              An in-flight tuple STORED into a slot whose members are each nullable is the
              sanctioned move: the nulls land, and the slot then holds a tuple that exists.  A
              slot with a non-null member keeps N-Store's report, per member.

              ⚠ The in-flight tuple carries NO TAG (the layout a stack `(τ, τ)?` would need is
              declined in DESIGN_DECISIONS C119), so an element that EXISTS holding all-null
              members is indistinguishable from a miss — `v[i] == null` is true for both.  That
              is a consequence of the representation, not a defect, and `1477b` pins it.

              Owner rulings: 2026-09-07 (loft#1423) a tuple has no faithful document form, so
              its absence is presented as the tuple that exists with nothing in it; 2026-09-16
              settles the rest of the paragraph above — `?`/`??` reach the in-flight tuple only,
              and a tuple a program writes down exists.  D-tup-10 closed there.
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
              A tuple PATTERN over the binding, and a DESTRUCTURE of it (`(a, b) = p`), are
              each a projection of every element at once, so both read each element through the
              reference exactly as `p.i` does, and a heap member read that way borrows the tuple
              rather than owning a copy.  Reading the binding as a tuple has ONE home,
              `Parser::ref_tuple_subject`; a site that tests the type for `Type::Tuple` instead
              answers no for both representations and refuses what this rule admits.
  (T-Ref-Rep) the tuple a `&(…)` names is STACK-backed when every τᵢ is a scalar, and a
              `__tuple<τ₁, …, τₙ>` RECORD otherwise — the same record a heap-tuple RETURN and
              the loop variable over a `vector<(…)>` already are.  A tuple LOCAL that is the
              source of such a link is built as that record; every other tuple local keeps its
              stack form, so a program with no `&(…)` is unchanged by this rule.  The record's
              members are the LOCAL's types, never the literal's: `w: (integer, Entity) =
              (1, IceWall { … })` is a `__tuple<integer,Entity>`, because a variant widens
              into its enum (types.md `(C-Var)`) and the link names the declared tuple.
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

What stays refused is what the `__tuple<…>` record cannot spell or lay out as a field: a
NULLABLE element, a fn-ref, a nested tuple. The refusal names the element type and the two
cures — a **struct** instead, whose fields of any type write through a `&` parameter, or the
tuple by value with a new one returned.

⚠ **This paragraph and the conformance entry below both read as a blanket `text` refusal until
2026-09-10, and that had stopped being true on 2026-09-03**, when `(T-Ref-Rep)`'s record-backed
form landed and `&(text, text)` began swapping a caller's pair like any other. The claim
survived because it is prose beside a rule that contradicts it, and the guard that would have
caught it (`reference-tuple-heap-elements-link.loft`) asserts the ADMISSION rather than the
refusal — so nothing red ever pointed here. A conformance line is a measurement with a date on
it, not a standing fact.

---

## Deviations

**OPEN: 1.**  D-tup-15 opened and closed 2026-09-22 ([history](tuples-history.md)).

- **D-tup-16** *(OPEN 2026-09-25, loft#1673)* — `(T-Ref-El)` says of a `&(…)` binding
  *"Never a runtime fault and never an ICE"*, and a whole-value READ or WRITE of the
  STACK-backed form is an ICE on both backends: `take(p)`, `return p` and `q = p` panic in the
  codegen link-read ladder, `p = (…)` in its write twin.  The RECORD-backed twin answers all
  four correctly, which is what makes this a deviation rather than an undecided edge — the two
  representations `(T-Ref-Rep)` gives are meant to differ only in where the tuple lives, and
  `@FR-B-Ref-Uniform` says no operation on a `&τ` is special-cased.

  Two smaller faces of the same "the representation is observable" defect travel with it:
  `print("{p}")` refuses on the stack-backed form and prints the compiler's own
  `{_0:…,_1:…}` field spelling on the record-backed one, and both `==` diagnostics name
  `&__tuple<text,text>`, a type no author writes (`Type::source_name` is the home for that).

  The DESTRUCTURE half of the same walk is closed (2026-09-25): `(a, b) = p` answers on both
  representations and from both sources, guarded by
  `tests/scripts/a-destructure-unpacks-a-reference-tuple.loft`.  What remains open is the
  whole-value position, whose cure must leave the record-backed form passing the record — at
  those positions it already works, and materialising it would turn a reference into a deep
  copy of its heap members.

- **D-tup-10** *(CLOSED 2026-09-16, loft#1423 / loft#1451)* — `(T-Absent)` said no
  `Optional(Tuple)` exists while the code minted one wherever absence is synthesised:
  `Type::optional` wrapped a `Tuple` like any other type.

  ✅ **CLOSED 2026-09-16 by an owner ruling, because the rules could not settle it.**  The last
  live cell was the `?` discharge: it answered the members' defaults on the in-flight
  `(τ₁, …, τₙ)?` and was REFUSED on the `(τ₁?, …, τₙ?)` this entry's own rule called the same
  type.  One expression decided it — `u = v[i]; u?` answered while
  `u: (integer?, text?) = v[i]; u?` refused — and the axis was member-nullability, not the home:
  the in-flight spelling over a `vector<(τ?, τ?)>` refused too, and so did the boxed one.

  Measuring the cure is what showed the rules did not reach: `(D-Opt)` gives
  `construct_default(τ?) = null`, so a literal member-wise default of `(integer?, text?)` is
  `(null, null)` — the absent tuple itself, not `(0, "")`.  The case that separates the readings
  is a PARTLY PRESENT tuple, which the written spelling can hold and an index miss cannot
  produce: `??` and `==` are WHOLE-wise on both spellings and agree, so typing the discharge
  `(integer, text)` would put a live null in a non-null slot on the pass-through path — the lie
  `(N-Store)` exists to refuse.  So the `≡` above held at the two ENDS, all-present and all-null,
  and never as type equality.

  **The ruling:** absence belongs to the `?`, not to the members.  Only an out-of-range read
  makes a tuple that is not there; `(null, null)` written down is a tuple that EXISTS holding two
  nulls.  `?` and `??` discharge an absence, so they reach the in-flight tuple alone and are a
  compile error on a tuple that exists; `t == null` is false for one; `==` compares tuples that
  exist and answers false when a side is absent; and an in-flight tuple STORED into a slot whose
  members are each nullable is the sanctioned move.  `(T-Absent)` above is rewritten to that, and
  "every member null" is demoted from the DEFINITION of absence to how the in-flight value is
  represented.  Guards: `1477` (the member fold, its cells moved to the in-flight spelling),
  `1477b` (the null question in both spellings, plus the no-tag consequence),
  `a-tuple-that-exists-has-nothing-to-discharge.loft` (the refusal and its `v[0]` controls), and
  `an-absent-tuple-meets-the-type-its-author-declares.loft` (the landing).

  ⚠ **The refusal is gated on the SUBJECT, not the type, and that is measured.**  `(N-Index)`
  trusts a CONSTANT index, so `v[0]` is typed without the `?` while still being an element read
  that can miss — nine shipped scripts discharge one, and `823` asserts it outright.  A gate
  written on the type alone passes every refusal cell and breaks all nine.

  ⚠ **It also retired a close of its own.**  `??` over the boxed member-nullable spelling was
  made to work on 2026-09-16 (guard `a-boxed-tuple-discharges-its-null-like-the-stack-one.loft`)
  and the ruling makes it an error; that guard is deleted and its one surviving control, the
  generic `-> T?` route, moved into the refusal guard.  Unmerged, so the cost was one commit.

  That close left CODE behind, and it is recorded here rather than removed on the spot:
  `Parser::boxed_tuple_members_modulo_null` answers *"do this boxed tuple's members differ from a
  stack tuple's elements by nothing but each member's `?`"*, which was the gate widening that let
  the boxed `??` through.  It is not DEAD — the `??` refusal reports and CONTINUES, so the default
  still parses and the arm still runs — and neither clippy nor the gate flags it; what it no
  longer has is a reason.  Removing it is a separate measured step, because the predicate is also
  what keeps a coalesce result from being typed as the NON-null stack tuple, which `(N-Store)`
  refuses, and nothing currently distinguishes those two callers.  Whoever takes it should score
  the refusal cells AND `1477`'s partly-present cells, not the refusal alone.

  The record below is kept as it stood, because the measurements in it are what the ruling was
  made on.

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

  ⚠ **RE-MEASURED AGAIN 2026-09-12: the one cell still stands, and both issues it names are now
  CLOSED.**  `w: (integer?, integer?) = v[i]` is still refused on both backends with the same
  *"cannot change type from `(integer?, integer?)` to `(integer, integer)?`"*.  So this entry is
  the case that says an issue closing is not a deviation closing — loft#1423 and loft#1451 each
  closed on their own cells while the notion the entry is about did not.  That is why
  `rule_tags.py registers --issues` reports such a pair to RE-MEASURE rather than calling it
  closed; four of the five entries it flagged the first time it ran were stale, and this one
  was not.

  ⚠ **TRIED 2026-09-15 and taken back out: building the member-nullable tuple in `Type::optional`
  does not close this entry, because the rules around it disagree once the in-flight spelling is
  gone.**  `Type::optional((τ₁, …, τₙ))` returning `(τ₁?, …, τₙ?)` — the rule's own sentence, at
  the one home every producer of absence asks — was built and measured on both backends:
  - it CLOSED the three refusals: `w: (integer?, text?) = v[i]` by a plain local index (the cell
    above; a LOOP-variable index already landed, so a probe written with one reads green on the
    unfixed build), `fn get(…) -> (integer?, text?) { return v[i]; }` (refused with a false
    "nullable stored into a non-null return" warning), and a generic `T?` instantiated at a tuple
    (`.0` refused on `__tuple<integer,text>?`);
  - and it BROKE `v[i]?` with a plain local index (*"`?` cannot build a default for
    `(integer?, text?)`"* — the refusal a written `t: (integer?, integer?)` has always met), and
    put two false warnings on `x: (integer, text) = v[i] ?? (0, "d")`.

  The `Optional(Tuple)` was carrying the one fact the member spelling cannot: an index miss nulls
  EVERY member at once.  Without it the rules meet head on.  `(N-Coal)` types `e ?? d` as `τ`
  (non-null members), `(T-Absent)` reads `t ?? d` as "every member null", and the partly present
  tuple fixed below (`(null, 2) ?? (9, 9)` keeps `(null, 2)`) then holds a null in a member typed
  non-null — the silent lie the refusal in `fields.rs` exists to prevent.  Typing the result
  member-nullable instead is the false-warning half.  So the entry wants a DESIGN call before a
  cure, and the three ways to decide it are:
  1. `??` and `?` on a tuple discharge MEMBER-wise — `t ?? d` is `(t.0 ?? d.0, …)`, typed
     `(τ₁, …, τₙ)`, the way `(N-Store)` already polices a tuple element by element.  No lie and no
     false warning; the observable change is a partly present tuple, `(null, 2) ?? (9, 9)` →
     `(9, 2)`.
  2. Keep the all-or-nothing fact in flight — `Optional(Tuple)` stays, and `(T-Absent)`'s "not even
     in flight" is amended to "never DECLARED"; the landing cell is then a `decl_accepts` arm.
  3. Keep both as they are and accept the member warnings after `??`.
  The probe matrix (thirteen cells with `text` members, both backends) is in the session record of
  2026-09-15 and is the starting guard for whichever is chosen.

  ⚠ **Owner ruling 2026-09-16: option 2 — the in-flight spelling STAYS and only the DECLARATION
  is refused, so the rule above now reads "never declared" rather than "not even in flight".**
  The three refusals closed by teaching each site that met the two spellings to read both, and
  every one of them was a SHAPE question asked without peeling (`@FR-N-Shape`):
  - the landing (`w: (integer?, text?) = v[i]`) — `change_var_type` admits the in-flight tuple
    when every declared member admits its own `τᵢ?`, through the same `decl_accepts` a slot
    already asks, so a member declared non-null still earns `(N-Store)`'s refusal;
  - the RETURN — the two boxing gates (`block_result`'s and `parse_return`'s) asked
    `matches!(t, Type::Tuple(_))` bare, so an absent tuple was never boxed into the declared
    record and then failed `convert` with *"expected `__tuple<…>`, got `(τ…)?` on return"*.  This
    was never about the member-nullable declaration: a plain `-> (integer, text)` refused the
    same read, and now takes it with `(N-Store)`'s per-member report;
  - a generic `T?` at a tuple — the record-backed member read peels now, and `Parser::tuple_elems`
    is the one home for "what are this type's tuple element types" across all six spellings.

  ⚠ **CLOSED 2026-09-16, the null QUESTION half: the record home answers by its MEMBERS.**  Both
  spellings now do, which is what `(T-Absent)` says — the rule is stated on the TYPE, not on
  where the value lives.  The defect reached that home by TWO routes, and measuring which one a
  cell took is what kept the cure from being written at the wrong site:

  - a **bare** `ref(__tuple<…>)` — a declared tuple return — matched no flag in the `== null`
    classification and fell to the generic `==`, which compared the reference against the null
    sentinel (`OpEqRef`);
  - an `Optional`-wrapped `ref(__tuple<…>)?` — a generic `T?` at a tuple — matched `ref_null`
    and took `OpRefIsNull`.

  Both answer by the RECORD, so both read a return buffer holding two nulls as PRESENT.  The
  second route was RIGHT wherever it was measured before, because the shapes reached for it had
  a genuinely absent record (`rec == 0`), where the record's answer and the members' agree — an
  agreement that made the guard's a3 cell pass over an open defect.  The cells that separate
  them are a record that EXISTS with every member null.

  Cure, one notion at one home: `is_tuple_shape` answers *"is this a tuple in ANY of its
  homes?"*, and both the classification and `null_test`'s own gate ask it, so a third spelling
  is added there rather than at either.  `coalesce_not_null` gained the record arm — members
  read at the synthetic struct's offsets through the same `get_val` that `.0` uses, since
  `Value::TupleGet` addresses a stack tuple by var and index and has no spelling for a record
  field.  The presence test comes FIRST and short-circuits, because a record that is not there
  has no members to read.  That arm precedes the `Optional(Reference)` one on purpose: a boxed
  tuple matches that too, and answering it there is the second route above.

  ⚠ **CLOSED 2026-09-16, the DISCHARGE half: `??` over the boxed spelling.**  `(T-Absent)` names
  `t == null` and `t ?? d` as ONE question, so closing only the first left the rule half-kept.
  The coalesce typed its result from the boxed subject, and the stack default then had nowhere
  to go — *"`??` default of type `(integer, text)` is not assignable to `__tuple<integer?,text?>`"*.

  The arm that answers this pair already existed (loft#1451): when the subject is boxed and the
  default is a stack literal, take the STACK spelling, because the value unboxes.  What declined
  was its GATE — `unboxes_stored_tuple` demands the record's members be `is_equal` to the
  destination's elements, and `__tuple<integer?,text?>` against `(integer, text)` differs by
  exactly the `?` per member that `(N-Coal)` is about to discharge.  Measured on the other side:
  the same `??` over a generic `-> T?` at a tuple has always worked, because that boxes
  `__tuple<integer,text>` with NON-null members and the equality holds.

  Cure: the result takes the record's OWN members in their stack spelling, `(integer?, text?)` —
  which is the type the stack home already produces for the same notion, so the two homes agree
  rather than one inventing an answer.  Nothing is widened: the subject's conversion is then the
  unbox that arm already names, with its equality holding on both sides.

  ⚠ **Typing it as the NON-null stack tuple would have been unsound, and loudly so:** the
  subject's conversion would have had to unbox NULLABLE members into NON-NULL elements, which is
  the store direction `(N-Store)` exists to refuse.  Keeping the members is what makes the cure a
  retyping rather than a widening.

  Three refusals were measured BEFORE the change and are byte-identical after, each at its own
  site: a local's retype (*"Variable 'z' cannot change type …"*), a declared non-null tuple
  RETURN (*"expected `__tuple<integer,text>`, got `__tuple<integer?,text?>` on return"*), and a
  mixed-home `if` join (*"… on else"*).  `(N-Store)`'s per-member report appears at none of them
  — every route refuses earlier, on a type comparison — so the boxed member-nullable tuple has no
  reachable path into a non-null slot for this close to have loosened.  Guard cell a5.

  ⚠ **Not this entry, and the cure must not absorb it:** `?` on a member-nullable tuple is
  refused in BOTH homes — `(integer?, integer?)` on the stack refuses identically to
  `__tuple<integer?,text?>`.  That is the pre-existing refusal recorded above, not a record-home
  fact, and a probe that reads it as one is measuring member-nullability while believing it is
  measuring the home.

  Guard: `tests/scripts/an-absent-tuple-meets-the-type-its-author-declares.loft` (a1/a2/a3 plus
  eight controls); its a2 cell now pins `q == null` on both the absent and the present return.

  ⚠ **loft#1478, CLOSED 2026-09-09, and its lesson is about this doc's own oracle.**  A `text`
  MEMBER of an element read by a variable index did not COMPILE on `--native` (E0308, `&str`
  into a `String` slot), and under that a nested tuple member emitted a move where a clone was
  owed (E0382).  Both from ONE omission: `generation::dispatch`'s assignment arm asks *"is this
  slot a tuple?"* NINE times to pick a member's coercion, and five of them asked it BARE — so
  the `Optional` this rule's own `(N-Domain)` wrapper puts on the slot hid the slot from its own
  coercions, while the Rust type rendered is the bare tuple either way.  `generation::
  var_tuple_elems` is the one home now.

  **A CONSTANT index could not reach either refusal** (`(N-Index)` trusts it, so no wrapper is
  built), and **an all-`integer` tuple could not reach them at all**, because the coercions
  missed are the ones only a non-`Copy` member needs.  That is the blind population: this
  entry's cells, and this doc's counts, have been read over `(integer, integer)` shapes.  **Any
  cell added here should carry a `text` member.**

  ✅ **That instruction is DISCHARGED for this entry's own cells, 2026-09-10.**  All six were
  re-run over `(integer, text)` on both backends and answer exactly as the `(integer, integer)`
  re-measurement above records: the LANDING type is still refused (*"cannot change type from
  `(integer?, text?)` to `(integer, text)?`"*), `v[i].0` / `.1` by a variable index answer
  `null(oob)` / `null`, `==` `null` answers `true` on BOTH spellings, `v[i] ?? (7, "def")` gives
  `(7, "def")`, `v[i]?` gives the members' defaults `(0, "")`, and a PARTLY present
  `(null, "keep") ?? (9, "def")` keeps the `"keep"` and reports `== null` false.  So the
  deviation is one cell over the heap population as well as the scalar one, and the count above
  is not an artefact of the shape it was read on.  The instruction still stands for cells ADDED
  here — it is the entry's measured cells that are now clear, not the class.

  A third defect came out from under it — the same read through a struct FIELD's vector leaks
  its work-ref record on `--native` (loft#1479) — which is newly REACHABLE rather than newly
  broken, since that cell did not compile before.

  ⚠ **And a cure that suggests itself is measured WRONG.**  `(N-Opt)`'s side condition got its
  home in `data::has_null` on 2026-09-09 (loft#1478), and the obvious next step — have the `τ?`
  constructors ask it, so the index stops minting `(integer, text)?` — makes this worse, not
  better: the read then types `(integer, text)`, non-null members holding nulls, with no
  diagnostic.  It trades a type the language cannot spell for one that LIES about what it holds.
  "No `Optional(Tuple)`" is not "no absence"; the tuple's absence has a form and the cure is to
  BUILD it.  `data::constructs_optional` is that carve-out.

  ⚠ **The carve-out is PERMANENT, and the sentence above it said otherwise until 2026-09-16.**
  It read *"it exists to be deleted when this entry closes — the gap between the two predicates
  IS this deviation, in code"*, and the owner's option-2 ruling retired that: `(T-Absent)` now
  refuses `(τ₁, …, τₙ)?` at every DECLARATION and has the compiler carry an arriving absence
  with the `?` on the OUTSIDE.  So the two predicates answer two different questions —
  `has_null` whether a `τ?` may be DECLARED for this τ, `constructs_optional` whether absence
  may be MARKED for it in flight — and both stay.  The same stale claim stood in two other
  homes (`types.md (N-Opt)`'s side-condition note and `QUALITY.md`'s unspan row, which also
  predicted the row would "go back down by one"); all three are corrected.  What this entry
  still carries is the `?` DISCHARGE, below — not the existence of the in-flight spelling.

  **Removal** (stale, kept for the reasoning it records): it named `Type::optional` as the one
  home a `Tuple` would map to its member-nullable form.  Option 2 declined exactly that — the
  in-flight spelling stays — so this is not the removal path any more.

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
  refused — that half is `1423-a-nullable-tuple-type-is-refused-by-name.loft` and does not move.

  ⚠ **RE-MEASURED 2026-09-16 — and SUPERSEDED the same day by the ruling above, which closed the
  entry.**  What follows is the state the ruling was made ON, kept because it is the evidence
  behind it.  The re-measure said this entry did NOT close and that ONE cell remained, the `?`
  discharge.  It was owed because the option-2 ruling had turned the entry's own headline claim
  (*"the code still mints an `Optional(Tuple)`"*) into a description of what the rule
  PRESCRIBES rather than a deviation, so the count could only be settled by asking the cells
  again.  Its guards answered green on both backends
  (`an-absent-tuple-meets-the-type-its-author-declares.loft` a1-a4 + c1-c8, and the
  boxed-discharge guard the later ruling deleted), and the declaration refusal held.  What did
  not:

  - `?` on the IN-FLIGHT spelling answers the members' defaults and discharges to the NON-NULL
    tuple: `x: (integer, text) = v[i]?` lands, reading `0 ""`.
  - `?` on the WRITTEN `(integer?, text?)` is REFUSED on both backends — *"`?` cannot build a
    default for `(integer?, text?)` — discharge with `?? <default>` instead"*.
  - The same value decides it, so no other axis is in play: `u = v[i]; u?` answers, and
    `u: (integer?, text?) = v[i]; u?` refuses.  One expression, inferred versus annotated.
  - The axis is MEMBER-NULLABILITY, not the home — the in-flight spelling over a
    `vector<(integer?, text?)>` refuses too, and so does the BOXED one, naming
    `__tuple<integer?,text?>`.  A probe that reads this as a home fact is measuring the wrong
    thing while believing otherwise.

  **Mechanism:** `Data::has_default` admits `Type::Optional(_)`; `Parser::build_default`'s tuple
  arms recurse per member WITHOUT peeling and have no `Optional` arm, so one nullable member
  sends the whole tuple to `_ => None`.  The caller's own comment already names that class —
  *"`has_default` admitted the type and the builder cannot form its value — the two disagree,
  which is a compiler defect"*.  A top-level `τ?` never shows it, because the caller peels
  `base` before asking; only a MEMBER reaches the builder unpeeled.

  ⚠ **And the obvious cure is not obviously right, which is why this is a RULING and not a fix.**
  `(D-Opt)` says `construct_default(τ?) = null`, so a literal member-wise default of
  `(integer?, text?)` is `(null, null)` — the absent tuple itself, not `(0, "")`.  The entry's
  own sentence *"`t?` reads the members' defaults"* resolves two ways depending on which
  spelling's members are read.  The case that separates them is a PARTLY PRESENT tuple, which
  the written spelling can hold and an index miss cannot produce: measured, `??` and `==` are
  WHOLE-wise on both spellings and agree — `(null, "keep") ?? (9, "d")` keeps `(null, "keep")`
  and `== null` is false.  So typing a written tuple's discharge as `(integer, text)` would put
  a live null in a non-null slot on the pass-through path, the lie `(N-Store)` exists to refuse.
  `(T-Absent)`'s `≡` therefore holds at the two ENDS — all-present and all-null — and not as
  type equality.  Three admissible answers, none derivable from the rules as written:
  1. member-wise `?` → `(0, "keep")`, typed `(integer, text)`.  Sound, but splits `?` from the
     `??`/`==` pair `(T-Absent)` names as ONE question.
  2. whole-wise `?` with the result still `(integer?, text?)` — replaces an all-null tuple with
     the member defaults and passes anything else through.  Consistent with `??`, but then `?`
     does not remove nullability, which it does everywhere else in the language.
  3. keep the refusal and amend `(T-Absent)` to say `?` reaches the in-flight spelling only,
     recording that the written one has states the in-flight one cannot reach.

  Until one is chosen the refusal is the safe answer: it is loud, both backends agree, and no
  guard pins it, so whichever way it is ruled costs no expectation.  `OPEN` stays **1**.

D-tup-14 OPENED AND CLOSED 2026-09-15 (loft#1532): `(T-Cons)` copies a heap member in and
`layout.md (L-Tuple)` makes a member a field, and a member WRITE honoured neither.  `w.i = e`
put `e`'s handle into the slot, so a named source — a local, a field, a vector element, another
tuple's member, a vector local — stayed a second name for the member, and the record the write
replaced was released by nobody; for a droppable member the construction's own scope-end release
then ran on a freed store.  A literal member built from a field PROJECTION aliased the same way,
because `Parser::tuple_member_owned_copy` copied a local or a tuple member and nothing else.
Closed in two halves.  The write copies through the literal's own helper, which now takes a
record projection as a place — for a record that owns no DROPPABLE.  A droppable projected out of
a live container keeps its alias as a member, literal or written: copied, it would be released
twice, the double release its struct-field, payload, push and vector-literal siblings already
carry in `tests/ownership_drop_gate.baseline` (`c_field_field`, `c_elem_push`, …), and the
hand-off that moves the container's release to the copy is the cure those share.  The scope pass hands a member a later write names to the tuple
ALONE at its literal (`Scopes::written_tuple_members`), so each write releases the record it
displaces — without its hook, `heap.md (H-Drop-Not)`, as the struct-field write does — and
disarms the claimant of the value it writes.  Guards: `a-value-written-into-a-tuple-member-is-copied-in.loft`
(the copy, and a droppable member's trace) and `a-tuple-member-write-releases-the-record-it-replaces.loft`
(the release).

D-tup-13 OPENED AND CLOSED 2026-09-10 (loft#1509): `(T-Proj)` says `t.i` reads that element and
`heap.md (H-Drop)` runs the hook once per resource, and a RETURNED member satisfied neither —
it read as a ZEROED record, or its resource went missing and its store leaked.  A tuple
member's own backing is minted by `Parser::tuple_member_owned_copy`, which asked the SHARED
`__ref_N` sequence; that mint deliberately re-finds a promoted RETURN BUFFER when the type
matches, because pass 2 re-minting the same name for the same ROLE is how the buffer is found
again (`Vars::retypes_argument`).  A member's backing is a different role.  So in a function
whose return type is that record, the member's store and the return buffer were ONE variable,
and the materialised-view return re-mints the buffer before copying into it: the member's
record was cleared just before it was read.  Returning the member that DREW the buffer handed
the caller the cleared record; returning anything else lost that member's resource, because the
literal's copy had already suppressed its source's own drop.

The scope is the RETURN TYPE, not the tuple.  `fn r() -> S { s = S{…}; t = (s, 5); return S{…} }`
— where nothing in the source connects the two — lost the member's resource and leaked its
store, while the same body returning a DIFFERENT record type was always correct.  That pair is
what the guard pins the axis with.

The rule was already written for the site.  `Vars::work_refs_p2` exists because *"a mint site
that can fire ONLY on pass 2 must not draw from the shared `__ref_N` sequence"* (loft#848, the
same collision for a block used as a value), and `tuple_member_owned_copy` opens with
`if self.first_pass { return None; }`.  Cure: that one mint draws from the pass-2 sequence.
Guard `a-returned-tuple-member-is-not-the-return-buffer.loft`, nine cells, scoring the release
trace with the leaked store as an independent second channel.

⚠ Three shapes the same guard's neighbours reach are NOT this entry and stay in `heap.md`
D-heap-1: a copy off a tuple PARAMETER's member, a bound projection returned
(`x = t.0; return x`), and a nested member returned (`return t.0.0`).  Each releases twice for
its own reason and none of them was the buffer collision — measured against a pristine control
build, where all three read exactly as they do with this one fixed.

D-tup-12 closed 2026-09-10: `(T-Proj)` spells a projection `.i` with a LITERAL index, and a
tuple has no other member.  A record-backed tuple is carried as the SYNTHETIC STRUCT
`__tuple<…>`, whose attributes are named `_0`, `_1`, …, and that home's projection site
claimed the member only when the next token was ALREADY an integer — a guard on the token
rather than an answer about it.  So a named member fell past it to the ordinary struct-field
reader, and `_0` was a second, undocumented spelling of `.0` that both READ and WROTE: `t._0`
answered `11` and `t._0 = 99` reached the vector's bytes, on both backends, while the same
source over a plain local was refused by name.  Which spelling a program could use was decided
by a REPRESENTATION choice with nothing in the source to show it — the stack local, the vector
element by a constant or variable index, the struct field, the `&(…)` parameter, the function
parameter and an all-integer return all refused; the `vector<(τ, τ)>` loop variable, the nested
loop variable and a heap-carrying return admitted.

Two further defects came out of the same fallthrough.  A named member reported *"Unknown field
`__tuple<integer,text>`.name"* — loft#1498's class exactly, a diagnostic naming a def the
author cannot write, about a member kind a tuple does not have; `Data::def_is_authored` is the
predicate that exists for it and this path never reached one.  And the refusal at all THREE
homes left the offending member in the token stream, so every one of them dragged a second
`Expect token ;` behind it — the cascade loft#868 removed from the unknown-receiver path,
still standing on this one.  Consuming the name is not enough on the LEFT of an assignment
(`t._0 = 9` then reads as `t = 9`, and the reader is told their tuple *"cannot change type
from `__tuple<integer,text>` to integer"*), so the errored member carries `fields.rs`'s
`Value::Drop` marker, which the assignment path already reads.

Two smaller faults on the same rule came out of the same pass and are closed with it.  The
index was read with `has_integer`, which matches only `LexItem::Integer`; the lexer switches
to `LexItem::Long` above `i32::MAX`, so `t.2147483647` reported out-of-range and
`t.2147483648` reported *"requires a numeric index"* about a literal that plainly is one —
one value apart, deciding which refusal on a width the author never chose.  And `.i` on a
receiver that is NOT a tuple reported *"Expect a field name"*, the token rather than the
situation; it now names the receiver's type and the cure for its kind, which is where
`(T-Paren)`'s warning about `(e)` finally reaches a reader.

Closed by giving the two questions ONE home each — `tuple_member_not_a_literal` and
`tuple_index_out_of_range` in `parser/operators.rs`, cited from all three sites.  Three copies
of one refusal is what let a fourth spelling of it be a token GUARD instead: the guard reads
as an answer until you ask what happens when it does not hold.  Guards: the six cells in
`102b-pass1-expected-errors.loft` (falsified against `2e408acc8` on both backends; the two
stack-tuple cells are the control that says the three homes were made to AGREE rather than all
moved).  `contract: settled` — the rule already said the member is a literal index; nothing
about the language changed, only which spellings reach it.

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
- **Destructure (`T-Destr`)** — `(a, b) = (5, 9)` binds `a=5, b=9`.  With a HEAP member and on
  both backends (2026-09-10), each binding is a COPY as `B-Copy` requires, checked by mutating
  each side and reading the other: from a LOCAL tuple (`t = ("alpha", v)`, then `v += […]` and
  `b += […]` leave `t.1` at its own length), from a CALL return, from a vector ELEMENT by a
  constant index, from a struct FIELD, from a LOOP variable, and at arity 3 with the heap member
  in the MIDDLE.  By a VARIABLE index it is refused — that value is `(τ, τ)?` by `(N-Index)` and
  the refusal names the two cures, which is `(T-Absent)` and not a gap.
- **Return independence (`T-Ret`)** — a tuple built from a LOCAL that dies at the return is
  live at the caller: `fn mk() -> (text, vector<integer>) { local = [7,8,9]; ("alpha", local) }`
  unpacks to a 3-element vector reading `7` and `9` at its ends, both backends, under strict
  stores.
- **Tuple return + unpack (`T-Ret` + `T-Destr`)** — `fn pair() -> (integer,integer) { (2,3) }`,
  `(x, y) = pair()` binds `x=2, y=3`.
- **Reference tuple (`T-Ref`)** — `fn sw(p: &(integer, integer)) { t = p.0; p.0 = p.1; p.1 = t }`
  swaps the CALLER's tuple: `(1,2)` reads back `2,1`. Verified on both backends for every
  admitted element type — `integer`, `float`, `single`, `character`, `boolean` — uniform and
  mixed (`&(integer, boolean, character)`), and at width 3 so the last element is reached
  (`tests/scripts/1006-reference-tuple-element-types.loft`).
- **Refused element types (`T-Ref-El`)** — a NULLABLE element (`&(text?, text)`), a fn-ref
  (`&(fn() -> τ, …)`) and a NESTED TUPLE are STATIC errors naming the element type, never an
  ICE (`tests/scripts/102-expected-errors.loft`). A bare `text` element and a struct element
  are ADMITTED — the record-backed form, measured 2026-09-10 on both backends and guarded by
  `tests/scripts/reference-tuple-heap-elements-link.loft`.
- **Not a tuple (`T-Paren`)** — `.i` on a receiver that is not a tuple names the receiver's
  type and the cure for its KIND: `v.0` on a vector says to index it (`[0]`), `s.0` on a
  struct says to name the field, and a scalar gets the rule itself — `(e)` is grouping, not a
  1-tuple.  That last is the case `T-Paren` exists for: `x = (5); x.0` reads a tuple element
  off an `integer`, and the reader used to be told only *"Expect a field name"*, about the
  token rather than the situation.  A trailing comma is refused separately and by name
  (*"Tuple literals require at least 2 elements"*), so `(5,)` is not a 1-tuple either.
- **Static index (`T-Proj`)** — `t.5` on a 2-tuple is a compile error, not a runtime null.
  Asked of all THREE homes a tuple has (2026-09-10, both backends): the stack `Type::Tuple`,
  the record-backed `__tuple<…>` a `vector<(τ, τ)>` loop variable and a heap-carrying return
  carry, and a `&(…)` reference tuple.  Each reports the same two refusals — an out-of-range
  literal index, and a member that is not a literal at all — and each reports exactly ONE
  error, because the refusal now consumes the offending member instead of leaving it for the
  statement parser (`102b-pass1-expected-errors.loft`).  The index is read as a LONG, so an
  index above `i32::MAX` reports out-of-range like every smaller one: the lexer changes token
  kind there, and which of the two refusals a program earned used to turn on that width.  The positive half, that `.0`/`.1`
  read the same element from every home and still WRITE through a loop variable, is
  `822-vector-tuple-spellings.loft`.  The home is what made this worth asking three times:
  see D-tup-12.
- **Heap element is COPIED (`T-Cons`)** — `t = (h, 9); h[2] = …` leaves `t.0` at its old length
  for EVERY heap element type, not just the vector the paragraph above names: `hash`, `hash<τ>?`,
  `sorted`, `index`, `trie`, `spatial`, and a DEEP case with a nested `vector<text>` inside the
  element. Both backends
  (`tests/scripts/1230-a-keyed-tuple-element-owns-a-copy.loft`).

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on a
tuple's element order, values, or a projection is the definitional error this doc names.
