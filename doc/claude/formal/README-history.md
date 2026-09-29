<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/README-history.md — superseded state lines of [README.md](README.md)

## Open-deviation counts moved out of README.md on 2026-09-29

The chapters' own `OPEN: n` lines and `scripts/rule_tags.py registers` are the count; these
restatements had drifted from them.

**Twelve deviations are open, in five chapters:** heap.md 5, closures.md 2, operational.md 2,
binding.md 2 and performance.md 1 (`rule_tags.py registers`, re-measured on the joined tree
2026-09-22, after heap.md's `D-heap-26` closed and `D-heap-28` to `D-heap-32`, tuples.md's
`D-tup-15` and coroutines.md's `D-cor-4` each opened and closed that day, coroutines.md's
`D-cor-3` and collections.md's `D-col-5` closed, and coroutines.md's `D-cor-5` opened with
`(G-Hold)`; re-measured again 2026-09-22, after heap.md's `D-heap-34` and `D-heap-35` opened and
closed with loft#1597, and coroutines.md's `D-cor-5` closed with loft#1601; and once more,
after heap.md's `D-heap-36` opened with loft#1600 as a design question, `D-heap-37` opened and
closed beside it, and `D-heap-38` opened; and closures.md's `D-clo-35` opened and closed with loft#1606, `D-clo-36` and `D-clo-37`
opened beside it).

- binding.md row: **1 open** (D-bind-38, `(B-Ref-Lvalue)`: a link to a TEXT place is refused, where the rule says it links; D-bind-39, the narrow-INTEGER face, closed 2026-09-23 with loft#1567) — 

- heap.md row: **1 open** (D-heap-9 — `H-Copy-Lease`'s `OpCopy` hook, @PLN163 P4; `H-Copy-Refuse` and `H-Spent` are compile errors since 2026-09-23) — 

- closures.md row: **2 open** (`D-clo-36` a returned closure's capture hooks, `D-clo-37` a loop-body capture's reused backing) — 
