<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 177 — Growable call stack: a call that would not fit gets a new stack block

## Status

Open — no implementation.  Tracker: [@PLN177](https://github.com/loft-lang/plans/issues/177).
Until it lands, the recursion cap is a caveat: [CAVEATS.md § Recursion depth](../CAVEATS.md#recursion-depth-is-capped-and-the-cap-halts-the-run).

**The owner has a design for this that predates the agent-driven phase.**  It is the
authority where it and this file disagree; attach it here (§ The owner's design) before
phase 1 starts.  What follows is the ground it will be built on, measured on the tree.

## Goal

A call whose frame does not fit in the stack space left runs in a new stack block that
holds it, so no stack size limits a program.  A production program has no depth limit at all;
a development or test build keeps a CALL-DEPTH limit that halts and reports the recursion
fully to the developer (owner ruling 2026-09-28 — the same allowance `panic` and `assert`
have).

## Why

loft has no runtime exceptions: nothing a running program does may stop it, except `panic`
and `assert`, and those only in tests and development builds
([DESIGN_DECISIONS.md C80](../DESIGN_DECISIONS_FAILURE.md#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation),
owner ruling 2026-09-28).  Every calculation fault already degrades to null and logs.  The
call-depth cap is the one runtime halt left:

| backend | today | where |
|---|---|---|
| `--interpret` | halts at `State::MAX_CALL_DEPTH` (10 000 frames) in development; production logs and returns from `fn_call` without the call's result | `State::fn_call`, `src/state/mod.rs` |
| `--native` | halts at the same count, at callee entry, through `report_and_exit` | `cr_call_push` / `cr_stack_overflow`, `src/codegen_runtime.rs` |
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
- **Native frames are sized by rustc, not by loft.**  Generated code recurses on the OS
  stack of the `main` thread, which runs on a 512 MiB virtual stack (`NATIVE_MAIN_STACK`) so
  that 10 000 frames fit.  `frame_hwm` does not bound a native frame.
- **Only a function on a call-graph cycle can recurse.**  A function whose whole call tree
  is acyclic has a static depth bound; the generator already computes leaves and frameless
  chains (`is_elidable_leaf`, `is_frameless_chain` in `src/generation/mod.rs`), so the entry
  check can be confined to functions in a recursive strongly-connected component.
- **Other stacks exist:** coroutine frames (COROUTINE.md), `par` worker threads
  (THREADING.md), placed-library workers (PLACEMENT.md).  Each needs the same answer.

## The owner's design

*(to be attached)*

## Phases

Each phase compares against the build before it and can go red on its own.

| # | Phase | Validated by | E |
|---|---|---|---|
| 0 | **Probe** — measure, per backend, the worst frame of every function in the corpus and the deep-recursion scripts: interpreter `frame_hwm` + eval depth; native stack pointer delta per call (a probe build that records `psm`-style stack addresses at entry).  Answers: how big a block, how big a red zone, and whether a per-function native bound is derivable or needs a fixed red zone | a table in this file; the probe falsifies "a fixed red zone covers every native frame" if any frame exceeds it | XS |
| 1 | **Interpreter: blocks, no cap** — at `fn_call`, if the space left in the current block is below the callee's `frame_hwm` + eval bound, continue in a new block; return steps back.  `MAX_CALL_DEPTH` leaves the interpreter | `rec(9_999)` answers identically before and after; `rec(1_000_000)` answers where it halted; every existing test unchanged; `LOFT_POISON` / `LOFT_VERIFY_STACK` sweeps green | M |
| 2 | **Native: blocks for recursive functions only** — an entry check in functions on a call-graph cycle: when the OS stack left is below the bound phase 0 derived, the rest of the call runs on a fresh segment.  Non-recursive functions emit byte-identical code | emitted Rust byte-identical for every non-recursive function (the codegen skill's refactor gate); `rec(1_000_000)` answers the interpreter's value; `make perf-portal` rows unchanged within noise | M–H |
| 3 | **wasm** — the host stack is not the module's to grow; decide between stack switching, asyncify, or a documented cap that degrades per C80 | a decision recorded here and in WASM.md, with the chosen spelling measured under `wasmtime` and a browser | M |
| 4 | **Depth limit is development-only, and reports fully** — `MAX_CALL_DEPTH` becomes a development/test limit (never reached in production, where the stack only grows); its report names the recursing cycle, the entry point and the repeating arguments, identically on both backends; the caveat and CONTROL.md item 11 are rewritten to say so | a production-mode run of `rec(1_000_000)` answers; a development run halts with the full report, pinned by a both-backend test; no production path can raise the kind | S–M |

## Open questions

1. **Settled (owner, 2026-09-28):** development and tests bound recursion by a call-depth
   limit with a full report; production does not bound it.  Still open: whether the
   development limit stays 10 000 or becomes settable, and what "full report" includes
   beyond the cycle, the entry point and the repeating arguments.
2. **Native segment switch mechanism** — a stack-switching crate (`stacker`/`psm`, a new
   dependency) or a hand-written switch per target; the dependency policy decides.
3. **Block size** — fixed, or sized to the callee (the interpreter knows the frame exactly).
4. **Coroutines and workers** — do they carry blocks of their own, or share the policy?

## Cross-arc dependencies

- @PLN157's lean tier keeps the depth count as the only per-call cost; phase 2 must not
  add a check to non-recursive functions or the lean rows move.
- Placement (PLACEMENT.md): a relayed fault from a worker is how a worker's overflow
  reaches the caller today.

## See also

- [CAVEATS.md § Recursion depth](../CAVEATS.md#recursion-depth-is-capped-and-the-cap-halts-the-run) — the caveat this retires.
- [CONTROL.md](../CONTROL.md) item 11 — `MAX_CALL_DEPTH` in the control census.
- [WASM.md § How deep a program can recurse](../WASM.md).
