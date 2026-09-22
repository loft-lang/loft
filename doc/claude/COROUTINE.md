
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Coroutine Design

> **Status: completed in 0.8.3.** CO1.1–CO1.6 implemented; `yield from` (CO1.4) deferred to 1.1+.
> Open enhancement: **native lazy loop yields** ([Design: lazy loop yields (CL-9)](#design-lazy-loop-yields-cl-9))
> — slice 1 has landed (loft#836), and the `while` half of slice 3 with it (loft#1586): a `for` or
> `while` loop with one `yield` on its body's straight line is lazy on `--native`, statements
> after the yield included. Slices 2–4 (multiple / conditional yields, nested loops, text-in-loop
> interning) are what remains; each keeps the eager buffer.

Coroutines give loft programs generator functions: functions that can suspend
execution with `yield`, return a value to the caller, and resume from the same
point on the next iteration. The suspended function's entire stack — including
any nested calls active at the point of `yield` — is serialised to a
heap-allocated, extensible frame. This makes loft coroutines **stackful**: a
`yield` inside a helper called from the generator is valid, and `yield from`
delegates cleanly to a sub-generator.

---

## Contents

- [Goals](#goals)
- [Language Syntax](#language-syntax)
- [Exposed Types](#exposed-types)
- [Stack Layout and Execution Model](#stack-layout-and-execution-model)
- [Coroutine Frame Design](#coroutine-frame-design)
- [Runtime Design](#runtime-design)
- [Safety Concerns and Mitigations](#safety-concerns-and-mitigations)
- [Implementation Phases](#implementation-phases)
- [Known Limitations](#known-limitations)
- [Non-Goals](#non-goals)
- [See also](#see-also)

---

## Goals

- Allow any function returning `iterator<T>` to use `yield` to produce values
  lazily, one at a time.
- Preserve the full call stack at the point of `yield` (stackful semantics),
  not only the generator's own locals. This allows `yield` inside helper
  functions and makes `yield from` implementable without a trampoline.
- Integrate naturally with the existing `for item in expr { }` loop syntax.
- Keep the hot path (all existing code that does not use coroutines) completely
  unaffected: no overhead on ordinary `fn_call` / `fn_return`.
- Store suspended frames on the heap in extensible allocations so the frame
  can grow to any call depth without preallocating a fixed stack size.

---

## Language Syntax

### Generator function

A function whose return type is `iterator<T>` is a **generator function**.
Its body may contain `yield` expressions. The compiler rejects `yield` in any
function not declared with an `iterator<T>` return type.

```loft
fn count_up(start: integer) -> iterator<integer> {
    i = start;
    loop {
        yield i;
        i += 1;
    }
}
```

The function body is **not** executed when the function is called. Instead, a
suspended coroutine frame is allocated and returned as an `iterator<integer>`
value. Execution begins on the first `next()` call or the first iteration of a
consuming `for` loop.

### `yield` — produce one value and suspend

```loft
yield expr;
```

Evaluates `expr` to type `T`, produces it as the next element of the iterator,
and suspends the coroutine. Control returns to the consumer. The coroutine
resumes from the statement immediately after `yield` on the next advance.

`yield` may appear at any call depth inside the generator, including inside
helper functions called from it (stackful semantics).

### `yield from` — delegate to a sub-generator

```loft
yield from expr;    // expr must have type iterator<T>
```

Exhausts the sub-generator `expr`, forwarding each element it produces to the
consumer, then continues execution in the outer generator. Equivalent to:

```loft
for item in expr { yield item; }
```

but dispatches the sub-advance directly without rebuilding a for-loop frame.
Useful for tree traversal and recursive generators:

```loft
fn all_leaves(node: reference<TreeNode>) -> iterator<integer> {
    if !node.left && !node.right {
        yield node.value;
    } else {
        yield from all_leaves(node.left);
        yield from all_leaves(node.right);
    }
}
```

### Consuming a generator

**In a `for` loop (primary use case):**

```loft
for n in count_up(0) {
    if n > 10 { break; }
    say("{n}");
}
```

The for-loop attributes `#first` and `#count` work on generator iterators; they
are maintained by the for-loop wrapper, not by the generator itself.  `e#index`
is a **compile-time error**: it is the position to give to `v[i]`, and a
generator's values have none (`#count` numbers them).  `e#remove` is a
**compile-time error** on a generator iterator too — a
generator cannot remove a value it has already yielded
(see [SC-CO-11](#sc-co-11--eremove-inside-a-generator-for-loop-must-be-a-compile-time-error)).

**Explicit advance:**

```loft
gen = count_up(5);
a = next(gen);    // 5 — gen is now suspended after the first yield
b = next(gen);    // 6
if exhausted(gen) { say("done"); }
```

`next(gen)` returns null when the generator is exhausted. `exhausted(gen)`
tests the frame's `status` field and returns true regardless of whether the
generator ever yielded a null value (see
[Known Limitations CL-1](#known-limitations)).

**A yielded value is the consumer's** (`formal/coroutines.md` `(G-Own)`).  A record,
struct-enum or vector that `next` or a `for` receives belongs to the receiver: it
outlives the generator, and a `for` releases each one when its iteration ends.  A
value built at the `yield` is handed over as it is; a value the generator keeps —
a local, a parameter, a field — is copied, so the generator changing it later does
not change what the consumer holds:

```loft
fn totals() -> iterator<Stat> {
    s = Stat { n: 0 };
    for x in 1..4 { s.n += x; yield s; }   // each consumer sees 1, 3, 6
}
```

A type with a drop hook (`OpDrop`) cannot be copied, so yielding an existing one
is a compile-time error; yield a value built at the `yield` instead.

### Generator function with parameters

Parameters are captured into the frame at construction time. They are
accessible as locals inside the generator body and may be read or mutated.

```loft
fn range(from: integer, to: integer) -> iterator<integer> {
    i = from;
    while i < to { yield i; i += 1; }
}
```

### Exhausting a generator early

A generator has **no `return`** — values leave it only through `yield`, so `return e`,
a bare `return;`, and a body whose tail is a value are all static errors
(`formal/coroutines.md` (G-Return), enforced at all three spellings). End early with
`break`: the loop stops, the body reaches its end, and the generator is exhausted.

```loft
fn first_positive(v: vector<integer>) -> iterator<integer> {
    for x in v {
        if x > 0 { yield x; break; }
    }
}
```

Consuming it yields the one value and then stops: `next` answers `5`, the next `next`
answers null, and `exhausted` is true.

---

## Exposed Types

Declared `pub` in `default/05_coroutine.loft`, loaded after `04_stacktrace.loft`.

### `CoroutineStatus` — lifecycle state

```loft
pub enum CoroutineStatus {
    Created,      // allocated; body not yet entered
    Suspended,    // yielded; waiting to be resumed
    Running,      // currently executing; re-entrant advance is a runtime error
    Exhausted,    // returned or fell off the end; next() always returns null
}
```

### `iterator<T>` — the generator handle

`iterator<T>` is the existing loft iterator type. A generator function returns
an `iterator<T>`; at the language level no new type name is needed. The runtime
representation differs from a collection iterator (it uses `store_nr ==
COROUTINE_STORE` in the DbRef), which is how the for-loop compiler and the
`OpCoroutineNext` opcode distinguish a coroutine from a store-backed iterator.

### `next(gen: iterator<T>) -> T`

Advances the iterator by one step and returns the yielded value, or null when
the generator is exhausted. Bound to `OpCoroutineNext`.

### `exhausted(gen: iterator<T>) -> boolean`

Returns true if and only if the frame's `status` is `Exhausted`. Safe to call
on a null iterator, `iterator<T>?` (returns true). Lowered by the compiler to
`OpCoroutineExhausted` for an iterator argument; it is not a stdlib function
over `reference`, so a program may define `exhausted` for its own iterator
types, and a call on a type that declares none is refused.

---

## Stack Layout and Execution Model

### Stack layout overview

```
         lower addresses
         ┌────────────────────────────────────────────┐
         │  ... caller locals (e.g. the gen DbRef) ...│  ← caller's frame
         ├────────────────────────────────────────────┤  ← new_base = stack_pos at resume
         │  generator parameter 0                     │
         │  generator parameter 1                     │
         │  generator local variable A                │
         │  generator local variable B                │
         │  ... nested helper call locals ...         │
         ├────────────────────────────────────────────┤  ← stack_pos
```

The generator frame is placed immediately above the caller's current stack top
(`new_base = self.stack_pos`) at each resume. It is never at a fixed absolute
position; `frame.stack_base` is updated to `self.stack_pos` at the start of
every `OpCoroutineNext` (see [SC-CO-7](#sc-co-7--absolute-stack_base-stale-when-the-caller-pushes-locals-after-creation)).

When the generator yields, the region `[new_base .. value_start)` is serialised
into `frame.stack_bytes` and the stack pointer is rewound to `new_base`. The
yielded value is slid down to `new_base` as the return value of `next()`.

### Return address handling

Unlike `fn_call`, which writes the return address onto the loft stack,
`OpCoroutineNext` stores the continuation in the frame itself:

```
frame.caller_return_pos = self.code_pos   // instruction after OpCoroutineNext
```

`OpYield` and `OpCoroutineReturn` both jump to `frame.caller_return_pos` to
return control to the `next()` call site. This avoids placing any return
address on the loft stack and decouples the coroutine's execution from the
caller's stack depth.

### Lifecycle state machine

```
                 next() or for-loop
    Created ──────────────────────────► Running
                                            │
               ┌────────────────────────────┤
               │ yield                      │ end of body
               ▼                            ▼
           Suspended ──── next() ────► Running    Exhausted
           (frame saved)                (frame          │
                                         active)        │ next()
                                                        ▼
                                                   (null pushed)
```

### Construction (call → `Created`)

When the compiler encounters a call to a generator function, it emits
`OpCoroutineCreate` instead of `OpCall`:

1. The call-site arguments have already been pushed onto the stack.
2. The runtime copies those argument bytes into `frame.stack_bytes`, processes
   any dynamic text slots (see [SC-CO-1](#sc-co-1--text-ownership-in-the-saved-stack),
   [SC-CO-8](#sc-co-8--dynamic-string-objects-leaked-during-yield-serialisation)),
   and sets `frame.code_pos` to the function's entry bytecode position.
3. `frame.stack_base` is set to `0` — it has no meaning until the first resume.
4. The argument bytes are removed from the live stack.
5. The frame's index (as a `COROUTINE_STORE` DbRef) is pushed as the result.
6. **The function body is not entered.**

### First advance and resume (`Created` / `Suspended` → `Running`)

`OpCoroutineNext` on a `Created` or `Suspended` frame (both cases are identical):

1. Check `active_coroutines` for a re-entrant advance (see
   [SC-CO-3](#sc-co-3--re-entrant-advance-corrupts-the-live-stack),
   [SC-CO-9](#sc-co-9--coroutine_sp-scalar-cannot-represent-yield-from-nesting)).
2. Record `frame.caller_return_pos = self.code_pos` (continuation address).
3. Set `frame.call_depth = self.call_stack.len()` — the frame's call frames
   will go above the current call stack depth at this resume (see note below).
4. Set `frame.stack_base = self.stack_pos` — place frame at the current stack
   top (SC-CO-7).
5. Patch `Str` pointers in `frame.stack_bytes` to point to the current
   `text_owned` buffer addresses (SC-CO-1).
6. Write `frame.stack_bytes` into `State.stack` at `frame.stack_base`.
7. Set `stack_pos = frame.stack_base + frame.stack_bytes.len() as u32`.
8. Append `frame.call_frames` onto `State.call_stack`.
9. Mark frame `Running`; push its index onto `active_coroutines`.
10. Set `code_pos = frame.code_pos`; execution continues normally.

**Note on `call_depth`:** `frame.call_depth` is reset at every resume to the
current `call_stack.len()`. This keeps the "coroutine's own frames" slice
`call_stack[call_depth..]` accurate regardless of the caller's nesting depth
at the time of the advance.

### Yielding (`yield expr` → `Suspended`)

`OpYield` fires with `value_size` (the byte width of type `T`) as operand:

1. The yielded value occupies `stack[value_start..stack_pos]` where
   `value_start = stack_pos - value_size`.
2. Serialise the **entire** region `stack[stack_base..stack_pos]` (locals
   **and** yielded value together) to detect any `Str` in the yielded value
   itself (see [SC-CO-10](#sc-co-10--yielded-text-value-is-not-serialised-its-string-may-be-freed)).
   Split the result: `frame.stack_bytes = bytes[..locals_len]`, and keep the
   updated value bytes separately for the slide step below.
3. Free original dynamic `String` allocations via the text side-table
   (SC-CO-8).
4. Save `call_stack[call_depth..]` into `frame.call_frames`; truncate
   `call_stack` to `call_depth`.
5. Set `frame.code_pos = self.code_pos` (instruction after `OpYield`).
6. Mark frame `Suspended`; pop its index from `active_coroutines`.
7. Slide the (pointer-updated) yielded value bytes to `frame.stack_base`.
8. Set `stack_pos = frame.stack_base + value_size`.
9. Set `code_pos = frame.caller_return_pos`; execution returns to the
   `next()` call site.

### Exhaustion (end of body → `Exhausted`)

`OpCoroutineReturn` fires with `value_size` as operand:

1. Drop `frame.text_owned` (frees all owned String allocations).
2. Clear `frame.stack_bytes`.
3. Truncate `call_stack` to `frame.call_depth`.
4. Mark frame `Exhausted`; pop its index from `active_coroutines`.
5. Set `stack_pos = frame.stack_base`.
6. Push typed null (value_size null bytes) at `frame.stack_base`.
7. Set `code_pos = frame.caller_return_pos`; execution returns to the
   `next()` call site.

### `yield from` (`OpYieldFrom`)

`yield from sub_gen` is implemented as a tight inner loop inside the outer
generator's execution:

1. Load `sub_gen` DbRef from the generator's local variable.
2. Loop:
   a. Dispatch `OpCoroutineNext` on `sub_gen` — this pushes the sub-generator's
      yielded value (or null) directly onto the live stack at the current
      `stack_pos`.
   b. If the value is non-null, dispatch `OpYield` with it — this suspends the
      outer generator and returns the value to the consumer.
   c. On outer resume, `stack_pos` is back inside the outer generator's frame.
      `sub_gen`'s DbRef is restored from `frame.stack_bytes`. Go to step (a).
   d. If the value is null, `sub_gen` is exhausted. Clear `sub_gen` from the
      local; continue execution past `yield from`.

The outer generator's frame is saved/restored on each outer yield exactly as
in the normal yield path. The sub-generator lives in `State.coroutines`
independently; its DbRef is just another local variable in the outer frame and
is preserved across suspensions.

---

## Coroutine Frame Design

### Why frames are not in the loft Store system

The loft `Store` system holds fixed-schema records; every record of a given
type has the same byte layout. A `CoroutineFrame` contains two variable-length
fields (`stack_bytes` and `call_frames`) whose sizes depend on the call depth
at the point of `yield`. Storing them in a fixed-schema store would require
either capping the frame size or adding an indirect pointer chain.

Instead, frames are held in a side-table on `State`:

```rust
pub coroutines: Vec<Option<Box<CoroutineFrame>>>,
```

`None` entries are freed slots available for reuse. Index 0 is permanently
`None` to make the null-sentinel rule (`rec == 0 → null`) work without special
cases.

### `CoroutineFrame` — internal Rust struct

```rust
pub struct CoroutineFrame {
    /// Definition number of the generator function.
    pub d_nr: u32,
    /// Lifecycle state.
    pub status: CoroutineStatus,
    /// Bytecode position to resume at (points to the instruction after the
    /// last OpYield, or to the function entry for a Created frame).
    pub code_pos: u32,
    /// Absolute stack position of the frame's first byte during execution.
    /// Set to 0 at construction; updated to self.stack_pos at every resume
    /// (SC-CO-7). Read by OpYield to know the frame's extent.
    pub stack_base: u32,
    /// Return address in the consumer — the instruction to execute after the
    /// generator yields or exhausts. Stored here to avoid mixing coroutine
    /// return addresses into the loft stack layout.
    pub caller_return_pos: u32,
    /// Serialised stack contents: locals from stack_base up to (but not
    /// including) the yielded value, as of the last suspension. Empty for a
    /// Created frame (the parameters are the initial contents).
    pub stack_bytes: Vec<u8>,
    /// Owned copies of every dynamic text slot that was live in the frame at
    /// the last yield. The u32 is the byte offset within stack_bytes at which
    /// the Str's pointer must be patched on resume. u32 (not u16) to handle
    /// large frames without silent truncation (SC-CO-12).
    pub text_owned: Vec<(u32, String)>,
    /// Saved entries from State.call_stack that belong to this frame's
    /// execution (indices [call_depth..] at the time of the last yield).
    pub call_frames: Vec<CallFrame>,
    /// Index into State.call_stack at the start of this frame's execution.
    /// Reset to self.call_stack.len() at every resume so that the "coroutine's
    /// own frames" slice is always consistent with the caller's current depth.
    pub call_depth: usize,
}
```

### DbRef encoding for coroutines

```
store_nr == COROUTINE_STORE   (reserved u16 constant, not a real allocations index)
rec      == index into State.coroutines   (non-zero for a live frame)
pos      == 0   (unused)
```

Null sentinel: `rec == 0` (index 0 is permanently `None`). This matches the
standard loft null rule for references: `rec == 0 && store_nr == …` is null.

### Extensibility

`stack_bytes`, `text_owned`, and `call_frames` are Rust `Vec`s and grow on
demand. There is no preallocated frame size. A generator that calls N levels
of helpers at the point of yield will have `call_frames.len() == N` and
`stack_bytes.len()` proportional to the total locals across those N frames.

---

## Runtime Design

### State additions

```rust
// New fields on State:
pub coroutines:        Vec<Option<Box<CoroutineFrame>>>,
pub active_coroutines: Vec<usize>,  // indices of all currently-running coroutines,
                                     // innermost (most recently resumed) last.
                                     // A coroutine is "active" from the moment
                                     // OpCoroutineNext enters it until OpYield or
                                     // OpCoroutineReturn exits it.
```

`active_coroutines` replaces the scalar `coroutine_sp` that was considered
in earlier drafts. The `Vec` correctly handles `yield from` nesting where
both the outer and the inner generator are simultaneously active (SC-CO-9).

### Opcode table

| Opcode | Operands | Emitted by | Purpose |
|---|---|---|---|
| `OpCoroutineCreate` | `d_nr: u32`, `args_size: u16` | call to `fn ... -> iterator<T>` | Allocate frame; serialise args; push DbRef |
| `OpCoroutineNext` | `value_size: u16` | `next(gen)`, for-loop advance | Resume frame; push yielded value or null |
| `OpYield` | `value_size: u16` | `yield expr` | Suspend; serialise frame; slide value to base; return to consumer |
| `OpYieldFrom` | (none) | `yield from expr` | Drive sub-generator loop; forward each value via OpYield |
| `OpCoroutineReturn` | `value_size: u16` | end of generator body (a source `return` is refused — (G-Return)) | Exhaust frame; push null; return to consumer |
| `OpExhausted` | (none) | `exhausted(gen)` | Push true if frame status == Exhausted or DbRef is null |

All coroutine opcodes require direct `&mut self` access (same pattern as
`OpStackTrace`). They are not routed through the `library` table.

### `OpCoroutineCreate` pseudocode

```rust
OpCoroutineCreate { d_nr, args_size } => {
    let def = &data.definitions[d_nr as usize];
    let args_start = self.stack_pos - u32::from(args_size);

    // Serialise the argument bytes and take ownership of any dynamic text.
    // The argument region is the only content for a Created frame.
    let mut initial_bytes = self.stack[args_start as usize
                                        .. self.stack_pos as usize].to_vec();
    let text_owned = serialise_text_slots(
        &mut initial_bytes, &def.attributes, &mut self.database);

    let frame = CoroutineFrame {
        d_nr,
        status:             CoroutineStatus::Created,
        code_pos:           def.code_pos,
        stack_base:         0,            // set on first resume (SC-CO-7)
        caller_return_pos:  0,            // set on first resume
        stack_bytes:        initial_bytes,
        text_owned,
        call_frames:        vec![],
        call_depth:         0,            // set on first resume
    };

    let idx = allocate_coroutine(&mut self.coroutines, frame);
    // Remove the now-serialised arguments from the live stack.
    self.stack_pos = args_start;
    // Push the coroutine DbRef as the result.
    self.put_stack(DbRef { store_nr: COROUTINE_STORE, rec: idx as u32, pos: 0 });
}
```

### `OpCoroutineNext` pseudocode

```rust
OpCoroutineNext { value_size } => {
    // The DbRef to advance is on top of the stack (already popped by codegen
    // into a register, or read via self.get_stack).
    let db_ref: DbRef = self.pop_stack();

    // Null iterator — push typed null immediately.
    if db_ref.rec == 0 {
        self.push_null(value_size);
        return;
    }
    debug_assert_eq!(db_ref.store_nr, COROUTINE_STORE);
    let idx = db_ref.rec as usize;

    // Re-entrant advance check: any currently-running frame (SC-CO-3, SC-CO-9).
    if self.active_coroutines.contains(&idx) {
        self.runtime_error(
            "coroutine advanced re-entrantly; this generator is already running");
    }

    let frame = self.coroutines[idx].as_mut()
        .expect("coroutine DbRef refers to freed slot");

    match frame.status {
        CoroutineStatus::Exhausted => {
            // Always null; no stack restoration needed.
            self.push_null(value_size);
            return;
        }
        CoroutineStatus::Running => {
            // Caught by active_coroutines check above; unreachable here.
            unreachable!()
        }
        CoroutineStatus::Created | CoroutineStatus::Suspended => {
            // --- Save the continuation address ---
            frame.caller_return_pos = self.code_pos;

            // --- Anchor the frame to the current stack top (SC-CO-7) ---
            frame.stack_base = self.stack_pos;

            // --- Record the call-stack depth for this resume (see call_depth note) ---
            frame.call_depth = self.call_stack.len();

            // --- Patch Str pointers in stack_bytes to point to text_owned buffers ---
            // text_owned[i] = (offset, String); the String is on the Rust heap;
            // its buffer address is stable as long as no push reallocates the Vec.
            // We pin by ensuring no push occurs between here and the copy below.
            for (offset, s) in &frame.text_owned {
                let patched = Str::new(s.as_str());
                write_str_at(&mut frame.stack_bytes, *offset as usize, patched);
            }

            // --- Restore the generator's locals onto the live stack ---
            let bytes_len = frame.stack_bytes.len();
            let base = frame.stack_base as usize;
            self.stack[base .. base + bytes_len]
                .copy_from_slice(&frame.stack_bytes);
            self.stack_pos = frame.stack_base + bytes_len as u32;

            // --- Restore saved call frames ---
            self.call_stack.extend_from_slice(&frame.call_frames);

            // --- Mark running ---
            frame.status = CoroutineStatus::Running;
            self.active_coroutines.push(idx);

            // --- Jump into the generator ---
            self.code_pos = frame.code_pos;
            // Normal bytecode dispatch continues; the next OpYield or
            // OpCoroutineReturn will pop active_coroutines and return to
            // caller_return_pos.
        }
    }
}
```

### `OpYield` pseudocode

```rust
OpYield { value_size } => {
    let idx = *self.active_coroutines.last()
        .expect("OpYield outside active coroutine");
    let frame = self.coroutines[idx].as_mut().unwrap();

    let value_size = value_size as usize;
    let stack_top  = self.stack_pos as usize;
    let base       = frame.stack_base as usize;
    let value_start = stack_top - value_size;
    let locals_len  = value_start - base;

    // --- Serialise the full frame region including the yielded value (SC-CO-10) ---
    // Work on a copy so that text pointer patching does not alias the live stack.
    let mut full_buf: Vec<u8> = self.stack[base .. stack_top].to_vec();

    // serialise_text_slots processes ALL text slots in the region (locals + value),
    // writes new Str pointers (into the text_owned buffers) into full_buf, and
    // returns the (offset, owned-String) pairs (SC-CO-1, SC-CO-8, SC-CO-10).
    let text_owned = serialise_text_slots(
        &mut full_buf, &def.all_text_slots, &mut self.database);

    // Split the serialised buffer: locals go into frame.stack_bytes;
    // the updated value bytes are held separately for the slide step.
    frame.stack_bytes = full_buf[.. locals_len].to_vec();
    let updated_value = full_buf[locals_len .. locals_len + value_size].to_vec();
    frame.text_owned  = text_owned;

    // --- Save call frames above the base depth ---
    frame.call_frames = self.call_stack[frame.call_depth ..].to_vec();
    self.call_stack.truncate(frame.call_depth);

    // --- Suspend ---
    frame.code_pos = self.code_pos;           // instruction after OpYield
    frame.status   = CoroutineStatus::Suspended;
    self.active_coroutines.pop();

    // --- Slide the (pointer-updated) value to frame.stack_base ---
    self.stack[base .. base + value_size].copy_from_slice(&updated_value);
    self.stack_pos = frame.stack_base + value_size as u32;

    // --- Return to the consumer ---
    self.code_pos = frame.caller_return_pos;
}
```

### `OpCoroutineReturn` pseudocode

```rust
OpCoroutineReturn { value_size } => {
    let idx = *self.active_coroutines.last()
        .expect("OpCoroutineReturn outside active coroutine");
    let frame = self.coroutines[idx].as_mut().unwrap();

    // --- Drop all serialised state ---
    // Dropping text_owned frees the owned String allocations (Rust RAII).
    frame.text_owned.clear();
    frame.stack_bytes.clear();

    // --- Restore call stack to consumer depth ---
    self.call_stack.truncate(frame.call_depth);

    // --- Exhaust ---
    frame.status = CoroutineStatus::Exhausted;
    self.active_coroutines.pop();

    // --- Rewind stack to frame base; push typed null ---
    self.stack_pos = frame.stack_base;
    self.push_null(value_size);

    // --- Return to the consumer ---
    self.code_pos = frame.caller_return_pos;
}
```

### `OpExhausted` pseudocode

```rust
OpExhausted => {
    let db_ref: DbRef = self.pop_stack();
    let result = if db_ref.rec == 0 {
        true   // null iterator is considered exhausted
    } else {
        debug_assert_eq!(db_ref.store_nr, COROUTINE_STORE);
        let frame = self.coroutines[db_ref.rec as usize].as_ref()
            .expect("OpExhausted: invalid coroutine index");
        matches!(frame.status, CoroutineStatus::Exhausted)
    };
    self.put_stack(result as u8);
}
```

### `serialise_text_slots` — implementation contract

```rust
/// Process all text (Str) slots within `bytes`, which covers the stack region
/// [frame_start .. some_end] including any yielded value bytes.
///
/// For each non-null Str slot that points to a dynamic allocation:
///   1. Call `to_owned()` to make an independent String copy.
///   2. Free the original String via `database.free_dynamic_str(ptr)` so it
///      is not leaked (SC-CO-8).
///   3. Write a new Str pointing to the owned buffer back into `bytes`.
///   4. Record `(offset as u32, owned_string)` in the returned Vec (SC-CO-12).
///
/// Static Strs (pointers inside `text_code`) are left untouched; they need no
/// ownership transfer.
///
/// `type_slots`: an iterator or slice of (byte_offset_in_bytes, Type) pairs
/// describing every text slot in the region. This is computed from the
/// function definition's layout information for the current stack extent.
fn serialise_text_slots(
    bytes:    &mut Vec<u8>,
    type_slots: &[(usize, &Type)],
    database: &mut Stores,
) -> Vec<(u32, String)> {
    let mut owned = Vec::new();
    for &(offset, typ) in type_slots {
        if !matches!(typ, Type::Text) { continue; }
        let str_ref = read_str_at(bytes, offset);
        if str_ref.ptr == STRING_NULL.as_ptr() { continue; }   // null text
        if database.is_static_text(str_ref.ptr) { continue; }  // static pool
        // Dynamic: copy, free original, patch.
        let s = str_ref.str().to_owned();
        database.free_dynamic_str(str_ref.ptr);                 // SC-CO-8
        let new_str = Str::new(s.as_str());
        write_str_at(bytes, offset, new_str);
        owned.push((offset as u32, s));                         // SC-CO-12 u32
    }
    owned
}
```

At resume time (`OpCoroutineNext`), each `(offset, String)` pair is used to
patch the `Str` in `stack_bytes` to point to the current (stable) buffer
address of the owned `String`, before copying `stack_bytes` onto the live
stack. No extra allocation occurs on the resume path.

---

## Safety Concerns and Mitigations

### SC-CO-1 — Text ownership in the saved stack

**Problem:** `Str { ptr, len }` slots in `stack_bytes` may point to a dynamic
`String` that was freed when the stack was rewound after the yield. On resume,
those pointers dangle.

**Mitigation:** `serialise_text_slots` converts every dynamic text slot to an
owned `String` in `frame.text_owned`, and writes a `Str` pointing to the new
buffer into `stack_bytes`. On resume, the owned `String` addresses are patched
back into `stack_bytes` before the bytes are written to the live stack, keeping
all `Str` pointers valid.

---

### SC-CO-2 — DbRef locals may dangle if stores are freed mid-suspension

**Problem:** A generator may hold a `DbRef` local that refers to a heap record.
If the consumer frees or reallocates that record between iterations, the
suspended frame holds stale coordinates.

**Mitigation:** This is no different from any other loft local holding a
`DbRef`. The generator does not add a new risk. Document as a Known Limitation
(CL-2); the caller must not free records that a suspended generator still
references.

**P2-R5 — Text-specific variant: store-backed `Str` at yield**

When a generator reads a text field from a store record, the resulting `Str`
value is a zero-copy pointer directly into the store's raw allocation
(`{ ptr: store.ptr + rec*8 + 8, len }`).  If this `Str` is live in a local at
a `yield` point, `stack_bytes` encodes the raw pointer.

If the consumer:
1. Deletes the record (`database.free(r)` / `store.delete(rec)`), OR
2. Frees the entire store (`database.free(db_ref)`)

…between the yield and the next resume, and the store word is subsequently
reused for different data, the `Str.ptr` in `stack_bytes` points to unrelated
bytes — **silent data corruption** on resume.

**Invariant (CL-2b):** Any `Str` value derived from a store record field (via
`store.get_str()` or equivalent) must be treated as a borrow of the store's
memory.  If such a `Str` is live at a `yield` point, the caller must not delete
the backing record or free the store before the generator is exhausted or the
local is overwritten.

This is more dangerous than the `DbRef` case (SC-CO-2) because a `Str` looks
like a plain value, not an obvious reference.

**Long-term fix:** CO1.3d's `serialise_text_slots` (P2-R3) will deep-copy
store-derived `Str` values into owned `String` objects at yield time, eliminating
this class entirely.

---

### SC-CO-3 — Re-entrant advance corrupts the live stack

**Problem:** Advancing a `Running` generator overwrites live stack bytes with
saved bytes, corrupting the currently executing frame.

**Mitigation:** `OpCoroutineNext` checks `active_coroutines.contains(&idx)`
before resuming. If the index is already active, execution aborts:

```
runtime error: coroutine advanced re-entrantly; this generator is already running
```

The `contains` check is O(depth) but depth is bounded by nested `yield from`
chains, which are shallow in practice.

---

### SC-CO-4 — `yield` inside a `par(...)` body crosses thread boundaries

**Problem:** Each parallel worker has its own `State` and `coroutines` table.
A `COROUTINE_STORE` DbRef produced inside a worker cannot be advanced by
the enclosing scope's `State`.

**Mitigation:** The compiler must reject `yield` and generator function calls
inside `par(...)` bodies. `iterator<T>` values must not be assigned to
cross-thread shared variables. Until the compiler check is implemented, this
is a hard documented restriction:

> Generator functions and `yield` may not be used inside `par(...)` bodies.
> The `iterator<T>` DbRef must not cross thread boundaries.

---

### SC-CO-5 — Stack serialisation cost is O(depth) per yield

**Problem:** A deeply recursive generator copies a large `stack_bytes` and
`call_frames` on every yield and resume.

**Mitigation:** Document the cost model. Use iterative generators for
performance-critical paths; recursive `yield from` is for correctness-first
code. No optimisation is planned for the initial implementation.

---

### SC-CO-6 — Advancing an exhausted generator must always return null

**Problem:** If an exhausted frame were freed and its slot reused, a subsequent
`next()` on the old DbRef would access the wrong frame.

**Mitigation:** every handle carries a **generation stamp** in `DbRef::pos`, taken
from the frame it was made for, and each entry point (`coroutine_next`,
`coroutine_exhausted`, `free_coroutine`) compares it before touching the slot. A
handle whose frame is gone reads as exhausted and frees nothing, so the slot can
be recycled the moment its generator finishes — which is what makes the
scope-exit free of a handle safe (loft#835). `OpCoroutineNext` on an `Exhausted`
frame pushes null immediately without entering the body.

---

### SC-CO-7 — Absolute `stack_base` stale when caller pushes locals after creation

**Problem:** If `stack_base` were fixed at creation time (position `P`), any
local the caller pushes after creation would live at `P + …`. Resuming the
generator would then write `stack_bytes` starting at `P`, overwriting those
locals.

```loft
gen = count_up(0);  // suppose base captured at P
x   = 42;           // x at P+12 (after DbRef)
for n in gen { say("{n} {x}"); }  // would overwrite x!
```

**Mitigation:** `OpCoroutineNext` always sets `frame.stack_base = self.stack_pos`
before writing any bytes. The frame is placed at the current top of the caller's
stack, above all live caller locals. This is safe because no slot in
`stack_bytes` contains an absolute stack address; `Str` pointers reference
`text_code` or `text_owned` buffers, and `DbRef` values reference the store
heap — neither is affected by relocating the frame base.

---

### SC-CO-8 — Dynamic String objects leaked during yield serialisation

**Problem:** `str.str().to_owned()` creates a new `String` but does not free
the original. The original dynamic allocation has no remaining owner and leaks.

**Mitigation:** `serialise_text_slots` calls `database.free_dynamic_str(ptr)`
on the original allocation immediately after `to_owned()`. The exact API
mirrors `OpFreeText`; the implementation must align with how `text.rs` manages
the scratch/side-table of dynamic `String` objects.

---

### SC-CO-9 — Scalar `coroutine_sp` cannot represent `yield from` nesting

**Problem:** `yield from` makes two coroutines simultaneously active — the
outer (waiting in the `OpYieldFrom` loop) and the inner (executing). A single
`usize` cannot represent both, so the re-entrant check would miss the outer
generator while it is mid-`yield-from`.

**Mitigation:** `active_coroutines: Vec<usize>` holds the indices of all
currently active frames. `OpCoroutineNext` checks membership before resuming.
`OpYield` and `OpCoroutineReturn` pop the last entry.

---

### SC-CO-10 — Yielded `text` value not serialised; its String may be freed

**Problem:** If serialisation covers only the locals region (below the yielded
value), a `Str` in the yielded value still points to a dynamic `String` that
`SC-CO-8`'s mitigation then frees. The consumer receives a dangling pointer.

**Mitigation:** `OpYield` serialises the **entire** region from `stack_base`
to `stack_pos` (locals + yielded value) in one pass. After serialisation, the
yielded value bytes contain `Str` pointers that point into `text_owned` buffers
(not freed originals). Only the locals portion is stored in `frame.stack_bytes`;
the value portion is slid to `stack_base` as the `next()` return value.

---

### SC-CO-11 — `e#remove` inside a generator for-loop must be a compile-time error

**Problem:** Generators do not back a store record; any remove opcode emitted
against a generator iterator would operate on garbage coordinates, potentially
corrupting an unrelated record.

**Mitigation:** The compiler must detect `e#remove` on a generator-typed
iterator (identified at the `for` loop's type-resolution step by the element
type's source — a `COROUTINE_STORE` DbRef) and report:

```
error: `e#remove` is not valid on a generator iterator;
       generators do not back a store — use a collection if removal is needed.
```

---

### SC-CO-12 — `text_owned` offset stored as `u16` silently truncates

**Problem:** A `u16` offset caps at 65535 bytes. A deeply recursive generator
can exceed this (e.g. 3000 nested calls × ~22 bytes/CallFrame ≈ 66 KB).
Truncation silently patches the wrong `Str` slot on resume.

**Mitigation:** Use `u32` for the offset field (`Vec<(u32, String)>`), giving
4 GB headroom — sufficient for any realistic frame size.

---

### Summary of safety concerns

| ID | Concern | Severity | Resolution |
|---|---|---|---|
| SC-CO-1 | Dynamic text slots dangle after stack rewind | High | `text_owned` deep-copy; pointer patch on resume |
| SC-CO-2 | DbRef locals dangle if caller frees records mid-suspension | Medium | Documented (CL-2); caller responsibility |
| SC-CO-3 | Re-entrant advance overwrites live stack | High | `active_coroutines.contains()` check; runtime error |
| SC-CO-4 | `yield` inside `par(...)` crosses thread boundaries | High | Compiler error; documented hard restriction |
| SC-CO-5 | Serialisation cost O(depth) per yield | Low | Documented cost model; no optimisation planned |
| SC-CO-6 | Advancing exhausted generator after slot reuse | Medium | Exhausted frames kept alive; null pushed without frame entry |
| SC-CO-7 | Fixed `stack_base` overwritten by caller locals pushed after creation | High | Set `stack_base = stack_pos` at every resume |
| SC-CO-8 | Original dynamic String leaked after `to_owned()` | High | `database.free_dynamic_str(ptr)` in `serialise_text_slots` |
| SC-CO-9 | Scalar active-coroutine tracker fails for `yield from` nesting | Medium | `active_coroutines: Vec<usize>` replaces scalar |
| SC-CO-10 | Yielded `text` value's `Str` not serialised; freed by SC-CO-8 mitigation | High | Serialise entire `[stack_base..stack_pos]` region in one pass |
| SC-CO-11 | `e#remove` against generator emits store-remove opcode; corrupts unrelated records | Medium | Compile-time error at `for` loop type resolution |
| SC-CO-12 | `text_owned` `u16` offset truncates for frames > 65535 bytes | Low | `u32` offset field |

---

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

---

## Known Limitations

| ID | Limitation | Workaround |
|---|---|---|
| CL-1 | `next()` returns null for both exhaustion and a yielded null value — indistinguishable | Use the `for` loop (which tracks exhaustion separately), or wrap `T` in a struct with an `is_null: boolean` field |
| CL-2 | DbRef locals held across a yield dangle if the caller frees or reallocates the referenced record | Do not free records that a suspended generator still holds; advance the generator to exhaustion first |
| CL-2b | A `text` value derived from a store record field (store-backed `Str`) that is live at a `yield` point will dangle if the consumer frees or reuses the backing store record before the next resume | Do not delete the backing record or free the store while the generator is suspended with a store-derived text local; CO1.3d will deep-copy these at yield time once implemented (P2-R3) |
| CL-7 | A `text` value produced by `yield` is a zero-copy reference into the generator's frame (or into a `text_owned` buffer once CO1.3d lands); it is valid only for the current loop body iteration (or until the next `next()` call for explicit-advance code) | To keep the text beyond one iteration, copy it: `stored = "{value}"` or pass it to a function that calls `set_str` |
| CL-3 | ~~Exhausted frames are not freed until the `iterator<T>` DbRef goes out of scope; without GC, frames are leaked if the variable is abandoned~~ **FIXED (loft#835).** A generator handle is freed at the end of the scope holding it, which releases the frame and every heap local the generator still owned; an exhausted frame is freed where it exhausts, and a generation stamp keeps the later scope-exit free off the slot's next occupant. Abandoning a generator needs no care from the author, on the interpreter. On `--native` the handle's free is still a no-op, so an abandoned native generator's own locals are not yet reclaimed | none needed on the interpreter |
| CL-4 | Generator `iterator<T>` values must not cross `par(...)` boundaries | Accumulate parallel results in a collection, then iterate the collection outside `par(...)` |
| CL-5 | Serialisation cost per yield is O(frame depth); deeply recursive `yield from` chains are slow | Flatten recursive generators iteratively using an explicit `vector` stack local |
| CL-6 | Mutable-reference parameters (`&vector<T>`) in a generator function are not visible to the frame copy | Pass collections by value or use `reference<T>` and write through the reference |
| CL-8 | On `--native`, a generator yielding a tuple with a **text element** (`iterator<(text, integer)>`) does not yet compile — a yielded `text` is a `&str`, so riding the unified yield codec needs a store intern (`db_from_text`) with a lifetime question still open. Scalar and DbRef-ref tuple elements (`(integer, float)`, `(vector, integer)`, …) work on both backends. | Yield the text from a separate single-`text` generator, or wrap the pair in a record and yield its `reference<S>` |
| CL-9 | **Mostly fixed (loft#836 slice 1; loft#1586 `while` and statements after the yield).** A `for` or `while` loop with ONE `yield` on its body's straight line is lazy on `--native` too: one iteration per advance, the cursor persisted in the coroutine struct, and statements after the yield run at the start of the next advance (the loop is rotated). An infinite or early-`break`-consumed loop-generator therefore stops when its consumer does — `while true { …; yield x; }` included. These shapes still take the eager `ForLoopBody` buffer, so their side effects still interleave differently: more than one yield per iteration, a yield inside an `if`/`match`, a nested loop and a `continue` (axes A2–A4).  A closure in the loop body is lazy since loft#1587, which made a lambda's fn-ref and closure record persistent fields. ⚠ An eager loop runs to its end before the first value is handed out, so an ENDLESS loop of one of those shapes — `while true { if ready { yield x; } }` — never hands one out on `--native`: it fills its buffer until the process runs out of memory. `LOFT_TIMEOUT` does not stop it first. A yield of a tuple / fn-ref (the `next_into` channel) or of a struct / vector also stays eager; a struct / vector pushed into the eager buffer is a per-yield SNAPSHOT in a store the generator owns (released when it is exhausted or abandoned), so the consumer reads the value as it was at the yield rather than the record's final state, and the statements after the loop run once the buffer is filled (loft#1356). Values agree throughout. | Keep one `yield` on the loop body's straight line — not under an `if` — and yield a scalar or `text`; that shape is lazy on both backends, `for` and `while` alike. Otherwise use **straight-line** yields, or fully drain the generator. Remaining slices: **[Lazy loop yields (CL-9)](#design-lazy-loop-yields-cl-9)** below. |

### Native yield codec — status (@PLAN16 phase 02)

The native value channel is **layout-driven**: a yielded tuple is flattened into
transport slots derived from `T`'s slot kinds (`src/coroutine_layout.rs`) — each
scalar slot inline as one `i64`, each reference slot as its full `DbRef` across
two — and *both* the producer (`generation/coroutine.rs`) and the consumer
(`generation/ops/coroutine.rs`) derive the **same** walk from the **same** `T`, so
they agree by construction (no per-shape template, no runtime shape tag).
`tests/coroutine_matrix.rs` is 18/18 green on both backends.  Open tail:
text-*element* tuples (CL-8), the tuple-through-higher-order / comprehension cells
(`y4_x3` / `y4_x4`), and deleting the legacy `next_i64` / `next_text` /
`next_dbref` channels once every shape routes through the codec (pure subtraction).
Full record: the @PLAN16 closure doc at
[`plans/finished/16-coroutine-validation/README.md`](plans/finished/16-coroutine-validation/README.md).

---

## Design: lazy loop yields (CL-9)

> **Status: slice 1 built (loft#836, 2026-08-10); the `while` half of slice 3 and the statement
> after the yield built (loft#1586, 2026-09-22); slice 2, nested loops and slice 4 open.** A loop
> with ONE `yield` on its body's straight line is lowered to a header+body state pair — one
> iteration per advance, the cursor persisted — and everything else keeps the eager buffer.  A
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
> last yield, where its scope-exit frees live) once the buffer is filled; it used to drop it,
> which leaked every persistent heap local and lost a `print` after the loop (loft#1356;
> guard `tests/scripts/1356-a-record-yielded-from-a-loop-body-is-the-value-at-the-yield.loft`).
> The rest below is the original design, kept as the map for slices 2-4.

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

---

## Relationship to Rust's `gen` / `async gen` (upstream status)

Loft's coroutines do **not** depend on any unstable Rust feature.  The
native backend (`src/generation/coroutine.rs`) compiles each generator
function into a hand-written state machine: a Rust `enum` with one
variant per yield point plus `Exhausted`, a wrapping `struct` carrying
locals, and a `next()` method dispatching on the enum.  All on stable
Rust.

This is the same shape Rust's own `gen` blocks desugar to internally,
but done by loft's own codegen.  The trade-off is deliberate:

- **Pro:** ships on stable Rust *today*; no MSRV bump when the user's
  toolchain is behind; full control over frame layout, drop order, and
  error spans.
- **Con:** a small amount of state-machine code lives in
  `src/generation/` that rustc could eventually generate for us if we
  opted into `gen`.

### Upstream timeline (checked April 2026)

- **Sync `gen` blocks** (rust-lang/rust#117078): active, not stabilised.
  Edition-2024 keyword reservation done; `gen fn` + `FusedIterator`
  implementation in place; unresolved design questions remain.  No
  public stabilisation date.
- **`AsyncIterator` / `Stream`** (rust-lang/rust#79024): still nightly.
  API is being redesigned (PR #119550: rename back to `Stream`,
  introduce AFIT-based `AsyncIterator`).  WG-async explicitly states
  "no internal consensus on the right API".
- **Async generators** (rust-lang/wg-async#301): "In Progress" under
  a slipped "DRAFT: Async 2024" milestone.  WG-async's own language:
  "far enough in the future that many details may change"; "if the
  team did prioritize async generators, they would have to pick
  something else to deprioritize".  **Realistic earliest: 2027+.**

### Implication for loft

- No reason to wait for sync `gen`.  When it stabilises, revisit
  `src/generation/coroutine.rs` as a *maintenance* refactor — capability
  is unchanged.
- **Async gen is off the planning horizon.**  If loft adds async I/O
  (see [WEB_SERVER_LIB.md](lib_plans/future/08-server/README.md)), the implementation
  will hand-roll async state machines the same way sync coroutines do
  today — not wait for upstream.  The pattern is well understood.

---

## Non-Goals

- **Symmetric coroutines** — two coroutines transferring control directly to
  each other. The design is asymmetric: a generator always yields to its
  consumer.
- **`async`/`await`** — asynchronous I/O concurrency. Coroutines here are
  synchronous; the consumer blocks for each yielded value.
- **Cross-thread coroutine migration** — a suspended frame cannot be resumed
  on a different thread than the one that created it.
- **Mutable yielded values** — yielded values are copies; the consumer cannot
  mutate a value and have the mutation visible inside the generator.
- **Garbage collection of abandoned frames** — the design does not implement
  automatic frame cleanup when a generator goes out of scope without exhausting.

---

## See also

- [STACKTRACE.md](STACKTRACE.md) — `call_stack: Vec<CallFrame>` (Phase 1) is
  a prerequisite; `CallFrame` is shared between the two features
- [INTERMEDIATE.md](INTERMEDIATE.md) — `State` layout, `fn_call`/`fn_return`,
  stack frame conventions, `Str` vs `String`, `STRING_NULL` sentinel
- [THREADING.md](THREADING.md) — `par(...)` execution model; coroutines must
  not cross `par` boundaries (SC-CO-4)
- [SLOTS.md](SLOTS.md) — stack slot layout; understanding the two-zone design
  is important for `serialise_text_slots` and `stack_bytes` construction
- [LOFT.md](LOFT.md) — iterator protocol, for-loop attributes, existing
  `iterator<T>` type semantics
- [PLANNING.md](PLANNING.md) — enhancement backlog; coroutine priority
