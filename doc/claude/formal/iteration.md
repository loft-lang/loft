<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/iteration.md — small-step semantics for iteration (strict)

**Catalogue:** @F3 (scalar/collection core), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the small-step relation for
> loft's **iteration**: the `for … in …` loop, the iterator protocol it desugars to, ranges
> and text iteration, and the collection **combinators** (`map` / `filter` / `reduce` and the
> `[for … { … }]` comprehension). It extends [operational.md](operational.md)'s scalar core
> and reads/writes the heap via [heap.md](heap.md). It is another written contract for a part
> operational.md's D-op-1 named unwritten — the piece where the two backends differ most
> (interp walks a store index; native emits a Rust loop).
>
> Scope: **sequential** iteration. The parallel form `par(…)` reorders and is its own contract
> — see [concurrency.md](concurrency.md).

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩` and [heap.md](heap.md)'s heap `H`.

- An **iterator** is a pair `it = ⟨i, src⟩`: a cursor `i` (an integer index, or a text byte
  position) into a source `src` (a vector/collection reference, a range, or a text value).
- `len(src)` is the element count; `elem(src, i)` reads the `i`-th element via `H-Index`
  ([heap.md](heap.md)); both are `null` past the end (they never fault).
- `x` is the loop variable, bound fresh each round; `body` is the loop body.
- **Also backs pattern matching** ([matching.md § PEG patterns](matching.md), @PLN35 SPEC-FIRST):
  the same `⟨i, src⟩` cursor walks a slice pattern — *anchor* = save `i`, *revert* = restore `i` —
  so backtracking over a vector needs no new primitive; a read past the end is `null` (`I-Done`),
  never a fault. (An *iterator* pattern cursor cannot re-index, so it adds a memo buffer + two ops —
  matching.md `P-Anchor`/`P-Revert`.)

---

## Rules

### The `for` loop desugars to an index cursor

```
  (I-For)      for x in src { body }
                 ≡  it := ⟨0, src⟩ ;
                    loop { if i ≥ len(src) { break } ;
                           x := elem(src, i) ; i := i + 1 ;
                           body }
  (I-Next)     ⟨next(it), σ⟩ → ⟨elem(src, i), σ⟩   then i ← i+1        when i < len(src)
  (I-Done)     ⟨next(it), σ⟩ → ⟨done, σ⟩                                when i ≥ len(src)
```

**In words.** `for x in src { … }` runs the body once per element, **in index order 0, 1, 2, …**,
binding `x` to each element, and stops exactly when the cursor reaches the length. The cursor is
re-read each round, so it observes the length **as it is at that step** (loft does not snapshot
the length of a collection it walks; a body cannot append to that collection — the parser refuses
it — but it can remove the current element with `#remove`).  The SOURCE is not re-read: see
*The source is evaluated ONCE* below. The loop is a pure desugaring to
[operational.md](operational.md)'s `loop`/`break`/`if`, so its control flow is already pinned;
`I-For` only fixes the ORDER and the stop condition.

### Ranges and text iterate the same shape

```
  (I-Range)    for x in a..b { body }   iterates the integers a, a+1, …, b-1 (empty if a ≥ b);
                                        x is the value, not an index.
  (I-RangeIncl) for x in a..=b { body } iterates a, a+1, …, b — b INCLUDED (empty if a > b).
                                        The stop is decided on the value just YIELDED, so the
                                        sequence is exact when b is the type's MAXIMUM and the
                                        loop never has to represent b + 1.
  (I-Text)     for c in t { body }      binds c : character to each Unicode CODEPOINT of t, left
                                        to right — one iteration PER CODEPOINT, NOT per grapheme
                                        cluster.  The cursor is a BYTE position advanced by the
                                        codepoint's UTF-8 width (1–4), so `c#index` is that
                                        codepoint's starting byte offset and `c#next = c#index +
                                        width`.
```

**In words.** A range `a..b` yields the half-open integer sequence (never includes `b`); an
empty range (`a ≥ b`) runs the body zero times. `a..=b` is the same sequence with `b` included,
and empty when `a > b`.

