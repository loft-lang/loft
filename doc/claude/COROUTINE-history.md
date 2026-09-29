<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# COROUTINE-history.md — the implementation record

The timeline behind [COROUTINE.md](COROUTINE.md): the phased implementation plan (phases 1–5) as it was worked, all completed in 0.8.3.

## Implementation Phases

---

### Phase 1 — Infrastructure (`src/state/mod.rs`, `src/data.rs`)

Introduce all runtime data structures without any language surface.

1. Define `CoroutineStatus { Created, Suspended, Running, Exhausted }` in
   `src/data.rs`.

2. Define `CoroutineFrame` (all fields above) in `src/state/mod.rs`.

3. Add `coroutines: Vec<Option<Box<CoroutineFrame>>>` and
   `active_coroutines: Vec<usize>` to `State`; initialise both in the
   constructor. Pre-push one `None` at index 0 (null sentinel).

4. Add helper functions to `State`:
   - `allocate_coroutine(frame: CoroutineFrame) -> usize` — finds the first
     `None` slot at index ≥ 1 or pushes a new slot; returns the index.
   - `free_coroutine(idx: usize)` — sets the slot to `None` and zeroes the
     capacity hint.
   - `coroutine_frame_mut(db_ref: DbRef) -> &mut CoroutineFrame` — asserts
     `store_nr == COROUTINE_STORE` and `rec != 0`; panics on invalid index.

5. Define `COROUTINE_STORE: u16 = u16::MAX` (or another value that cannot
   clash with `Stores.allocations` indices, which are limited by `Stores.max`).

6. Add `serialise_text_slots` as described in the Runtime Design section.
   Leave `free_dynamic_str` as a stub (panics) until the text side-table API
   is confirmed.

#### Tests — Phase 1

| Test | What it verifies |
|---|---|
| `coroutine_allocate_nonzero` | `allocate_coroutine` never returns 0 |
| `coroutine_allocate_retrieve` | allocated frame is retrievable via `coroutine_frame_mut` |
| `coroutine_free_reuse` | after `free_coroutine`, the slot is `None`; next allocation reuses it |
| `coroutine_null_dbref` | DbRef with `rec == 0` is treated as null by `coroutine_frame_mut` |
| `serialise_static_text` | `serialise_text_slots` does not add to `text_owned` for static `Str` |
| `serialise_null_text` | `serialise_text_slots` skips null `Str` (ptr == STRING_NULL) |

---

### Phase 2 — Type and opcode declarations (`default/05_coroutine.loft`, `src/main.rs`)

1. Create `default/05_coroutine.loft` declaring:
   - `CoroutineStatus` enum
   - `next(gen: iterator<T>) -> T` bound to `OpCoroutineNext`
   - `exhausted(gen: iterator<T>) -> boolean` bound to `OpExhausted`

2. Add the file to the default load order in `src/main.rs` after
   `04_stacktrace.loft`.

3. Add all six coroutine opcodes to the `Op` enum in `src/data.rs` (or
   wherever opcodes are defined) with their operand types.

4. Add stub implementations in `fill.rs` for all opcodes (abort with
   "not yet implemented" to catch accidental emission during later phases).

5. Add parser recognition:
   - A function with return type `iterator<T>` is flagged as a generator.
   - `yield expr` is parsed as a new statement type; compile error if outside
     a generator function.
   - `yield from expr` is parsed as a new statement type.
   - `e#remove` on a generator iterator emits a compile error (SC-CO-11).
   - `yield` inside `par(...)` emits a compile error (SC-CO-4).

#### Tests — Phase 2

| Test | What it verifies |
|---|---|
| `gen_types_declared` | `CoroutineStatus`, `next`, `exhausted` resolve in a loft program |
| `yield_non_generator_error` | `yield` in a plain function is a compile error |
| `yield_par_error` | `yield` inside `par(...)` is a compile error |
| `remove_generator_error` | `e#remove` in a `for` loop over a generator is a compile error |

---

### Phase 3 — Creation and exhaustion (`src/fill.rs`, `src/state/mod.rs`)

Implement the frame lifecycle without the yield/resume cycle.

1. Implement `OpCoroutineCreate` as in the pseudocode above:
   - `serialise_text_slots` for the argument region.
   - Allocate frame; push DbRef.

2. Implement `OpCoroutineReturn` as above:
   - Clear `text_owned` and `stack_bytes`; truncate `call_stack`;
     mark `Exhausted`; push null; jump to `caller_return_pos`.

