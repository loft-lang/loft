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
| the dispatch loop | generated: `fill.loft`'s `run`; `selfhost/src/fill_run.loft` drives it (`call` for one function, `run_main` for a program) and calls the natives |
| the library functions `static_call` reaches | the hand-built runner's native ports (`brun::call_native`), over the same stack store |

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
- A stack boolean or enum arrives as its byte (0, 1, 255 for null), as in `fill.rs`; a
  character as its code point; a reference as a `state::DbRef`.
- A text answer is a view the template pushes itself (`s.put_field_text(v1, fld);`): a loft
  text value is not a view into a store or the code.  The generated function is named
  `op_<operator>`, so none takes a standard-library name.
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
| which operator stops each kernel function | `selfhost/bench/fill_census.loft <program image> <run image>` |
| the corpus: each plain program of `tests/scripts` against `loft --interpret` (`stores`: equal but for the `store_memory()` report, difference 3), and the operator that stops each one not run to its end (the worklist) | `python3 selfhost/bench/fill_survey.py` |
| where the top is after an operator | `LOFT_LOG=full loft --interpret` prints each operator with the top before it; `LOFT_FILL_TRACE=1` prints the runner's |

The first two are run by hand; neither is a test yet.

## What is still to build

| Part | State |
|---|---|
| operator templates and `State` methods | `create.loft … --missing` lists what is not carried, `(State)` marking a method; the survey orders them by the programs each stops.  Not started: the `OpParallel*` operators (a `par` block of arms; a `par` loop runs), the slot-reference family (`OpGetSlotRef`, …), the store-buffer family (`OpStoreBuf`, …) |
| natives (`OpStaticCall`) | called from the hand-built runner's ports (`brun::call_native`); to move into a module of their own |
| `par` | a queue's or a fold's workers run one after another by the hand-built runner's scheduler (`brun::par_start` / `par_return`), entered from `fill_run.loft`; `State.par_depths` hands the run back at each worker's return |
| compiled library code | not built: every library function runs as bytecode (difference 9).  To build: the standard library's and each `use`d library's compiled bodies linked into the runner and called at `OpStaticCall`, as Rust calls its bridges |
| a survey mode in the test suite | the survey is a tool; the tests are `selfhost/tests/fill.loft` over the kernels |
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
| 5 | **Texts.**  A local text is a Rust `String` in its 24-byte slot in Rust, a C-runtime text of the same size here; the 16-byte view is the same.  A text popped as a template's argument is copied into a loft text. | nothing, unless a program observes capacity |
| 6 | **Arithmetic by loft's operators, reports as texts.**  A template writes `v1 + v2`, compiled to the C runtime's routine, where Rust calls `ops::op_add_int`; the kernels compare the values.  What Rust reports through `raise_recoverable` (a division by zero, a dropped write, an overflow) is recorded as a text in `State.reported`, and an overflow's report comes from the C runtime, not from the template. | the corpus survey, once `main` runs; Rust's error kinds when diagnostics are compared |
| 7 | **One dispatch.**  Rust has three tables (checked, direct, register); the runner has one loop, `run`, whose arms are the register table and whose fallback, `execute`, is the checked one. | see Speed |
| 8 | **No instruments.**  Rust's stack verifier, poison of a reserved frame, and text-position tracking have no counterpart. | only if the runner is to host those sweeps |
| 9 | **The standard library is interpreted.**  Rust calls a standard-library function it compiled into its binary through a bridge (`loft_shared_<def>`, `src/compiled_stdlib.rs`), and interprets the loft body only where the program's type table does not start as the compiled one does.  The runner always runs the loft body as bytecode (`State::static_call`), which is Rust under `LOFT_NO_COMPILED_STDLIB=1`: the same answers, slower. | compiled standard-library bodies linked into the runner, if its speed on library-heavy programs matters |

## Speed

The dispatch loop is generated (`fill.loft`'s `run`) by the Rust interpreter's executor rules
(`formal/rewrites.md`):

| rule | in the runner |
|---|---|
| `(R-RegisterTable)` | the code position, the stack top and the stack's capacity are locals of `run`; an operator whose template needs no more of the `State` is an arm of it, any other templated operator with scalar operands an arm that writes the positions to the `State`, runs, and reads them back |
| `(R-CodeBase)` | operands are read through the code's address (`crt::code_base`, `crt::cget_*`), taken again after an operator that ran on the `State` |
| `(R-OperandSpan)` | one test, `ip <= safe`, covers an arm's whole operand span |
| `(R-StackBase)` | a slot is read through the stack store's base (`crt::stack_base`, `crt::fget_*` / `fput_*`) |
| `(R-DispatchStop)` | `State.stopped`, set with `fault` by `State::stop` |
| `(R-FrameHeadroom)` | `fn_call` grows the stack by the default headroom; an arm tests one compare against the capacity |

Every check the accessors make is a compare in the arm's guard; an arm whose guard fails
leaves the step to the operator's function, which reports it.  An operator with a text or
reference argument runs through `execute`.

Measure with `selfhost/bench/fill_speed.sh` (the generated runner against `loft --interpret`
on the integer kernels); `selfhost/bench/hand/` holds the step loop written by hand in C, the
target.  The gate is the generated runner at the
Rust interpreter's time on the integer kernels; it takes 1.1 to 1.7 times as long.  What is
left is in the emitted C: checked arithmetic on the positions, and calls and returns through
the `State`'s fields (the frame list).
