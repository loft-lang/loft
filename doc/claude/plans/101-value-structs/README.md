<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 101 — Zero-cost value structs

> **Status — DONE / SHIPPED 2026-07-08.** `value struct` is implemented, proven zero-cost on
> both backends, and lands via the finishing PR (`Closes @PLN101`). User-facing syntax:
> [LOFT_DATA.md § Value structs](../../LOFT_DATA.md). Per-slice build log: [STEPS.md](STEPS.md).

## What shipped

A `value struct T { … }` kind with **value (copy) semantics** that is **zero-cost as a record
field and as a vector element** — the cases the user flagged as mandatory ("these types will
normally only be used as parts of other records or directly inside vectors; those cases need
zero overhead"). It is the zero-cost-abstraction half of first-grade types (@PLN99 shipped the
*semantics* — operators/format/conversions; this shipped the *cost*).

Proven: a `vector<value struct>` and a value-struct **field** of a record allocate a **constant
number of stores, flat in N** (4, equal to the reference-struct baseline, one above the raw
scalar backing) — no per-element / per-field allocation. Full first-grade `DateTime` composes on
both backends. See the regressions:

- `tests/scripts/515` — `DateTime`/`Duration` as `value struct`: construction, formatting,
  operators, conversions, `now()` — both backends, value semantics.
- `tests/scripts/516`–`518` — copy semantics (field/element/loop read), lifetime-bearing
  (text/vector) fields, local-to-local.
- `tests/scripts/519` + `tests/value_struct_alloc.rs` — the zero-cost proof (O(1) allocs, flat
  in N, parity with a reference struct) and its elision-soundness boundary.

## How it was built (the load-bearing decision)

**A value struct is an ordinary `Type::Reference` record, marked only by
`Data.value_structs: HashSet<u32>`.** The originally-planned distinct `Type::Value` variant
(packed at `base + offset`, DbRef dropped) was **tried and reverted** (commit 1240f459) — it
caused a broad `if let Type::Reference` blast radius across runtime sites plus an unbounded
embedded-size refinement obligation. The user's directive settled it: *"can you make this less
complex, without wiring it in directly."*

What actually delivers the semantics + the zero cost is **one isolated ~250-line IR pass,
`scopes::value_struct_copy`** (runs in `check`, early-returns when a program has no value
structs — so non-value-struct code is untouched):

1. **Storage is already inline** — no work needed. `finish_type` sizes a struct field by the
   nested record's full bytes, and a `vector<V>` inlines its elements into one backing (reference
   structs inline too). So a value-struct field/element costs no separate allocation out of the
   box.
2. **Value (copy) semantics** — a value-struct bind *from a view* (`e = rec.f` / `e = vec[i]`,
   ownership `Borrowed`) is rewritten to `OpDatabase` + `OpCopyRecord`: the local mints its own
   store and deep-copies, so mutating it can't write back through the view. Sound with no
   ownership-oracle change — the emitted `OpDatabase` makes the local classify `Owned` on its
   own via `db_vars`. `OpCopyRecord` already deep-copies inner `text`/`vector` handles, so
   lifetime-bearing value structs work with no extra machinery.
3. **Zero-cost read-only elision** — the copy is *skipped* (the bind stays a zero-cost view, like
   a reference struct) when a plain view is observably identical to a copy: neither the local nor
   its projection base can be mutated under the view or escape it. This is what makes
   `for p in ps { …read… }` cost nothing. The soundness oracle is a **tainted** set — seeded from
   field/element writes + escapes, closed over pure-`Var` alias edges, and rescoped per loop body
   (construction runs before the loop, so it can't diverge an in-loop read). loft permits aliasing
   writes, so this boundary is exact.
4. **Non-null** — a `value struct` is inline bytes with no null sentinel, so `value struct?` is
   rejected at `parse_type`, and DN1 already rejects an uninitialised non-null slot.

No `Type` variant, no size-function change, no assignment-path surgery, no ownership-oracle
change. Recipe + per-slice detail: [STEPS.md](STEPS.md).

## Slices (all done)

| Slice | Shipped |
|---|---|
| **0** | alloc harness (`LOFT_ALLOC_REPORT`, `Stores::stores_allocated`) — and it corrected the cost map: the cost was transient copies, not storage |
| **1** | `value struct` fields + vector elements via the isolated copy pass; `515`/`516` |
| **2–3** | zero-cost inside records + `vector<V>` (storage already inline) |
| **4** | lifetime-bearing (text/vector) fields — supported via `OpCopyRecord` deep-copy; `517` |
| **1.4** | non-null enforcement (`value struct?` rejected) |
| **5** | the zero-cost proof + read-only copy elision; `519` + `tests/value_struct_alloc.rs` |

## Closed design questions

- **Representation** — value structs stay `Type::Reference` + an isolated pass (not `Type::Value`).
- **Nullable `value struct?`** — value structs are non-null; rejected at the type suffix.
- **Standalone local cost** — kept simple by user directive: a standalone local may own its store
  (negligible; a loop reuses the slot). The zero-cost target was fields + vector elements, met.
- **Keyword** — `value struct`.
- **C65** (DESIGN_DECISIONS.md) stays accurate: loft still has no `Type::Value` *distinct from*
  `Reference` — the value-struct kind is a marker + copy pass over `Reference`, not a new variant.

## Follow-on (no plan needed)

Flipping the *actual* installable DateTime **library** from `struct` to `value struct` is consumer
adoption (its own repo/PR + `loft api`), not part of this language plan — `515` is a standalone
prototype that already proves the feature. Any residual polish is bug-fix-level work.

## See also

- Supersedes **@PLN99 Arc D** (the deferred "someday-perf" framing).
- Mechanism: `src/scopes.rs` (`value_struct_copy`), `src/parser/{definitions,mod}.rs`, `src/data.rs`.
- Reference: [LOFT_DATA.md § Value structs](../../LOFT_DATA.md), [STEPS.md](STEPS.md), DESIGN_DECISIONS.md C65.
- Tracker: [@PLN101](https://github.com/loft-lang/plans/issues/101).
