<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/concurrency-history.md — the deviation register for [concurrency.md](concurrency.md)

> **The rules are next door.**  [concurrency.md](concurrency.md) states what must always be true
> of `par`; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.

OPEN: **0** (D-conc-1 and D-conc-2 opened and closed 2026-10-07).  The chapter read *"a rules
doc — adds no code deviation"* until the formal-rule walk of 2026-10-07 measured both.

### D-conc-2 — OPENED AND CLOSED (2026-10-07, loft#1918): a worker writing its input through a view compiled and halted at run time

C93 makes a write to a `par` worker's captured state a compile error at the write.  The check
followed a write rooted at the parameter itself and at a callee's parameter, but not one rooted
at a local VIEWING it — `x = e.inner; x.k = …`, `x = &e.inner`, `x = e.list[0]`, `for l in
e.list { l.k = … }` — so these compiled and stopped at run time on the read-only store, on both
backends, against C80 as well.  The check follows the local's dep chain to the parameter; a
whole-value copy has no dep and stays legal.  Guards
`1918-a-par-worker-writing-through-a-view-is-refused.loft` and its control
`1918b-a-par-worker-may-write-a-copy-of-its-input.loft`.

### D-conc-1 — OPENED AND CLOSED (2026-10-07, loft#1917): a pure worker reading a vector constant panicked the interpreter

`(C-Det)`: a pure worker answers what the sequential loop answers, for every N.  A worker
`State` started with an empty constant table, so a worker that read a top-level vector
constant indexed it and panicked on `--interpret` ("the len is 0 but the index is 825") where
`--native` answered.  The table travels with every worker program and through the parallel
context.  Guard `1917-a-par-worker-reads-a-vector-constant.loft`.
