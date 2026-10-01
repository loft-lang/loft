<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# NATIVE_SWITCHES-history.md — the timeline of [NATIVE_SWITCHES.md](NATIVE_SWITCHES.md)

[NATIVE_SWITCHES.md](NATIVE_SWITCHES.md) states what each switch does now; this file keeps how each got there.

## Older dated lines moved from NATIVE_SWITCHES.md on 2026-10-01

- **`LOFT_NO_VECTOR_BASE`.**  Since 2026-09-21 a mint left on its templates counts as growth: a
  base bound beside one answered `null` on native.  Since 2026-09-18 a null-discharge buffer's
  mint does not count.
- **`LOFT_NO_PUSH_WINDOW`.**  The window took RECORD mint groups from 2026-09-22.
- **`LOFT_NO_INVARIANT_HOIST`'s verify form** found loft#1534, a by-reference argument the
  non-sentinel proof's escape collector could not see.
- **The range proof (`LOFT_NO_RANGE_ARITH`).**  Its counted-range-counter clause was INERT until 2026-09-21 (loft#1558):
  the seeding looked for the counter's seed INSIDE the loop, so no counted loop was ever ranged.
  Nothing showed it, because the pin that should have (`range_arith` a4) was recording a plain
  form the CHAIN GUARD supplied.  The proof read the STATIC TYPE from 2026-09-22, once loft#1593
  made every store into a narrow slot refuse an unprovable value.
- **`LOFT_NO_VALUE_RECORD`** went default-ON 2026-09-14.  It was opt-in for two days because its
  call-site gate did not hold over the script corpus (376 compile errors, 218 of them the
  fn-ref DISPATCH); a `__lift_` temp bound from a CALL was declined by shape until
  `(R-ValueLocal)` made it a value local.  A `__lift_` temp bound from a VIEW read as a view
  leaf leaked the store its copy mints, one record per call; found 2026-09-15 with the generic
  instance's statement join, whose arms bind the join local the same way and did not compile.
- **`LOFT_NO_VALUE_LOCAL`** went default-ON 2026-09-25.
- **`LOFT_NO_VIEW_FIELD`** and **`LOFT_NO_FORWARD_TUPLE`** went default-ON 2026-09-17.
- **`LOFT_NO_ELEMENT_FIRST` / `LOFT_NO_ELEMENT_PLACE`.**  A read through an element view between
  the declaration and the append was loft#1553; a local declared `[]` and rebound under an `if`
  left the element empty (loft#1552).
- **Null-discharge buffers (`LOFT_NO_NULL_BUFFER_HOIST`).**  The buffer's allocation declined every header in the loop around
  it — the drawing bench's polygon crossing loop hoisted nothing; since 2026-09-18 it does not
  block the loop's BASES either.
