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
> Scope: single-value `yield` (CO1.1–CO1.6, shipped 0.8.3) **and `yield from`** (delegation),
> which ships and is specified below as `(G-Delegate)`.
>
> ⚠ That line read *"`yield from` is deferred to 1.1+ ([COROUTINE.md](../COROUTINE.md) CO1.4) and
> is not specified here"* until 2026-09-25, long after the construct shipped and was guarded
> (`tests/scripts/1277-…`, loft#1277).  A shipped construct declared out of scope has no rule, so
> nothing measured it and this chapter's `OPEN: 0` read green over three defects in it at once —
> one of them silent.  A scope line is a claim to re-read whenever the chapter is opened.

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

```
  (G-NoRefParam)  a generator declares no `&` parameter: refused at its definition, on both
                  backends.  Its body runs after the call has returned (G-Call), so a `&` —
                  which writes through to the caller's place (calls.md F-ParamRef) — could name
                  a place whose frame is gone.
```

**In words.** A generator cannot take a `&` parameter, because it keeps running after the call
that lent the reference has returned. Nothing is lost by the refusal: a struct or vector
argument is already shared with the caller (calls.md F-Param*), so a scanner that must leave its
position behind takes a cursor record and advances its field (`fn tokens(c: Cursor)`, writing
`c.pos`) — and that record lives in a store, where it cannot dangle.  Decided 2026-09-25
(loft#1680) with no program in the corpus, the libraries or the consumers using the pattern; a
real case can lift the refusal later, which breaks nothing.

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
  (G-YieldDepth)  the FRAME is stackful: `fr` holds the generator's whole saved call stack,
                  so a nested call active across a suspension is preserved with it and
                  delegation needs no trampoline.  What this does NOT give is a `yield` in a
                  function that is not itself a generator: `(G-Yield)`'s static clause refuses
                  it, because a `yield` in a function whose return type is not iterator<T> has
                  no type to produce into.  The surface the stackful frame buys is
                  `(G-Delegate)` below, where the deeper frame is a generator of its own.

  (G-Delegate)   ⟨yield from g₂, ⟨ρ, H⟩⟩ where g₂ : iterator<T> —
                   the sub-generator is created ONCE, on the first advance that reaches this
                   point, and each later advance takes its next value (G-Next) and forwards
                   it unchanged.  When g₂ is done (G-Done) it is released and the outer body
                   continues after the `yield from`.
                   A delegation SUSPENDS, so it is a resume point like any other yield: the
                   statements that led to it belong to the slice BEFORE it and run once per
                   activation, never again on an advance the delegation serves.
```

**In words.** `yield v` hands `v` to whoever advanced the iterator and freezes the generator
exactly where it is — including any helper functions it was in the middle of calling (the
stackful property). On the next advance it thaws and continues from the statement right after the
`yield`, with every local restored. `yield` is rejected by the compiler outside a generator
function (a `yield` where the return type is not `iterator<T>` is a static error, not a runtime
one) — and that includes a plain helper called from a generator, so "stackful" is a property of
the saved FRAME and not a licence to write `yield` anywhere.  `yield from g₂` is how a generator
hands a stretch of its sequence to another one: `g₂` is built on the first advance that reaches
the delegation, its values pass through unchanged, and when it is done the outer body carries on.
Because the delegation suspends, everything the generator did to reach it has already happened —
an advance that resumes a delegation resumes INSIDE it, and does not re-run the statements
that led there.

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
               - `yield` of a LAMBDA hands over its closure record, and every HEAP value the
                 lambda captures is COPIED into a store the record owns and releases
                 (@FR-L-CapOwn) — at the yield, whatever the capture's source.  The lambda's
                 writes therefore reach its copy, not the generator's local, and two yielded
                 lambdas over one local hold two copies.  A capture whose type owns a droppable
                 is refused (H-Copy-Refuse); a `&` capture keeps aliasing (B-Ref-Alias).  An
                 implementation may share the store where no difference is observable.
```

**In words.** A generator hands out values, not windows into its own state. What `next` answers
belongs to whoever received it: a record returned past the generator's life still reads, a
drained `for` releases each record once its iteration is done, and a value the consumer holds
does not change when the generator later rewrites the local it came from. A record built at the
yield is simply handed over; anything the generator keeps using is copied first. A type with a
drop hook cannot be copied, so yielding an existing one is a compile-time error that asks for a
value built at the yield instead.

### A generator held by a record or a collection is its holder's

```
  (G-Hold)   an iterator<T> value is a HANDLE: it names its frame (G-Call) and OWNS it — the
             frame, and every heap local the body allocated, are data the ownership model
             frees, not a resource with a hook.  A handle is a value like any other: a field,
             an element and a tuple member can hold one, and whatever holds it releases the
             frame, ONCE —
               - a local at its scope end, and at its REBIND the frame it held;
               - a field or an element given a new handle, the frame the old one named;
               - a record or a collection at its death, and an element at its REMOVAL
                 (`v.remove(i)`, `x#remove`, `h[k] = null`), every frame it holds, through
                 its inline records, its vectors and its keyed collections.
             No `OpDrop` hook runs for a frame, so (H-Drop-Not) does not bound these.
             A local given to a field, an element or a literal MOVES there (H-Move).  A handle
             READ out of a member — `h = t.g`, `h = tasks[i]`, a `for` over a collection of
             handles, a tuple binder — is a VIEW (binding.md B-View): advancing it advances
             the member's generator, and the member keeps it.
```

**In words.** A generator can be kept anywhere a value can: in a struct, in a vector, in a
tuple — which is what a scheduler needs, a set of live behaviours advanced once per tick.  The
holder owns the generator, and the generator is released exactly once, when its holder stops
holding it: the holder dies, gives the slot another generator, or the element is removed.
Reading a generator back out does not take it: it is the same generator, and advancing it
advances the one the container holds.

---

## Deviations

**OPEN: 0.**  D-gen-lambda opened and closed 2026-09-25 (loft#1676, below).

- **D-gen-lambda** *(CLOSED 2026-09-25, loft#1676)* — `(G-Own)` named no rule for a yielded
  LAMBDA, and the code gave the closure record a handle into the generator's frame.  A lambda
  kept past its generator read a released store; two lambdas over one local corrupted the
  generator's copy of it, differently per backend and silently on `--native`; a drained `for`
  over lambdas freed a value the exhausting advance never produced (`BUG (#306)`, and on
  `--native` the generator's own store); and `--native` refused `yield from` of lambdas with
  the loop-body collector's message.  Closed by the owner's ruling, written into `(G-Own)`
  above: a yielded lambda's heap captures are copied into stores its record owns, and an
  implementation may still share a store where that is safe.  The exhausted fn-ref is the
  fn-ref NULL on both backends, the eager collector carries a fn-ref as two slots, and a
  `yield from` advance asks `next_operands` for its channel.  Guards:
  `tests/scripts/1676-a-yielded-lambda-owns-copies-of-what-it-captures.loft` and its refused
  twin `1676b-…`.

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
- **Stackful (`G-YieldDepth`)** — a nested non-yielding CALL active across a suspension is
  preserved with the frame and resumes correctly past it, on both backends.  A `yield` inside a
  helper that is not itself a generator is REFUSED, identically on both backends — *"yield is
  only allowed inside generator functions (return type must be iterator<T>)"*.
  ⚠ This line read *"a `yield` inside a helper called from the generator produces the value and
  resumes correctly past the helper"* until 2026-09-25, when it was measured and does not.
  [VERIFICATION.md](VERIFICATION.md) had said so in three places — *"deferred G-YieldDepth — a
  `yield` INSIDE a helper (true stackful) needs `yield from`"* — so the two records disagreed
  about the same rule, and this is the one a reader of the rules meets.  A conformance line is a
  measurement, not a restatement of the rule above it.
- **Delegation (`G-Delegate`)** —
  `tests/scripts/a-delegations-prefix-runs-once-per-activation.loft`: the statements before a
  `yield from` run once per activation (a counter, three statements, a loop, and an append to a
  vector the program reads afterwards), a delegation before and after a lazily-lowered loop, an
  prefix between two of them, and a consumer that stops after one value — values on both
  backends.  `a-delegation-beside-a-loop-compiles.loft` is the exit-channel half: a delegation
  after a lazily-lowered loop, after one with a resume slice, after an EAGER loop, and two of
  them around a loop.  `tests/scripts/1277-…` covers arguments and exhaustion, and
  `tests/coroutine_matrix.rs`'s X5 column crosses delegation with the yielded TYPE — integer,
  text, record, tuple, float, single and enum.
  ⚠ Before 2026-09-25 the native state machine re-ran a delegation's prefix on every advance it
  served (silent: the produced sequence is unchanged), a `yield from` beside a loop in either
  lowering did not compile, and a delegated TUPLE did not compile.  The X5 column that crosses
  exactly this was closed on the stale scope line above, which is how all four survived.  The
  delegated fn-ref is loft#1676 and still open.
- **Exhaustion (`G-Done`)** — a finite generator produces its sequence then reports done; further
  advances stay done (no restart, no fault).
- **Ownership (`G-Own`)** — `tests/scripts/1589-a-yielded-record-is-the-consumers.loft`: a
  record returned past its generator, a drained and a broken `for`, a `match`, `yield from`, a
  loop-body yield, copies of a local, a member, a parameter, a text-holding record and a
  vector, and a tuple copied, drained and kept — values, drop traces and a clean store census
  on both backends.
- **Holding (`G-Hold`)** — `tests/scripts/1585-a-generator-can-be-held-in-a-field-or-an-element.loft`:
  a vector, a struct field, a nested record, a vector field and a tuple member holding a
  generator; the round-robin scheduler; a field, an element and a local given a new generator;
  a removal by index and by `#remove`, of a handle and of a record holding one; views out of a
  field, an element and a `for`; a local that views on one path and owns on another; records
  holding generators yielded and kept — values on both backends, and the leak gate says every
  frame was released once.  A vector of vectors of them, at death and with a row removed:
  `tests/scripts/1597-a-vector-of-vectors-releases-its-inner-elements.loft` `c12`.  A keyed
  collection of records holding them — `sorted`, `hash`, `index`, `spatial`, as a local, a
  field, rebound, a record taken out, nested, and beside a hook that stays unrun:
  `tests/scripts/1601-a-generator-held-in-a-keyed-collection-is-released.loft`.
- **Interchangeable at `for` (`G-For`)** — `for x in gen() { … }` and `for x in vec { … }`
  visit their elements by the same loop; swapping a generator for the equivalent vector changes
  only timing, not the values or their order.

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on a
generator's produced sequence, laziness, or exhaustion is the definitional error this doc names.
