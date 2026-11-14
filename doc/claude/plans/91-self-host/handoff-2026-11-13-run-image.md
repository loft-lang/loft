<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 — handoff, 2026-11-13: the interpreter runs the stored bytecode

From the `laptop-c-perf` stream (session loft-4f) to the self-host stream.  It changes the
interpreter half of strand 1b: read it before the next `run.loft` or `lower.loft` change.

## The decision (owner, 2026-11-13)

**The loft interpreter runs the Rust bytecode, stored in the image, and never touches it on
loading.**  `--emit-image` writes the bytecode the Rust writer (`compile::byte_code`) produced;
the loft runner maps the image and executes those bytes in place with the Rust stack image —
one stack store, the Rust frame and temporary positions, the same operators.  No lowering on the
run path, and no IR for it: **the run image is about 2 % of today's IR image.**

Owner's words: "it should use the same byte-code and stack image like the rust version does.
This is why we are so much slower." — "We should just store the bytecode and never touch it on
loading." — "the IR is much bigger than the byte-code and we frankly do not need it for the
stdlib functions." — "It should be about 2% of the size."

This replaces 1b-frame.md's **F-2** ("the lowering emits the bytecode writer's operators"): the
interpreter no longer waits for the port of the bytecode writer.  `lower.loft` stays the port of
`state/codegen.rs` for the self-hosted COMPILER, gated byte for byte against the stored bytecode
(1b-compare.md's `bytecode` row) — a separate gate from running programs.

## Why: the measurement

`selfhost/bench/slowest.py` (commit `ebbfbaa61` on `laptop-c-perf`, see § Tools) runs every
plain program of the census corpus on `loft --interpret` and on the C-built loft runner, and
times compile, lowering and run apart.  First survey, merged tree at `13dd20c73` + the harness:

- 931 of 1302 programs answer alike; the loft run is a **median 3.9× the Rust run** (the 52
  programs whose Rust run takes ≥ 2 ms), the worst 20–420×.
- Step counts are close (t3 below: ~13.8 M loft steps against ~9.9 M Rust, the Rust count from
  `LOFT_PROFILE` samples); each loft step costs ~50 ns against ~2 ns.

The top five, profiled with callgrind on the C-built runner (`perf` call graphs are refused on
that laptop):

| program | loft / Rust run | where the time goes |
|---|---|---|
| R-TailSelf/a-parameter-passed-through-is-the-pointer-the-call-binds | 5.94 s / 14 ms (421×) | text reads: 50 % `loft_utf8_valid`, 20 % memmove |
| R-TailSelf/a-self-call-in-tail-position-runs-to-any-depth | 252 / 5.2 ms (48×) | the per-step `Op` copy, the per-call `Lowering` copy |
| tests/scripts/a-tail-recursive-small-record-function-is-a-loop | 700 / 16.5 ms (42×) | the per-step `Op` copy: 68.5 % of all instructions |
| N-Decl/a-comprehension-takes-its-declared-element-type | 426 / 10.4 ms (41×) | the `Op` copy (72 %), the `Lowering` copy |
| tests/scripts/a-rebind-from-a-fresh-result-exchanges-the-stores | 2.97 s / 77 ms (38×) | the `Op` copy: 46 M of them, 67 % |

Each cause is a property of the current design, not of a step:

1. **The per-step `Op` copy** — `op = prog.ops[pc]!` (run.loft:355) compiles to `loft_bind_copy`
   of the 24-byte `Op` into a FRESH STORE (store made, claimed, freed) every step.
2. **The per-call `Lowering` copy** — `prog = callee_prog` (run.loft:554) deep-copies the
   callee's whole lowering (344 bytes, 40 fields, its vectors) on every call that is not a tail
   jump; the return path (`prog = progs[def]!`, line 631) is a view.
3. **Text reads copy the text** — a read of a text variable (`KText` push, run.loft:378, through
   `crt::slot_text`) copies the text out of its slot twice and validates it; a 100 000-step walk
   over a 100 000-byte text is quadratic.
4. **The per-run table** — `run_on` (run.loft:256) builds `lower::empty()` for every definition
   in the image (869, nearly all stdlib), copies each into `progs` and frees the table at the
   end: ~1.2 ms, 86 % of a trivial program's run.

All four disappear by construction with the stored bytecode: an op is bytes read in place, a
call is a jump to a code position, a text on the stack store is a view, and there is no
per-function table to build.  **Do not fix them on the current design** — that work is redone.

## The run image: what it holds (measured)

The bytecode of the program plus the stdlib, `LOFT_TRACE_BYTECODE`'s `fn` lines summed:

| | trivial program | tests/scripts/37-stress.loft |
|---|--:|--:|
| bytecode, all 314 generated functions | 10 KB | 28.7 KB |
| today's IR image (`--emit-image`) | 3.08 MB | 3.08 MB |

The stdlib's functions are mostly native: in bytecode a native is a 16-bit index for
`OpStaticCall`, while the IR image carries every stdlib definition whole.  The run image:

| part | 37-stress |
|---|--:|
| the bytecode, one stream (a call operand is an absolute code position) | 28.7 KB |
| the type schema (today's `.dschema` sidecar) | 11.7 KB |
| the function table: name (errors, stack traces), code position, frame size, the entry's arguments | ~9 KB |
| the native index → name table, the natives the bytecode calls only | a few KB |
| the constant stores (`OpConstRef`) | small |
| **total** | **~55–60 KB ≈ 2 %** |

What lies outside the bytes, and how it binds without editing them:

- **Natives**: `OpStaticCall` carries an index into the Rust registry (`State::library_names`,
  `src/state/mod.rs:574`).  The image stores index → name; the runner binds each name to its own
  function once at load — a table, never a rewrite of the bytecode.
- **Constants**: `consts::build` builds them at start today; the image is a store, so they can be
  stored as stores.
- **Text constants** are inline in the bytes (`State::string_from_code`); **types** are schema
  numbers, already in the image.

The IR stays what the self-hosted compiler reads, for the program it compiles — never the stdlib's.

## Steps (each can fail on its own)

1. **The writer** — `--emit-image` (or a sibling flag) writes the run image: the bytecode
   `compile::byte_code` leaves in `State::bytecode`, the function table, the native table, the
   schema, the constants.  **Gate**: the run image of `tests/scripts/37-stress.loft` is at most
   2 % of the IR image (~62 KB) — over that, IR leaked in; and its bytecode hashes equal
   `LOFT_TRACE_BYTECODE`'s `fn` lines.
2. **The loader** — the runner maps the image; nothing is decoded, copied or rebuilt.  **Gate**:
   load of the trivial program ≪ today's ~1.2 ms.
3. **The loop** — `run.loft` becomes a loop over the Rust operators (`LOFT_TRACE_BYTECODE`'s `op`
   lines list ~360, their numbering, class and operands), on one stack store at the Rust
   positions; most operators are calls into the C runtime the crt shim already reaches.
   Port the operators the census corpus uses first; refuse the rest by name, as today.
4. **The gate** — `slowest.py`: the run ratio near 1× is the target, and an outlier is a defect
   in one operator, not a design gap.  The census agreement count is rebuilt on the new loop.

## Tools

On `laptop-c-perf` at `ebbfbaa61` (pushed together with the merge of `tuxedo-pln204-int32`):

- `selfhost/bench/slowest.py` — `LOFT_SUITE_DIR=<suite> python3 selfhost/bench/slowest.py
  [--chapter <c>] [--top 25]`; rows in `target/selfhost-slowest.tsv`.  The Rust side reads
  `LOFT_TIMING` (front_end, codegen) and the `LOFT_TIMING_LEDGER` `run` phase.
- `selfhost/bench/run_timed.loft` — the runner the survey builds (`--native-c --ship`): prints
  `timed load= setup= lower= run=` (µs) on stderr.
- `run::Outcome.lowered_us` — the lowering time inside `run_on`, so a run is the program's own
  steps.  It goes with the lowering when the loop runs stored bytecode.
- Profiling the runner: `valgrind --tool=callgrind` on the C-built binary, then
  `callgrind_annotate --tree=caller` for a function's callers; `--native-c --native-emit out.c`
  shows the C a loft line became.

## Progress, 2026-11-14 (tuxedo-pln204-int32)

Steps 1–3 are built: `loft --emit-run-image` (step 1), `selfhost/src/run_image.loft` (step 2) and
`selfhost/src/brun.loft`, the loop over the stored bytecode on one stack store at the Rust
positions (step 3).  The stored bytecode is the interpreter's byte for byte
(`selfhost/bench/run_image_bytecode.py`: every plain corpus program identical).

What the runner carries is measured by `selfhost/bench/run_image_survey.py`, which builds the
runner, runs each program from its run image and compares it with `loft --interpret`: the
output, and the number of stores left at exit (the runner's own C-runtime leak check against
the interpreter's report).  On 2026-11-14, `tests/scripts`: 370 of 443 programs agree on both,
one differs (`store_iter_scratch`: only `store_memory()`'s description of the runtime itself —
the stack store's size, one store fewer before the program's, each store's creating bytecode
position), none crash; the rest are refused by name.  The suite's coroutines chapter: 10 of 11.

The image gained, beyond step 1's plan: generator frame facts per generator (`RunGen`, read
from one home, `src/state/gen_facts.rs`, which the interpreter's coroutine operators now share),
and per native the definition a compiled-stdlib native (`loft_shared_<def>`) was compiled from
(`RunImage.shared`), whose loft body the runner runs in its place.

Open:

- **`par` (`n_parallel_queue`, 7 programs) needs a decision**: the runner can run the workers
  in order (the same answers — a worker shares no mutable state) or on threads, which needs a
  runner state and isolated stores per worker, as Rust's `WorkerStores`.
- Store persistence (`n_store_persist_copy`, `n_store_load`, `n_store_verify`,
  `n_store_load_key`: 11 programs), `n_file_map`, linked-group adds (`G` in the ranking),
  `n_stack_trace`.
- loft#2481 (a function's answer that is an element of its parameter, unwrapped with `!`,
  leaks a store per call, both backends, on main) — the runner's `types.loft` works around it.
- The Rust compiler's bytecode for two stdlib functions (`t_4File_lines`,
  `t_4text_char_slice`) differs between runs of the same binary on the same program: the same
  operators, every stack offset in them shifted by the same amount (two frame layouts).  It
  came in streaks (ten emissions one way, thirty the other), not per run, and neither a cold
  nor a warm `.loft` cache reproduced it.  Both layouts run the same; it matters to anything
  that compares bytecode across processes.  Not yet localised.
