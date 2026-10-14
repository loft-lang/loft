<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN129 — Lazy database-backed stores

**Status — SHIPPED 2026-08-06.** A collection binds to a store image or to
`sqlite:<path>` and faults on a miss, with the `SELECT` derived from the
collection's own type. Both backends.

**The reference content moved to [LAZY_STORES.md](../../LAZY_STORES.md)** — the
model, the derivation, the failure channel, the binding contract, what is refused,
and the open tails. The API is in [STDLIB.md](../../STDLIB.md); the feature entry
is [`@F108`](https://github.com/loft-lang/features/issues/108).

This directory is the closure record: how it was built, and what each step
answered.

## What shipped

| arc | delivered |
|---|---|
| **A** — the miss path | `store_bind_lazy(c, source)`; a miss fetches one record and keeps it |
| **B** — schema → query | equality on any keyed kind, and a key range as ONE query (`store_lazy_range`); nothing enumerated ahead of time |
| **B2** — the explicit query | `store_lazy_query(c, condition)` populates the COLLECTION, so a `LIKE` and a keyed lookup reach one record |
| **B3** — the mapping + schema check | `Mapping` (table/column overrides, quoting, placeholders) feeding one builder; the schema and INDEX interrogated once per binding, before any answer a program could believe |
| **B4** — collection-valued fields | an owner-parameterised query per collection, with no new language surface |
| **C** — the failure channel | `store_lazy_error` / `_faults` / `_clear`; unreachable told from absent, faults STICKY |
| **D** — one world per traversal | pinned at bind; drift refused and reported through C |
| **E** — a way down | `= []` reclaims, keeps the binding, preserves held references |
| **F** — the gate | the persons/companies graph over SQL: fetches == records touched, identity across paths |

**Effort:** H. **Closed:** 2026-08-06.

## The documents

- **[ARC-B-STEPS.md](ARC-B-STEPS.md)** — the implementation sequence: eleven steps
  ordered by RISK rather than dependency, what each one answered, and the three
  findings that changed the design.
- **[QUERIES.md](QUERIES.md)** / **[BINDING.md](BINDING.md)** — the design as it
  was argued, kept for the reasoning. What is TRUE of the built system is in
  [LAZY_STORES.md](../../LAZY_STORES.md); read these for why it took that shape.

## Two drafts that died, and why

Both were killed by measurement rather than taste, which is the useful part.

- **An image/page backing.** Ruled out by the owner: the database holds real rows
  that other tools query, so loft binds to a foreign relational schema rather than
  to its own bytes stored remotely. That single decision is what the whole
  derivation half exists for — and what separates this from the paged sibling
  ([REMOTE_STORES.md](../../REMOTE_STORES.md)).
- **A `(type, key) → rec` identity map.** Unnecessary: the resident collection
  already IS the cache, so identity falls out of the lookup that was going to
  happen anyway. Deleting it also deleted the divergence hazard a second structure
  brings.

A third was rejected on a count rather than a measurement: faulting at
`Store::addr` (N = 1, one site, both backends) lost to the collection's miss path,
because `valid()` compiles out in release — so the "one site" does not exist yet,
and creating it taxes every read in every program.

## What the plan asked that is now answered

| question | answer |
|---|---|
| How is "not resident" represented? | **Dissolved, not answered.** It is absence from the collection, which `find` already reports as `rec: 0`. No third block state, no cost in `valid()`. |
| Does `--native` route reads through the same accessors? | **Yes**, measured. The native `#rust` bodies call the same getters. The one bypass is the `#c` argument crossing, which is a stated contract rather than a second fault site. |
| What is a query, exactly? | One table per type, columns from the descriptor, `WHERE` from its keys; a reference is a foreign key followed by a further lookup. **No join has to be derived, because the traversal IS the join.** |
| Does eviction break the invariant? | No. The rec stays claimed; only latency changes. |

## See also

- [LAZY_STORES.md](../../LAZY_STORES.md) — the reference doc this plan produced.
- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) § @C104 — why a slice on a
  lazily-bound collection does not fetch.
- [loft-lang/plans#129](https://github.com/loft-lang/plans/issues/129).
