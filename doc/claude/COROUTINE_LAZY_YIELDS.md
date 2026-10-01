# Coroutines — lazy loop yields (CL-9)

The design for native lazy loop yields.  Part of [COROUTINE.md](COROUTINE.md), whose Known Limitations table lists CL-9.

## Design: lazy loop yields (CL-9)

> **Status: slices 1–3 built (loft#836, #1586, #1798) — every loop whose yields are statements
> is lazy on `--native`; slice 4 (text-in-loop interning) open.**  Two lowerings share the work,
> described under § Re-descent below: the ROTATED loop for one yield on the body's straight
> line, and RE-DESCENT for every other shape.  What still takes the eager buffer is a `yield`
> inside an expression re-descent cannot hoist (COROUTINE.md § CL-9).  A loop
> with ONE `yield` on its body's straight line is lowered to a header+body state pair — one
> iteration per advance, the cursor persisted.  A
> `while` is the same lowering with no setup (the parser emits it as a bare `Loop`).  Statements
> after the yield ROTATE the loop: a third state runs them at the start of the next advance,
> before the header, so a `break` among them still ends the loop.  A lazy loop's hidden record
> buffers (`__ref_*`) persist in the struct, and so do a lambda's fn-ref and its `___clos_*`
> record (loft#1587), which is what lets a loop that builds a closure per iteration stay lazy.
> Guard: `tests/scripts/1586-a-while-loop-generator-yields-before-its-next-iteration.loft`. `tests/scripts/836-lazy-loop-yields.loft`
> and `tests/oracle/26-coroutine-laziness.loft` assert the interleaving; VALUES cannot, which is
> why the value-only oracle reported agreement across a difference this wide.
> The eager buffer holds VALUES, not handles: a struct or vector yield is copied into a
> snapshot store the generator owns (`coroutine_snapshot`, one store per generator, freed at
> exhaustion and from `drop_stores`), because the whole loop runs before the consumer reads
> and a handle to a per-iteration record would alias its final state — three yields of
> {7,17,27} summed to 81. The factory also runs the generator's TAIL (the statements after the
> last yield, where its scope-exit frees live) once the buffer is filled, so every persistent
> heap local is freed and a `print` after the loop runs (guard
> `tests/scripts/1356-a-record-yielded-from-a-loop-body-is-the-value-at-the-yield.loft`).
> The sections after § Re-descent are the original design, kept as the map for slice 4.

### Re-descent

Rust has no jump into a loop or a branch, so a lowering that must resume at an arbitrary yield
emits the body as the Rust `loop` / `if` / block it is and re-enters it FROM THE TOP
(`YieldSegment::Resumable` in `src/generation/coroutine.rs`):

- every `yield` is numbered `1..=n` in source order, and a suspending one stores its number in
  the struct field `__resume_<seg>`;
- the next advance starts with `__seek` set to that number.  While seeking, a statement that
  holds no yield is skipped, a statement holding the target is entered, an `if` takes the
  branch holding it without testing its condition, and a loop is entered without its header
  running; the target yield clears `__seek`, and execution goes on after it;
- a `continue` or `break` is the Rust one of the loop it names — nothing to translate;
- every local the body carries from one of its units (a run of yield-free statements, a
  condition, a yielded value) to another is a struct field (`resumable_carried`), so what the
  re-descent skips is exactly what already ran;
- a yield inside a VALUE is first rewritten into a statement (`Sink`): the statements in front
  of the value move in front of the statement using it, an assignment sinks into each branch
  of a value `if`, and an operand's yield is hoisted only when the operands before it are
  literals or scalar locals the hoisted statements do not write;
- each yield hands over through the code a straight-line yield uses (`emit_yield_handover`),
  so every channel — value, `next_into` for tuples and fn-refs, a record the generator then
  forgets (`(G-Own)`) — works the same lazily.

The rotated loop stays for the shape it already covers.  A loop it cannot carry (a record or
tuple channel, a setup or resume slice with a local that is not a field) is re-descended
instead.  `LOFT_NO_REDESCENT=1` restores the eager buffer for every re-descended loop (the bisect
step, NATIVE_SWITCHES.md § Generators).  Guards:
`tests/scripts/a-generator-resumes-at-the-yield-it-suspended-at.loft` (control-flow shapes) and
`tests/scripts/a-record-yielded-from-a-loop-is-handed-over-lazily.loft` (every channel).

### The problem, precisely

Native coroutine lowering (`generation/coroutine.rs`) scans a generator body into **segments**
(`Simple`, `YieldFrom`, `ForLoopBody`) and emits a state machine (`LoftCoroutine::next_i64`).
`Simple` (a straight-line `yield`) is a real **lazy state-machine step** — control returns to the
consumer at the yield and resumes after it. `YieldFrom` is lazy too, and it covers more than its
name suggests: `detect_yield_from` matches a loop that does nothing but yield its own loop
variable, so `for x in <iterable> { yield x }` delegates lazily and never reaches the eager path.
`ForLoopBody` catches everything else — any `Block`/`Loop`/`If` containing a yield — and is the
shortcut: it **runs the loop eagerly and buffers every yield into a `Vec<i64>`**, then serves the
buffer, chosen (the code comment) to avoid "a full state-machine decomposition of the
range-iteration IR." The interpreter, by contrast, serialises the whole frame at each yield and is
lazy everywhere. CL-9 is exactly this gap — and its real edge is one statement wide, not one
construct wide.

### The invariant (the hypothesis to build against)

> **A `yield` returns control to the consumer BEFORE the next generator statement runs — in a
> loop body no less than in straight-line code.** Equivalently: the generator's observable step
> sequence is `next()`-driven, one body-slice per advance, with the loop's *cursor* (index /
> iterator position) and any loop-carried locals PERSISTED in the coroutine frame across advances.

