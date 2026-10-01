<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# COROUTINE_LAZY_YIELDS-history.md — the timeline of [COROUTINE_LAZY_YIELDS.md](COROUTINE_LAZY_YIELDS.md)

The design doc states how lazy loop yields are lowered now; this file keeps how they got there.

## Moved from COROUTINE_LAZY_YIELDS.md on 2026-10-01

- **The generator tail.**  The eager factory dropped the statements after the last `yield`,
  which leaked every persistent heap local and lost a `print` after the loop (loft#1356).  It
  runs the tail once the buffer is filled since that fix.
- **Re-descent** was built as slices 2–3 of the CL-9 design (loft#1798); the section heading
  carried the slice numbers and the issue.

## Older dated lines moved from COROUTINE_LAZY_YIELDS.md on 2026-10-01

- **The slices** were built under loft#836 (slice 1, the rotated loop), loft#1586 (`while`) and
  loft#1798 (slices 2–3, re-descent); the status line carried the three issue numbers.
- **Closures in a lazy loop.**  A lambda's fn-ref and its `___clos_*` record persist in the
  struct since loft#1587, which is what let a loop that builds a closure per iteration stay lazy.
- **The eager path, measured 2026-08-09.**  A `for i in 0..1000000000 { yield i; }` generator
  consumed three values and stopped, while `{ print("p{i} "); yield i; }` over `0..1000` ran all
  1000 iterations before the consumer's first advance.
