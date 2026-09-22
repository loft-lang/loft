<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/coroutines.md — small-step semantics for generators (strict)

**Catalogue:** @F34 (coroutines / generators, 0.8.3), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the small-step relation for
> loft's **generators**: a function returning `iterator<T>` whose body may `yield`. It extends
> [operational.md](operational.md) (control flow), [heap.md](heap.md) (the suspended frame is a
> heap store), and [iteration.md](iteration.md) (a `for` over a generator advances it). It is the
> written contract for the "coroutines" part operational.md's D-op-1 named unwritten — and the
> single area where the two backends differ **most**: the interpreter suspends by serialising a
> frame, native compiles a resumable **state machine**.
>
> Scope: single-value `yield` (CO1.1–CO1.6, shipped 0.8.3). `yield from` (delegation) is deferred
> to 1.1+ ([COROUTINE.md](../COROUTINE.md) CO1.4) and is not specified here.

## The model in one line

A generator is a function whose **entire call stack at the point of `yield` is preserved** on the
heap (STACKFUL), so `yield` may sit inside a helper called from the generator. Calling it does
**not** run the body — it allocates a suspended frame and returns it as an `iterator<T>` value;
the body runs, in slices, on each advance.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩` and [heap.md](heap.md)'s heap `H`.

- **`fr`** — a suspended **coroutine frame**: a heap value ([heap.md](heap.md) `H-Alloc`) holding
  the generator's saved call stack (all locals of the generator AND of any active nested calls)
  plus a **resume point** `pc` (where to continue). An `iterator<T>` value IS a reference to `fr`.
- **`state(fr)`** ∈ `{ suspended@pc, running, done }`.
- `advance(fr)` runs the frame from its resume point until the next `yield` or the end.

---

## Rules

### Calling a generator suspends immediately

```
  (G-Call)   ⟨g(args), ⟨ρ, H⟩⟩ → ⟨fr, ⟨ρ, H'⟩⟩
               where g's return type is iterator<T>,  fr = fresh frame with g's args bound,
                     state(fr) = suspended@entry,  H' allocates fr.  The BODY DOES NOT RUN YET.
```

**In words.** Calling a generator function is **not** a normal call — it does not execute the
body. It allocates a suspended frame ([heap.md](heap.md)), binds the arguments, sets the resume
point to the entry, and returns the frame as an `iterator<T>` value. Nothing the body would do
(a side effect, a `yield`) has happened yet; the first slice runs on the first advance.

### `next` / a `for` advance runs one slice, up to the next `yield`

```
  (G-Next)   ⟨next(fr), ⟨ρ, H⟩⟩ → ⟨v, ⟨ρ, H'⟩⟩
               when state(fr) = suspended@pc and advance(fr) reaches `yield v`:
                 v is produced, the FULL stack (generator + nested calls) is saved into fr,
                 state(fr) := suspended@(after that yield),  H' = the updated frame.
  (G-Done)   ⟨next(fr), ⟨ρ, H⟩⟩ → ⟨done, ⟨ρ, H'⟩⟩
               when advance(fr) reaches the generator's end without a further yield:
                 state(fr) := done.  Every later next(fr) is `done` (idempotent exhaustion).
  (G-Return) a generator has NO `return`.  Values leave a generator only through `yield`, so
             `return e` in a function whose return type is `iterator<T>` is a STATIC error —
             the value has nothing it could mean — and so is a bare `return;`, because
             ending early is what `break` does and the body then reaches its end (G-Done).
             The rule is asked at BOTH spellings of the same act: the `return` keyword, and
             a body whose TAIL is a value.
```

**In words.** Advancing a generator resumes its saved stack and runs until the **next** `yield`,
which produces one value and re-suspends — so a generator computes lazily, one value per advance,
and its side effects happen interleaved with the consumer's. When the body runs off the end
without yielding again, the iterator is **done**; advancing a done iterator stays done (it never
restarts and never faults).

### `yield` produces one value and suspends the whole stack

```
  (G-Yield)  inside advance(fr), ⟨yield v, ⟨ρ, H⟩⟩ suspends:
               v becomes next's result, ρ (the generator's locals AND every active nested
               call's frame) is serialised into fr, and control returns to the consumer.
               Execution resumes at the statement AFTER this yield on the next G-Next.
  (G-YieldDepth)  `yield` is valid at ANY call depth within the generator (stackful): a
                  `yield` inside a helper `h()` called from g suspends g's WHOLE stack, not
                  just h's frame.
