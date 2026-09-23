
# Stack Slot Assignment — Design and Implementation

This document describes how `assign_slots_v2` assigns stack positions to local
variables and the invariants codegen enforces.

---

## Overview

`assign_slots_v2` (`src/variables/slots_v2.rs`) runs after `compute_intervals` and
before codegen.  It assigns `stack_pos` to every local variable.  Codegen
(`src/state/codegen.rs::generate_set`) reads the pre-assigned position and
asserts it matches the runtime stack pointer (TOS).

**Key invariant:** `assign_slots_v2` is the single authority for slot positions.
Codegen never moves variables.  If a variable's slot doesn't match TOS at
first assignment, that's a bug in `assign_slots_v2` — not something codegen
should silently fix.

---

## Frame Layout

```text
┌──────────────────┐  ← frame base (args + return address)
│  zone 1: small   │  ≤8-byte types, greedy interval colouring
│                  │  at each Block/function scope
├──────────────────┤
│  zone 2: large   │  >8-byte types (text, refs, vectors)
│                  │  placed sequentially in IR-walk order
└──────────────────┘  ← frame_hwm (reserved at function entry)
```

**Single function-entry reserve (@PLAN04 B.3).**  A single
`OpReserveFrame(frame_hwm)` at function entry covers every local
slot — both zone-1 greedy placements and zone-2 IR-walk placements.
`frame_hwm` is the maximum `stack_pos + size` across all non-argument
variables.  Per-block `OpReserveFrame(block.var_size)` is gone; slot-move
+ gap-fill in `gen_set_first_at_tos` is gone; every first-assignment is
a positional init (`OpInitText(pos)` / `OpInitRef(pos)` /
`OpInitRefSentinel(pos)` / `OpInitCreateStack(pos, dep_pos)`).

