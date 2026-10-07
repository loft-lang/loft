<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Interpreter performance

How the interpreter executes a program, which parts of that have been made fast and how, and
how to measure an interpreter change.  The method for any optimisation, and where the numbers
are, is [PERFORMANCE.md](PERFORMANCE.md); past measurements are in
[PERFORMANCE-history.md](PERFORMANCE-history.md).

## Why the interpreter is optimised at all

`--native` is the optimisation TARGET: shipped programs run compiled, through LLVM.  The
interpreter runs the same program while it is being DEVELOPED and DEBUGGED, and its job is that
a program does not fall off a speed cliff there.  So it gains on the back of native: a rule
native already proves (a record it never builds, a move it never makes) is moved into the IR
phase, where both backends read it (@PLN180 moved R-ValueRecord and R-ValueLocal).  Two
consequences:

- A change that makes the interpreter faster and native slower is wrong.  Check native's
  signatures and rewrites before and after, because an IR rewrite can take away a shape a native
  rule relied on.  @PLN180's first materialisation step turned three forwarding functions back
  into record returns on native, and the fix was to support the forward in the IR.
- Interpreter-only machinery (the lean loop, the operand fusion already built) comes second to
  moving a native rule into the IR, and no NEW combined opcode is the answer to ONE hot spot:
  it is a kernel in disguise, fast for one operand shape (`(Perf-Order)`).  The
  method is [PERFORMANCE.md § How to optimise](PERFORMANCE.md#how-to-optimise--the-checklist).

The bar is measured, not felt: the per-routine ratio of the OPTIMISED interpreter to OPTIMISED
native code, in real time (`make interp-gap`).  A cliff is about **100×**.  A routine above it
is the work queue.  Faster is wanted everywhere, and no change may make any routine's ratio
worse.

A routine over the bar is closed by making its LOFT code fast, not by replacing it: a kernel
(one Rust body standing in for a loop) is a stop-gap that fixes one name, while a faster
interpreter fixes every loop of that shape, a program's own included.  Loft compiles through
rustc, so unlike CPython it has no lasting need for hand-written routines.  Each kernel that
exists, why, and when it goes: [KERNELS.md](KERNELS.md) (`make kernel-ratio`).

## What to optimise: the data that leaves the cache

The interpreter's own bookkeeping — dispatch, stack slots, operand decoding — runs on a hot
frame that lives in L1.  Cutting it makes every run faster by a constant factor, and it has
been cut (below).  What decides how an ALGORITHM scales is the data that flows out of the
caches: records claimed, grown and relocated, blocks copied, stores created.  So the first
question for an interpreter routine is not *how many instructions* but **does the interpreter
do more work on stores than the compiled code does for the same routine?**  `LOFT_STORE_CENSUS`
answers it for both backends with the same counters ([§ Store work](#store-work-interpreter-against-native)),
and a row where the interpreter does more is where a native rule avoids an object or a move —
the candidate to move into the IR phase, where both backends get it.

## Dispatch loop (`src/state/mod.rs`)

The loop fetches one opcode byte and dispatches the operator in that slot (`src/fill.rs`,
generated from the `#rust` templates in `default/*.loft`).  Bytes 0–254 are one-byte opcodes;
**byte 255 is an escape prefix** — the loop reads a second byte `ext` and dispatches slot
`255 + ext` (`emit_op`).  Which operators get a one-byte slot is chosen in `default/`: the
`#hot` ones first, the `#cold` ones last (formal/rewrites.md `(R-OpPriority)`).

There are two loops in `execute_argv`.  The **lean loop** runs whenever nothing watches
individual ops, and does per op only what an ordinary run needs: publish the allocation site
(`alloc_pc`), which is also the crash report's position (`crash_report::LeanSource`,
`(R-DispatchPublish)`); dispatch; and ONE test of `Stores::dispatch_stop`, which every rare
event that ends the loop sets where it happens (`(R-DispatchStop)`).  It carries the bytecode
position and the stack top in registers — each operator receives and returns them
(`(R-RegisterTable)`), a `#hot` one runs inline on them (`(R-HotInline)`; `LOFT_NO_HOT=1`
compares) — and an op checks all its operands at once (`(R-OperandSpan)`), each report out of
line.  The **full loop**
carries every per-op instrument — the debugger and profiler (`debug_check`), live reload, the
stack census, the stack shadow, allocation paths, the UAF scans — and takes over the moment one
is armed, including a debugger attaching mid-run.  Measured: the full loop's
bookkeeping was 62 of an op's 152 instructions on a vector-writing loop.  `LOFT_NO_LEAN_LOOP=1`
takes the full loop for a plain run — the A/B switch for what the lean loop buys.

A `par` worker's frame runs the same lean loop under the same conditions, and finds the
function it enters once per function position rather than once per element
(`(R-WorkerLean)`).  Its stop test differs in one point: a worker ends its own loop, and the
main loop reports a fault a worker raised.  So a one-thread `par` costs what the loop without
`par` costs; `LOFT_NO_WORKER_LEAN=1` keeps a worker on the checked loop.

## Stack and variable access (`src/state/mod.rs`)

The execution stack is a single flat region inside a `Stores` record, addressed by
`stack_cur: DbRef` and `stack_pos: u32`.  `get_stack`, `put_stack`, `get_var` and `put_var`
have a **direct path** (`State::fast_stack`): a base pointer cached in `State` plus the offset,
inlined into every operator — the cache re-derived wherever the stack's buffer can move and
every other buffer move refusing the stack store (formal/rewrites.md `(R-StackBase)`; the
re-derivation per access was three dependent loads on a simple op's critical path).  The bytecode is
read the same way (`(R-CodeBase)`), through a base and length cached in `State`.  Every op is
compiled for both stack modes and the lean loop dispatches the direct one when the run allows
it (`(R-FastTable)`), and a frame makes its room once at entry, so a direct push tests no
capacity (`(R-FrameHeadroom)`; falsifier `LOFT_HEADROOM_VERIFY=1`).  An element read
in range and an append that fits each take one straight path (`(R-ElementPath)`).  The general store path re-checks on every push and pop what the stack
guarantees by construction — its store is live, not foreign, not locked, and `ensure_stack`
grows the buffer and the record together — and cost 43 % of the interpreter's time on that
loop.  The checked path (`*_checked`, out of line) runs whenever an instrument that watches
stack accesses is armed — `verify_on`, `LOFT_STACK_CENSUS`, `LOFT_UAF_GEN`,
`LOFT_STRICT_STORES`, the `stack_align_guard` feature — and in every debug-assertions build.
`LOFT_NO_FAST_STACK=1` takes the checked path on purpose: the A/B switch for what the direct
path buys, and the first bisect step for a wrong answer only the interpreter gives.

## Operand fusion — superinstructions

The bytecode generator emits the most frequent operator shapes as ONE op that reads its
operands in place instead of pushing them first.  Each fused op calls the unfused operators'
own functions in the same order, so fusion changes where operands come from and nothing about
what is computed; `LOFT_NO_FUSE=1` emits the unfused form (R-Switch), and
`tests/scripts/an-integer-operator-over-locals-runs-as-one-op.loft` is the guard.

| fused op | replaces | chosen by |
|---|---|---|
| `OpIntVV` / `VC`, `OpCmpIntVV` / `VC` | an integer operator over locals and literals; a comparison with its literal on the LEFT is mirrored (`3 < a` is `a > 3`, kinds `GT`/`GE`) — never an arithmetic one, whose overflow report names its operands in order | `fusable_int` |
| `OpIntVVPut` / `VCPut` | `x = a op c`, e.g. `i += 1` | `set_var` |
| `OpCmpIntVVJump` / `VCJump` | an `if` or loop test and its jump | `gen_if_test` |
| `OpTextWalkStep` | the step of `for c in T` | `hoist::char_walks` (native's `(R-CharWalk)` matcher) |
| `OpTextNullJump`, `OpTextEndJump` | a text walk's two end tests | `emit_text_end_test` |
| `OpVecGetInt[Nullable]`, `OpVecSetInt` | an integer element of a local vector at a local index | `emit_fused_vec` |
| `OpVecEndJump` | `for x in v`'s end test | `gen_if_test` |

A position operand is taken at the stack height the op STARTS at, and a fused op whose
generated body pops a value first takes its local positions before that value is pushed;
both are the defects the guard's planted-defect cells catch.  The shapes were chosen from
`LOFT_OP_NGRAMS`, the statically adjacent operator runs over the bench lanes (PROFILING.md).

What these buy is constant-factor speed on work that already runs in cache.  Over the 79 bench
routines (`bench/stats.py --lanes interp`, one binary, each path switched off with its
`LOFT_NO_*` switch, fusion on throughout, every routine's output hash identical): the fast
stack path and the lean loop together **2.4×** (median; interquartile 1.95–3.6×, range
1.1–4.7×), the direct stack path alone 2.2×, the lean loop alone 1.3×.  The fusion above
adds its own share on top (`LOFT_NO_FUSE=1` is its switch); up to 5.2× on text walks.
Measured the same way, **none of it changed a single store operation** — which is why the
next work is in the section below, not in more fusion.

## Store work: interpreter against native

`LOFT_STORE_CENSUS=<file>` (PROFILING.md) counts, at the chokepoints both backends share,
stores created and freed, records claimed, deleted, grown and relocated, and the bytes block
copies and text writes move — one line per `ticks()` call, so a bench routine's work is the
difference between the two lines around its timed loop.  Build the interpreter and the native
program against the `op-census` feature (the counters are compiled only there):

```bash
cargo build --release --lib --bin loft --features op-census --target-dir target/op-census
LOFT_STORE_CENSUS=i.tsv target/op-census/release/loft --interpret bench.loft --n 2
target/op-census/release/loft --native-emit n.rs --lean bench.loft
rustc -C opt-level=3 --edition=2024 --extern loft=target/op-census/release/libloft.rlib \
      -L target/op-census/release/deps -o n n.rs && LOFT_STORE_CENSUS=n.tsv ./n --n 2
```

Measured on `14_stdlib_vector`, per op (interpreter / native):

| routine | claims | grows | relocations | bytes relocated |
|---|--:|--:|--:|--:|
| `push` | 4 / 1 | 4 / 0 | 4 / 0 | 81,526 / 0 |
| `record_append` | 3 / 2 | 6 / 0 | 1 / 0 | 233,380 / 0 |
| `grid` | 392 / 137 | 260 / 4 | 4 / 4 | 1,210 / 824 |
| `copy`, `remove_front` | equal | equal | equal | equal — shared runtime work |
| element reads and writes | none | none | none | none on either side |

The interpreter's extra store work is **vector growth**: native sizes a vector it fills once
(`(R-Push)`, `(R-PushFill)`, `(R-PushRec)`) where the interpreter grows it step by step and
relocates the data on each step.  That is NOT worth moving into the IR phase: with the whole
vector reserved by hand before the loop, interpreted, `push`'s loop ran 280–283 ms either way, a record-append loop 632–647 → 632–637 ms, and `f32_build` (603 KB
relocated per op, the most of any routine) 595–640 → 592–596 ms.  Growth doubles, so its cost
is amortised to nothing; `vector_append` is 5 % of `push`'s cycles, and the dispatch of its 13
ops per element is the rest.  The lever there is fewer ops per iteration:

- **The reservation before a literal append** — `OpPreAllocVector(v, n, size)`, which the parser
  emits before every literal append to a local vector — claims a record only for an ABSENT
  vector, `max(n, 11)` elements wide, and `vector_append` claims the same 11 on its own.  So
  for n <= 11 the interpreter does not emit it (`keys::prealloc_elide_enabled`,
  `LOFT_NO_PREALLOC_ELIDE=1` keeps it); the IR keeps it, because `--native`'s append-group
  recognisers read their head and stride from it.  Measured: `push` 279 → 236 ms (−16 %), a
  record append 623 → 578 ms (−7 %).  Guard
  `tests/scripts/a-literal-append-claims-its-vector-once.loft`, pin `tests/prealloc_elide.rs`.
- **The loop variable** was copied from the range's index each round (`VarInt` + `PutInt`).
  Where nothing but that copy writes the variable and nothing in the body writes the index,
  the slot allocator gives the variable the index's slot and the copy is not emitted
  (`slot_alias`, `LOFT_NO_LOOP_VAR_ALIAS=1` keeps it; SLOTS.md § A loop variable in its
  index's slot).  Measured: `push` 236 → 197 ms (−17 %), a tight `for` 540 → 433 ms (−20 %).
  Guard `tests/scripts/a-counted-loop-variable-shares-its-index-slot.loft`.
- **The back edge**: a loop tested at the top and jumped back at the end, two jumps a round.
  A loop whose first statement carries its exit test — a counted range's iterator, a
  `while`'s `if !c { break }` — is laid out with the test at the bottom: entered by one jump
  to the test, which jumps back to the body while the loop goes on (`gen_rotated_loop`,
  `LOFT_NO_LOOP_ROTATE=1` keeps the top-tested form).  The test keeps the loop's own line, so
  stepping off the body pauses on the `for` line as before.  Measured: `push` 200 → 188 ms,
  a tight `for` 430 → 398 ms, a `while` 538 → 511 ms (−5 to −7.5 %).  Guard
  `tests/scripts/a-loop-tests-at-its-bottom.loft`; both layouts pinned by
  `tests/loop_layout.rs`.
- **A computed range start** (`for k in from..len(v)`) ran two counters, one a step ahead,
  and copied one into the other every round.  Where the loop rotates and nothing else names
  the counters, the index is seeded from the start and stepped at the test, and the loop is
  entered past the step (`(R-StartStep)`, `LOFT_NO_START_STEP=1` keeps both).  The IR keeps
  both counters, so native is unchanged.  Measured: `dot_product` 16 → 14 ops a round, −15 %.
  Guard `tests/scripts/a-computed-range-start-steps-one-counter.loft`, pin
  `tests/start_step.rs`.
- **Still open**: a `while`'s test is `OpNot` over an unfused compare and an
  `OpGotoFalseWord`, three ops where a counted `for` takes one.

## Function calls

`fn_call` pushes the return address onto the stack and jumps `code_pos` to the callee. The
callee's locals live above the caller's on the same flat stack record — there is no frame
allocation. A return slides the return value down with `copy_block`, which the store census
counts as copied bytes (8 per integer return).  A frame records the call's position and not
its source line: the line is looked up only when a stack is rendered (`State::call_line`),
since that lookup on every call was a quarter of a recursive function's time.

## The compiled standard library

An interpreted program runs the standard library's LOOPING loft functions compiled: their
native code is built into the loft binary, so `--interpret` needs no rustc at run time
(C71: libraries compile, scripts interpret — the standard library is a library).  It is the
same path an installed library takes, with three differences:

- **What is compiled** — `compiled_stdlib::export_set`: non-generic functions the shared-store
  dispatch can cross, with a loft body that loops.  A one-operation body gains nothing from the
  bridge; output and file primitives stay out; generics are monomorphised per program and stay
  interpreted (`sum<T>`).
- **How it ships** — `make compiled-stdlib` generates `src/compiled_stdlib_gen.rs` (`@generated`,
  exempt from `rustfmt`) with the native bodies, a name → bridge table, the standard library's
  type PREFIX and the source hash it was compiled from.  `extensions::wire_shared_native_fns`
  looks a bridge up in that table before `dlsym`.  Regenerate after a stdlib change, or a
  code-generator change that alters what these functions emit;
  `tests/compiled_stdlib.rs::compiled_stdlib_up_to_date` fails until you do.
- **When it is used** — `compiled_stdlib::mark` checks, at start-up, that the program's type
  table begins with that prefix and that `default/*.loft` hashes the same.  On any mismatch the
  program runs the loft bodies interpreted: slower, never wrong.  `LOFT_NO_COMPILED_STDLIB=1`
  interprets them anyway — the A/B for a wrong answer or a timing, and this section is its home.

The kernels it retired are in KERNELS.md § Removed (`split`, `lines`).  The design record is
[plans/181-compiled-stdlib.md](plans/181-compiled-stdlib.md).

**Open work.**  Each of these still runs the loft bodies interpreted — correct, not yet fast:
`loft test`'s in-process runner, the REPL, the debugger, a host embedding, and the browser
(no bridge dispatcher).  If regenerating the file after a code-generator change proves frequent,
generating it in the build is the fix.

## Measuring an interpreter change: pin the layout first

Two ordinary builds of the interpreter can differ by 15 % on one loop with IDENTICAL instruction
counts: where the dispatch loop and the operator functions land in memory decides how the CPU's
front end serves them (bench 14 at 34.461 G vs 34.452 G instructions read +6 % cycles; a hash
loop +14 %, then −4 % once pinned).  So an interpreter before/after is measured on
two builds that differ ONLY in the change, made from one directory with every function and
branch target cache-line aligned:

```bash
RUSTFLAGS='-C llvm-args=-align-all-functions=6 -C llvm-args=-align-all-nofallthru-blocks=5' \
  CARGO_TARGET_DIR=target/al-before cargo build --release --bin loft     # and al-after
```

and read beside `perf stat -e instructions,cycles`: instructions are deterministic, so a change
that removes work and still reads slower is layout until the pinned builds say otherwise.

⚠ **Both builds come from ONE tree, which differs only by the change.**  A binary bakes in the
compiled stdlib (`src/compiled_stdlib_gen.rs`) while the bench reads `default/*.loft` from the
tree it runs in, so a binary built before a rebase runs a stdlib it was not compiled against
and falls onto slow paths: a pre-rebase "before" made `join` read −95 % and `split` −90 % for
a change that touched neither.  Across a rebase, build the "before" from a
detached worktree at the rebased commit just below the change (`git worktree add --detach`),
with the current derived files copied in, and check `git diff --stat` between the two trees
names only the change.  A comparison across the rebase measures main's commits too.

And when a change moves inlining, read `ld_blocks.store_forward` beside cycles: a helper that
returns a `DbRef` through memory writes it as narrow fields the caller re-reads as one wide load,
a stall per call that the instruction count does not show (`(R-ElementPath)`).