> **Why the inclusive form states WHERE it stops, and not only what it yields.** Until
> 2026-09-13 it had no rule at all, and its implementation ended the loop with the same
> overshoot compare the half-open form uses — a test on the value about to be yielded. That
> cannot end a range whose end is the type's maximum: the step past `b` overflows to the null
> sentinel (`i64::MIN` for `integer`), and `till < null` is false under the plain order. So
> `for i in (m - 2)..=m` with `m = i64::MAX` did not terminate on either backend — it RESTARTED
> (the null-init counter reads the sentinel as "not started yet" and re-seeds at the start), and
> after @PLN157 § V-ab removed that null test it ran on forever with `i = null` instead
> (loft#1525). Deciding the stop on the value just YIELDED removes the need for `b + 1` to exist,
> which is why the rule says it. The half-open form is unaffected — its test fires before the
> step — and so is any inclusive range whose end is below the maximum. Pinned by
> `tests/scripts/1525-an-inclusive-range-stops-on-the-value-it-yielded.loft`. Text iterates by **Unicode codepoint** — each
`c` is a `character` (a scalar value), and a combining sequence is **multiple** iterations, NOT
one: `for c in "e" + U+0301 + "X"` runs **three** times with `c#index = 0, 1, 3` (the combining
accent is its own codepoint at byte 1, 2 bytes wide), because loft iterates **codepoints, not
grapheme clusters**. This is the load-bearing text choice — it is what both backends must agree
on (a grapheme-cluster walk in either one would be a divergence). The cursor is a byte position
that jumps by each codepoint's UTF-8 width (a 4-byte emoji advances by 4), so `c#index`/`c#next`
expose byte offsets and the offset sequence is not `0,1,2,…` for non-ASCII text. Same cursor
shape as a vector, differing only in `elem` (decode one codepoint) and the stride (its width).

**The source is evaluated ONCE.** `it := ⟨0, src⟩` — the `for` evaluates `src` before the
first round and never again; only the cursor is re-read.  That holds for every spelling of the
source: a text source (a call, a literal, an expression, and a PLACE — a variable or a field
path) is bound to a hidden local in the walk's prelude (`Parser::iterator`, cited
`@FR-I-Text`), and a range's bounds that are not literals are bound the same way
(`parse_in_range_body`, cited `@FR-I-Range`), so `a` and `b` in `(I-Range)` are the values they
had when the loop started.  A body that writes a place the source read therefore changes the
place and not the loop: `m = 3; for i in 0..m { m = 10 }` runs three rounds, `for c in s { s =
"zz" }` walks every character `s` held, and `for i in 0..=f()` calls `f` once.  Such a write is
reported (`loop-source-written`, a warning: a loop written to follow a moving end computes
something else), because the program that wrote it expected otherwise — a loop whose end moves
is a `while`.  The warning reads the write's TARGET against the places the source read: the
place itself, or a place holding it (`st` holds `st.n`); a write into the place (`w[0] = …`
under `len(w)`), a sibling field, and a write through a callee are not reported, and the loop's
answer is the rule's in all of them.  The cursor over a COLLECTION is the one re-read left, and
a body cannot grow the collection it walks (`Cannot add elements to 'v' while it is being
iterated`).  Pinned by `tests/scripts/iter-text-source-once.loft` and
`tests/scripts/1619-a-loop-reads-its-bounds-and-its-text-once.loft`.

## Deviations

**OPEN: 0.**  Every deviation is closed; the record is in the companion
[iteration-history.md](iteration-history.md) — the latest, D-iter-6 (loft#1619, a place source
and a range bound re-read per round), closed 2026-09-23 by owner ruling.

## Conformance

- **Order + length (`I-For` / `I-Map`)** — `[1,2,3,4,5,6].map(|x| x*2)` is `[2,4,6,8,10,12]` in
  that order, length 6, on both backends; `filter(|x| x%2==0)` is `[2,4,6]`, relative order kept.
- **Left fold (`I-Reduce`)** — `[1,2,3,4].reduce(0, |a,x| a+x)` is `10`; a non-commutative `g`
  (e.g. subtraction) exposes the fold direction and must match.
- **Text codepoints + byte cursor (`I-Text`)** — `for c in "1😊8"` visits 3 codepoints whose
  `c#index` values are `0, 1, 5` (the emoji is 4 bytes), not `0,1,2`. And the codepoint-vs-grapheme
  case: `for c in "e" + U+0301 + "X"` visits **3** codepoints (`c#index = 0, 1, 3`), not 2
  graphemes — proven identical on both backends. `t.map(…)` is a static error (`Unknown field
  text.map`); `[for c in t { … }]` is how you map over text.
- **Empty/null (`I-Empty` / `I-NullSrc`)** — `for x in [] { … }` and a `for` over a null
  collection both run the body zero times and continue.
- **Fresh result (`I-Comp`)** — `ys = xs.map(f)` leaves `xs` unchanged (a new store, `H-Alloc`).
- **The destination is a legal source (`I-Comp`)** — `a = [for i in 0..a.len() { a[i]*2 }]`
  reads what `a` held when the statement began, never the result being built, whichever part
  does the reading (source, range bound, `if` guard, body) and however many times the
  statement is executed. The cell to run is a comprehension whose source is a FOREIGN vector
  and whose BODY reads the destination: it keeps the right length while every value is wrong,
  so a length- or emptiness-only check passes on it. **Run it for all three destination
  kinds** — a local, a struct field, and `+=` — because one mechanism serves them and they
  broke together; and run each inside a surrounding LOOP, since a buffer reused across
  executions of the same site fails only on the second one. A LITERAL is the same build without the loop and is held to the same
  sentence: `v = [v[1], v[0]]` reverses, `v += [len(v), len(v)]` appends the length twice, on a
  local, a parameter and a struct field alike (D-iter-4).
- **A comprehension and its combinator agree** — `xs = xs.map(f)` and
  `xs = [for x in xs { f(x) }]` answer the same thing, on the same destination kinds. The
  combinators were correct while the comprehension was not, for every cell above, so this
  pairing is the cheapest oracle the doc has for this rule.

Any program where the interpreter and `--native` disagree on an iteration's order, length,
element values, or the source's immutability is the definitional error this doc names.
