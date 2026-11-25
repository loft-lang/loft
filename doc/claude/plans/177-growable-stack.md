<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 177 — Growable call stack: a call that would not fit gets a new stack block

## Status

Open.  Tracker: [@PLN177](https://github.com/loft-lang/plans/issues/177).  Until it lands, the
recursion limit is a caveat: [CAVEATS.md § Recursion depth](../CAVEATS.md#recursion-depth-is-capped-and-the-cap-halts-the-run).

**Done (2026-11-25), ahead of the phases:** a `--ship` C unit defines no `LOFT_MAX_CALL_DEPTH`
(`c::unit_defines`), so a shipped program counts no calls and recurses until the machine's stack
is full; the development builds keep the 10 000-call limit, and its report carries a `help:`
line naming the two forms that need no frame (`runtime_error::STACK_OVERFLOW_HELP`, the same
line on both backends).  A compiled program that crashes is named by `loft` instead of ending
with a silent exit 1 (`native_utils::report_crash`, `platform::crash_of`).  Guards:
`tests/native.rs` `a_shipped_program_recurses_past_the_development_cap_and_names_its_crash`,
`tests/runtime_errors.rs` `i1058_the_call_stack_budget_is_the_same_on_both_backends`.

**The owner's design** (§ below) is the authority where it and this file disagree.  What
follows it is the ground it is built on, measured on the tree.

## Goal

A call whose frame does not fit in the stack space left runs in a new stack block that
holds it, so no stack size limits a program.  A production program has no depth limit at all;
a development or test build keeps a CALL-DEPTH limit that halts and reports the recursion
fully to the developer (owner ruling 2026-09-28 — the same allowance `panic` and `assert`
have).

## Why

loft has no runtime exceptions: nothing a running program does may stop it, except `panic`
and `assert`, and those only in tests and development builds
([DESIGN_DECISIONS.md @C80](../DESIGN_DECISIONS_FAILURE.md#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation),
owner ruling 2026-09-28).  Every calculation fault already degrades to null and logs.  The
call-depth cap is the one runtime halt left:

| backend | today | where |
|---|---|---|
| `--interpret` | halts at `State::MAX_CALL_DEPTH` (10 000 frames) in development; production logs and returns from `fn_call` without the call's result | `State::fn_call`, `src/state/mod.rs` |
| `--native`, `--native-debug` | halts at the same count, at callee entry | `loft_frame_push` / `loft_stack_overflow`, `src/c/loft_rt.h` |
| `--ship` | no count: runs until the machine's stack is full, then crashes, which `loft` names | `c::unit_defines` writes no cap |
| wasm | the host engine's stack; running out is a trap (4 000 frames answer, 8 000 trap under `wasmtime`) | [WASM.md § How deep a program can recurse](../WASM.md) |

Answering null at the cap was considered and not taken: it makes a deep recursion answer
wrong where more stack would answer right.

**The two build kinds differ, and only in this** (owner ruling 2026-09-28):

- **production** — the stack grows in blocks; nothing limits depth.
- **development / tests** — the stack grows in blocks too (the STACK is never the limit), and
  a separate call-depth limit halts the run with a FULL report: not only "N frames", but what
  the developer needs to act — the cycle of functions that recursed, where it was entered, and
  the argument values that repeated.  Today's report names the running function and the frame
  chain; phase 4 makes it the full one.

**Why the split is the right one (owner, 2026-09-28).**  Games are developed in development
mode, so a runaway recursion is caught there, with the full report, before it ships.  The
production build of a game that needs a deeper stack gets it.  A small device may then run
out of memory on a very deep recursion; that failure mode is ACCEPTED — it is the device's
limit, not a check loft adds.

## What is already known

- **Interpreter frame size is exact and compile-time.**  Codegen computes each function's
  frame high-water mark and emits one `OpReserveFrame(frame_hwm)` at entry
  (`src/state/codegen.rs`, the Plan-04 B.3 bundle).  The evaluation stack above it is bounded
  by the deepest expression, also known at codegen.
- **The interpreter's value stack already grows.**  It is a store whose buffer grows by
  reallocation (`State::ensure_stack` → `Store::grow_words`); the cap is a FRAME COUNT
  (`call_stack.len()`), not a byte limit.  Growth by reallocation copies the whole stack;
  chained blocks would not.
- **Native frames are sized by the C compiler, not by loft.**  Generated C recurses on the OS
  stack of the thread it runs on (the `main` thread's, whatever size the host gives it; a `par`
  worker's `LOFT_PAR_STACK`).  `frame_hwm` does not bound a C frame, and a frame holds the
  function's locals, so the call count says little about the bytes.  Measured 2026-11-25 on
  macOS arm64 (8 MB main stack, wasmtime defaults), main @ 7c6972f86:

  | program | `--interpret` | `--native` | `--ship` | `--ship --native-wasm` |
  |---|---|---|---|---|
  | one-integer frame, 9 990 deep | answers | answers | answers | trap |
  | 48 text locals, 9 990 deep | answers | SIGSEGV | SIGSEGV, between 3 000 and 5 000 deep | trap, between 1 000 and 1 500 deep |
  | small frames in 4 `par` workers, ~9 980 deep | answers | — | answers | — |

  So the development limit protects only programs whose frames fit, and on wasm not even the
  smallest frame reaches it.
- **Only a call on a cycle of the EMITTED calls can recurse without bound.**  A rewrite
  changes which calls are cycles: `@FR-R-TailSelf` turns a self-call in tail position into a
  loop on both backends (`count(1 000 000, 0)` runs in one frame), while a mutual tail
  recursion stays a call (`even`/`odd` 20 000 deep reaches the limit).  So the cycles are read
  off the calls the C writer emits, after every rewrite, at the one place a call is written —
  never off the source call graph, which a later rewrite would silently disagree with.  The
  call graph's components are already computed for the store records (`src/store_records.rs`,
  callees first).
- **The C backend already keeps a frame off the machine stack: a generator's.**  A function
  returning `iterator<T>` becomes a frame struct holding every local and temporary, and a
  resume function that jumps to the label after the last `yield` (C_BACKEND.md
  § Generators).  A call on a cycle can take the same shape.
- **Other stacks exist:** coroutine frames (COROUTINE.md), `par` worker threads
  (THREADING.md), placed-library workers (PLACEMENT.md).  Each needs the same answer.

## The owner's design

Rulings of 2026-11-25 (owner):

- **Nothing is precalculated or claimed up front.**  A stack sized for the worst case spends
  memory a run may never need, so a run that could have finished runs out of memory, and that
  limits the devices a game runs on.  The frames grow on use.  So no large thread stack for
  `main`, and no byte budget derived from a fixed size.
- **The same code on every OS, and it works on wasm.**  No signal handler, no per-OS stack
  switching (`sigaltstack`, fibers, `makecontext`), no per-target assembly.
- **A shipped build has no limit**: it runs as long as the hardware allows, and crashes only
  then.  **A development build keeps the limit**: a runaway recursion stops there instead of
  hanging the developer's machine, and the developer fixes the design — so the report says
  what to write instead.

The design that follows from them: a call on a cycle of the emitted calls runs with its frame on
a stack the program holds in its own memory, in blocks that are added as the recursion deepens
and released as it returns; a tail call to another member of the cycle replaces the frame
instead of adding one.  Every other call stays an ordinary C call, whose depth the acyclic call
graph bounds.  Because the deep part is in the program's memory, wasm needs nothing of its own.

## Phases

Each phase compares against the build before it and can go red on its own.

| # | Phase | Validated by | E |
|---|---|---|---|
| 0 | **Probe** — measure, per backend, the worst frame of every function in the corpus and the deep-recursion scripts: interpreter `frame_hwm` + eval depth; native stack pointer delta per call (a probe build that records `psm`-style stack addresses at entry).  Answers: how big a block, how big a red zone, and whether a per-function native bound is derivable or needs a fixed red zone | a table in this file; the probe falsifies "a fixed red zone covers every native frame" if any frame exceeds it | XS |
| 1 | **Interpreter: blocks instead of one growing buffer** — at `fn_call`, if the space left in the current block is below the callee's `frame_hwm` + eval bound, continue in a new block; return steps back, so a deep recursion no longer copies the whole stack as it grows.  The interpreter is a development run, so `MAX_CALL_DEPTH` stays (owner, 2026-11-25) | `rec(9_999)` answers identically before and after; every existing test unchanged; `LOFT_POISON` / `LOFT_VERIFY_STACK` sweeps green | M |
| 2 | **C: frames of cycle calls in blocks** — the C writer marks each call on a cycle of the calls it emits; such a call pushes a frame struct (the generator's shape) on a block stack in the program's memory and continues at the callee's entry, a return pops it and resumes at the caller's label, and a tail call within the cycle replaces the frame.  Calls through a function value or closure, and a C library's callback into loft, follow a conservative rule written down first.  A fault releases the frames it jumps over.  Every other function emits byte-identical C | emitted C byte-identical for every function on no cycle (the codegen skill's refactor gate); the table above answers in every cell on `--native` and `--ship`; `even`/`odd` 1 000 000 deep answers; `make perf-portal` call-heavy rows within noise, or the cost stated | M–H |
| 3 | **wasm** — phase 2's frames live in the module's memory, so the engine's stack holds only the acyclic depth: measure the table's cells under `wasmtime` and a browser | the wasm column answers in every cell | S |
| 4 | **The development limit reports fully** — the shipped build already counts nothing (done 2026-11-25); the development report names the recursing cycle, the entry point and the repeating arguments, identically on both backends | a development run halts with the full report, pinned by a both-backend test | S–M |

## Open questions

1. **Settled (owner, 2026-09-28):** development and tests bound recursion by a call-depth
   limit with a full report; production does not bound it.  Still open: whether the
   development limit stays 10 000 or becomes settable, and what "full report" includes
   beyond the cycle, the entry point and the repeating arguments.
2. **Settled (owner, 2026-11-25):** no stack switching — not a crate (`stacker`/`psm`), not a
   switch per target: a call on a cycle keeps its frame in the program's memory (§ The owner's
   design).
3. **Block size** — fixed, or sized to the callee (the interpreter knows the frame exactly).
4. **Coroutines and workers** — do they carry blocks of their own, or share the policy?

## Cross-arc dependencies

- A `--ship` build has no per-call cost for the depth (it counts nothing); phase 2 must add
  none to a function on no cycle, or the shipped perf rows move.
- Placement (PLACEMENT.md): a relayed fault from a worker is how a worker's overflow
  reaches the caller today.

## See also

- [CAVEATS.md § Recursion depth](../CAVEATS.md#recursion-depth-is-capped-and-the-cap-halts-the-run) — the caveat this retires.
- [CONTROL.md](../CONTROL.md) item 11 — `MAX_CALL_DEPTH` in the control census.
- [WASM.md § How deep a program can recurse](../WASM.md).
