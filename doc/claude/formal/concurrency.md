<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/concurrency.md — small-step semantics for `par` and `parallel { }` (strict)

**Catalogue:** @F (threading / `par`), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the relation for loft's two
> parallel constructs: the `par(b = worker(a), N)` clause on a `for … in` loop, and the
> `parallel { arm; arm }` block (`C-Arms`). It extends
> [iteration.md](iteration.md) (the loop it decorates) and [heap.md](heap.md) (results are
> deep-copied into a fresh store). It is the written contract for the "parallel reductions"
> part operational.md's D-op-1 named unwritten — and the ONE place loft **deliberately gives up
> execution order**, so its determinism is a *conditional* guarantee that must be stated exactly.
>
> There is no other concurrency surface: no shared-memory threads, no locks in user code, no
> `async`. `par` is a data-parallel map, and a `parallel` block runs whole statements side by
> side, each on a read-only view of its frame — nothing more.

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

### The arms of a `parallel` block run at once

```
  (C-Arms)  parallel { e1; e2; … ; en }
              ≡  snapshot σ := the enclosing frame and heap at the block's entry ;
                 run e1 … en AT ONCE, each on its own thread, each against its own copy of σ ;
                 continue when every ei has ended.        (each ei's value is discarded)
            where an ei reads σ and writes none of it (assigning an enclosing local, mutating
            one through a reference and naming a parameter are compile errors), and whatever
            an ei makes — locals, texts, stores — is its own and is gone when it ends.
```

**In words.** Each top-level expression of the block is an arm, and every arm is running
before any arm has to finish: the arms are concurrent, not merely unordered. That is what lets
arms wait on each other through the outside world — one arm accepting on a socket that another
arm connects to, one waiting for a file another writes — where an arm run after the one it
waits for would never see it start. An arm reads the enclosing locals at their values on entry
and changes none of them, so nothing an arm does is visible to another arm or to the code after
the block except through the outside world (a file, a socket, the output). Arms that use the
outside world share it as separate threads do: two arms appending to one file interleave in an
order the language does not define (`C-Impure`). A build with no threads (WASI, a
microcontroller, a C build without POSIX) runs the arms in order, so there an arm that waits for a later one waits in
vain.

### Each thread's own state — the runtime's half of `C-Par` and `C-Arms`

The rules above are about values; these are about what a running program keeps while its
threads run, and they hold on every backend that runs threads (@PLN226).  Nothing is a
thread-local: a thread's state is a value it is handed, and every crossing between threads is
one of the points named here.

```
  (C-Own)       a thread's own state is its CONTEXT — the C runtime's `loft_cx`, the
                interpreter's `Stores` — and a context is used by ONE thread at a time.  The thread
                that starts a worker makes the worker's context before the thread starts; the
                worker alone uses it until it ends, and ends it itself; the starter reads what the
                worker left only after joining it.  Thread start and join are the only hand-overs.
  (C-Shared)    a worker's store table is ITS OWN copy of its starter's, in which every store the
                starter had is shared and read-only.  A context writes only the stores its own
                table made; a shared store is never grown while workers run (a call that would
                build into one works in a copy).  That the worker writes none is `C-Par`'s pure
                worker, which the compiler checks and the runtime relies on.
  (C-Adopt)     a worker's results cross back on its STARTER, after the join, one worker at a
                time: rows taken into the starter's table with their stores (`H-Copy` for the
                rest); everything else the worker made ends with its context.
  (C-Nest)      a worker that starts workers is their starter: `C-Own`, `C-Shared` and `C-Adopt`
                apply to it unchanged.  Two `par` loops that run at once — siblings in one `par`,
                loops in two `parallel` arms, a loop nested in a worker — share no context and no
                table; what they share is their ancestors' stores, read-only by `C-Shared`.
  (C-Main)      state only the main thread keeps (a work buffer or record store parked between
                calls) is plain static data that a worker never touches: every take and every
                put asks the context first, and a worker's context, or a call bridged in from a
                host, takes and parks nothing.
  (C-Boundary)  state that crosses a library boundary belongs to the CALLER's context: a hosted
                unit keeps a context per interpreter `Stores` that calls it; a component keeps one
                of its own per caller context, made by it and handed to its every symbol first; a
                library native's callback finds the caller's context in the store handle it was
                given.  The context is never found by asking which thread is running.
  (C-Publish)   data more than one thread reads and any may write — a lazily resolved symbol, a
                lazily opened log, a program-wide flag — is, in this order of preference: owned
                by a context; set before the first worker starts; built per thread and merged
                after the join; one atomic word.  A lock is the last resort, and every lock is
                listed with its reason and its lock-free alternative (LOCKS.md).
```

**In words.** A thread owns exactly what it was handed when it started, and nobody else touches
that while it runs.  It reads its starters' stores and writes only stores it made itself;
its results come back on the thread that started it, after that thread waited for it.  Because
those three rules are about a starter and its workers, they repeat unchanged at every level of
nesting, so `par` loops running at the same time in different workers or arms cannot reach each
other's state at all.  The few things that are deliberately not per thread — the main thread's
parked buffers, a library's per-caller state, a lazily set program-wide value — each say how a
second thread is kept out, and a lock is the last way: [LOCKS.md](../LOCKS.md) lists each one
that remains, why it is needed, and what would replace it.

**What each costs.**  Measured against the same program with the state in thread-locals
on arm64 macOS, the median of nine alternated runs; the numbers are in @PLN226 § Cost.

| Rule | Paid where | Measured |
|---|---|---|
| `C-Own` | every call passes `cx` | a context argument costs what a global does (39 M calls: 0.183 s against 0.182 s) |
| `C-Own` | a worker's context made and ended | 3 000 `par` loops of 8 rows on 4 threads: 99.6 ms against 105.8 ms — a context is 4.4 KiB, its transient arena allocated by the first transient value only |
| `C-Boundary` | a hosted bridge call asks its `Stores` for the unit's context | 2 M stdlib calls from the interpreter: 130.3 ms against 130.3 ms |
| `C-Boundary` | a component call looks up its context | 40 M `net` calls: 22.4 ms against 22.9 ms |
| `C-Main` | a park take or put tests `no_park` | inside the two rows above: no difference measured |
| — | the compile passes' memos behind a lock (`Data::PassCell`) | `--emit-c` of the corpus's largest script: 544.1 ms against 544.5 ms |

---

## Deviations

OPEN: **0**.  The closed entries are in [concurrency-history.md](concurrency-history.md); conformance is shared with operational.md's D-op-1.

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
- **Arms at once (`C-Arms`)** — an arm that waits (bounded) for a file a LATER arm writes sees
  it; a chain of three arms each waiting for the next; arms opening, writing and reading files
  at the same time; an arm building a `vector<text>`, running a `par` loop or a nested `parallel`
  block — each result read back through a file, on both backends.
- **Loops at once (`C-Nest`)** — two `par` loops running in two `parallel` arms and a `par` loop
  inside a `par` worker answer the sequential values on both backends, and the C build of the
  program reports no data race under ThreadSanitizer, twenty runs in a row.
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
