# Interpreter performance — history

## The literal-append reservation (`@FR-R-FirstClaim`)

First form: every reservation of n <= 11 elements was skipped, because `pre_alloc_vector` then
claimed `max(n, 11)` — the append's own first claim.  Measured then: `push` 279 → 236 ms
(−16 %), a record append 623 → 578 ms (−7 %).  Once a known count was reserved exactly
(`@FR-Col-Capacity`), skipping it changed the first block (`[1, 2]` took 12 words where the C,
keeping every reservation, took 3 — 60 % of the programs the interpreter/C allocation
differential reported different), so the skip was narrowed to the one count where the claims
agree.  Measured on the narrowing (interleaved, benches 14 `push`, `record_append`,
`comprehension`, `grid`): no separable difference with the skip on or off.

