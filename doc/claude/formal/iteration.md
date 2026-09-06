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
re-read each round, so it observes the length **as it is at that step** — a body that appends to
the very collection it iterates keeps seeing the new elements (loft does not snapshot the length;
this is a deliberate, both-backends-shared choice). The loop is a pure desugaring to
[operational.md](operational.md)'s `loop`/`break`/`if`, so its control flow is already pinned;
`I-For` only fixes the ORDER and the stop condition.

### Ranges and text iterate the same shape

```
  (I-Range)    for x in a..b { body }   iterates the integers a, a+1, …, b-1 (empty if a ≥ b);
                                        x is the value, not an index.
  (I-Text)     for c in t { body }      binds c : character to each Unicode CODEPOINT of t, left
                                        to right — one iteration PER CODEPOINT, NOT per grapheme
                                        cluster.  The cursor is a BYTE position advanced by the
                                        codepoint's UTF-8 width (1–4), so `c#index` is that
                                        codepoint's starting byte offset and `c#next = c#index +
                                        width`.
```

**In words.** A range `a..b` yields the half-open integer sequence (never includes `b`); an
empty range (`a ≥ b`) runs the body zero times. Text iterates by **Unicode codepoint** — each
`c` is a `character` (a scalar value), and a combining sequence is **multiple** iterations, NOT
one: `for c in "e" + U+0301 + "X"` runs **three** times with `c#index = 0, 1, 3` (the combining
accent is its own codepoint at byte 1, 2 bytes wide), because loft iterates **codepoints, not
grapheme clusters**. This is the load-bearing text choice — it is what both backends must agree
on (a grapheme-cluster walk in either one would be a divergence). The cursor is a byte position
that jumps by each codepoint's UTF-8 width (a 4-byte emoji advances by 4), so `c#index`/`c#next`
expose byte offsets and the offset sequence is not `0,1,2,…` for non-ASCII text. Same cursor
shape as a vector, differing only in `elem` (decode one codepoint) and the stride (its width).

> **Combinators are vector methods, not text methods.** `t.map(…)` / `t.filter(…)` are **not**
> valid — `.map`/`.filter`/`.reduce` (`I-Map`/`I-Filter`/`I-Reduce`) dispatch on a vector (or a
> keyed collection), and text is `Unknown field text.map`. Text participates in the combinator
> world only as a **comprehension source**: `[for c in t { f(c) }]` builds a `vector<…>` of
> per-codepoint results (this IS how you "map over text"). So text is a first-class `for` /
> comprehension source but never a `.method` combinator receiver.

### Combinators desugar to a comprehension over the same loop

```
  (I-Map)      src.map(f)         ≡  [ for x in src { f(x) } ]
  (I-Filter)   src.filter(p)      ≡  [ for x in src { if p(x) { x } } ]     (keeps x where p(x))
  (I-Reduce)   src.reduce(a, g)   ≡  { acc := a ; for x in src { acc := g(acc, x) } ; acc }
  (I-Comp)     [ for x in src { e } ]
                 ≡  out := alloc(vector) ;                    (a FRESH store, heap.md H-Alloc)
                    for x in src { append(out, e) } ;         (per element, heap.md H-NewRec)
                    out
```

**In words.** The combinators are not primitive — each is the same left-to-right `for` loop
building a **fresh** result vector (`I-Comp`): `map` appends `f(x)` for every element; `filter`
appends `x` only where the predicate holds; `reduce` folds a running accumulator and yields it
(not a vector). The result is a new store ([heap.md](heap.md) `H-Alloc`), so the source is
untouched — `xs.map(f)` never mutates `xs`. Because they all lower to `I-For`, they inherit its
**deterministic order**: `map` preserves order, `filter` preserves relative order, `reduce`
folds left. The lambda `f`/`p`/`g` is an ordinary closure ([capabilities.md](capabilities.md)
gates its body when sandboxed). A combinator on a LITERAL receiver (`[1,2,3].map(f)`) is the
same rule — the literal is a fresh source value (`#501` fixed the parser so the literal is a
self-contained receiver, not a reuse of the assignment target).

### Empty and null sources

```
  (I-Empty)    for x in src { body }   runs body ZERO times when len(src) = 0.
  (I-NullSrc)  for x in nullref { body }   runs body ZERO times (a null source is empty,
                                           consistent with heap.md H-ReadNull — no halt).
```

**In words.** An empty vector, an empty range, or a **null** source all iterate zero times and
fall through — never a fault. A null source is treated as empty (the same null-continue
discipline as a read through `nullref`).

**`nullref` is a RUNTIME null of a NON-nullable type, and the distinction is the whole content
of this rule.** A collection field never filled, or a call whose declared `vector<τ>` return
answers null, is a `nullref`: it iterates zero times with no guard and no fault (measured, both
sources). A source whose TYPE is `τ?` is a different question and is REFUSED — `for x in v` with
`v: vector<integer>?` does not compile, because [types.md](types.md) `(N-Coal)`/`(N-Default)`
admit no implicit unwrap and a `for` is not an exception to that. The discharge is one character
and gives exactly this rule's answer: `for x in v?` and `for x in v ?? []` each run zero times.

Until 2026-09-07 this paragraph ended *"so a `for` over a possibly-null collection is safe
without a guard"*, which reads as a promise about the `?` spelling — the one spelling the rule
does not cover and the compiler refuses. The formal line was right and its gloss reached one
case past it (QUALITY.md B8i).

---

## Deviations

**OPEN: 0.**  Every deviation this doc has carried is closed; the record is in
the companion [iteration-history.md](iteration-history.md).

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
