<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 strand 1b — the frame: the Rust interpreter's memory structures

A child document of [1b-interpreter.md](1b-interpreter.md).  Status: **PF1 built (the layout
guard, `selfhost/tests/frame.loft`); F-1a built — every integer, float and reference variable
in the stack store at its Rust byte; F-1b (the texts) not built.**

## The rule

Goal constraint 5: a reimplementation, not a new language.  The frame is not a separate
decision; it is that rule applied to memory (*"keep the same memory structures"*, owner,
2026-10-30): the loft interpreter keeps every value where the Rust interpreter keeps it.

## What the Rust interpreter's memory is (measured)

- **The value stack is a store.**  `State::stack_cur = db.database_defined(1000)`
  (`src/state/mod.rs:960`): one store of the run, holding every frame and every evaluation
  temporary.  It is an ordinary store, so a reference into it is an ordinary `DbRef`.
- **A frame** starts at the caller's top of stack at the call: the arguments, each at its
  STEPPED argument size, then a 4-byte return-address slot, then the locals from `local_start`
  (`src/scopes.rs:979`, `src/state/codegen.rs:201–209`).  `LOFT_ALIGN` steps every size to keep
  the base 8-aligned; it is identity when off.
- **Every local's position is decided before the image is saved; an argument's is not.**  The
  bytecode writer places the arguments when it writes the function (`codegen.rs`,
  `set_stack_pos`): each at the sum of the stepped argument sizes before it, in variable order,
  so the image carries `stack_pos` 65535 for every argument and `frame.loft` ports that layout
  (PF1 measured it).  `assign_slots_v2`
  (`src/variables/slots_v2.rs:112`), run inside `scopes::check` (`src/scopes.rs:1002`), writes
  `Variable.stack_pos`, which the image carries (`ir.loft` `Variable.stack_pos`).  Slots are
  REUSED: small locals by greedy colouring of their live intervals (`compute_intervals`), large
  ones and loop locals in IR-walk order, a loop variable on its range index's slot
  (`@FR-R-LoopSlot`).  The intervals themselves are not stored; `loft introspect` prints them.
- **The sizes** (`variables/mod.rs:6084`, `Context::Variable`): boolean and plain enum 1,
  single and character 4, integer and float 8, a reference (record, vector, struct enum,
  keyed) 12, a text local 24 (a Rust `String`); as an ARGUMENT a text is a borrowed `&str`, 16.
  The C runtime's `loft_text` (`char *p; size_t len, cap`) is 24 bytes and its `loft_str` 16, so
  the C values fit the same slots.
- **`&` of a local** (`OpCreateStack(v)`) is a reference into the stack store at `v`'s slot;
  the ordinary store operators then read and write the local through it.  The positional
  variant `OpInitCreateStack` writes such a reference into a slot.
- **The reserve does not initialise** (`OpReserveFrame` moves the top only): a slot holds what
  an earlier call left until its first assignment's positional init writes it
  ([SLOTS.md](../../SLOTS.md)).

## The design: one stack store, frames at the Rust positions

**The invariant: every value the Rust interpreter keeps in its stack store, the loft interpreter
keeps in ONE C-runtime store of its own, at the same byte position of the same frame, with the
same width.**  So a slot is reused exactly when the Rust interpreter reuses it, and a reference
to a local is a reference into that store.

- **The stack store**: minted at the run's start by `crt::database` (the C runtime's
  `loft_database`), grown by the runtime as the Rust store grows.  Its record is the stack; the
  frame base is a byte offset into it.
- **The frame**: on a call, the base is the current top; the arguments are written at their
  positions (stepped sizes), the return-address slot is written with the interpreter's own
  return (its op index, 4 bytes), and the top moves to `base + frame_hwm`
  (`OpReserveFrame`).  On return the top goes back to the base.  Nothing is initialised that the
  Rust interpreter does not initialise: first assignments are the bytecode writer's positional
  inits (`OpInitRef`, `OpInitRefSentinel`, `OpInitText`, `OpInitCreateStack`), lowered as
  1b-s3.md § Binds and frees lowers them.
- **Access**: a local's `Load`/`Store` reads or writes `base + stack_pos` through the runtime's
  own field routines at the local's width (`loft_get_int`, `loft_get_int4`, `loft_get_float`,
  `loft_get_single`, a byte for a boolean or plain enum, the three parts of a reference), all
  already in `crt`.  The lowering takes `stack_pos` from the image and lays out nothing itself:
  `var_slot` and the per-kind frame parts go.
- **Texts**: a text local is a `loft_text` at its slot, made, appended, cleared and freed by
  the C runtime's text routines through the shim (marshalling only: the slot's address and the
  operand); a text read for a loft-side operation (a format, a compare) is copied out.  A text
  argument is a `loft_str` view at its slot.
