# formal/iteration-history.md — the deviation register for [iteration.md](iteration.md)

> **The rules are next door.**  [iteration.md](iteration.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** — and the zero is new, not the one this line used to carry. That one was
re-measured on 2026-08-30 and did not hold: it rested on a corpus that varies the element
TYPE (`tests/scripts/1074-combinators-over-tuple-elements.loft`) but never varies what the
comprehension READS. Every cell drew from a source the destination does not name, so
`I-Comp`'s "fresh result, source untouched" clause was pinned only where it is trivially
true. Varying that one axis broke all THREE destination kinds at once — a local, a struct
field and a `+=` — and all three are now closed with their own guards. D-iter-1 remains
CLOSED.  D-iter-4 opened and closed 2026-09-06: the LITERAL was the fourth kind, found by walking
`@FR-O-Detach` rather than this doc's own corpus — which varied what the comprehension reads and
never asked whether a literal reads at all.  D-iter-5 opened and closed the same day: the two
destinations D-iter-4's snapshot could not NAME.  D-iter-6 opened and closed 2026-09-23 (loft#1619, below): a
place source and a range bound were re-read per round.

> **D-iter-4 — OPENED AND CLOSED (2026-09-06). A vector LITERAL that reads its destination.**
> `I-Comp` was walked three times for the comprehension (D-iter-1..3) and the literal — the same
> build without the loop — never once: `v = [v[1]?, v[0]?]` answered `[0, 0]`, `len(v)` inside
> the literal read `0` then `1`, a struct element read its `?? default`, and a parameter, a
> typed local, a struct field and a `+=` all read the result being built — sixteen spellings,
> both backends, silently (QUALITY.md B8a).  The build's detach (`create_vector`'s `=` repoint,
> `clear_vector_field`) sat at the head of the build's ops, ahead of the element reads.
> Closed by giving the comprehension's snapshot one home, `Parser::snapshot_read_destination`,
> which the literal asks too: the destination is copied before the first write, every read in
> the parts is renamed to the copy, and the two detach sites insert after it.  Guard
> `tests/scripts/a-vector-literal-reads-what-its-destination-held.loft`, falsified at 6f9c0886.
> Two destinations the snapshot cannot NAME remain — a field reached through an element, a
> captured collection — filed as loft#1391 and closed the same day as D-iter-5.

