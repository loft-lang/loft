<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# facts.md — history

The record companion of [facts.md](facts.md): closed deviation entries and what closed them.

## Deviations carried by facts.md, @PLN211

### D-fact-2 — CLOSED 2026-11-06 (@PLN211) — interval entry facts exist only in native's writer
- **Violates:** `Fact-Engine` (every writer reads the same answers)
- **Where:** `src/generation/ranged_call.rs` (the seeding, the guarded copy, `__rg`)
- **Effect:** the C writer has no `(R-RangedCall)`: 384 native admissions on the benches, none
  on the C (`bench/portal/analysis/c-libraries.md` § The census against rustc).
- **Removal:** the `entry` kind's interval domain in the routine; `__rg` a twin key of
  `Fact-Twin`, emitted by both writers.
- **Closed by:** the interval entry facts moved out of native's writer into
  `opt::ranged_call` — the gain test, the argument and scalar-input tests, the seeds and the
  guarded copy's plan (`guard_plan`) — read by both writers; the C emits `__rg` twins
  (`c::RG_BIT`) and guarded copies.  The C's census: `R-RangedCall` 0 → 705.  Not as a kind of
  `facts(kind, reach)` (that is D-fact-1's handle), and `__rg` keyed by the C's own mask bit,
  not a `Fact-Twin` registry (D-fact-3).

### D-fact-4 — CLOSED 2026-11-18 (@PLN211) — no kind has its run-time falsifier
- **Violated:** `Fact-Oracle` as it then read ("every kind has a falsifier that runs the
  program"), and through it `Fact-ReachMin`'s guard.
- **Where:** `LOFT_FACT_VERIFY` was read nowhere in `src/`; the interpreter checked no `entry`
  fact on entry, no result fact at return and no `writes` / `mints` summary against the call.
- **Closed by a change of the rule, not by code** (owner): a standing falsifier for every kind
  asks for the whole fact engine first — five of the nine kinds are kinds of the routine, some
  answers depend on the asker, and the `writes` fact is keyed by record type where a run sees
  a store.  And a run-time self-check is refused outright (owner): validation is the suite,
  the dogfood programs and the libraries, never a flag under which the compiler checks
  itself.  The rule now asks a widened reach for a suite cell compared with the rewrite's
  switch off, and for the libraries and consumers re-validated.  The suite ledger records
  `Fact-Oracle` as a process rule with no cell of its own.