3. Implement `OpCoroutineNext` for `Created` and `Exhausted` cases only:
   - `Exhausted`: push null immediately.
   - `Created`: restore frame, push `call_frames`, jump to `code_pos`.
   - `Running` / re-entrant: runtime error.

4. Implement `OpExhausted`.

5. In `codegen.rs`, emit `OpCoroutineCreate` (instead of `OpCall`) when the
   called function is a generator; emit `OpCoroutineReturn` at each `return`
   and at the implicit end of a generator body.

#### Tests — Phase 3

| Test | What it verifies |
|---|---|
| `gen_create_returns_dbref` | calling a generator function returns a non-null `iterator<T>` DbRef |
| `gen_empty_body_null` | a generator with an empty body returns null on first `next()` |
| `gen_return_immediately` | a generator that returns without yielding is exhausted after one `next()` |
| `gen_exhausted_next_null` | `next()` on an exhausted generator always returns null |
| `gen_exhausted_flag` | `exhausted(gen)` returns true after the generator finishes |
| `gen_reentrant_error` | advancing a `Running` generator produces the expected runtime error message |
| `gen_null_iterator` | `next(null_gen)` returns null without crashing |

---

### Phase 4 — Yield and resume (`src/fill.rs`, `src/state/mod.rs`)

Implement the suspend/resume cycle.

1. Implement `OpYield`:
   - Serialise `stack[stack_base..stack_pos]` including the yielded value
     (SC-CO-10); split into `stack_bytes` and `updated_value`.
   - Save `call_frames`; truncate `call_stack`.
   - Mark `Suspended`; pop `active_coroutines`.
   - Slide `updated_value` to `stack_base`; jump to `caller_return_pos`.

2. Extend `OpCoroutineNext` for the `Suspended` case:
   - Save `caller_return_pos`, update `stack_base` and `call_depth` (SC-CO-7).
   - Patch `text_owned` pointers into `stack_bytes`; write to live stack.
   - Restore `call_frames`; mark `Running`; push to `active_coroutines`.
   - Jump to `frame.code_pos`.

3. In the compiler, emit `OpYield` at each `yield expr` statement.

4. In the for-loop code generator, emit `OpCoroutineNext` (instead of a
   collection-iterator advance) when the iterator is a generator (detected
   by the `COROUTINE_STORE` type tag on the `iterator<T>` value).

#### Tests — Phase 4

| Test | What it verifies |
|---|---|
| `gen_single_yield` | a generator that yields once produces exactly one value then exhausts |
| `gen_multiple_yield` | a generator that yields three values produces them in order |
| `gen_for_loop` | `for n in count_up(0)` with break at 5 produces 0..4 |
| `gen_resume_local` | the generator's local variable retains its value across a yield |
| `gen_text_local` | a text local is correctly preserved across a yield (SC-CO-1, SC-CO-8) |
| `gen_text_yield` | a generator that yields a `text` value does not dangle (SC-CO-10) |
| `gen_caller_local_intact` | a caller local pushed after creating the generator survives resumption (SC-CO-7) |
| `gen_infinite_break` | an infinite generator with a break in the consumer terminates cleanly |
| `gen_count_attribute` | `n#count` counts from 0 across iterations |
| `gen_first_attribute` | `n#first` is true only on the first iteration |

---

### Phase 5 — `yield from` (`src/fill.rs`, parser)

1. Implement `OpYieldFrom`:
   - Inner loop: `OpCoroutineNext` on sub-generator; if non-null, `OpYield` it;
     on outer resume, loop; if null (sub exhausted), exit loop.
   - `active_coroutines` naturally contains both outer and inner indices while
     both are active; the check in Phase 4 covers the nested case (SC-CO-9).

2. In the compiler, emit `OpYieldFrom` at each `yield from expr` statement.

#### Tests — Phase 5

| Test | What it verifies |
|---|---|
| `yield_from_flat` | `yield from range(0, 3)` produces 0, 1, 2 |
| `yield_from_chain` | two sequential `yield from` calls produce their values in order |
| `yield_from_recursive` | recursive `yield from` on a tree produces leaves in left-to-right order |
| `yield_from_empty` | `yield from` an already-exhausted sub-generator produces no values |
| `yield_from_reentrant` | advancing the outer generator while inside `yield from` produces the expected runtime error (SC-CO-9) |