```

**In words.** `yield v` hands `v` to whoever advanced the iterator and freezes the generator
exactly where it is — including any helper functions it was in the middle of calling (the
stackful property). On the next advance it thaws and continues from the statement right after the
`yield`, with every local restored. `yield` is rejected by the compiler outside a generator
function (a `yield` where the return type is not `iterator<T>` is a static error, not a runtime
one).

### A `for` over a generator is I-For over `next`

```
  (G-For)    for x in g(args) { body }
               ≡  fr := g(args) ;                    (G-Call — suspended)
                  loop { r := next(fr) ;             (G-Next — run one slice)
                         if r = done { break } ;     (G-Done)
                         x := r ; body }
```

**In words.** Consuming a generator with `for` is exactly [iteration.md](iteration.md)'s loop,
with `next(fr)` as the cursor's `next` and `done` as the stop signal. So a generator is
interchangeable with a vector at the `for` site — the difference is only that its elements are
computed lazily, on demand, rather than read from a store.

### What an advance produces is the consumer's

```
  (G-Own)    the value an advance produces is the CONSUMER's: `next(fr)` is a call and a call's
             result is FRESH (heap.md H-Move), so it moves into whatever binds it — a variable,
             a `for` variable (released when its iteration ends), the buffer a `match` over the
             generator collects into — and the generator keeps no hold on it.
               - `yield e` of a FRESH e — a literal, a constructor, a call result that views
                 none of the generator's variables — HANDS e over;
               - `yield e` of an EXISTING value — a local, a parameter, a member, a view —
                 places a COPY (H-Move lists no yield among the positions that move), so a
                 later change the generator makes does not reach the value the consumer holds;
                 for a type that owns a droppable that copy is refused (H-Copy-Refuse).
             Scope: a record, a struct-enum, a vector, and a tuple whose reference members
             are those (each member is handed over or copied on its own).
```

**In words.** A generator hands out values, not windows into its own state. What `next` answers
belongs to whoever received it: a record returned past the generator's life still reads, a
drained `for` releases each record once its iteration is done, and a value the consumer holds
does not change when the generator later rewrites the local it came from. A record built at the
yield is simply handed over; anything the generator keeps using is copied first. A type with a
drop hook cannot be copied, so yielding an existing one is a compile-time error that asks for a
value built at the yield instead.

---

## Deviations

**OPEN: 0.**

Every deviation this doc has carried is closed; the record is in the companion
[coroutines-history.md](coroutines-history.md).

## Conformance

- **Lazy, one-per-advance (`G-Call` / `G-Next`)** — a generator's side effects interleave with the
  consumer's, one value per advance. STRAIGHT-LINE yields obey this on both backends
  (`print("a"); yield 1; print("b"); yield 2` → `a g1 b g2`). A LOOP with one yield on its body's
  straight line — `for` or `while`, statements after the yield included — does too (`y0 g0 y1 g1`),
  so an endless one hands out each value as it is asked for.  The loop shapes CL-9 has not reached
  (more than one yield per iteration, a yield under an `if`/`match`, a nested loop, a `continue`,
  a tuple or record yield) still run EAGERLY on native (`y0 y1 g0 g1`):
  the values agree and the side effects do not, an interleaving difference COROUTINE.md § CL-9
  records rather than a divergence of values — and an ENDLESS loop of one of those shapes never
  hands out a value on native, so write it with the yield on the straight line.
- **Stackful (`G-YieldDepth`)** — a `yield` inside a helper called from the generator produces
  the value and resumes correctly past the helper — the same sequence on both backends.
- **Exhaustion (`G-Done`)** — a finite generator produces its sequence then reports done; further
  advances stay done (no restart, no fault).
- **Ownership (`G-Own`)** — `tests/scripts/1589-a-yielded-record-is-the-consumers.loft`: a
  record returned past its generator, a drained and a broken `for`, a `match`, `yield from`, a
  loop-body yield, copies of a local, a member, a parameter, a text-holding record and a
  vector, and a tuple copied, drained and kept — values, drop traces and a clean store census
  on both backends.
- **Interchangeable at `for` (`G-For`)** — `for x in gen() { … }` and `for x in vec { … }`
  visit their elements by the same loop; swapping a generator for the equivalent vector changes
  only timing, not the values or their order.

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on a
generator's produced sequence, laziness, or exhaustion is the definitional error this doc names.
