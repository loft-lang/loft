<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# loft-dap advanced debug tools — design (@PLN63 LSP.3 follow-ups)

> **Identity:** a design sub-doc of `@PLN63` (loft-lsp), companion to
> [DAP.md](DAP.md) (the D0–D6 DAP MVP, LANDED). This doc designs the four advanced
> debug tools the MVP deferred, each as a **small-step spine** landable behind the
> existing `tests/dap_transport.rs` harness.
> **Status:** DESIGN — no code yet. The MVP was a *pure translation* over the `--rpc`
> engine (`src/rpc.rs`); three of these four need ONE engine step first (the RPC surface
> genuinely lacks the capability), so each spine is split **engine → RPC → DAP**.

## Why this doc exists — the MVP's honest leaves

The D0–D6 adapter surfaces the engine's flat, single-frame, forward-only pause: every
local is a leaf, the stack is one frame, there are no data breakpoints, and there is no
stepping backward (DAP.md § Refusals). Each boundary was an honest capability bit or a
clean error, never a wrong picture. This doc turns each into a working tool.

**These are not adapter oversights — three are engine gaps.** Before designing, each
candidate was probed on the proven `loft debug --rpc` path (the same chokepoint loft-dap
drives). The probes are the grounding — and the reason two "obvious" wirings (reverse
stepping via the existing `undo`; data breakpoints via the existing `setWatch`) would
have shipped a *wrong picture*:

| Tool | Probe | Result | What it means |
|---|---|---|---|
| **VE** variable expansion | `eval` of a struct / vector local at a stop | `{"value":{"x":9,"y":2}}` / `[10,20,30]` (proven by `tests/rpc.rs`) | **No engine change.** `debug_eval_json` already walks values → adapter-only. |
| **SF** multi-frame stack | — (`call_stack: Vec<CallFrame>` exists, `src/state/mod.rs:164`) | the data is present but `frame_field` (`rpc.rs:408`) renders only the top `BreakHit` | Engine step: **surface `call_stack`** on the RPC, then translate. |
| **DB** data breakpoints | `setWatch {expr:"x"}` on a stack local `x` | `"not a watchable scalar region"` | `resolve_watch_region` (`mod.rs:3243`) resolves only **store/heap** DbRefs; a stack scalar isn't an allocation. Engine step: **watch stack scalars**. |
| **RX** reverse execution | `undo` after a `stepOver` | `"nothing to do"`, frame degraded to `replmain_1` | The engine's `undo`/`redo` is @PLN16 **M2 reverse-*edit*** (revert a `setValue` journal, `debugger.rs:107`) — **not** reverse execution. Needs real execution checkpointing. |

**Order of work — by cost, cheapest first (VE ships alone; the rest each stack one engine
step):** VE → SF → DB → RX. VE and SF are the highest value per unit cost (they make the
locals panel and call stack real); DB is medium; RX is the largest (a new engine
capability) and is scoped last.

The invariant that binds all four (inherited from DAP.md): **loft-dap adds no debug
semantics.** Every advanced tool is the engine's own capability surfaced on the RPC and
translated — if the engine can't do it truthfully, the adapter refuses it, never fakes it.

---

## VE — structured variable expansion (adapter-only) — **BUILT**

> **Status: LANDED.** `src/bin/loft-dap.rs` (`build_variables`/`expand_node`/`mint_value`);
> gate `tests/dap_transport.rs::variable_expansion_walks_structs_vectors_and_nesting`
> (VE0–VE3). No engine change, as designed. **Known limit (a `frame_field` issue, not VE):**
> a **bare top-level heap-vector local** appears in the frame under its `__vdb_N` compiler
> backing (or is absent), so it may not show under its source name — VE still expands
> whatever IS shown and evaluable, and a vector **nested inside** an evaluable value (a
> struct field, an element) expands fully via the JSON tree. Making the top-level frame
> locals source-faithful for heap vectors is a separate frame-fidelity follow-up (see SF's
> per-frame locals, SF1).

**Gap.** Every `variables` entry is a leaf (`variablesReference: 0`); a struct or vector
local shows its one-line rendering with no drill-in (DAP.md Decision 3).

