<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The bytecode runner as a port of the Rust interpreter

The loft bytecode runner runs the Rust compiler's bytecode from a run image on a single
stack, built the way the Rust interpreter is built: an operator table GENERATED from
templates beside each operator's declaration, over a hand-written `State`.  This page says
what the port consists of, what is still to build, and where it differs from the Rust
interpreter.  Each difference is listed with what closing it would take, so it can be
closed or left as the need arises.

## Contents
- [How it is built](#how-it-is-built)
- [What a template is](#what-a-template-is)
- [The checks that hold it to Rust](#the-checks-that-hold-it-to-rust)
- [What is still to build](#what-is-still-to-build)
- [Differences from the Rust interpreter](#differences-from-the-rust-interpreter)
- [Speed](#speed)

## How it is built

| Rust interpreter | loft runner |
|---|---|
| `#rust"…"` template per operator, `default/*.loft` | `#loft"…"` template beside it (the Rust parser skips it) |
| `src/create.rs::generate_code_into` writes `src/fill.rs` | `selfhost/src/create.loft` writes `selfhost/src/fill.loft`; run by `selfhost/front/create.loft` |
| `src/state/mod.rs::State` (`code`, `get_stack`, `put_stack`, `get_var`, `put_var`) | `selfhost/src/state.loft`, the same names with the width in the name (`code_u16`, `get_stack_i64`) |
| `State` methods for operators with no template (`fn_call`, `var_text`, …) | methods of the same name in `state.loft` |
| `src/ops.rs` helpers (`fused_int`, `fused_cmp`) | `selfhost/src/ops.loft`, the same names |
| `Stores` (`src/store.rs`, `src/database/`) | the C runtime's store routines, through `selfhost/crt` (STORE_MODEL.md: the C model is the standard) |
| the dispatch loop | `selfhost/src/fill_run.loft::call` |

Regenerate the table after changing a template or porting a `State` method:

```bash
LOFT_NO_NATIVE_LIBS=1 loft --interpret --lib selfhost/src --lib selfhost/crt/src \
  selfhost/front/create.loft $PWD/default $PWD/selfhost/src/state.loft $PWD/selfhost/src/fill.loft
```

It prints `N of M operators carried`; `--missing` lists the rest, `--slots` the slot order,
`--reads` each operator's operand and stack reads.

## What a template is

A `#loft` template is plain loft.  The generated function binds each operand (a `const`
parameter, read from the code) and each stack argument (popped last first) to a local of
the parameter's own name, runs the template, and pushes a non-void answer:

```loft
fn OpAddInt(v1: __word, v2: __word) -> __word;
#rust"ops::op_add_int(@v1, @v2)"
#loft"v1 + v2"
```

- `s` is the `State`.  Braces are doubled, as in `#rust` (`{{ … }}`).
- An operator with NO `#rust` template is a `State` method that reads its own operands;
  the generator emits it once `state.loft` declares `pub fn <name>(self: State`.
- A stack boolean arrives as its byte (0, 1, 255 for null), as in `fill.rs`.
- The answer local is typed and nullable, so a template may answer `null`.

Two language features exist for the port:

- **`#c_twin "sym" "sig"`** on a function with a loft body names a C routine the C backend
  calls in its place (PACKAGES_C_BINDING.md).  The typed reads of the code bytes are twins.
- **`f.bits()` / `float_from_bits(v)`** and the `single` pair move a float's stored bits
  (STDLIB.md § Bits of a float).

## The checks that hold it to Rust

| Check | How |
|---|---|
| slot numbers equal Rust's | `create.loft … --slots` against `src/op_layout.rs`'s `OPERATOR_NAMES` |
| each templated operator reads what `fill.rs` reads (names, order, widths) | `create.loft … --reads` against the reads in `src/fill.rs` |
| a twin's body and its C routine agree | `tests/c_emit.rs::a_c_twin_runs_its_body_and_the_c_backend_its_routine` |
| a function answers what the Rust interpreter answers | `selfhost/bench/run_fill.loft <program image> <run image> <function> <args…>` against `loft --interpret` |
| no kernel function differs from the IR runner; the integer kernels' hand-computed answers | `selfhost/tests/fill.loft`, run by `tests/selfhost.rs` |
| which operator stops each kernel function (the worklist) | `selfhost/bench/fill_census.loft <program image> <run image>` |
| where the top is after an operator | `LOFT_LOG=full loft --interpret` prints each operator with the top before it; `LOFT_FILL_TRACE=1` prints the runner's |

The first two are run by hand; neither is a test yet.

## What is still to build

| Part | State |
|---|---|
| operator templates | arithmetic, comparisons, shifts, casts, locals of every scalar type, jumps, the fused integer operators, calls and frames, reference locals, and the field reads and writes of every width; the rest listed by `--missing` |
| `State` methods | calls (`fn_call`, `fn_return`, `reserve_frame`), texts (`string_from_code`, `var_text`, `arg_text`, `append_text`, `put_text`, `init_text`, `clear_text`, `free_text`), references and stores (`init_ref`, `init_ref_sentinel`, `database`, `free_ref`, `init_create_stack`); the rest (record and vector building, formatting, text walks, coroutines, `par`, keyed walks) listed by `--missing` as `(State)` |
| natives (`OpStaticCall`) | none; the hand-built runner `brun.loft` holds ports of about 110 natives over the same `crt` routines, to be moved under `State::static_call` |
| running a program's `main` | needs the natives and the text operators; today one integer function is called directly |
| records, vectors, keyed collections | no operator carried; each is a template calling the `crt` routine the C backend calls for it |
| a survey over the corpus | `selfhost/bench/run_image_survey.py` drives the hand-built runner; it needs a mode for this one |
| the two table checks as tests | see above |

The hand-built runner (`selfhost/src/brun.loft`) stays until this one runs the corpus; it
is the source of the `crt` calls each operator needs, not a design to follow.

## Differences from the Rust interpreter

What matches is the frame: the stack is a store of its own, values take 8-byte steps,
`get_var` reads at `stack_pos - pos`, `put_var` writes at `stack_pos + step - pos`,
`fn_call` pushes the return position, and a return leaves the answer at the arguments' base
with the top just above it.  The differences:

| # | Difference | What closing it takes |
|---|---|---|
| 1 | **Frame headroom is the default for every definition.**  Rust grows the stack at each call to the callee's own frame height plus a margin (`frame_headroom`); the runner grows to Rust's default, 65,536 bytes, which covers any frame (a frame position is a `u16`). | the frame height per definition in the run image, if the stack's size matters |
| 2 | **Faults stop the run with a text.**  Each operand read, pop and frame slot is checked as in Rust; where Rust raises a typed runtime error with a position, the runner sets `State.fault` and stops. | Rust's error kinds and positions, when a program's diagnostics are compared |
| 3 | **The store layer is the C runtime's.**  The buffer layout is the same; the allocator's choices differ where STORE_MODEL.md lists an open divergence, the store table is the C one, and Rust's store instruments are absent.  Store numbers can differ, which `store_memory()` shows. | nothing here: STORE_MODEL.md closes divergences on its own terms |
| 4 | **The stack's address.**  Rust's `stack_cur` is a reference with its own record and position; the runner's frame area is the stack store's primary record at offset 8. | a `stack_cur` in `State`, if a program can observe the numbers |
| 5 | **`fn_return` releases no fn-ref buffer.**  The frames are kept as Rust keeps them (definition, call position, arguments' base and size); the buffers a returning frame releases (`release_fnref_bufs`) are not, as no fn-ref operator is carried. | with the fn-ref operators |
| 6 | **Texts.**  A local text is a Rust `String` in its 24-byte slot in Rust, a C-runtime text of the same size here; the 16-byte view is the same.  A text popped as a template's argument is copied into a loft text. | nothing, unless a program observes capacity |
| 7 | **Arithmetic by loft's operators, reports as texts.**  A template writes `v1 + v2`, compiled to the C runtime's routine, where Rust calls `ops::op_add_int`; the kernels compare the values.  What Rust reports through `raise_recoverable` (a division by zero, a dropped write, an overflow) is recorded as a text in `State.reported`, and an overflow's report comes from the C runtime, not from the template. | the corpus survey, once `main` runs; Rust's error kinds when diagnostics are compared |
| 8 | **One dispatch.**  Rust has three tables (checked, direct, register) and inlines the `#hot` operators; the runner has one `execute`. | see Speed |
| 9 | **No instruments.**  Rust's stack verifier, poison of a reserved frame, and text-position tracking have no counterpart. | only if the runner is to host those sweeps |

## Speed

Not a goal yet.  Measured through `run_fill.loft`, the call only, against `loft --interpret`:
the runner takes several times as long on the integer kernels.  A profile shows why: every
`State` field access is a store lookup, the accessors and the `crt` wrappers are separate C
functions, and `execute` is a chain of comparisons.  The cures are in the C backend
(a struct parameter's fields held in locals across a loop, small functions inlined without
their stack-trace frame, an integer `match` as a `switch`) or in the generator (the code and
stack positions as locals of one dispatch function, as Rust's register table passes them).
