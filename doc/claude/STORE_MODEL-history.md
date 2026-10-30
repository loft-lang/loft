<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The store model — closed divergences

The rows of [STORE_MODEL.md](STORE_MODEL.md) that have converged, each with the commit that
closed it.  Numbers are kept: a closed number is never reused.

## Closed on laptop-c-perf (the performance agent's fix arc, @PLN206)

| # | Divergence | Closed |
|---|---|---|
| D1 | foreign stores are not locked in C (`loft_st_make_foreign` vs `Store::make_foreign`) | closed — 340d54eff (laptop-c-perf) |
| D2 | a narrow-integer native loses its null answer in C (sign-extended `int32_t`) | closed — 340d54eff (laptop-c-perf) |
| D3 | a foreign vector as an FFI call's first heap argument is handed over unchanged in C (`bridge_args` not ported) | closed — 340d54eff (laptop-c-perf) |
| D4 | `loft_vec_reverse` leaves elements wider than 64 bytes unreversed | closed — 340d54eff (laptop-c-perf) |
| D5 | the constant store is not locked in C | closed — 340d54eff (laptop-c-perf) |
| D7a | `loft_st_delete` performs a delete the interpreter refuses on a locked store | closed — 340d54eff (laptop-c-perf) |
| D29 | the C B-tree read a LEAF node's child words (never written) in `slot_of` during a split, merge or borrow; the interpreter's `slot()` reads them too, but its buffer is initialised | closed — 340d54eff (laptop-c-perf): child read only for an internal node, no zeroing |

## Closed on tuxedo-pr2229-next (@PLN206, this stream)

| # | Divergence | Closed |
|---|---|---|
| D6 | equality through a `reference<T>` to a removed record: the interpreter follows the raw slot, C reads null (`@FR-H-Removed` says C is right) | closed — 851411c46: the interpreter reads through `get_db_ref` (`tests/scripts/a-reference-to-a-removed-record-compares-as-null.loft`) |
| D10 | a locked write ends a production run in C; the interpreter discards it and continues | closed — 3ecd6e7fc: C discards it whole and logs it (`@FR-H-WriteLocked-Whole`); the walk-removal hang it exposed on all three backends is loft#2235 (`a_locked_write_in_production_is_discarded_on_every_backend`) |
| D19c | no store-memory ceiling on C test runs | closed — 28e980335: the runner hands its ceiling to a compiled child, both writers (`the_default_ceiling_reaches_a_compiled_test`) |
| D19e | NaN and ±0 sort in different orders | closed — 8b1b43c19 (loft#2236): both were wrong — a null (a NaN) landed anywhere, a null text after `""`.  Both now sort as `<` orders (`@FR-E-NullArg`, null first) and stably, so the zeros keep their order (`tests/scripts/sort-places-null-first-and-keeps-equal-floats-in-order.loft`); the keyed-collection half is loft#2237, closed by c555ae956 (null keys first in every keyed collection, old images converted; `tests/scripts/a-null-key-orders-first-in-every-keyed-collection.loft`) |