**Why it's free.** `debug_eval_json(expr)` (`repl.rs:2261`) evaluates any expression in the
paused frame and returns its value as JSON — a scalar, or a nested object/array — proven
for structs (`rpc_eval_struct_as_json`), bare vectors (`rpc_eval_bare_vector_live`), and
keyed collections (`rpc_eval_in_a_keyed_collection_frame`). The DAP `variablesReference`
tree IS a JSON tree; the adapter walks it.

**Invariant.** A `variablesReference` expands to **exactly** the immediate JSON children of
the value `debug_eval_json` returns for that node — no synthesized structure — and a node
is a leaf **iff** its JSON value is a scalar. *Falsify:* expand a struct → its fields and
only its fields; expand a scalar → no expand arrow (ref 0).

**Chokepoints.** `debug_eval_json` via the RPC `eval` verb (already driven by loft-dap's
`evaluate`); the adapter's per-stop handle table (mint/invalidate, DAP.md Decision 3).

**Steps.**

- **VE0 — expandability at the top level.** In the Locals `variables` handler, for each
  local `name`, drive RPC `eval name`; if the value parses as a non-empty JSON object/array,
  mint a `variablesReference` and register the value; else keep `0`. Display value stays the
  flat-frame rendering (unchanged). *Gate:* at a stop with a struct local `p` and a scalar
  `a`, `p.variablesReference != 0` and `a.variablesReference == 0`.
- **VE1 — one-level expansion.** `variables {ref}` for a registered handle returns the JSON
  value's immediate children: object → `{name: field, value: render(child)}`; array →
  `{name: "[i]", value: render(child)}`. A child that is itself object/array gets its own
  minted handle (VE2 walks it); a scalar child gets `0`. *Gate:* expand `p` → `x=9, y=2`;
  expand `nums` → `[0]=10, [1]=20, [2]=30`.
- **VE2 — arbitrary nesting, no re-eval.** Cache the parsed JSON **tree** at the handle;
  a child handle points at a sub-node of the cached tree, so deeper expansion navigates the
  tree in memory (one `eval` per top-level local, none per drill-down). This sidesteps the
  keyed-collection edge (a hash child's path expression isn't a field access) — children are
  read from the parent's tree, never re-evaluated. *Gate:* expand a `vector<Struct>` →
  element → its fields (two levels), values correct.
- **VE3 — handle lifetime.** Invalidate the whole handle table on every resume and mint a
  fresh generation per stop (reuse the `locals_ref` invalidation VE inherits): a stale child
  reference after a resume returns empty, never a wrong subtree. *Gate:* expand at stop A,
  resume, reuse the handle → empty.

