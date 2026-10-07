<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/concurrency.md — small-step semantics for `par` (strict)

**Catalogue:** @F (threading / `par`), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the relation for loft's ONE
> parallel construct: the `par(b = worker(a), N)` clause on a `for … in` loop. It extends
> [iteration.md](iteration.md) (the loop it decorates) and [heap.md](heap.md) (results are
> deep-copied into a fresh store). It is the written contract for the "parallel reductions"
> part operational.md's D-op-1 named unwritten — and the ONE place loft **deliberately gives up
> execution order**, so its determinism is a *conditional* guarantee that must be stated exactly.
>
> There is no other concurrency surface: no shared-memory threads, no locks in user code, no
> `async`. `par` is a data-parallel map, nothing more.

## The model in one line

`for a in src par(b = worker(a), N) { body }` runs `worker(a)` for each element **in parallel**
across `N` threads, then runs `body` over the results **in source order** with `b` bound to each
`worker(a)`. The parallelism is confined to the worker; the body is still a sequential,
in-order loop.

## Notation

Uses [iteration.md](iteration.md)'s `for` and [heap.md](heap.md)'s heap `H`.

- `worker` is a **pure** function of its element `a` (plus forwarded context args): it reads only
  `a`/its args and returns a value; it does not read or write shared mutable state.
- `flatten(src)` materialises any for-source (vector / range / iterator / text / keyed
  collection) into a flat vector — `par` partitions a vector across the thread queue.

---

## Rules

### `par` is a parallel map consumed in source order

```
  (C-Par)   for a in src par(b = worker(a), N) { body }
              ≡  v   := flatten(src) ;                          (materialise to a vector)
                 res := parallel_map(worker, v, N) ;            (workers run in ANY order, N threads)
                 for i in 0 .. len(v) { b := res[i] ; body }    (results consumed IN ORDER, i=0,1,2,…)
              where res[i] = worker(v[i]) for every i, regardless of the order the workers ran.
```

**In words.** The clause splits into two phases: a **parallel** phase that computes
`worker(a)` for every element (the threads may run these in any order, interleaved), and a
**sequential** phase — the ordinary loop body — that consumes the results `res[0], res[1], …` in
**index order**. So `b` is `worker` applied to the element in the same position: the body sees
results in source order even though they were computed out of order. A struct/reference result is
**deep-copied** into the result vector ([heap.md](heap.md) `H-Copy`), so no worker's local store
escapes.

### Determinism is CONDITIONAL on a pure worker

```
  (C-Det)      if worker is PURE (depends only on its element + context args, mutates no shared
               state), then  for a in src par(b=worker(a), N) { body }  is OBSERVABLY EQUAL to
               the sequential  for a in src { b := worker(a) ; body }  — same b values, same
               order, same result — for every N.  N is a performance knob, not a semantic one.
  (C-Impure)   if worker is NOT pure, the result is UNDEFINED across runs/backends/N.  loft does
               not define an interleaving; a data race is a program error, not a language step.
```

**In words.** The whole point of `par`: because the worker is a pure function of its element, it
does not matter what order the threads ran in — position `i` always gets `worker(v[i])` — so a
`par` loop computes **exactly** what the sequential loop would, just faster. The thread count `N`
never changes the answer. This equivalence is the guarantee, and it holds **only** for a pure
worker; a worker that reads or writes shared mutable state has no defined behaviour (loft pins no
interleaving), so such a program is simply wrong — the contract is "make the worker pure," not
"loft will pick an order for you."

### Source order — with the keyed-collection exception

```
  (C-Order)   res is consumed in the order of flatten(src):
                - vector / range / iterator / text → the natural sequence order;
                - a HASH source → its UNSORTED bucket walk (NOT key order), because the parallel
                  queue has no use for key order.  So `for x in h par(…)` may visit elements in a
                  DIFFERENT order than the sequential `for x in h` (which is key-ordered).
```

**In words.** The body sees results in the order `flatten` produced them, which for a vector,
range, iterator, or text is the obvious sequence. The one wrinkle: a **hash** is flattened by an
unsorted bucket walk (the queue ignores key order), so a `par` loop over a hash can visit
elements in a different order than the sequential, key-ordered `for x in h`. This is a
**deliberate, documented** difference — the only case where `par` is not order-identical to
sequential — because a hash has no inherent order to preserve. `sorted`/`index` sources keep
their order.

---

## Deviations

OPEN: **0** (a *rules* doc — it shrinks operational.md's D-op-1, adds no code deviation).

- **Conformance is differential** — `par` is enforced across the two backends by the @PLN89
  differential oracle (D-op-1), which carries a parallel-reduction program
  (`14-parallel-reduce`): the interpreter runs a real thread pool, native emits its own parallel
  dispatch, and both must produce the SAME result vector (`C-Det`). A divergence — a result that
  depends on `N`, or on which thread finished first — is caught there and is, by `C-Impure`, a
  sign of an impure worker slipping the contract.
- **The hash-order difference (`C-Order`) is a spec'd edge, not a deviation** — it is stated in
  the rule; the par order follows the bucket layout, which the hash seed decides, so it is not
  promised to repeat across runs or to agree between backends.

## Conformance

- **N-independence (`C-Det`)** — `for a in xs par(b = square(a), N) { sum += b }` computes the
  same `sum` for `N = 1, 2, 8` and equals the sequential loop, on both backends.
- **In-order results (`C-Par`)** — with an order-sensitive body (e.g. building a string), the
  output matches the sequential loop's element order for a vector/range/text source.
- **Deep-copied struct results (`C-Par` + `H-Copy`)** — `par(b = make(a), N)` where `make`
  returns a struct: each `b` is an independent copy; field access on `b` in the body works and no
  worker store leaks.
- **Hash walk order (`C-Order`)** — `for x in h par(…)` may differ in order from `for x in h`,
  and from one run or backend to the next (the bucket walk follows the hash seed); a body that
  depends on it is outside `C-Det`.
- **The SEQUENTIAL loop is `C-Det`'s oracle, and the differential one cannot replace it** —
  the check above compares the two backends, so a rule violation they SHARE is invisible to
  it however many programs the corpus grows to. loft#1060 was exactly that: `par(b = f(a.n), N)`
  discarded the written argument, handed the worker the whole record and reinterpreted it as the
  parameter's type, identically on both backends — while `b = f(a.n)` in a plain `for` was
  refused at compile time (*"expected integer, got Sq"*). `C-Det` names the sequential form as
  the standard precisely so that "both backends agree" cannot be mistaken for "the rule holds";
  the accept/reject sides must agree with it too, not only the values.

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on a
`par` loop's result — for any thread count — is the definitional error this doc names (and, per
`C-Impure`, most such disagreements are an impure worker, which the contract forbids).