- **`&` of a local**: `OpCreateStack(v)` is the reference `(stack store, stack record, base +
  stack_pos(v))`, and every store operator works through it unchanged — which is what the 57
  census programs refused with *"a reference to a variable that is not a text"* need.  The
  interpreter's own `&text` (`RefOf`, a text index) goes: a `&text` is the same reference.
- **The evaluation stack** is the same store too: the Rust interpreter keeps its temporaries
  there, at the positions its bytecode writer's stack model tracks (`Stack`, `stack.position`
  in `codegen.rs`).  So the lowering becomes the port of the bytecode writer outright: it
  emits the Rust interpreter's operators with their operands and positions, and the run loop
  is the port of their implementations.  F-1 moves the frames; F-2 the temporaries, with the
  ops that push and pop them.

## What it settles

- **The slot and lifetime comparison (1b-s3.md § The comparison)**: every local is addressed by
  the image's `stack_pos`, so the slot table is equal by construction; what B-0 then checks is
  that every access addresses `stack_pos`, and the store events.
- **Stale-slot defects reproduce** exactly as `--interpret` shows them (the step-G crash was a
  free of a stale reused slot): the loft interpreter becomes a differential tester of the slot
  allocator and the positional inits, not a fresh-slot machine that hides them.

## Cost (measured, and to measure)

A local's read and write through the store, against a loft vector index, compiled as the census
runner is (`--native-c --ship`, 20 M rounds of two reads and a write): **82 ms against 32 ms**
(4.1 ns against 1.6 ns per access).  The share of executed ops that are local accesses decides
what that does to the whole interpreter; it is measured on the S1 kernels (`fib`, `collatz`,
`mix`, `selfhost/bench/measure.py`) before and after F-1, and reported — speed is reported,
never a gate (the order of the goals, 1b-interpreter.md).

## Probes before building (each can fail on its own)

- **PF1** — the layout is the Rust one: for every function of every kernel, each variable's
  slot and width as the interpreter addresses them equal `loft introspect`'s `slot [a, b)`.
  **Built**: `selfhost/src/frame.loft` (`width`, `position`, `slot`) against the table the
  harness writes beside each kernel (`<kernel>.slots`); every row equal.  It needed the
  arguments' layout ported (above) and the table fixed for a text argument (loft#2309).
- **PF2** — slot reuse: two locals with disjoint intervals sharing a slot (introspect shows
  it), the second's first assignment written by its positional init, values by hand.
- **PF3** — `&` of a record, a vector and an integer local handed to a `&` parameter, written
  through it, read back by the caller: values by hand; three of the 57 census programs.
- **PF4** — a text local's lifecycle in the frame: built, appended, cleared in a loop, freed;
  the C heap is not grown per round (the run's peak measured at two loop counts).
- **PF5** — the frame base under recursion: `fib` at depth 25 answers by hand, and the stack
  store's size returns to its start after the entry returns.

## Steps

- **F-1** — the frames in the stack store: locals at `stack_pos`, `&` of a local a reference
  into it.  Built in two parts:
  - **F-1a (built)**: the integer, float and reference variables.  `crt::stack_new` mints the
    stack store (pinned, its record spanning the store), `stack_grow` is `State::grow_stack`,
    and a reference slot is twelve raw bytes (`slot_ref` / `set_slot_ref`: no record marked,
    no tombstone read through, as the Rust frame holds one).  A narrow local is read and
    written at its width (`var_access`).  A call moves its arguments off the evaluation
    stacks into their bytes of the callee's frame, last first; the locals still start null,
    which the Rust does not do (PF2 removes it).  It found one defect the per-kind frame hid:
    the lowering's `mark` left `stack_allocated` / `skip_free` set on the image, so lowering a
    function again — the next run's — read the last walk's marks and took a first bind for a
    rebind, freeing a reused slot's other value; `lower` now puts the image's flags back.
    Guarded by `kernels/frames.loft` (a recursion 3000 deep that grows the store).  The S1
    kernels, loft interpreter through C (`--native-c --ship`), best of three, before → after:
    fib 2.24 s → 2.38 s, mix 4.83 s → 4.58 s, collatz 16.06 s → 15.32 s — within noise of each
    other; the per-access cost measured above does not show at the kernels' share of local
    accesses.
  - **F-1b**: the texts — a `loft_text` at a local's slot, a `loft_str` view at an argument's,
    `&text` a reference into the stack store, `OpFreeText` a release.
- **F-2** — the temporaries in the same store: the lowering emits the bytecode writer's
  operators at its stack positions, so its output is the Rust bytecode.

**The gate of both: the bytecode.**  `loft introspect` prints the Rust bytecode of every
function; the loft lowering's output, written in the same form, must equal it byte for byte
(F-2), and the frame positions must equal it from F-1 on.  Both come after 1b-s3.md's B-0 (the
comparisons), so the comparisons measure each change.