All four steps are loft-dap-local; no engine or RPC change — **all four LANDED.** The gate
drives the reliable path (a struct `sq` shown by name → expand its vector field `members`
and nested struct field `lead` → their leaves, through the cached JSON tree). Note VE2's
tree-cache is load-bearing, not just an optimization: a probe showed `eval sq.members`
returns `null` (a direct vector field-access doesn't evaluate), so children MUST be read
from the parent's cached tree, never re-evaluated by path.

---

## SF — multi-frame stack trace (engine → RPC → DAP) — **BUILT**

> **Status: LANDED.** Engine: `State::break_stack` (`mod.rs`, @PLN16 B3) **already**
> captured the full call stack with per-frame locals — proven by
> `tests/debugger.rs::b3_full_stack_includes_indirect_caller`; SF only added a per-frame
> `line` to `BreakHit` (from `line_at(pc)`) and exposed `ReplSession::paused_stack()`. RPC:
> `stackTrace` now returns a `frames` array beside the legacy single `frame` (additive).
> DAP: `src/bin/loft-dap.rs` fans out one `StackFrame` per frame (`frameId = FRAME_ID +
> depth`, the synthetic `replmain_*` entry wrapper filtered), and `scopes {frameId}` reads
> any frame's locals. Gate `tests/dap_transport.rs::stack_trace_walks_all_frames_and_reads_caller_locals`.
> **Two honest limits:** (1) a caller frame's locals are **leaves** — `debug_eval_json`
> evaluates in the top/paused frame only, so a caller's struct/vector can't expand truthfully
> (VE applies to the top frame). (2) A frame's locals inherit the read-dominated liveness
> under-show (`capture_frame_at`): a local not yet read at the call-site pc may be absent
> (e.g. `main` showing `__work_1` rather than `a`) — a `frame_field` fidelity limit, not SF.

**Gap.** `stackTrace` returns the current frame only; a call three deep shows one frame
(DAP.md § Risks). The client's call-stack panel is effectively blind.

**Why the data is there.** The paused `State` holds `call_stack: Vec<CallFrame>`
(`mod.rs:164`); each `CallFrame` carries `d_nr` (the called function → its name via
`Data`), `call_pos` + `line` (the call-site source line, TR1.4), and `args_base` +
`args_size` (the frame's argument region). `frame_field` (`rpc.rs:408`) renders only the
top `BreakHit`; TR1.3's `n_stack_trace` snapshot (`mod.rs:653–722`) already walks
`call_stack` for a *runtime* stack-trace value — the same walk, for the debugger pause.

**Invariant.** The DAP stack is the engine's `call_stack`, **one `StackFrame` per
`CallFrame`, innermost first**, each frame's `name`/`line` read from its `CallFrame` — never
a synthesized, reordered, or truncated stack. *Falsify:* a 3-deep call chain
`main → a → b`, breakpoint in `b` → exactly `[b, a, main]` with each frame's call-site line.

**Chokepoints.** `State::call_stack` + `Data` def names; a new engine renderer beside
`paused_frame`; a widened RPC `stackTrace`; loft-dap's `stackTrace` fan-out.

**Steps.**

- **SF0 — engine: render the frame list.** Add `ReplSession::paused_stack() -> Vec<Frame>`
  that walks the paused `State.call_stack` innermost-first, resolving each `d_nr` to a
  function name and each frame's source line (reuse the TR1.3 line-resolution at
  `mod.rs:679–717`). *Gate (unit):* a 3-deep pause returns 3 frames, names + lines correct.
- **SF1 — engine: per-frame locals.** Extend each rendered frame with its locals, reusing
  the `BreakHit` slot-read that renders the top frame today (read the frame's slot region at
  `args_base`); a frame whose slots aren't recoverable yields an empty locals list (honest),
  never wrong bytes. *Gate:* the caller frame's parameter shows its value.
- **SF2 — RPC: widen `stackTrace`.** The `stackTrace` verb (`rpc.rs:320`) returns
  `{"frames":[{function, line, locals}, …]}` (was the single `frame`); keep the old single
  `frame` field too for one release (additive — the compatibility rule). *Gate:* `tests/rpc.rs`
  asserts the multi-frame array for a nested call.
- **SF3 — DAP: fan out the frames.** loft-dap's `stackTrace` emits one `StackFrame` per RPC
  frame with a **distinct `frameId`** (e.g. `1000 + depth`); `scopes`/`variables` key off the
  requested `frameId` so the client can inspect any frame's locals, not just the top. *Gate:*
  `tests/dap_transport.rs` walks a caller frame's variables.
- **SF4 — the current-frame source marker.** The top frame carries the parked line; the
  rest carry their call-site line (so the editor underlines the call in each caller). *Gate:*
  the caller frame's `line` is the call site, not the callee's body.

SF depends on nothing but itself; it is the second increment (highest value after VE).
**All five steps LANDED** — SF0/SF1 were mostly the pre-existing `break_stack` (the design
under-counted the engine work: the walk + per-frame locals already existed; only the line +
exposure were new).

---

## DB — data breakpoints via watchpoints (engine → RPC → DAP) — **BUILT**

> **Status: LANDED.** Engine (DB0/DB1): `resolve_watch_region` now resolves a bare scalar
> local to its **stack slot** (via `frame_slot` + a `Type`→content map) and tags the
> `Watchpoint` with the frame it belongs to (`StackWatchFrame`); `poll_watchpoints` drops a
> stack watch the instant its frame returns (a retain pass over `call_stack`), so a reused
> slot never fires — the heap-vs-stack split the engine deliberately avoided is now handled,
> not dodged. RPC (DB2): `setWatch`'s existing ok/err IS the `verified` signal (no wire
> change). DAP (DB3/DB4): `dataBreakpointInfo` reconstructs the watch target from
> (container, name) — a top-level local is its name, a nested field/element extends the VE
> handle's tracked path (`sq.lead`, `sq.members[0]`); `setDataBreakpoints` clears + sets each
> via `setWatch`; a hit rides the existing `stopped{reason:"data breakpoint"}` map.
> Gates: `tests/rpc.rs::{rpc_watch_stack_local_fires_on_change, rpc_watch_stack_local_drops_on_frame_exit}`,
> `tests/dap_transport.rs::data_breakpoint_on_local_fires_on_change`. **Two honest limits:**
> a data breakpoint can only be **set at a stop** (the frame must exist to resolve a local —
> not during pre-run config); and a **caller** frame's locals aren't watchable (the resolve
> uses the top/paused frame, `call_stack.last()`).

