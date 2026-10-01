<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# COROUTINE_LAZY_YIELDS-history.md — the timeline of [COROUTINE_LAZY_YIELDS.md](COROUTINE_LAZY_YIELDS.md)

The design doc states how lazy loop yields are lowered now; this file keeps how they got there.

## Moved from COROUTINE_LAZY_YIELDS.md on 2026-10-01

- **The generator tail.**  The eager factory dropped the statements after the last `yield`,
  which leaked every persistent heap local and lost a `print` after the loop (loft#1356).  It
  runs the tail once the buffer is filled since that fix.
- **Re-descent** was built as slices 2–3 of the CL-9 design (loft#1798); the section heading
  carried the slice numbers and the issue.
