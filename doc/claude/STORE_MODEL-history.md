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