**Gap.** DAP data breakpoints ("break when this variable changes") aren't offered. The
engine HAS watchpoints (`add_watchpoint`, `poll_watchpoints`, `mod.rs:3242`+) and a watch
hit already rides the RPC as `stopped{reason:"watch"}` → loft-dap already maps that to
DAP `data breakpoint`. **But** `resolve_watch_region` resolves only store/heap DbRefs, so a
plain stack local `x` returns `"not a watchable scalar region"` (probed) — the common case
fails.

**Invariant.** A data breakpoint fires **iff** the watched scalar's bytes change (stack OR
heap); a target that cannot be watched comes back `verified: false` — never a silent miss,
never a false stop. *Falsify:* watch a local, mutate it a line later → a stop with the old →
new value; watch an unwatchable expression → `verified:false`, no stop.

**Chokepoints.** `resolve_watch_region` + `poll_watchpoints` (`mod.rs:3243`, `3267`); the
`Watchpoint` struct (`debugger.rs`); RPC `setWatch`/`clearWatch` (already present); DAP
`dataBreakpointInfo` + `setDataBreakpoints`.

**Steps.**

- **DB0 — engine: watch a stack scalar.** Give `Watchpoint` a region **variant**: the
  existing store region `(store_nr, rec, off, len)` OR a new **stack region** (the paused
  frame's absolute slot address + width). Extend `resolve_watch_region` to resolve a scalar
  local to its stack slot when it isn't a store DbRef. *Gate (unit):* `add_watchpoint("x")`
  on an integer local returns `true`.
- **DB1 — engine: poll the stack region.** `poll_watchpoints` reads the stack region's bytes
  each stepped op (mirror the store `read_span`), emitting the same `WatchHit{label, old,
  new}`. A watch whose frame has returned (slot gone) is dropped, not read stale — mirror the
  freed-store skip (`mod.rs:3274`). *Gate:* mutating the watched local fires exactly one hit.
- **DB2 — RPC: verified feedback.** `setWatch` already returns ok/err; surface the
  resolvability as a `verified` flag per watch so a dead data breakpoint is reported at set
  time (mirror `setBreakpoints`' `verified`). *Gate:* `tests/rpc.rs` — a watchable local
  verifies, an unwatchable expression does not.
- **DB3 — DAP: `dataBreakpointInfo`.** Answer with `{dataId: <expr>, description, accessTypes:
  ["write"]}` when the expression resolves; `{dataId: null}` otherwise (the client then greys
  out the option). Advertise `supportsDataBreakpoints`. *Gate:* info on a local returns a
  `dataId`; info on a literal returns null.
- **DB4 — DAP: `setDataBreakpoints`.** Replace the watch set: RPC `clearWatch`, then one
  `setWatch` per `dataId`, returning `{breakpoints:[{verified}]}`. A watch hit → the existing
  `stopped{reason:"data breakpoint"}`. *Gate:* `tests/dap_transport.rs` — set a data
  breakpoint on a local, continue, assert the stop + the changed value.

DB is self-contained after DB0–DB1; the DAP half (DB3–DB4) is a thin translation.
**All five steps LANDED.**

---

## RX — reverse execution (the large one: engine checkpointing → RPC → DAP)

**Gap.** No stepping backward. The probe confirms the engine's `undo`/`redo`
(`debugger.rs:107`) is **reverse-*edit*** — it reverts a `setValue` journal, and returns
`"nothing to do"` after a *step* because a step records no edit journal. True reverse
execution needs the engine to remember prior execution **states**, which it does not.

### What the investigation found (this refutes the first design decision)

The original sketch said "reuse the journal, don't snapshot the world." A read of the real
machinery shows that does **not** hold — the journal cannot see ordinary execution:

| Finding | Code point | Consequence for RX |
|---|---|---|
| The M2 undo journal records modifies **only at explicit edit sites** (armed by `begin_edit_journal`, snapshotted by `edit_before`/`edit_after`) — never during opcode execution. | `mod.rs:2663`–`2684`, `2689` | A step's writes are invisible to the journal; "reuse the journal" can't reverse a step. |
| `debug_step` runs **N opcodes** per step (until the line/depth changes), each mutating stores through raw `addr_mut` scattered across `fill.rs`. There is **no central write chokepoint** to hook cheaply. | `mod.rs:4458`–`4531` | Journaling every execution write means instrumenting the hot path — unacceptable for a language runtime. |
| The `@PLN16.J` recording (`start_recording`/`take_journal`) captures **structural** changes (claim / delete = `StoreChange::Insert`/`Free`) only, **not** data modifies. | `database/mod.rs:751`, `764`; `store.rs:118` | Reusing it gives allocation/free reversal but misses value writes — insufficient alone. |
| `debug_step` **clears** the undo/redo history on every step ("resuming reuses frame slots → a stale-slot undo"). | `mod.rs:4472` | RX's history must be a **separate** ring the step loop does not wipe. |
| `Stores::clone()` deliberately **drops `allocations`** ("runtime-only… only valid during execution"). | `database/mod.rs:508` | A snapshot cannot reuse `clone`; the heap needs an explicit byte copy. |
| A per-store **deep byte-copy already exists** — `clone_locked` allocates a fresh buffer and `copy_nonoverlapping`s the bytes (used for `par` worker stores). | `store.rs:1224` | The snapshot primitive is proven code, not new `unsafe` — model the heap snapshot on it. |

### Design decision (revised) — a bounded **snapshot ring**, all cost off the hot path

Reverse execution **checkpoints the execution-mutable state before each step** and restores it
to step back — correct **by construction** (restore = copy the exact bytes back), and it keeps
**zero** instrumentation on the normal execution path (the snapshot is taken only while a
reverse-step-armed session steps; a normal run and even a normal debug run pay nothing). The
cost is one heap byte-copy per stepped step, **bounded** by a ring of the last N steps.

A checkpoint is the execution-mutable subset of `State` (the compile-time maps — `bytecode`,
`vars`, `calls`, `line_numbers`, `const_refs`, … — never change during a step, so they are
**not** copied):

- **registers:** `code_pos`, `call_stack`, `call_depth`, `stack_cur`, `stack_high`,
  `stack_pos`, `stack_cap_bytes`, `arguments`, `coroutines`, `active_coroutines`
  (`mod.rs` `struct State`);
- **heap:** a deep byte-copy of every `database.allocations[i]` (modelled on `clone_locked`).

This also **removes** the slot-reuse hazard that forced the M2 edit-journal to self-clear
(`mod.rs:4472`): a full heap+stack snapshot restores the entire stack store, so a reused slot
is never a problem — a strict improvement over the edit journal.

**Invariant.** `stepBack` lands on **exactly** the state before the last forward step —
frame, locals, stack pointer, and store contents byte-identical to never having taken it.
*Falsify (RX0):* snapshot the frame + all locals + the heap bytes, `next`, `stepBack`,
re-snapshot → byte-identical; then a store-mutating step (`v[0] = 9`), `stepBack` → the store
reverts too.

**Chokepoints.** `clone_locked` (`store.rs:1224`, the heap-copy primitive); the step loop
`debug_step` (`mod.rs:4458`) — snapshot at its entry; a new ring on `Debugger`
(`debugger.rs`), distinct from `undo_stack`; a new RPC `stepBack` verb (NOT the edit
`undo`/`redo`, which stay edit-scoped); DAP `stepBack`/`reverseContinue`.

### Small, safe steps

- **RX0 — falsification + sizing probe FIRST. ✅ DONE** (`tests/debugger.rs::rx0_reverse_execution_absent_and_snapshot_size`).
  (a) Confirmed reverse execution is **absent**: after a forward step, `debug_undo` is a no-op
  and the state does not move back (it is edit-scoped — this stays true after RX, which adds a
  *separate* `step_back`, so the probe is permanent, not throwaway). (b) **Sizing:** a
  full-heap snapshot (`Σ len·8` over `allocations`, the `clone_locked` cost) is **≈ 10.7 KB**
  for a trivial program (3 stores, stack-store-dominated), unchanged across an in-place
  `v[0]=99` step. The copy is a full byte-copy of every store buffer, so it scales **linearly**
  with total heap. **Decision → PROCEED with the full-snapshot ring:** cheap for typical debug
  sessions (small heaps), and a bounded ring (RX3) caps depth; the linear heap-scaling is the
  documented caveat, with copy-on-write-per-record as the fallback only if a heap-heavy session
  proves it necessary.
- **RX1 — engine: the checkpoint primitive. ✅ DONE.** `Store::snapshot_copy` (`store.rs`) — a
  **writable** deep byte-copy that, unlike `clone_locked`, keeps the free tree + claims (a
  restored store resumes allocation); `Stores::snapshot_heap() -> Option<HeapSnapshot>` /
  `restore_heap(&HeapSnapshot)` (`database/mod.rs`) — copies every allocation, **`None` when a
  durable/mmap store is live** (its file isn't reversible — the honest boundary); and
  `State::snapshot_checkpoint() -> Option<StepCheckpoint>` / `restore_checkpoint`
  (`state/mod.rs`) wrapping the heap with the register subset (`code_pos`, `call_stack`,
  `call_depth`, `stack_cur/high/pos/cap_bytes`, `arguments`, `coroutines`, `active_coroutines`).
  *Gate:* `tests/debugger.rs::rx1_checkpoint_restores_heap_and_registers` — snapshot → edit a
  heap local (`n=999`) + move registers (PC + push a frame) → restore → `n` back to 42, PC +
  `call_stack` restored, and the checkpoint is reusable (copied, not consumed).
- **RX2 — engine: `step_back`. ✅ DONE.** A `reverse` flag + `reverse_ring:
  Vec<StepCheckpoint>` on `Debugger` (`debugger.rs`), distinct from the M2 `undo_stack` the
  step self-clears; `debug_step` pushes a checkpoint at entry **only when armed** (a
  default-off branch — normal stepping pays nothing); `State::set_reverse(on)` arms it and
  `State::step_back(data)` pops the top, restores it, and refreshes the paused frame. *Gate:*
  `tests/debugger.rs::rx2_step_back_reverses_a_step` — the RX0 falsification FLIPPED: arm
  reverse, step over a mutating line (`a=99`), `step_back` → line + local revert to the exact
  pre-step state (byte-level), and a `step_back` past the floor is a clean no-op. Interpreter
  only (reverse execution does not apply to `--native`).
- **RX3 — engine: bounded ring. ✅ DONE.** `reverse_ring` is a `VecDeque` capped at
  `reverse_cap` — a push past the cap drops the oldest (`pop_front`), `step_back` takes the
  back; `set_reverse` sizes it from `LOFT_REVERSE_DEPTH` (default 200), `set_reverse_depth(n)`
  overrides (DAP layer / tests). *Gate:*
  `tests/debugger.rs::rx3_ring_is_bounded_to_the_depth` — cap 2, three steps → exactly two
  `step_back`s succeed, the third is a clean floor, and the dropped (oldest) step's line is
  unreachable (the cap held; no unbounded growth, no corruption).
- **RX4 — RPC: a `stepBack` verb. ✅ DONE.** Two RPC verbs (`rpc.rs`): `setReverse {on}` arms
  the session (the client sends it before offering reverse), and `stepBack` reverses one step
  → `report(..., "step", session.is_debugging())` (the refreshed frame + `stopped{reason:
  "step"}`; reverse never terminates, so it always reports the current — moved or floor —
  frame). `ReplSession::set_reverse` stores the pref + applies it to the paused frame;
  `resume_continue_with` re-arms the running `State` before each step so the ring fills;
  `debug_step_back` wraps `State::step_back`.  `undo`/`redo` stay the edit-scoped verbs. *Gate:*
  `tests/rpc.rs::rpc_step_back_reverses_a_step` — arm reverse, run to a breakpoint, `stepOver`
  (line 3→4, `a` 1→2), `stepBack` → back at line 3 with `a` reverted to 1 (was a no-op via
  `undo`), no termination.
- **RX5 — DAP: `stepBack` / `reverseContinue`. ✅ DONE.** loft-dap advertises
  `supportsStepBack`, arms reverse at `configurationDone` (RPC `setReverse on:true`, so forward
  steps checkpoint), maps `stepBack` → RPC `stepBack` → `stopped{reason:"step"}`, and
  `reverseContinue` → drives `stepBack` until the RPC `moved:false` (the ring floor),
  suppressing the intermediate stops and reporting the one landing frame. *Gates:*
  `tests/dap_transport.rs::{step_back_reverses_a_step_over_dap, reverse_continue_walks_back_to_the_floor}`
  — step over `a=a+1` (line 3→4, `a` 1→2), `stepBack` → line 3 with `a` back to 1; and from
  line 5 (`a`=4) `reverseContinue` → the floor (line 3, `a`=1).

**RX COMPLETE** (RX0–RX5): reverse execution works from a DAP editor. The `moved` flag was
added to the RPC `stepBack` response so `reverseContinue` knows when to stop. Ring depth is
`LOFT_REVERSE_DEPTH` (default 200).

### Honest limits (state these up front, they are inherent)

- **I/O is not reversed.** The snapshot is heap + registers only: a `print` during the
  reversed step stays in the console, and file / DB writes are **not** rolled back. Reverse
  execution restores **program memory state**, not external side effects — true of every
  time-travel debugger. `reverseContinue` then re-running forward re-executes those effects.
- **Bounded depth.** Only the last **N** steps are reversible (the ring); older history is
  gone. `reverseContinue` reverses to the ring floor, **not** to a prior breakpoint (there is
  no backward breakpoint matching in v1) — surfaced, not faked.
- **Interpreter only.** No `--native`; `supportsStepBack` is advertised only over the
  interpreter-backed adapter.
- **File-backed stores.** A durable/mmap store's bytes are copied in memory but the underlying
  file is not rewritten on restore (an I/O limit as above); v1 either refuses reverse-arming
  when a durable store is live, or documents the caveat — decide at RX1 from the RX0 findings.
- **Cost.** One heap deep-copy per stepped step while armed (opt-in). If RX0 shows the copy is
  too heavy for large heaps, the follow-up is **copy-on-write per record** — extend the
  `@PLN16.J` store `recording` (`store.rs`) to snapshot a record on its first write during a
  step, so only touched records are captured (efficient, but it re-introduces a hot-path hook,
  which is exactly why it is the *refinement*, not the v1).

RX is the deepest change (a genuinely new engine capability) and is scoped last; RX0's probe
is the gate — the ring approach ships if the per-step byte-copy is affordable, else the
copy-on-write refinement is the fallback.

---

## Not designed here (still honest refusals)

- **`pause` (async interrupt).** Orthogonal to these four — an interrupt path into the running
  interpreter.  **Built 2026-09-25** (DAP.md § pause).
- **Multi-worker `par` threads.** One synthetic thread today; one-per-worker rides SF's
  multi-frame machinery once `par` worker frames are surfaced — a follow-up over SF.

## See also

- [DAP.md](DAP.md) — the D0–D6 MVP these extend (the translation invariant, the envelope,
  the flat frame each tool drills into).
- [@PLN16](../../plans/16-debugger/README.md) — the debug engine (A–F, M1–M5); RX generalizes
  its M2 edit-journal, DB extends its M3 watchpoints, SF surfaces its `call_stack`.
- [16.P PROTOCOL.md](../../plans/16-debugger/PROTOCOL.md) — the RPC contract each spine widens.
- `src/rpc.rs` — `DebugDriver`/`handle` (the chokepoint), `frame_field` (the flat frame SF
  and VE replace), `report` (the stop/event path DB and RX ride).
- [STACKTRACE.md](../../STACKTRACE.md) — TR1.3 `vector<StackFrame>` (the call-stack walk SF0 reuses).
