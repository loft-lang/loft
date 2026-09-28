<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 0 — Characterize current slot-assignment behaviour

**Status:** open.

**Goal:** lock the existing placement decisions with tests that any
future allocator must reproduce.  Without this, Phase 2's equivalence
check (V1 vs V2) proves nothing — it would just mean "V2 agrees with
V1 in the cases V2 authors happened to think of."

## What we do

1. **Enumerate call sites.**  Every path that reads a `stack_pos` or
   calls an allocator helper.  Build the list by grepping:
   - `assign_slots`, `process_scope`, `place_large_and_recurse`,
     `place_orphaned_vars` (in `src/variables/slots.rs`).
   - `stack_pos` readers in `src/state/codegen.rs` and
     `src/generation/`.
   - The `set_stack_pos` assertion in `src/variables/mod.rs`.

2. **Exhaustive fixture catalogue.**  Every pattern currently
   documented in [SLOTS.md § Known Patterns](../../../SLOTS.md)
   becomes an explicit `tests/issues.rs::slot_v2_fixture_*` assertion
   that reads the assigned slot and compares against a hard-coded
   expected value.  Patterns to cover (initial list — extend as we
   discover more):
   - Many parent refs + child loop index (today: `parent_zone2_does_not_overlap_child_zone1`).
   - Call with Block arg (coroutine pattern).
   - Insert preamble (P135 lift).
   - Sequential lifted calls.
   - Parent var Set inside child scope.
   - Text block-return.
   - Sibling scope reuse (two blocks share the same frame area).
   - Comprehension then literal (P122p).
   - Sorted range comprehension (P122q).
   - Par loop with inner for (P122r).
   - **P178 repro** — `is`-capture in Insert-rooted body.
   - **P185 repro** — late local after inner text-accumulator loop.
   - **Loop-scope vars** — place sequentially, don't reuse zone 1.
   - **Zone 1 reuse** — two non-overlapping integers in the same block
     share a slot.

3. **Pin the "why this placement" rationale** for each fixture in
   the test's doc comment.  If the rationale reads "because
   `place_orphaned_vars` starts at `local_start`," that's a flag
   that V2 needs to preserve the mechanism *or* the fixture needs
   to be rewritten so the test states the invariant (overlap is
   forbidden) rather than the mechanism (slot is `local_start + 4`).

4. **Audit `place_orphaned_vars` callers.**  Document every branch
   that falls through to orphan placement (Insert-rooted bodies per
   P178; ... others to be catalogued).  Output: a table of "scope
   IR shape → does it hit the orphan placer?" so Phase 1 can design
   a single-pass algorithm that covers every shape without a fallback.

5. **Catalogue every size-or-shape branch in today's allocator.**
   Per the hard constraint in the README, V2 is not allowed to
   branch on variable size, scope kind, or set cardinality.  That
   means every such branch in V1 is a fixture target — V2 has to
   produce the same placement without the branch.  Grep for:
   - `size() > 8`, `size() <= 8`, `v_size > 8`, etc.
   - `Value::Block(_)` vs `Value::Loop(_)` dispatch in placement.
   - `place_orphaned_vars` call sites (the whole orphan path is a
     shape-dispatched branch).
   - `zone1_hwm`, `zone2_hwm`, `var_size` split points.
   - Any "if this scope has only one variable" / "if this scope
     has more than one variable" logic.

   Output: a table mapping each branch → the IR shape that exercises
   it → the fixture test that locks the observed placement.  Phase 1
   uses this table to prove the single-pass algorithm covers every
   row without re-introducing a dispatch.

## What we don't do in this phase

- No algorithmic changes to `assign_slots`.
- No deletions from `slots.rs` / `scopes.rs`.
- No changes to codegen.

Phase 0 is strictly additive: new tests, new documentation.  The
existing suite stays exactly as-is; if a new test fails against
today's allocator, it means the test has the wrong expected value
(or the allocator has a bug we just discovered — in which case file
it as PXXX before continuing).

## Deliverables

1. New test file `tests/slot_v2_baseline.rs` (or extension to
   `tests/issues.rs`) with ≥ 20 fixtures, each locking one
   placement decision.
2. Updated SLOTS.md § Known Patterns table referencing the new tests.
3. A new top-level section in SLOTS.md listing every scope-shape
   that currently hits `place_orphaned_vars`, with pointers to
   the fixture tests.
4. `cargo test --release` green before and after.

## Open questions (flag if discovered during fixture work)

- Are there variables whose slot is determined by codegen rather
  than `assign_slots`?  (If so, V2's scope widens.)
- Do `src/generation/` (native codegen) paths ever compute their
  own slots?  Or do they always read pre-assigned `stack_pos`?
- Can native codegen observe a different slot placement than the
  interpreter?  (If yes, Phase 2's equivalence check needs to cover
  both paths.)

## Rough size

Work: 3–5 sessions.  Output: mostly tests + documentation.

## Done when

- All fixtures in the SLOTS.md pattern table have an explicit
  `tests/slot_v2_baseline.rs` (or `tests/issues.rs::slot_v2_*`)
  entry.
- P178 and P185 have fixtures that lock the *observed* slot
  placement (not just the correctness assertion).  These will be
  the fixtures Phase 2 has to reproduce; when they do, we know the
  behaviour is preserved.
- `doc/claude/SLOTS.md` has a new § "Scope shapes and orphan-
  placement" that catalogues every way a local can fall through
  to `place_orphaned_vars`.