If that invariant holds, `--native` matches the interpreter and formal G-Call/G-Next, and the
decided edge closes.

### The mechanism (the lever — persist the loop cursor, decompose the loop into states)

The two halves are already in the tree:

1. **State persistence — REUSE the existing machinery.** `coroutine_persistent_locals` /
   `coroutine_persistent_vars` (`generation/coroutine.rs`, `dispatch.rs`) already lift a
   generator's locals that live across a yield into the coroutine **struct** (a heap record —
   the same heap-state-persistence approach a **closure record** uses; the coroutine struct *is*
   the analog). The fix ADDS the loop's **cursor** (the `#index` / range counter / iterator
   position) and any loop-carried locals to that persistent set — literally "add the loop
   variable to the coroutine state to remember through a pass."
2. **Control decomposition — the new work.** Replace the `ForLoopBody` eager-buffer segment with
   **resumable states**: a *loop-header* state (test the condition / advance the cursor from its
   persisted value), a *body* state that runs to the `yield`, writes the value to the yield codec,
   and returns (leaving the cursor persisted), and a *back-edge* so the next `next_i64` re-enters
   the header. This is the standard `async`/`await` loop-to-state-machine transform — Rust
   expresses it fine; loft simply has not decomposed loops yet.

### Failure axes (probe each before trusting the transform — where it gets hard)

The single-yield range/vector loop is easy; the transform's cost is dominated by these axes, each
a state the decomposition must model:

- **A1 — a loop whose body is a single `yield` PLUS at least one other statement** — one header +
  one body state, persist the index. Do this first.
  **Not** the bare `for x in <iterable> { yield x }` shape: `detect_yield_from` matches that
  exactly (a 2-op block whose loop's third op is `Yield(Var(item_var))`) and lowers it to the
  lazy `YieldFrom` segment, so it is already `next()`-driven on native and needs no work here.
  The eager path starts the moment the body holds anything else — `{ print(…); yield i; }` is
  `ForLoopBody`. Verified 2026-08-09: a `for i in 0..1000000000 { yield i; }` generator
  consumed three values and stopped, while `{ print("p{i} "); yield i; }` over `0..1000` ran all
  1000 iterations before the consumer's first advance.
- **A2 — multiple yields per iteration** (`for … { yield a; yield b }`) — each yield is its own
  state; the back-edge targets the header, but re-entry lands at the *next* yield-state.
- **A3 — a `yield` inside an `if`/`match` inside the loop** — conditional states; the resume point
  depends on which branch yielded.
- **A4 — nested loops with yields** — a cursor per loop level, all persisted; the state graph is
  the product.
- **A5 — `while` / bare `loop` (not just `for`)** — the header is a general condition, and a bare
  `loop { yield }` is the infinite-generator case CL-9's hazard names — the whole point of lazy.
- **A6 — text / tuple yields in a loop** — interacts with CL-8 (a yielded text is a `&str`); a
  store-derived text live across the loop back-edge must be interned/persisted (CL-2b / CL-7).

### The incremental plan (ship a slice, keep the fallback)

1. **Slice 1 — A1 only.** Decompose a single-yield `for` over a range or vector into header+body
   states, persist the index. Keep the eager `Vec` buffer as the FALLBACK for A2–A6 (a
   segment-scanner predicate: "simple single-yield counted loop → lazy; else → eager"). This
   closes CL-9 for the common case and shrinks the decided edge to "only complex loops."
2. **Slice 2 — A2/A3** (multiple + conditional yields): generalise the segment scanner to emit one
   state per yield within the loop body.
3. **Slice 3 — A4/A5** (nested + `while`/`loop`): a cursor stack; the infinite-generator case then
   works lazily on native (the biggest user win).
4. **Slice 4 — A6**: fold in the CL-8/CL-2b text-in-loop interning.

Each slice keeps the eager fallback for the axes it hasn't reached, so no generator regresses.

### Verification (how each slice is proven)

The falsifying program is a **side-effect interleaving** check the value-only oracle currently
misses: `fn g() -> iterator<integer> { for i in 0..3 { print("y{i} "); yield i } }` consumed by
`for x in g() { print("g{x} ") }` must print **`y0 g0 y1 g1 y2 g2`** (lazy) on `--native`, not
`y0 y1 y2 g0 g1 g2` (eager). Graduate it to `tests/oracle/` beside the straight-line
`26-coroutine-laziness.loft` guard; add an **infinite-generator + early `break`** case (must
terminate on native) once Slice 3 lands. `tests/coroutine_matrix.rs` + the differential oracle
guard the value equality throughout.

### Reassertion-site count (the design-protocol tell)

The invariant is asserted at ONE place — the segment scanner's "lazy-vs-eager" predicate +
the `ForLoopBody` codegen it feeds. Persistence rides the existing `coroutine_persistent_*`
chokepoint (one place). So `N ≈ 1` re-assertion site with the eager fallback as the explicit,
loud default — the transform is additive, not a spray. That is the signal this is a bounded
enhancement, not an open-ended rewrite.
