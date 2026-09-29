<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# SLOTS-history.md — the record behind [SLOTS.md](SLOTS.md)

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
