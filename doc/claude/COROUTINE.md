
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Coroutine Design

> **Status: completed in 0.8.3.** CO1.1–CO1.6 implemented, `yield from` (CO1.4) included —
> it ships, is guarded (loft#1277 and the two `a-delegation…` files) and is specified as
> `(G-Delegate)` in [formal/coroutines.md](formal/coroutines.md). This line said "deferred to
> 1.1+" until 2026-09-25, long after it landed.
> Open enhancement: **native lazy loop yields** ([Design: lazy loop yields (CL-9)](COROUTINE_LAZY_YIELDS.md))
> — slice 1 has landed (loft#836), and the `while` half of slice 3 with it (loft#1586): a `for` or
> `while` loop with one `yield` on its body's straight line is lazy on `--native`, statements
> after the yield included. Slices 2–4 (multiple / conditional yields, nested loops, text-in-loop
> interning) are what remains; each keeps the eager buffer.

Coroutines give loft programs generator functions: functions that can suspend
execution with `yield`, return a value to the caller, and resume from the same
point on the next iteration. The suspended function's entire stack — including
any nested calls active at the point of `yield` — is serialised to a
heap-allocated, extensible frame. This makes loft coroutines **stackful**, which is
what lets `yield from` delegate to a sub-generator with no trampoline. It does not
make `yield` legal in a helper that is not itself a generator: that is refused at
compile time, because a `yield` in a function whose return type is not `iterator<T>`
has no type to produce into (`formal/coroutines.md` `(G-Yield)` / `(G-YieldDepth)`;
this sentence claimed the opposite until it was measured on 2026-09-25).

---

## Contents

- [Goals](#goals)
- [Language Syntax](#language-syntax)
- [Exposed Types](#exposed-types)
- [Stack Layout and Execution Model](#stack-layout-and-execution-model)
- [Coroutine Frame Design](#coroutine-frame-design)
- [Runtime Design](#runtime-design)
- Safety Concerns and Mitigations — [COROUTINE_SAFETY.md](COROUTINE_SAFETY.md)
- Implementation Phases — [COROUTINE-history.md](COROUTINE-history.md)
- [Known Limitations](#known-limitations)
- [Non-Goals](#non-goals)
- [See also](#see-also)

---

## Goals

- Allow any function returning `iterator<T>` to use `yield` to produce values
  lazily, one at a time.
- Preserve the full call stack at the point of `yield` (stackful semantics),
  not only the generator's own locals, so `yield from` is implementable without a
  trampoline. ⚠ This goal also read "allows `yield` inside helper functions"; the
  surface never admitted that spelling and `(G-Yield)` refuses it — the realised
  form of the goal is delegation, where the deeper frame is a generator of its own.
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
(see [SC-CO-11](COROUTINE_SAFETY.md#sc-co-11--eremove-inside-a-generator-for-loop-must-be-a-compile-time-error)).

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

### Holding generators — a scheduler

A generator handle is a value like any other: a struct field, a vector element and a tuple
member can hold one, and whatever holds it owns the generator (`formal/coroutines.md`
(G-Hold)).  That is how a set of live behaviours is advanced once per tick:

```loft
fn patrol(id: integer, n: integer) -> iterator<integer> {
    for step in 0..n { yield id * 100 + step; }
}

fn main() {
    tasks: vector<iterator<integer>> = [];
    tasks += [patrol(1, 3), patrol(2, 1)];
    while len(tasks) > 0 {
        i = 0;
        while i < len(tasks) {
            x = next(tasks[i]);
            if x == null { tasks.remove(i); } else { println("{x}"); i += 1; }
        }
    }
}
```

The generator is released exactly once, by its holder: a local at its scope end or when it
is given another generator, a field or an element when it is given another one, and a record
or a collection when it dies — or, for an element, when it is removed, so removing a task that
has not finished (`tasks.remove(i)`, `t#remove`) ends it.  Putting a local into a container
(`tasks += [g]`, `Task { g: g }`) MOVES it there.  Reading one back out — `h = t.g`,
`h = tasks[i]`, `for t in tasks` — gives a view: advancing `h` advances the generator the
container holds, and the container keeps it.  A handle prints as `iterator`.  A keyed collection (`hash`, `sorted`,
`index`, `spatial`) releases the generators its records hold the same way, and runs no other
release for them (`formal/heap.md` (H-Drop-Not)).

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
every `OpCoroutineNext` (see [SC-CO-7](COROUTINE_SAFETY.md#sc-co-7--absolute-stack_base-stale-when-caller-pushes-locals-after-creation)).

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
   any dynamic text slots (see [SC-CO-1](COROUTINE_SAFETY.md#sc-co-1--text-ownership-in-the-saved-stack),
   [SC-CO-8](COROUTINE_SAFETY.md#sc-co-8--dynamic-string-objects-leaked-during-yield-serialisation)),
   and sets `frame.code_pos` to the function's entry bytecode position.
3. `frame.stack_base` is set to `0` — it has no meaning until the first resume.
4. The argument bytes are removed from the live stack.
5. The frame's index (as a `COROUTINE_STORE` DbRef) is pushed as the result.
6. **The function body is not entered.**

### First advance and resume (`Created` / `Suspended` → `Running`)

`OpCoroutineNext` on a `Created` or `Suspended` frame (both cases are identical):

1. Check `active_coroutines` for a re-entrant advance (see
   [SC-CO-3](COROUTINE_SAFETY.md#sc-co-3--re-entrant-advance-corrupts-the-live-stack),
   [SC-CO-9](COROUTINE_SAFETY.md#sc-co-9--scalar-coroutine_sp-cannot-represent-yield-from-nesting)).
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
   itself (see [SC-CO-10](COROUTINE_SAFETY.md#sc-co-10--yielded-text-value-not-serialised-its-string-may-be-freed)).
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

**Safety concerns and mitigations (SC-CO-1 … SC-CO-12)** — see [COROUTINE_SAFETY.md](COROUTINE_SAFETY.md).

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
| CL-9 | **Mostly fixed (loft#836 slice 1; loft#1586 `while` and statements after the yield).** A `for` or `while` loop with ONE `yield` on its body's straight line is lazy on `--native` too: one iteration per advance, the cursor persisted in the coroutine struct, and statements after the yield run at the start of the next advance (the loop is rotated). An infinite or early-`break`-consumed loop-generator therefore stops when its consumer does — `while true { …; yield x; }` included. So is a yield that ENDS an `if`/`match` arm, one per arm (axis A3, loft#1798): an iteration that takes a non-yielding arm runs on to the next within the same advance.  These shapes still take the eager `ForLoopBody` buffer, so their side effects still interleave differently: two yields on one path, a statement after a yield inside its arm, a nested loop and a `continue` (axes A2–A4).  A closure in the loop body is lazy since loft#1587, which made a lambda's fn-ref and closure record persistent fields. ⚠ An eager loop runs to its end before the first value is handed out, so an ENDLESS loop of one of those shapes — `while true { if ready { yield x; log(x); } }` — never hands one out on `--native`: it fills its buffer until the process runs out of memory. `LOFT_TIMEOUT` does not stop it first. A yield of a tuple / fn-ref (the `next_into` channel) or of a struct / vector also stays eager; a struct / vector pushed into the eager buffer is a per-yield SNAPSHOT in a store the generator owns (released when it is exhausted or abandoned), so the consumer reads the value as it was at the yield rather than the record's final state, and the statements after the loop run once the buffer is filled (loft#1356). Values agree throughout. | End every path through the loop body with at most one `yield` — on the straight line, or as the last statement of an `if`/`match` arm — and yield a scalar or `text`; that shape is lazy on both backends, `for` and `while` alike. Otherwise use **straight-line** yields, or fully drain the generator. Remaining slices: **[Lazy loop yields (CL-9)](COROUTINE_LAZY_YIELDS.md)** below. |

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

**Design: lazy loop yields (CL-9)** — see [COROUTINE_LAZY_YIELDS.md](COROUTINE_LAZY_YIELDS.md).

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
