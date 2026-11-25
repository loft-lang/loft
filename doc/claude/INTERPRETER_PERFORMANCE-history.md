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

## Store work, interpreter against the rustc writer (@PLN225 retired the writer, 2026-11-24)

The method and table INTERPRETER_PERFORMANCE.md § Store work carried while a native program
linked the op-census runtime rlib:

`LOFT_STORE_CENSUS=<file>` (PROFILING.md) counts, at the chokepoints both backends share,
stores created and freed, records claimed, deleted, grown and relocated, and the bytes block
copies and text writes move — one line per `ticks()` call, so a bench routine's work is the
difference between the two lines around its timed loop.  Build the interpreter and the native
program against the `op-census` feature (the counters are compiled only there):

```bash
cargo build --release --lib --bin loft --features op-census --target-dir target/op-census
LOFT_STORE_CENSUS=i.tsv target/op-census/release/loft --interpret bench.loft --n 2
target/op-census/release/loft --native-emit n.rs --lean bench.loft
rustc -C opt-level=3 --edition=2024 --extern loft=target/op-census/release/libloft.rlib \
      -L target/op-census/release/deps -o n n.rs && LOFT_STORE_CENSUS=n.tsv ./n --n 2
```

Measured on `14_stdlib_vector`, per op (interpreter / native):

| routine | claims | grows | relocations | bytes relocated |
|---|--:|--:|--:|--:|
| `push` | 4 / 1 | 4 / 0 | 4 / 0 | 81,526 / 0 |
| `record_append` | 3 / 2 | 6 / 0 | 1 / 0 | 233,380 / 0 |
| `grid` | 392 / 137 | 260 / 4 | 4 / 4 | 1,210 / 824 |
| `copy`, `remove_front` | equal | equal | equal | equal — shared runtime work |
| element reads and writes | none | none | none | none on either side |
