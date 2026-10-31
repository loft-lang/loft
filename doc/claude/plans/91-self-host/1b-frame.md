<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 strand 1b — the frame: the Rust interpreter's memory structures

A child document of [1b-interpreter.md](1b-interpreter.md).  Status: **F-1 built — every
variable in the stack store at its Rust byte (PF1 its layout guard, `selfhost/tests/frame.loft`);
PF2 built (no frame is initialised; each first assignment's positional init writes its slot);
PF3 built (a `&` of any local is a reference into the frame, read and written through at any
place); PF4 measured (loft#2310); PF5 built (the stack given back); F-T1 and F-T2 built (tuple
locals, text members too); F-2 not built.**

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
  - **F-1b (built)**: the texts.  A text local is the runtime's `loft_text` at its slot,
    set, appended, cleared and released there (`crt::slot_text*`, `OpFreeText` a release now,
    not a clear); a text argument is a `loft_str` view of a copy the call makes and its return
    releases — F-2 makes it a view of the caller's own value, once the temporaries live in the
    store; a `&text` is the absolute byte of the text it names.  The text part of the frame is
    gone: the loft-side text stack holds the temporaries only.
  - **PF4 (measured)**: a text loop's peak memory grows with its round count — before F-1 too
    (15 → 84 MB from 20 000 to 200 000 rounds; F-1b 14 → 68 MB).  The cause is not the frame:
    the interpreter's text stack is written by `put_text`, whose release `@FR-H-TextReplace`
    declines because its parameter is also appended on the other branch (loft#2310), so every
    text the run evaluates stays in that vector's store.
  - **PF2 (built)**: no frame is initialised, as `OpReserveFrame` initialises nothing — a slot
    holds what an earlier frame left until its first assignment's positional init writes it.
    Two inits the port lacked and the null-started frame hid: `gen_set_first_text`'s
    `OpInitText` (a text local's first set wrote over an earlier frame's `loft_text`), and
    `gen_keyed_null`'s `OpInitRef` before its first `OpDatabase` (the mint remade whatever store
    the slot's old bytes named — `kernels/hashes.loft` `intkey` crashed).  The 521 images
    measure the same with the frame uninitialised; `enums.loft` `listed` reuses its slots.
  - **PF3 (built)**: a `&` of a local that is not a text is the reference to its slot of the
    stack store (`OpCreateStack` → `StackRef`, `(stack store, 1, 8 + base + slot)`), a `&`
    variable (`KVarRef`) is read and written THROUGH that reference at the width of what it
    names (`Deref`, `Through`), and a `&` handed to a `&` parameter is the reference it holds.
    The reference may name any place — a frame slot, a field, an element — so `Deref` and
    `Through` address its `(store, rec, pos)`, as `OpGetInt` … `OpGetStackRef` do.
    - A set that BINDS a link (`@FR-B-Ref-Repoint`, `scopes::link_set_repoints`: `p = &q`, or
      a place for a link to a scalar) stores the reference in the link's own slot; a `&text`
      local binds the same way.  Every other set writes through it.
    - A fresh owned record, vector or collection written through a link displaces the store
      the place held: kept by value before the write, released after it unless the place still
      names it (`amp_owned_writeback`).
    - Not carried, refused: a `&` to a narrow integer place, and a narrow local a `&` names
      (`linked_narrow`), which the Rust interpreter reads through the width's own operator and
      minimum (`NarrowSlot::of_type`); a `&text` to a text field or element (`store_text_link`).
    - Guarded by `kernels/refs.loft`: an integer, a boolean, a float, a vector and a record
      written by the callee, and `linked`: a field, an element, a local link bound, written
      through and handed on, and a whole record written through a link with no store left.
    - Found on the way: the shim measured every text crossing the `#c` bridge with `strlen`, so
      a text holding a NUL byte was cut there (`102-escape-sequences`).  A text now crosses
      with its byte length beside it, -1 for null (`crt::bytes_of`).
    - Measured: the census passes 222 programs (199 before PF3) with none wrong, crashed or
      leaking; on the 521 images every program that ran before PF3 writes the same output,
      bind trace and allocator trace.
  - **PF5 (built)**: a run reports what the entry's return left on the stack — its frame base
    and the three evaluation stacks' depths (`Outcome.stack_left`) — and every kernel test
    asserts it is 0 (`ran_clean`); `fib(25)`, 242785 calls, answers 75025 and leaves 0.  The
    Rust interpreter never shrinks the stack store (`State::grow_stack` only grows it), so the
    store's SIZE does not return to its start, and the loft one does not either: what returns
    is the stack's top.  Falsified: a `Pop` that leaves its integer fails four kernel tests on
    this check alone, every value still right.
  - Found while probing: a self-recursive tail call the compiler turns into a loop
    (`R-TailSelf`, `NdLoop` in a value position) is refused by the lowering as an expression
    it does not carry.
- **F-T1 — a tuple local in the frame (built; 52 census programs refused `TyTuple`).**  A
  tuple lives in its slot as the Rust frame holds it: member `i` at the running sum of the
  stepped member widths before it (`data::element_stack_offsets_in`, `frame::member_at`), a
  nested tuple's members flattened in place.
  - **Its shape**: the list of leaves `(offset, kind, access)` a tuple type flattens to, built
    once per type by the lowering (`Lowering.shapes`).  A leaf is a scalar the frame already
    carries: integer, float, single, boolean, character, plain enum, each `?` too.  A `text`,
    reference, collection or `fn` member is refused by name in this step: a tuple local OWNS
    its text members (`TUPLE_LOCAL_TEXT_OWNED`) and its references take part in ownership,
    which are each a step of their own.
  - **A tuple value** is its leaves on their kinds' evaluation stacks, first member deepest: a
    literal (`NdTuple`) pushes its items in order; a whole-tuple read is `LoadTuple(slot,
    shape)`, a whole-tuple set `StoreTuple(slot, shape)`, popping the last leaf first.  A
    `null` tuple is null leaves (`@FR-T-Absent`: an absent tuple is a tuple of null members).
  - **A member**: `t.i` (`NdTupleGet`) is a `Load` at `slot + offset` with the member's kind
    and access; `t.i = v` (`NdTuplePut`) a `Store` there.  A nested tuple member is a
    `LoadTuple` at its offset.
  - **Calls**: a tuple argument's leaves move off the stacks into the callee's slot, last leaf
    first, as every other argument moves; a tuple result is carried past the frame restore by
    `Ret` reading the function's result shape (`Lowering.result_shape`).
  - **Failure paths**: a member kind outside the leaf set refuses the function with its type;
    a shape whose leaves would not fit its slot (`frame::width`) is a lowering defect, asserted.
  - **Tested by**: `kernels/tuples.loft` — a literal, member reads and writes, a nested tuple,
    a tuple argument, a tuple result destructured, a null tuple, each kind of leaf — values by
    hand; and the census before and after (no program that passed may change).
  - **Built**: `lower.loft` (`Leaf`, `Shape`, `shape_of`, `add_leaves`, `member_at`,
    `tuple_value`, `discard`; the `NdTuple`, `NdTupleGet`, `NdTuplePut` arms), `run.loft`
    (`LoadTuple`, `StoreTuple`, `PopTuple`; `Ret` and the call's argument move), `frame.loft`
    (`member_step`).  The census passes 245 programs (222 before), none wrong, crashed or
    leaking; on the 521 images every program that ran before is unchanged.  What still refuses
    a tuple is a member this step does not carry: a text (22 programs), a struct enum or
    reference (10), a vector (8).
  - **Design → code.**  Transcribed directly, but for two places.  A literal reaches `expr`
    with no tuple type handed down, as a block's tail (an inlined call's `{ (n, n * 2) }`), so
    `expr` carries one too, each item at its own kind; only a bare `null` item needs the member
    type `tuple_value` has.  And `tuple_value`'s `match` was refused by the parser (`Expect
    token ;`): its last arm calls `expr`, declared further down, which the first pass typed
    `Unknown` and took for a value — loft#2312, fixed in the parser, not around.
- **F-T2 — a tuple's text members (built; 22 census programs refused a `text` member).**
  A text leaf is held as a text variable is: in a tuple LOCAL its own `loft_text` (24 bytes,
  `TUPLE_LOCAL_TEXT_OWNED`), in a tuple ARGUMENT a view of a copy the call made (16 bytes),
  released at `Ret` with the other text arguments.  A tuple result holding a text is a record
  (`__tuple<…>`), not a frame tuple, so `Ret` carries no text leaf.
  - **First set**: the frame is not initialised (PF2), so the first set of a tuple local inits
    each text member (`InitText` at its byte) before the store writes it; a later set replaces
    the member's text as a text local's set does.
  - **Members as destinations**: the text-writing operators take a member of a local tuple as
    their destination (`OpFreeText(t.0)` frees it at its byte, as the scope's exit writes it);
    an argument tuple's member is a view, so writing one is refused by name.
  - **Tested by**: `kernels/tuples.loft`'s `named` — a text member built, appended to, copied
    with the whole tuple, handed to a tuple parameter, freed — values by hand, no store left.
  - **Built**: `add_leaves` takes a text leaf; `member_text` and `init_tuple_texts` in
    `lower.loft`; the text leaves in `run.loft`'s tuple ops, the argument copy and its release at
    `Ret`.  The census passes 254 programs (245 before), none wrong, crashed or leaking, and all
    254 are equal to the Rust interpreter in every layer the comparison has (out, alloc, leak,
    binds, slots).
  - **Design → code.**  Transcribed directly.  Measuring it with `leaks` found the interpreter's
    C build leaking 3 to 45 buffers a run, none of them tuple texts: the C backend's epilogue
    missed text temporaries declared after a `return` in a loop (loft#2313, fixed in the
    backend).  It leaks none now.
- **F-2** — the temporaries in the same store: the lowering emits the bytecode writer's
  operators at its stack positions, so its output is the Rust bytecode.

**The gate of both: the bytecode.**  `loft introspect` prints the Rust bytecode of every
function; the loft lowering's output, written in the same form, must equal it byte for byte
(F-2), and the frame positions must equal it from F-1 on.  Both come after 1b-s3.md's B-0 (the
comparisons), so the comparisons measure each change.
