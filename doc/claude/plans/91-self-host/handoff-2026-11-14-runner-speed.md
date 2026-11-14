<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 — handoff, 2026-11-14: the bytecode runner's speed — port the executor rules

From the `laptop-c-perf` stream (session loft-4f) to the self-host stream building the bytecode
runner (`tuxedo-pln204-int32`).  Read it before the next change to `selfhost/src/fill_run.loft`,
`state.loft` or the generator `create.loft`.  Measured on `tuxedo-pln204-int32` at `f6b842737`
merged into `laptop-c-perf`; the step loop is the same shape at `ce6cb61fd`.

## What was measured

Collatz (`selfhost/kernels/collatz.loft`, `n_longest 1 100000`), same answer `77031351` on all:

| runner | time |
|---|--:|
| the step loop written by hand in C (`selfhost/bench/hand/hand_steps.c`) | 0.29 s |
| `loft --interpret` | 0.335 s |
| the generated runner, shipped (`run_fill.loft`, `--native-c --ship`) | 3.71 s |
| the generated runner, built as the hand loop is | 4.13 s |

`selfhost/bench/hand/build.sh` reproduces all four lines.  The hand loop runs the SAME bytecode
from the SAME run image on the SAME stack store; it holds `State`'s code position, stack top,
stack capacity, code base and stack base in locals, switches on the operator, runs the 13
operators collatz uses inline, and hands every check's slow path (stack growth, a slot outside
the stack, division by zero) and every other operator to the generated loop for that one step.
So the bytecode and the stack image cost nothing; the generated runner's SHAPE costs 14×.

## Where the generated runner pays (callgrind, `perf`)

About 1 000 instructions per interpreted step, against about 14 cycles per step on the Rust
interpreter.  Per call: `code_has` 70 instructions, `code_u8` 63, `get_stack_i64` 58, `slot_ok`
52, `can_pop` 44; the per-step `s.fault == ""` (`loft_get_text` + compare) 8 %.

1. Every `State` field read and write (`code_pos`, `stack_pos`, `stack_cap`, `stk`) is a store
   lookup with its null and bounds tests.
2. Every operand read and stack access is its own function with its own bounds check, and the
   check's fault message is built in a work text inside the same function: the work text's
   set-up sits on the hot path and the function is too large for gcc to inline.  Moving the
   message out of `code_has`, `can_pop` and `slot_ok` by hand took the run from 4.24 s to
   3.21 s (−24 %); forcing them inline without that did nothing.
3. The loop compares the fault TEXT with `""` after every operator.

## What to change — the Rust interpreter's executor rules, ported

The Rust interpreter is fast because of executor rules written into its source by hand, each
in `doc/claude/formal/rewrites.md`.  The runner is a port of that interpreter; it should port
them too.  This is what the hand loop is, and none of it needs a new compiler rewrite.

| rule | Rust site | the loft port |
|---|---|---|
| `(R-RegisterTable)` — position and stack top passed in registers | `fill::OPERATORS_REG`, `State::regs_in` / `regs_out` | the generator emits ONE dispatch function whose locals are the code position, the stack top and the stack capacity; each operator's template is written against those locals (an arm of the dispatch, or a function taking and answering them).  `State` is written from the locals before anything that reads it (`fn_call`, `fn_return`, a native, a slow path) and read back after. |
| `(R-CodeBase)` — bytecode through a cached base and length | `State::code_base`, `code_len` | the code vector held in a local of the dispatch function for the run; an operand is read from it at the local position |
| `(R-OperandSpan)` — fixed-width operands bounds-checked once | the operator's operand span | one test `pos + span <= len` per operator, not one per operand |
| `(R-StackBase)` — a slot as one add to a cached base | `State::stack_base` | the stack store lives in the C runtime shim's OWN store table (`selfhost/crt/shim/crt.c`; `loft_stores` is a per-file static), so its base has to come from the shim: `build.sh` adds `uint8_t *loft_crt_stack_base(int64_t store)` to its copy of `crt.c`.  The port needs that accessor in the shim and a way for loft code to read and write a slot through it — or the shim's `stack_get` / `stack_put` inlined by the ship build's link-time optimisation, re-derived only where `ensure_stack` grows the store. |
| `(R-DispatchStop)` — one flag per step | `Stores::dispatch_stop` | a boolean `stopped` in `State`, set wherever `fault` is set (one setter), tested by the loop instead of `fault == ""` |
| `(R-FrameHeadroom)` — one capacity test per frame entry | `State::push_frame`, `State::frame_headroom` | the stack grown once at a frame's entry to the frame's highest stack position plus a margin, so a push inside the frame tests no capacity — the hand loop's `sp + 8 <= cap` goes |

The checks stay: each test the accessors make is kept as a compare, and its failure goes to
the existing slow path, which builds the message and sets `fault` (and `stopped`).

## What this stream does

- **`(R-Cold)` for loft functions in the C writer.**  `(R-Cold)` outlines a runtime helper's
  rare half today; a loft function's early exit that builds a text is not outlined, so
  `(R-Inline)` (at most twelve C statements) never admits an accessor with a fault message.
  The clause will outline it; it is general, and the accessors `State` keeps for the slow
  paths and the other callers benefit.  No change to `state.loft` is needed for it — keep the
  accessors as they read.
- **A `match` over integers as a C `switch`.**  No rule writes one; gcc may already make the
  `if` chain a jump table (`execute` is 9 % of the run).  Measured before any rule.
- The hand loop and `build.sh` are on `laptop-c-perf` (`ec79bc83a`); `1b-fill-port.md`
  § Speed there carries the target table.  Merge it, or take the two files and the section.

## The gate

`selfhost/bench/hand/build.sh`: the generated runner at the hand loop's time on collatz, and so
under `loft --interpret`'s.  Then `selfhost/bench/slowest.py` over the corpus: run ratios near
1×, where an outlier is a defect in one operator, not the design.
