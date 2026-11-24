<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# VERIFICATION-history.md — the record of [VERIFICATION.md](VERIFICATION.md)

## The coroutine decided edge, closed with the rustc writer (moved on 2026-11-24)

The rustc writer (`--native-rust`, retired with `@C170`) lowered a generator loop eagerly where
it could not decompose the loop into states; the C backend resumes at the label after each
`yield` and is lazy for every shape.  The ledger row and the open item as they last stood:

- ✓/edge **G-Call / G-Next laziness** — STRAIGHT-LINE yields are lazy on BOTH backends
  (`a g1 b g2`). A loop body of MORE than one statement is lazy on interp, EAGER on native — a
  DECIDED EDGE (rustc restriction, CL-9, [loft#836](https://github.com/loft-lang/loft/issues/836)),
  NOT a bug. The edge is one STATEMENT wide, not one construct wide: `for x in it { yield x }` is
  lazy on native too (the `YieldFrom` segment), verified over a 1e9 range. *Guard:
  `oracle/26-coroutine-laziness` (straight-line).* A record yielded from an EAGER loop body is
  a per-yield SNAPSHOT, so a read sees the value at the yield on both backends; a write through
  the consumer's binding lands in the copy where the interpreter's live frame would take it —
  the same edge one row deeper, not a second one. *Guard: `tests/scripts/1356-…`.*

Still open after verification:

1. **CL-9 / the coroutine decided edge** — native eager loop yields. Removal DESIGN written
   ([COROUTINE_LAZY_YIELDS.md § lazy loop yields](../COROUTINE_LAZY_YIELDS.md)); needs the build.