> **D-iter-6 — OPENED AND CLOSED (2026-09-23, loft#1619). A PLACE source, and a range bound,
> were RE-READ per round.**  (Filed in `iteration.md` as "D-iter-5", a number this register
> had already given to loft#1391; renumbered when it closed.)  `it := ⟨0, src⟩` and
> `(I-Range)`'s "the integers a, …, b-1" read `src` and `b` as VALUES taken once; the code
> re-read a place every round.  Measured, both backends alike: `s = "hello"; for c in s { s =
> "zz" }` walked two characters, `for c in st.s { st.s = "zz" }` the same; `w = [1,2,3]; for i
> in 0..len(w) { w += [9] }` ran five rounds, `m = 3; for i in 0..m { m = 10 }` ten, and `for i
> in 0..=f()` called `f` nine times for three rounds — `0..f()` four.  The value half (a text
> source that is a call) had closed that morning; the place half was filed as a language
> choice, since `(I-For)`'s own prose kept a collection's length re-read on purpose and the
> queue idiom `for i in 0..len(q) { …; q += [next] }` was written against it.
>
> **Closed by owner ruling (2026-09-23): the letter holds, and the body's write is told.**
> A non-literal range bound is bound to a hidden local in the loop prelude
> (`parse_in_range_body`), a text place is bound like any other text source
> (`Parser::iterator`), and the places a source read are recorded on the loop so a body that
> writes one warns `loop-source-written` — the variable-end and text-source shapes being the
> two the ruling named.  Pins that recorded the re-read moved to the ruled values:
> `iter-text-source-once` s15 (2 → 5), `158-char-walk` w6 (6006 → 2004) and w7 (3 → 6),
> `158-walk-accumulator` a8 (6 → 2 trips), and `string_scope`'s slot layout (the inner loop's
> end is a slot of its own, and `n`'s span now ends at the bind).  Guard:
> `tests/scripts/1619-a-loop-reads-its-bounds-and-its-text-once.loft`, thirteen cells on both
> backends.

> **D-iter-5 — OPENED AND CLOSED (2026-09-06, loft#1391). The two destinations the snapshot
> could not name.**  `(I-Comp)` is *whichever destination*, and D-iter-4's cure reached the
> ones `Parser::field_place` could name — a variable and a chain of `OpGetField`.  Two were
> left, both answering the EMPTIED result on both backends, silently: a field reached through
> an ELEMENT (`xs[0].items = [xs[0].items[1]?, xs[0].items[0]?]` -> `0,0`) and a collection a
> CLOSURE captured (`f = fn() { v = [v[1]?, v[0]?]; }` -> `0,0`).
>
> Closed in the place, not at the two sites.  `field_place` gained an ELEMENT step — the index
> must be one a single statement cannot change, a constant or a variable, and both element
> spellings normalise to one step because a nullable read of an element is that element — and a
> CAPTURE step, `OpGetDbRef(__closure, <offset>)`, which is what a closure reaches its capture
> through.  Beside it, two gates that had hidden the second destination: the place was computed
> only when the caller SAID the destination was a field, so a capture (neither variable nor
> field) had no read test at all, and a build-into-target literal ran its `OpClearVector` ahead
> of the whole block, so the snapshot copied an already-emptied destination — the clear now
> sits inside the block after the snapshot, where the field spelling has always put it.
>
> The controls are what keep the place from being over-wide, and each is a redirection that must
> NOT happen: a SIBLING field of the same element, ANOTHER element's field, and a nested plain
> field that was already right.  Guard
> `tests/scripts/a-build-reads-its-destination-through-an-element-or-a-capture.loft` (14 cells,
> including the loop that makes the snapshot survive a reused buffer store, the comprehension
> spelling at both destinations, and `+=`), falsified at 00ff5bb5.  A KEYED destination stays
> outside: its literal is built THROUGH it by construction (loft#703).

> **D-iter-2 — CLOSED (2026-08-30). A comprehension assigned to a struct FIELD it reads.**
> `s.v = [for i in 0..s.v.len() { s.v[i] * 2 }]` answered `[]` on both backends, silently.
> The whole-vector field replace emits `OpClearVector(s.v)` ahead of the comprehension's
> own ops, so the field was empty before the loop read it. Sweeps like its local sibling —
> body-only gives `[0,0,0]`, a foreign source gives `[1,3,4]` — with one control that
> shaped the cure: reading a SIBLING field (`s.v = [for … s.w …]`) is correct, so the test
> is on the FIELD EXPRESSION, not the struct's base variable.
> Fixed (loft#1195) by the fresh-buffer route below.
>
> **D-iter-3 — CLOSED (2026-08-30). A comprehension appended with `+=` to a vector it
> reads.** `a += [for i in 0..a.len() { a[i] * 2 }]` never terminated: it built into `a`'s
> own store while the bound re-read that store's growing length (`--native` overflowed in
> `store.rs` instead). Unbounded allocation, not merely a hang.
>
> Its boundary is NARROWER than the other two, and the difference is instructive: the BODY
> reading the destination is fine, because `+=` leaves the existing elements at their own
> indices, so only the loop's TERMINATION condition — the bound, or the source being the
> destination — was ever affected. The two body-only cells answer the same values under the
> fresh-buffer model as they did built in place, which is what made the cure additive.
> Fixed (loft#1196) by the same route.
>
> **The cure both took, and why it was already there.** `map` and `filter` build into a
> buffer of their own and let the destination's assignment deliver it, so
> `s.v = s.v.map(…)` and `a += a.map(…)` were correct on every cell throughout. The
> comprehension now takes that same route whenever it reads a destination it cannot serve
> by deferring a repoint — the reference route was the oracle, and the two spellings of one
> operation now agree.

> **D-iter-4 — CLOSED (2026-08-30). A comprehension assigned to a LOCAL it reads.**
> `a = [for i in 0..a.len() { a[i] * 2 }]` answered `[]`, and the shape needed neither a
> loop, a call, a struct nor a tuple. `#501`'s watermark reuse builds a comprehension
> straight into its destination, and the fresh store `create_vector` splices in for a `=`
> repoints the destination BEFORE the loop — so the source, the range bound, the `if`
> guard and the body all resolved through the empty result being built.
>
> **The filed scope was one of six cells.** Sweeping which PART does the reading says the
> source need not be the destination at all:
>
> | cell | answered |
> |---|---|
> | `a = [for i in 0..a.len() { a[i]*2 }]` — bound + body | `[]` |
> | `a = [for x in a { x*2 }]` — the source IS `a` | `[]` |
> | `a = [for i in 0..3 { a[i]*2 }]` — body only | `[0,0,0]`, length right |
> | `a = [for i in 0..a.len() { i*2 }]` — bound only | `[]` |
> | `a = [for x in b { x + a[0] }]` — a FOREIGN source, body reads `a` | `[1,3,4]` |
> | `a = [for i in 0..3 if a[i] > 7 { i }]` — the `if` guard | `[]` |
>
> The foreign-source cell is the sharpest: `b` drives the loop, the length is right, and
> every value is read out of the half-built result. The const-fill and const-unroll
> shortcuts (loft#884) build through the destination too and carried the same defect.
>
> Fixed (loft#1194) by holding back the ONE op that repoints the destination until after
> the loop, and snapshotting what the destination holds so the loop's reads resolve
> through that. The snapshot is what makes a SURROUNDING loop work: `OpDatabase` reuses
> the slot's store (`clear` + `claim`), so on a second execution of the same site the
> buffer store IS the one the destination was left pointing at — reordering alone left the
> reported "pop the last element" worklist idiom still empty. `.map` / `.filter` on the
> same vector were correct throughout (they already mint their own result), as was a
> `&`-alias read; both are controls in
> `tests/scripts/1194-a-comprehension-reads-its-destination.loft`.

> **D-iter-1 — CLOSED (2026-08-22). Every combinator was broken over a TUPLE element.**
> `xs.map(|t| { t.0 * 10 })` on a `vector<(integer, integer)>` answers
> `343597383710 1030792151070` on `--interpret` — a packed DbRef read as an integer, with
> no diagnostic — and does not compile on `--native`. `filter` SIGSEGVs, `reduce` mistypes
> its accumulator. So `I-Map`'s element values and `I-Reduce`'s fold both fail at one
> element type, in the `silent-wrong` direction.
>
> `for_type`'s P189b block deliberately gives a tuple loop var
> `Reference(__tuple<…>)`, because iteration yields a DbRef at the tuple's inline bytes —
> right for a `for` BODY. `parse_map` reuses that as the callback's ARGUMENT type while the
> lambda is generated taking the tuple BY VALUE, so a `DbRef` is passed where a tuple is
> declared. A struct element is unaffected because a struct IS a DbRef, so the two
> representations coincide; the tuple is the one element type where they do not.
>
> Fixed (loft#1074) by giving all four combinators ONE helper, `callback_element_arg`,
> which answers the element's VALUE and its TYPE together so they cannot drift apart —
> three copies of a tuple element list disagreeing is what loft#1006 was, in this same
> area. `filter` needed it twice, at the predicate's argument AND at the element it
> COLLECTS, which a length-only assertion would have missed. `reduce` needed a second,
> independent fix: its hint typed BOTH lambda parameters as the element, so an `integer`
> accumulator over a tuple vector was refused before it could run; the declared signature
> is `fn(U, T) -> U`, so the accumulator now takes the INIT's type.
>
> **What the corpus was holding fixed:** every combinator cell in § Conformance below runs
> on `vector<integer>`. Text iteration is a different SOURCE kind, not a vector of text, so
> the ELEMENT TYPE is never varied at all — the same axis that left
> [interfaces.md](interfaces.md) blind to scalar instantiation and [tuples.md](tuples.md)
> blind to `text`. Measured alongside: `vector<text>`, `vector<Struct>`, `vector<fn(…)>`
> and `vector<integer?>` elements are all CLEAN through `map`, so the tuple is the whole
> deviation and not the tip of a wider one.

- **Conformance is differential** — the iteration steps are enforced across the two backends by
  the @PLN89 differential oracle (D-op-1). Its corpus explicitly covers the combinators
  (`13-collections-map-filter`), comprehensions, and text iteration, precisely because the
  interpreter (a store-index walk) and native (an emitted Rust loop) implement them by the most
  different mechanisms. A divergence in order, length, or element value is caught there.
- **Order is a hard part of the contract, not incidental** — `map`/`filter` preserving order
  and `reduce` folding left are pinned here so a "faster" reordering in either backend is a
  definitional error, not an optimisation. The one place order is deliberately given up is
  `par(…)` ([concurrency.md](concurrency.md)).

## Carried by iteration.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [iteration.md](iteration.md) now states only what is open.

### the status line formal/README.md's area table carried until 2026-09-04

**rules written (2026-07-04), 0 own** — index-cursor `for`, deterministic combinator order, fresh result vector; conformance via the oracle

