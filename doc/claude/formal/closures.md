<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/closures.md — semantics for lambdas, closures, and fn-refs (strict)

**Catalogue:** @F22 (closures & value capture), @F23 (function references), @PLN89 (oracle).

> **Rules then deviations** (see [README](README.md)). This is the relation for loft's
> **first-class functions**: the two lambda forms, closure **capture**, function references, and
> application. It extends [calls.md](calls.md) (application is a call) and [heap.md](heap.md) (a
> capturing closure's environment is a heap record). Unlike the other operational files, this one
> has **open deviations**: the two lambda forms differ in capture, and one combinator path
> crashes. The Rules below are the **intended contract** (what a user should be able to rely on);
> the Deviations are exactly where today's implementation falls short — written so they can be
> driven to zero.

## The two forms — pure syntactic sugar (both capture)

loft has two lambda syntaxes, and (since 2026-07-04, D-clo-1 closed) they are **pure syntactic
sugar for the same thing** — both capture outer variables identically. The only difference is
ergonomics:

| form | captures outer locals? | ergonomics |
|---|---|---|
| `fn(p: T, …) -> R { body }` | **yes** | explicit parameter + return types; use anywhere |
| `\|p, …\| { body }` / `\|\| { body }` | **yes** | parameter types INFERRED from context (a `map`/`filter` callee's element type); no `->` return annotation |

A bare function name (`f`, not `f()`) is a **function reference** — a first-class value of type
`fn(T…) -> R`, a closure with an empty environment.

## Notation

Uses [calls.md](calls.md)'s call relation and [heap.md](heap.md)'s heap `H`. A **closure** is a
pair `⟨code, env⟩`: the lambda body plus a captured environment (the outer variables it names). A
**fn-ref** is a closure with an empty environment.

---

## Rules

### Construction — a closure captures the outer variables it names

```
  (L-Fn)     fn(p₁…pₙ) -> R { body }  AND  |p₁…pₙ| { body } / || { body }   both evaluate to a
             closure ⟨body, env⟩ where env captures every OUTER variable the body references but
             does not bind.  The two forms are equivalent modulo type-annotation ergonomics.
  (L-FnRef)  a bare function name f (in a value position / fn-typed context) is a fn-ref value —
             a closure with an empty environment.
```

**In words.** Both `fn(y) -> integer { y + x }` and `|y| { y + x }` build a closure that
**captures** `x` from the surrounding scope — they are the same construct, differing only in that
the `|…|` form infers its parameter types from context (so it is the ergonomic `map`/`filter`
callback) while the `fn(){}` form spells them out (so it works where no type context is
available). A bare `f` (a function's name used as a value) is a first-class function reference.

### Capture semantics — scalar by value at creation, heap shared

```
  (L-CapScalar)  a captured SCALAR is captured BY VALUE at closure creation: the closure sees the
                 value the variable had when the closure was formed.
  (L-CapHeap)    a captured HEAP value (struct/vector) is SHARED: a mutation-through the source
                 AFTER capture is visible inside the closure (consistent with calls.md
                 F-ParamHeap — capture, like a call, shares heap state, copies scalars).
  (L-CapWrite)   a captured SCALAR the closure REASSIGNS is SHARED IN THE WRITE DIRECTION:
                 the closure's write reaches the outer variable, and a later call sees what
                 the previous one wrote.  This does not weaken (L-CapScalar) — the closure
                 still starts from the creation-time value and still does not see the outer's
                 later writes — it says only where the closure's OWN write lands.  Sharing one
                 such variable between TWO closures is refused (a decided limit, not a gap);
                 the cure the refusal names is a struct field, which is (L-CapHeap) and shares
                 in both directions.  The rule is nullability-agnostic: `Ï?` and `Ï` agree on
                 all of it, which is what loft#1408 restored.
  (L-CapBox)     a captured scalar the closure REASSIGNS keeps its declared type exactly.
                 Its WIDTH, its RANGE and its reserved null sentinel are what the declaration
                 says, whatever storage the write is routed through — so a narrow capture
                 refuses the stores its type refuses, takes its own range's default on one
                 out of range, and answers identically to the same variable never captured
                 at all.  The share (L-CapWrite) grants is about WHERE the write lands; it
                 grants nothing about what the variable may hold.
  (L-CapOwn)     a captured heap store is freed ONCE, by whichever of the two outlives the
                 other.  A closure record that LEAVES its defining frame takes the release over:
                 its cascade frees what it adopted, so the frame must not.  A record left BEHIND
                 never frees at all — the fn-ref type carries its frame dep so the scope sweep
                 skips it — so there the frame's release is the store's only one.  The record's
                 reach is its CASCADE: a capture attribute the cascade does not follow is not
                 covered by any adoption, whatever the free-suppression believes.
  (L-CapRef)     capturing a `&T` parameter (calls.md F-ParamRef) captures its POINTEE: the
                 `&` is a channel to the CALLER's slot, so the share-or-copy question is asked
                 of what it points at.  A `&S` / `&vector<τ>` is then SHARED by (L-CapHeap) —
                 the same DbRef either way, so a field write, an element write and an append
                 from inside the closure all reach the caller — and a `&integer` / `&text` is
                 COPIED at creation by (L-CapScalar).  The `&` itself does NOT survive into
                 the closure, so a write that would replace what it points AT is REFUSED
                 (a decided edge, DESIGN_DECISIONS C115 — not a deviation: the copy is what
                 (L-CapScalar) requires, so no code change closes it).
```

⚠ **A REASSIGNMENT of the captured variable is not a mutation-through, and the two are worth
keeping apart.** `(L-CapHeap)` shares the store, so `b.v = 9` after the capture reads through;
`b = other` does not, because it rebinds the variable while the closure keeps the `DbRef` it was
built with. That is what a captured vector and a captured struct do — the closure answers the
build-time value. It is also the fact that decides which store the record takes over the free of
([ownership.md](ownership.md) `O-Latest`, loft#1324).

⚠ **A rebind INSIDE the closure now replaces at every keyed kind too (loft#1326).** `(L-CapHeap)`
shares the store, so `m = […]` written inside a closure over a captured `hash` / `sorted` /
`index` / `trie` replaces its contents, exactly as the captured vector spelling does and as the
same collection reached through a captured struct field always did. It used to EMPTY the
collection: the keyed replace is selected on the destination being a struct FIELD, and a capture
is an `OpGetDbRef` rather than an `OpGetField`, so the branch was skipped — the fourth time that
family of selector has been the narrow part while its lowering was already right.

⚠ **CLOSED 2026-09-08 (loft#1447): the keyed kinds now mint, and the rule said so all along.**
A rebind outside the closure reads the BUILD-time value at every kind, as the two paragraphs
above already state for a vector and a struct — `hash`, `sorted` and `index` answered the
reassigned value because `gen_keyed_null(first = false)` cleared the store in place and reused
`store_nr`, so the record's own handle saw the rebind.  The licence is POSITIONAL, which is why
the fix is in the PARSER: `is_captured` is a whole-FUNCTION fact, so minting on it alone orphans
the store a declaration just allocated.  The parser emits the mint at loft#895's local-replace
site, which only a NON-EMPTY keyed literal reaches, and keeps the `Set(v, Null)` after it so
`@FR-O-Latest`'s scan still learns the local was reassigned and the frame still frees what it
now names.  Guard `1447b-a-captured-keyed-local-rebinds-into-a-fresh-store.loft`;
`1324-…`'s keyed cell asserts the answer it used to leave open on purpose.

The paragraph it replaces, kept because the shape of the question is worth reading: `e =
[Row { k: 1, v: 51 }]` over a captured `hash` / `sorted` / `index` REFILLS the existing store
rather than minting one, so the closure reads the reassigned value where the vector and struct
spellings read the build-time one. Both are "the closure kept its `DbRef`"; what differs is
whether a rebind mints. Measured on all three keyed kinds, both backends, and unchanged by
loft#1324's fix — the store-lifetime half is correct either way, so this is a contract question
rather than a leak, and it is open.

⚠ **Measured 2026-09-08 (loft#1447), and the shape of the fix is now known even though the
contract call is not.** Both emission sites are the same two lines wearing different names —
`state::codegen::gen_keyed_null` and `generation::dispatch::emit_null_dbref`, each `if first {
… }` then `OpDatabase`, the `!first` arm reusing `store_nr`. Asking `rebind_must_mint(v)`
beside `first` at both makes `hash`, `sorted` and `index` answer the build-time value on both
backends, with the struct and vector controls unmoved — so making the keyed kinds AGREE with
the other two is one term in two places. What blocks it is not the contract: it leaks five
stores, because **`is_captured` is a whole-FUNCTION fact and the licence question is
POSITIONAL.** `set_captured` runs when the closure BODY is parsed, so the predicate is true for
assignments that precede the build — and `h: hash<K[id]> = []` emits TWO `Set(v, Null)` (the
declaration, then the statement's own lowering) with the record built after both, so minting at
the second orphans the store the first allocated. The dense spelling is correct only because
`parse_object` is a parser site where position is known. Closing this needs the parser to record
where the capture is BUILT — not a wider predicate.

**Boxing is invisible, and that is the rule.** A scalar the closure writes to has to live
somewhere both sides can reach, so it moves off the stack into a one-field record.  That is a
change of ADDRESS, never of type: a `u8` capture is still a `u8`, one byte wide, refusing what
`u8` refuses.  Read `(L-CapBox)` as the thing you may assume when you cannot see where a
capture is stored — the declaration is still the whole truth about it.

**In words.** A closure that captures an `integer x` freezes `x`'s value at the moment the closure
is built (verified: capture, then `x = 20`, still yields `10`).  If the closure ASSIGNS to `x`,
that write is not lost: it reaches the outer `x` and the closure's next call sees it
((L-CapWrite), verified on both backends for `integer`, `text`, `float`, `boolean` and a plain
enum, in each spelling with and without `?`, including when the closure is called through
another function — `tests/scripts/1408-…`).  The two halves read as one sentence: the closure
owns the variable's value from the moment it is built, and hands it back. A closure that captures a struct or
vector shares it — mutating a field of the captured value afterwards shows up when the closure runs
(verified: `b.v = 9` after capture yields `9`). This mirrors the parameter contract in
[calls.md](calls.md): heap is shared, scalars are copied.

### First-class — store, pass, return; application is a call

```
  (L-Apply)   ⟨c(args), σ⟩   applies closure/fn-ref c: bind its parameters to args (calls.md
              F-Args/F-Param*), run body in ⟨code, env⟩, yield the return (calls.md F-Call).
  (L-Escape)  a closure is a VALUE: it may be stored in a variable or struct field, passed as an
              argument, and RETURNED from a function — a returned closure keeps its captures
              (it escapes cleanly).
```

**In words.** A closure is an ordinary value. You can put it in a variable or a struct field, pass
it to another function, and return it — and a returned closure still remembers what it captured
(verified: `fn mk(n) -> fn()->integer { fn()->integer { n } }`, then `mk(7)()` yields `7`; a
closure in a struct field `h.f()` yields `42`). Calling it is just a call ([calls.md](calls.md)),
with the closure's environment in scope.

---

## Deviations

**OPEN: 1.**

- **D-clo-24** *(closed 2026-09-07, loft#1440)* — two closures over ONE store both adopted it,
  and their deaths are independent: the record left behind released what the escaped one still
  held.  Closed the way `(L-CapOwn)` says — among the records that adopted a store exactly one
  keeps it, the one that LEAVES the frame, and the rest borrow; where none leaves, the first
  keeps it, which is the single-record case unchanged.  The grouping is by the STORE a record
  adopted, not by the capture's name: a local assigned between two builds gives its two records
  different stores, and a name-keyed group made one borrow what the other never held.  Guard
  `1440-one-store-has-one-owner-among-two-closures.loft`.
- **D-clo-26** *(closed 2026-09-07, loft#1446)* — the same rule applied correctly still left a
  use-after-free where the shared local is REASSIGNED after the build: the frame kept its own
  free there (`capture_adoption_owns_free` declines to suppress it, loft#1324/#1388), and that
  free reached the store the escaped record holds rather than the one the local now names.
  Closed on the currency `@FR-O-Witness` names, store IDENTITY: a literal BUFFER holds one store
  for its whole life where a capture's NAME covers two, so the adoption is recorded against the
  buffer (`CaptureBuilds::buffer_adopted`) and the frame's release is decided on it
  (`escaping_record_holds_buffer`).  The runtime test the arm used to emit cannot express the
  question — `OpFreeRefIfDistinct(buffer, local)` compares against the LOCAL, which after a
  rebind names the other store, so the guard read "distinct" and freed exactly what the escaped
  closure was reading.  **The filed shape was wider than the defect:** it was reported as needing
  two closures with one escaping, and ONE closure reproduces it, so loft#1440's grouping is not
  involved.  Guard
  `1446-a-capture-reassigned-after-the-build-is-freed-by-store-identity.loft`.
- **D-clo-28** *(closed 2026-09-08, loft#1443)* — `(L-CapOwn)` says the record that LEAVES its
  defining frame takes the release over, and the code recognised exactly one way of leaving: a
  RETURN.  A `&fn(…)` link is a second, and loft#1443 opened it — before that the parameter did
  not compile — so the callee freed the record it had just written out and the cascade took the
  capture with it; the caller then called a closure over `0xDEADBEEF`.
  `scopes::record_leaves_frame` had reserved the place for it in as many words (*"when it is
  implemented this predicate gains a second source and must be told"*) and the implementing
  change did not tell it.  Closed by reading the link writes the way `returned_closure_records`
  reads the returns — the records the assigned value YIELDS, on every arm — at the three sites
  that each ask "does this leave the frame": the heap sweep, the fn-ref sweep whose free
  TRIGGERS the cascade, and `record_leaves_frame` itself.  Two refinements the return route does
  not need, because a return ends the frame and a link does not: a write DISPLACED by a later
  write to the same link in the same operator list delivers nothing and is the frame's to free
  after all, while an `if`'s two arms are separate lists and both deliver.  The source
  resolution is one home (`closure_records_of_source`) for both routes.  **The guard was green
  over the live defect**: all 13 cells of
  `1443-a-closure-written-through-a-fn-parameter-link.loft` pass on the broken build, because a
  freed arena slot still reads back the bytes it held; `LOFT_POISON=1` fails 8 of them, which is
  the nightly gate that caught it.
- **D-clo-29** *(closed 2026-09-08, loft#1464)* — `(L-CapOwn)` says a captured heap store is
  freed ONCE, and where the closure BUILD sits inside a conditional block it was freed ZERO times
  on the path that skips the build.  The frame gives up its own release in favour of the record's
  cascade (`capture_adoption_owns_free`), but the suppression is decided from the CAPTURE
  relation — a static fact about the function — while the cascade that replaces it happens only
  if the build EXECUTES.  Struct and vector captures leaked, text did not (its own free path is
  separate); a zero-iteration loop body is the same shape.  Not a link question: it reproduced
  with no `&fn` anywhere.
  **The filed scope was the narrow half.** Measured, the same sentence also covered a `match`
  arm, a conditional nested two deep, a record that ESCAPES the frame, a capture REASSIGNED after
  a conditional build (whose suppression runs through the owner witness instead), two captures in
  one build, and — the cell that says the fix cannot be keyed on "some record was built" — one
  record PER ARM, where `(L-CapOwn)`'s single owner is picked statically and the arm that builds
  only the BORROWING record still owes the store.
  Closed by keeping the frame's release there and making it conditional on the OWNING record
  existing.  The record local is the witness: `emit_lambda_code` inits it to the empty slot and
  the build is the only thing that mints into it, so *"a record is there"* and *"the cascade that
  replaces this free will run"* are the same fact — `@FR-O-Witness`'s currency, a fact only the
  run knows, read off a slot at the moment the answer is needed.  Which record that is comes from
  the same grouping the borrow-marking reads (`capture_store_adopters` + `adoption_owner_index`,
  one home), because naming an adopter the marking made BORROW would decline the frame's free in
  favour of a cascade that stops at that attribute.  The guard runs BEFORE the record's own free:
  the two backends do not agree on what a freed slot reads back as — the interpreter leaves it
  standing, `--native` nulls `store_nr` — so a test placed after it declines on one and
  double-frees on the other.  Guard
  `1464-a-capture-built-in-a-branch-is-released-on-the-path-that-skips-it.loft`.
- **D-clo-27** *(open, loft#1447)* — `(L-CapHeap)` says a rebind is not a mutation-through for a
  captured **struct** as much as for a vector, and the DENSE spelling breaks it: `d: C = C{a:5};
  out = fn() { d.a }; d = C{a:9}` answers 9 on both backends where its nullable twin answers 5.
  A dense local has no literal buffer — the literal is built straight into it and the rebind
  re-mints through `OpDatabase(d, …)`, which reuses the slot's store IN PLACE — so one store
  exists and the record's `DbRef` still names it.  Upstream of every free, so nothing is freed
  twice and no instrument fires; the in-place re-mint is deliberate (it is what keeps a loop from
  allocating per pass), and what is missing is that a local a record has ADOPTED cannot take it.
- **D-clo-25** *(closed 2026-09-07, loft#1444)* — "which record leaves the frame" was answered
  from the declared return type's `DepEntry::CalleeFrame`, which is published once per LAMBDA
  and overwritten, so wherever a function builds more than one it named the last one BUILT
  rather than the one the return delivers.  Two consumers read it for two questions — *is this
  fn-ref handed out, so its closure store must not be freed* and *which record outlives the
  frame* — and both were answered about the wrong record whenever the escaping closure was not
  written last.  Closed by asking the VALUES instead: the free-suppression also treats a fn-ref
  that is a RETURN SOURCE as handed out, and the ownership question reads
  `returned_closure_records` — the records named in RETURN POSITION, off the tail and off every
  `return`.  Guard `1444-the-returned-closure-is-the-one-that-keeps-its-capture.loft`.

> **An `OPEN: 0` is a claim to re-measure, and this one moved four times in a day** — 0 → 1 → 2
> → 0 → 1, each step a probe pushed one axis off what the oracle below holds fixed, and each
> answer measured rather than argued.  The
> closing guards are `1248-…` (a fn-ref `??` join's argument witness and single capture witness),
> `1248b-…` (the capture SLOT: two store-bearing captures, a captured collection, a capture
> beside a pure mint, a capture returned directly), `1257b-…` (a collection return freed by
> identity, every kind and spelling) and `1320-…` (a branch-joined binding).  What they hold
> FIXED: **every closure is built in the frame that calls it**, every witness variable is assigned
> once, and no closure is stored in a container or in a struct a container holds (a decided
> refusal, C115/#247).  ONE shape is still DECLINED and asserted by value only — `c ?? d`, where
> either capture may come back — and it keeps the leak it had.  The second, **a capture variable
> reassigned after the build, is no longer declined**: loft#1446 put it on store identity and
> `1446-…` asserts it, across one and two closures, one and two reassignments, a reassignment
> between the builds, `null` and a minting call as the source, a vector capture, and a build
> inside a loop.
>
> That first fixed axis is where loft#1439, loft#1440 and loft#1444 all live: a closure that
> OUTLIVES its frame.  `1439-an-escaping-closure-keeps-its-nullable-capture.loft` covers it now
> — the nullable capture that was read after release, with the dense, absent, collection, text,
> kept and record-enum cells beside it — and `1440-one-store-has-one-owner-among-two-closures.loft`
> covers two adopters, and `1444-the-returned-closure-is-the-one-that-keeps-its-capture.loft`
> moves the BUILD ORDER those two hold fixed — first, last and middle of three, delivered by a
> tail, by an `if` over two closures, by an explicit `return`, and written straight out.

`D-clo-18` and `D-clo-20` are decided refusals ([DESIGN_DECISIONS C115](../DESIGN_DECISIONS.md)),
not deviations: `(L-CapScalar)` gives a closure a COPY of a `&` scalar parameter, so a write to
it from inside the closure has no shared record to land in, and the heap twin takes the same
refusal one rule over.

The full register — every entry, open and closed, with its dates and issue numbers — is
the companion [closures-history.md](closures-history.md).

## Conformance

- **Both forms capture identically (`L-Fn`)** — `x=10; [1].map(|y| { y+x })[0]` is `11`, and the
  long form `[1].map(fn(y:integer)->integer{y+x})[0]` is also `11`; a captured heap value is shared
  (`b.v=8; [1].map(|z| { z+b.v })[0]` is `9`). A non-capturing `[1,2,3].map(|x| { x*2 })` is
  unchanged (`2`). (Guard `tests/scripts/85-short-lambda-capture.loft`.)
- **Capture semantics (`L-CapScalar` / `L-CapHeap`)** — a captured scalar reads its
  creation-time value; a captured struct reads its *current* field value (`b.v=9` ⇒ `9`).
- **First-class (`L-Escape`)** — a closure returned from a function, or stored in a struct field,
  works: `mk(7)()` is `7`; `h.f()` is `42`.
- **A fn-ref reaches every CONTAINER (`L-Escape`, measured 2026-08-22)** — vector element by
  literal and by `+= [f]`, keyed-collection value, struct-enum variant payload read
  per-variant, and struct-in-vector all carry one and call it back out, on both backends.
- **…and a place that ALREADY holds one takes a new fn-ref (`L-Escape`, D-clo-3)** — a live
  local, a live tuple member (guard
  `tests/scripts/fn-ref-reassignment-tops-up-the-pair.loft`), and a struct field, a vector
  element, an element's field, a field's element and a `&`-parameter's field (guard
  `tests/scripts/fn-ref-assigned-into-a-field.loft`), from a bare name, an inline lambda
  (capturing or not), a non-capturing local and a call — including over a field that already
  owns a closure record, and 200 times in a loop without the store growing. A source the
  LITERAL refuses (an `if`/`match` arm, P215; a capturing source into a collection, #247)
  is refused identically here, by the same diagnostic.
- **No-crash on an un-inferrable stored lambda (D-clo-2)** — `g = |y|{…}; xs.map(g)` now emits a
  clean "cannot infer" diagnostic on both backends, not a panic (guard
  `tests/leak.rs::dclo2_stored_short_lambda_map_no_crash`). The same diagnostic covers
  `any` / `all` / `sort_by` / `filter`: it fires at the LAMBDA, not per combinator.

Closures are a full first-class contract: construction, every container measured above, and
re-assignment into a place that already holds one. What a closure may not do is bounded by
two decisions rather than by gaps — one capture shape per fn-ref attribute, and no capturing
closure inside a collection (DESIGN_DECISIONS.md C116, loft#247) or inside a struct that a
collection holds (#318).
