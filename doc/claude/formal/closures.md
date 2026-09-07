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

⚠ **What is still open is the rebind OUTSIDE the closure, and the rule does not yet say which is
right.** `e =
[Row { k: 1, v: 51 }]` over a captured `hash` / `sorted` / `index` REFILLS the existing store
rather than minting one, so the closure reads the reassigned value where the vector and struct
spellings read the build-time one. Both are "the closure kept its `DbRef`"; what differs is
whether a rebind mints. Measured on all three keyed kinds, both backends, and unchanged by
loft#1324's fix — the store-lifetime half is correct either way, so this is a contract question
rather than a leak, and it is open.

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

**OPEN: 0.**  Every deviation this doc has carried is closed; the record is in
[closures-history.md](closures-history.md).

> **An `OPEN: 0` is a claim to re-measure, and this is what its oracle covers.** The closing
> guards are `1248-…` (a fn-ref `??` join's argument witness and single capture witness),
> `1248b-…` (the capture SLOT: two store-bearing captures, a captured collection, a capture
> beside a pure mint, a capture returned directly), `1257b-…` (a collection return freed by
> identity, every kind and spelling) and `1320-…` (a branch-joined binding).  What they hold
> FIXED: every closure is built in the frame that calls it, every witness variable is assigned
> once, and no closure is stored in a container or in a struct a container holds (a decided
> refusal, C115/#247).  Two shapes are DECLINED and asserted by value only — `c ?? d`, where
> either capture may come back, and a capture variable reassigned after the build — and each
> keeps the leak it had.

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