**The reserve does not initialise.**  `OpReserveFrame` only moves the top of the stack,
so a slot holds whatever an earlier call left there until its first assignment writes it.
A local declared ahead of a branch and first bound inside an arm (`if p { h = mk(1) } else
{ h = mk(2) }`) therefore reads stale bytes on the interpreter if anything reads it before
that bind, where `--native` starts every local at its zero value.  A single-cell probe does
not show it, because a fresh stack is zero: it takes an earlier call to dirty the slot.  So a
compiler-emitted read of a local's OLD value (a displaced-value release, a snapshot) must be
placed only where an earlier bind dominates it — see `Scopes::fnref_bound` (loft#1609).

**Blocks** use both zones.  Slots are assigned by `assign_slots_v2`;
codegen writes directly to each variable's pre-assigned position via the
positional init ops.  No per-block reserve, no per-block free: the whole
frame is owned by the function and released on return.

**Loops** skip zone 1 (same as before the B.3 refit).  `var_size = 0`,
no per-loop reserve.  All loop variables are placed sequentially via
the zone 2 IR-walk.  Loop variables persist across iterations;
`clear_stack` at the end of each iteration resets TOS to the loop start
without touching the reserved frame.

---

## Zone 1: Greedy Interval Colouring (Blocks only)

Small variables (≤ 8 bytes) in a Block scope are packed densely at
`frame_base`.  Dead variables' slots are reused within the same scope
if sizes match.  The zone-1 high-water mark contributes to `frame_hwm`,
which drives the single function-entry `OpReserveFrame`.

---

## Zone 2: Sequential IR-Walk Placement

Large variables (> 8 bytes) and ALL loop-scope variables are placed at
`*tos` in the order they appear during the IR tree walk.  `*tos` advances
by `v_size` after each placement.

### Special cases in the IR-walk

- **Block-return:** `Set(v, Block([..., result]))` — the block starts at
  `v`'s slot (non-Text refs share the frame).
- **Inner pre-assignments:** `Set(v, Insert([Set(__lift, ...), Call(...)]))` —
  the Insert's preamble Sets are processed first so `__lift` vars get
  lower slots than `v`.
- **Cross-scope `Set` + Insert-rooted bodies** — @PLAN05 extended the
  main walk to cover variables whose first `Set` lies in a child
  `operators` list or whose function body root is `Value::Insert`.
  The former post-walk orphan placer (`place_orphaned_vars`) is deleted;
  the main walk now reaches every local.

---

## Codegen Invariants

### `gen_set_first_at_tos` — positional write

Every first assignment is a positional init op: the codegen emits
`OpInit*(slot_pos)` with the slot the allocator chose, and the runtime
writes directly to that position.  No slot-move fix-up.  The assertion
that used to verify `pos == TOS` is gone — it became meaningless once
the allocator and codegen agree on absolute positions.

### `set_stack_pos` assertion

```rust
debug_assert!(pre_assigned_pos == u16::MAX || pre_assigned_pos == pos || argument);
```

Codegen never moves a variable after `assign_slots_v2` has placed it.

### `gen_loop` — no per-loop OpReserveFrame

Loops do not emit `OpReserveFrame`.  All loop variables are placed at TOS
by codegen on first encounter (same as before B.3).  `clear_stack` at
the end of each iteration resets TOS to the loop start.

---

## Invariant validation (from @PLAN04)

`src/variables/validate.rs::validate_slots` runs at the end of every
codegen pass in debug / test builds (`src/state/codegen.rs:155-160`)
and checks the following invariants against the final variable
table.  All fire as panics with a `[I#]` prefix so the diagnostic
is searchable.

| ID | Check |
|---|---|
| **I1** | No slot overlap between variables with overlapping live intervals. |
| **I2** | Args live in the argument region; locals live above. |
| **I3** | (placeholder; no check today). |
| **I4** | Every variable with a first-def has a placed slot. |
| **I5** | Slot-kind consistency — no mixing of `Inline` and `RefSlot` on a shared slot (drop-opcode safety). |
| **I6** | Loop-iteration safety — no slot shared across loop-body boundary without a reset. |
| **I7** | **Scope-frame consistency** — each variable's `stack_pos` lies within its declared scope's frame region `[frame_base(scope), frame_base(scope) + var_size(scope))`.  Catches the "slot above TOS" runtime panic class at compile time. |

I7 was added at the 2026-04-22 close-out of @PLAN04.  Invariants
I1–I6 were authored during @PLAN04 Phase 2 as correctness gates for
V2; they continue to run against V1's output unchanged.

**I8 — orphan-iterator-alias** was scoped in @PLAN05 Phase 2b as a
dep-chain-aware aliasing check guarding P185's failure shape.
Deferred and now dropped: with `place_orphaned_vars` gone, the
bug class it would catch is structurally prevented.  Revisit only
if a future slot-reuse aliasing regression surfaces.

## Plan-04 / @PLAN05 status (closed)

- **Plan-04** (`doc/claude/plans/finished/04-slot-assignment-redesign/`)
  aimed to replace the two-zone V1 allocator with a single-pass,
  scope-blind algorithm.  The retirement attempts
  (codegen-is-allocator and V2-drive) both failed AT THE TIME on variables
  declared at outer scope but first-Set in inner scope.  **V2 is now the
  production allocator**: `ac961e9b6` (2026-05-31, @PLAN53 / #236) deleted
  `src/variables/slots.rs`, and `scopes.rs` reaches the allocator through
  `crate::variables::assign_slots_v2`.  `LOFT_SLOT_V2=validate|drive`
  remains as the per-function shadow mode.  What
  did land: positional init primitives, function-entry frame reserve,
  `OpText` deletion, and invariant I7.
- **Plan-05** (`doc/claude/plans/finished/05-orphan-placer-elimination/`)
  deleted `place_orphaned_vars` by extending the main IR-walk to cover
  Insert-rooted bodies and cross-scope `Set`s.  P185 is fixed and
  its regression tests are un-ignored.

---

## Diagnostic Tools

### `validate_slots` (debug only)

After codegen, scans all assigned variables for spatial+temporal overlaps.
Panics with variable names, slots, and live intervals if a conflict exists.

### `.slots()` test assertions

The `Test::slots(spec)` harness in `tests/testing.rs` captures the
assigned-slot layout for `n_test` after codegen and compares it against
a multi-line visual spec ("name(scope)+size=slot [first_def..last_use]"
with depth bars).  Calling `.slots("")` triggers a panic that prints
the calculated layout ready for copy-paste — the intended workflow for
adding a new fixture.  The check fires in every build profile.

Two fixture catalogues use this harness:

- `tests/strings.rs` — string-scope regressions (2 fixtures).
- `tests/slot_v2_baseline.rs` — the **Phase 0 fixture catalogue from
  @PLAN04** (see
  [`plans/finished/04-slot-assignment-redesign/`](plans/finished/04-slot-assignment-redesign)).
  Every fixture locks one specific placement decision; the file now
  runs as a structural regression guard against V1's output and a
  correctness gate for V2's shadow validator.

Unit tests in `src/variables/mod.rs` and `src/variables/validate.rs` also
verify slot assignments for specific IR shapes without running codegen, by
constructing synthetic `Function` / `Value` structures directly.
(`slots_v2.rs` itself carries no test module.)

---

## Phase 0 Fixture Catalogue (plan 04)

Every pattern documented below has an explicit fixture in
`tests/slot_v2_baseline.rs` that locks the exact slot layout
produced by the two-zone allocator.  The catalogue is retained as a
regression guard after @PLAN04's close-out; V2 (shadow validator)
must still reproduce every layout under `LOFT_SLOT_V2=validate`.

| # | Pattern | Fixture | Status |
|---|---------|---------|--------|
|  1 | Zone-1 reuse (non-overlapping small ints share slot) | `zone1_reuse_two_ints_same_block` | ✅ |
|  2 | Loop-scope small vars placed sequentially | `loop_scope_small_vars_sequential` | ✅ |
|  3 | Text block-return vs child text | `text_block_return_vs_child_text` | ✅ |
|  4 | Insert preamble (P135 lift) ordering | `insert_preamble_lift_ordering` | ✅ |
|  5 | Sibling scope reuse (If-expression arms) | `sibling_scopes_share_frame_area` | ✅ |
|  6 | Sequential lifted calls (`body += pad(i)`) | `sequential_lifted_calls` | ✅ |
|  7 | Comprehension then literal (P122p) | `p122p_comprehension_then_literal` | ✅ |
|  8 | Sorted range comprehension (P122q) | `p122q_sorted_range_comprehension` | ✅ |
|  9 | Par loop with inner for (P122r) | `p122r_par_loop_with_inner_for` | ✅ fixed 2026-04-23 — outer `par()` iterator `a` inlined to `OpGetVector(items, idx)` (mirroring the inner `b` treatment) so it no longer needs a slot |
| 10 | Many parent refs + child loop index | `parent_refs_plus_child_loop_index` | ✅ |
| 11 | Call with Block arg (vector-comprehension in arg position) | `call_with_block_arg` | ✅ |
| 12 | Parent var Set inside child scope | `parent_var_set_inside_child_scope` | ✅ |
| 13 | P178 — `is`-capture in Insert-rooted body | `p178_is_capture_body` | ✅ |
| 14 | P185 — late local after inner text-accumulator loop | `p185_late_local_after_inner_loop` | ✅ passing since @PLAN05 retired `place_orphaned_vars` |
| 15 | Local after args-heavy signature (args-region isolation) | `fn_with_only_arguments` | ✅ |
| 16 | Nested If with Block branches | `nested_if_block_branches` | ✅ |
| 17 | Large vector followed by small int (zone-1/2 mixing) | `large_vector_then_small_int` | ✅ |
| 18 | Two sibling Blocks with shared outer var | `two_sibling_blocks_shared_outer` | ✅ |
| 19 | For-loop with two loop-scope locals | `for_loop_two_loop_locals` | ✅ |
| 20 | Nested for-in-for (two loop scopes) | `nested_for_in_for` | ✅ |
| 21 | Match with per-arm bindings | `match_with_arm_bindings` | ✅ |
| 22 | Vector block-return (non-Text frame-sharing) | `vector_block_return_non_text` | ✅ |
| 23 | Nested call chain `f(g(h(x)))` | `nested_call_chain` | ✅ |
| 24 | Vector accumulator loop (`acc += [...]`) | `vector_accumulator_loop` | ✅ |
| 25 | Early return from nested scope | `early_return_from_nested_scope` | ✅ |
| 26 | Method-mutation extends var lifetime | `method_mutation_extends_lifetime` | ✅ |

**Legend:** ✅ layout locked and passing; ⚠ fixture present but
`#[ignore]`-d pending an orthogonal fix (see the fixture's doc comment
for the specific blocker).


---

## Scope shapes — every local reached by the main walk

Plan-05 deleted `place_orphaned_vars` (the post-walk catch-net) by
extending `process_scope` / `place_large_and_recurse` to cover every
IR shape that previously left a variable orphaned.  The three
structural triggers were:

| Scope-shape trigger | How the main walk now reaches it | Fixture |
|---------------------|-----------------------------------|---------|
| Function body root is `Value::Insert` (not `Block`/`Loop`) | Insert at function-body root treated as a synthetic Block with scope 1 (@PLAN05 Phase 1a) | `p178_is_capture_body`, `insert_preamble_lift_ordering` |
| Parent-scope `Set` inside a child Block's `operators` | Cross-scope `Set(v)` where `v.scope != walker_scope` is handled in the parent's operator list (@PLAN05 Phase 1b) | `parent_var_set_inside_child_scope` |
| Insert preamble (`Value::Insert([Set(__lift_N, ...), ...])`) wrapping a Call or format-string | Exhaustive traversal of `BreakWith / Iter / Tuple / TuplePut / Yield / Parallel` (@PLAN05 Phase 1b) | `insert_preamble_lift_ordering`, `sequential_lifted_calls` |

The P178 `local_start` floor stays in the per-variable conflict check
to keep locals from overlapping the argument + return-address region.

---

## Files

| File | Role |
|------|------|
| `src/variables/slots_v2.rs` | `assign_slots_v2`, `apply_v2_result`, `slot_kind` |
| `src/state/codegen.rs` | `generate_set`, `gen_set_first_at_tos`, `gen_loop`, `generate_block` |
| `src/variables/mod.rs` | `set_stack_pos` assertion, `Function` struct |
| `src/scopes.rs` | `scan_set` (Insert flattening), `inline_struct_return` (P122 lift) |
| `src/stack.rs` | `size_code` (eval stack size), `loop_position` |
