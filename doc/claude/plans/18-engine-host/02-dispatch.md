# Phase 02 — tier 0: the per-function dispatch (N9 turned inward)

> **Status (2026-06-10): the tier-0 core SHIPPED** — `LOFT_LIVE_RELOAD=1`
> live-swaps a named fn of a RUNNING program.  What the build found: the
> dispatch table already existed in two halves — `State::fn_positions[d_nr]`
> (fn-ref calls re-resolve per call) and `State::calls` (the per-callee
> call-site patch list; direct `OpCall` sites embed `to` at opcode+11) — so
> reload = parse the edited fn under a versioned temp name in a parity-
> checked SHADOW session (`src/live_reload.rs`; no redefinition machinery
> at all), `def_code` the new body APPEND-ONLY at the stream end (the PC is
> live — save/restore `code_pos`; refresh the execute loop's cached length),
> then patch both halves.  Self-recursion names the original def, so it
> lands on the new body through the same patch.  The poll is a counter-gated
> check in the execute loop (~32k ops + 200 ms throttle; one predictable
> branch per op when off).  Proven: `tests/engine_host_reload.rs` — clean
> swap with world-state continuity, broken-edit → old body keeps serving,
> fix-up re-swap, signature-change rejection.  REMAINING in 02: the
> `--native`-baseline call-site indirection (compiled callers through the
> table — today's consumers run interpreted, where the swap is the whole
> story) and the 6b IDE wire-up (the IDE drives saves; the watcher already
> reacts to them).

> **Slice 1 DONE (2026-06-11)** — the re-entry claim PROVED and the frame
> contract MAPPED (`tests/dispatch_reentry.rs`, graduated to a standing
> contract test; bisect cells re-runnable via `PROBE_MODE`).  Verdict:
> host→interp re-entry is a dispatch — **320 ns/call release (incl. a
> 2-store-write + text-format body)**, full arg matrix correct
> (scalar/vector/struct/null), 200k-call churn clean in debug AND release,
> the resumed program verifies the world in loft.  The contract the bisect
> mapped (now enforced by `State::reenter`): (1) `stack_pos` is the
> transient EVAL height, not the frame top — live variable slots sit above
> it between statements, so a synthetic frame starts above the new
> `stack_high` high-water mark; (2) `fn_return` pops a `CallFrame`
> unconditionally — every re-entry pushes one; (3) args are pushed inside
> `reenter`'s closure, after the watermark lift.
>
> **PHASE CLOSED (2026-06-11).**  Slice-2 findings, read off real generated
> source (`LOFT_KEEP_NATIVE_RS=1`):
> - **The native ABI IS reenter's push contract.**  Generated fns are
>   `fn n_bump(cell: &UnsafeCell<Stores>, var_r: DbRef, var_n: i64) -> i64`
>   — records as `DbRef`, scalars as `i64`, over the shared Stores: the
>   marshaling for a native→interp flip is byte-for-byte what
>   `reenter`'s closure pushes.  No translation layer needed.
> - **Inlining is moot** (slice 1's last open probe): the dispatch check
>   belongs at the CALLEE ENTRY (inside the generated body), so it stays
>   semantic whether or not rustc inlines the fn — no `#[inline(never)]`
>   required, no per-call-site emission.
> - The native-baseline build itself (entry checks + the interp bootstrap
>   sharing the program's Stores + result retrieval for value-returning
>   fns) is **08 scenario S2's build** — one home, gated there; 02's own
>   scope (the interpreter-target dispatch, @C71 minimal, on the running
>   demo) is fully shipped: live reload + reenter + the frame contract.
> - The 6b wire-up shipped: `launch_game` sets `LOFT_LIVE_RELOAD=hold` (opt
>   out with `=0`) — every IDE-launched game is live-editable, and a saved
>   edit waits for the editor's Apply (`D!:apply`); `=1` applies on save,
>   as a direct run does.  The child's `live-reload:` stderr lines are the
>   structured feedback.

**Goal.** Editing one function of a RUNNING kernel game takes effect at the
next frame boundary, with ONLY that function dropping to the interpreter —
everything else (libraries AND the rest of the game program) stays compiled.
This is the tier-0 half of the
[LAVITION execution-granularity model](../../LAVITION.md#execution-granularity--per-function-interpret-over-a-compiled-baseline)
(canonical) and the substrate @PLN16 6b/6c (hot-swap `reload`,
breakpoint-in-game) is gated on.

**The invariant.** *Every meaning call crosses ONE dispatch point whose target
is per-function data* — native symbol today, interpreter bytecode after an
edit, (phase 03: wasm export) — *and a target change is observable only as
speed.*  Goal D (backend parity) is what makes that claim possible at all;
the differential sweep is its standing check.

## What already exists (build on, don't rebuild)

| Piece | State | Where |
|---|---|---|
| Interp caller → compiled callee over the shared store | **shipped** (N9/C71: `LibArg` bridge, `wire_shared_native_fns`, parity-gated) | [NATIVE.md § N9](../../NATIVE.md#n9--native-library-shared-store-dispatch-c71) |
| Edit → don't pay rustc | **shipped per-library** (dev-interpret-on-edit) | N9 `use`-flow |
| The frame boundary to swap at | **shipped** — the kernel loop owns it (between event drain and `on_tick`) | `lib/engine_host` `run`/`run_client` |
| The editor that knows WHICH fn changed | **shipped** (@PLN16 IDE slices 1–6a) | `--serve` session |
| Interpreted-fn cost bound | **measured** — interp ≈ 6× native (phase 00(a)); one thin frame-path fn interpreting fits a 33 ms tick with room | [probes/00b](probes/00b-baseline-2026-06-10.md) |

What phase 02 actually adds is the **inverse crossing** and the **table**:

1. **Compiled caller → interpreted callee.**  N9 crosses interp→native; tier 0
   needs native→interp re-entry for the one edited fn.  The host is the loft
   binary itself (the interpreter is ALWAYS present in the kernel process), so
   the thunk is "call `State::execute` at this fn's bytecode over the same
   `Stores`" — no new runtime, no marshalling (the store is the ABI, both
   directions).
2. **The dispatch table** — "the real build" (ENGINE_HOST Part 1): per-fn
   indirection with per-target values, stable fn identity across edits, and
   frame-boundary swap atomicity.

## Design sketch

- **Identity.** A function's dispatch slot is keyed by its definition identity
  (`d_nr` is parse-order-fragile across reloads — key on the qualified name,
  the same identity the IDE's edit events carry).  The slot survives reloads;
  its target changes.
- **The table.** `dispatch[fn] -> Target { Native(ptr) | Interp(d_nr) }`,
  owned by the kernel process, read at call sites through the same shape as
  N9's `LibArg` bridge (one uniform indirection — no per-signature thunks
  beyond what N9 already generates).
- **Swap atomicity.** Target writes are queued; the kernel loop applies them
  at the frame boundary (after `on_tick` returns, before the next pump) — a
  frame never sees two versions of one fn.  This mirrors how 05a's keyframes
  use the tick as the consistency point.
- **The edit flow (the 6b consumer).**  IDE save → parse the one fn → diff ok
  → enqueue `dispatch[fn] = Interp(new d_nr)` → applied at the boundary →
  the next frame runs the new meaning.  Breakpoints (6c) are the same flip
  with "and stop": `dispatch[fn] = Interp` + the debugger's pause hook —
  which is why 6b/6c wait on this phase, not the reverse.
- **Granularity floor.**  v1 flips ONE fn (+ optionally its direct callers
  when inlining would hide the flip — probe whether the native codegen ever
  inlines across fn boundaries today; if not, callers stay untouched).

## Slices (probe-first, consumer-driven)

1. **Probe the re-entry** (`/tmp`, no plan structure): from a `--native`
   program, call one fn through a forced-interpreter thunk over the shared
   store; assert byte-identical results + measure the crossing cost.  This is
   the falsifiable core claim — *"native→interp re-entry is a dispatch, not a
   rewrite"* — and the matrix axes are the N9 ones (scalar/struct/vector
   args, text returns, null).
2. **The table + boundary swap** in the kernel: target writes queued, applied
   between frames; `LOFT_DISPATCH_DEBUG=1` prints flips (the usage sentinel).
3. **The 6b wire-up**: `--serve` edit → single-fn parse → flip.  Acceptance:
   edit a constant in `probe_server_kernel.loft`'s `broadcast_tick` while 30
   clients run; the change shows next frame; the stamp chain shows no hitch
   beyond the fn's own 6× interp cost.
4. **Graceful failure**: a fn whose edit fails to parse keeps its old target
   (the live loop degrades to "stale meaning", never dies) — same philosophy
   as the wasm trap fallback in phase 03.

## Risks / gates

- **Inlining breaks per-fn flips** — if the native baseline inlines loft fns
  into callers, flipping the callee changes nothing.  Probe in slice 1; if
  real, the baseline build gets `#[inline(never)]` on dispatch-eligible fns
  (cheap, codegen-local).
- **State identity across reloads** — slots/locals layouts may differ between
  the old and new fn body; the flip happens at fn-call granularity (never
  mid-frame, never mid-call), so only the *definition* swaps, no live frames
  migrate.  Breakpoint-resume across an edit (6c) is explicitly out of scope
  for 02.
- **This phase does NOT compile game programs to the baseline by itself** —
  programs already run `--native` when invoked so; the kernel demos run
  interpreted today.  Tier 0 is meaningful in both worlds (an interpreted
  baseline just makes the flip a no-op speed-wise), so 02 lands against the
  `--native` path and the interpreted path stays the degenerate case.
