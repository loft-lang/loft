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
| D6 | equality through a `reference<T>` to a removed record: the interpreter follows the raw slot, C reads null (`@FR-H-Removed` says C is right) | closed — 71f0b4476: the interpreter reads through `get_db_ref` (`tests/scripts/a-reference-to-a-removed-record-compares-as-null.loft`) |
| D10 | a locked write ends a production run in C; the interpreter discards it and continues | closed — 32128291d: C discards it whole and logs it (`@FR-H-WriteLocked-Whole`); the walk-removal hang it exposed on all three backends is loft#2235 (`a_locked_write_in_production_is_discarded_on_every_backend`) |
| D19c | no store-memory ceiling on C test runs | closed — 68ff7debc: the runner hands its ceiling to a compiled child, both writers (`the_default_ceiling_reaches_a_compiled_test`) |
| D19h | the displaced list of a linked group is capped at 16 in C | closed — a860d16ad: sized by the member count (loft#2257; loft-suite `Col-Group-Dup/2257-every-displaced-record-leaves-a-group-of-many-members.loft`) |
| D19e | NaN and ±0 sort in different orders | closed — e5dfe58f7 (loft#2236): both were wrong — a null (a NaN) landed anywhere, a null text after `""`.  Both now sort as `<` orders (`@FR-E-NullArg`, null first) and stably, so the zeros keep their order (`tests/scripts/sort-places-null-first-and-keeps-equal-floats-in-order.loft`); the keyed-collection half is loft#2237, closed by ba7fd5451 (null keys first in every keyed collection, old images converted; `tests/scripts/a-null-key-orders-first-in-every-keyed-collection.loft`) |

## Closed in main by #2251 (7e4917203, the laptop-c-perf arc)

| # | Divergence | Closed |
|---|---|---|
| D7b | `loft_swap_in` exchanged locked or foreign stores | closed — 7e4917203: `Store::content_swappable`'s gate, only the released store's protection refuses |
| D7c | `loft_reset_pool_root` re-minted a locked, file-backed or foreign store | closed — 7e4917203: such a store is walked, not re-minted |
| D9a | `loft_keep_vector_range` released nothing from the dropped elements | closed — 7e4917203: each dropped element's claims released first, a linked one's record removed |
| D9b | `loft_vec_replace` / `_array` cleared without releasing | closed — 7e4917203: the clear releases (`@FR-H-ClearRelease`) |
| D9c | `loft_database_wrapper` zeroed the slot without `remove_claims` | closed — 7e4917203: the place's collection released, then emptied |
| D9d | a dying `File` never closed its handle in C, and a copy never leased it (`@FR-H-Handle`) | closed — 7e4917203: a `File` record leases its handle, closed when the last lease dies |
| D12 | the FFI claim callback zeroed in C and not in the interpreter | closed — 7e4917203, with D17: `loft_ffi_claim` is a plain `loft_claim` |
| D13 | `loft_append_text_bytes` worked per byte | closed — 7e4917203: one reservation, one copy, one length |
| D14 | `loft_vec_slice` worked per element | closed — 7e4917203: one block copy, into the room the vector has or after one growth |
| D15 | copying a text field malloc'd a temporary in C | closed — 7e4917203 |
| D16 | `OpReplaceKeyed` rebuilt every entry in C | closed — 7e4917203: a given-up source's store swaps in (`@FR-H-SwapIn`'s keyed clause) |
| D17 | `loft_claim` zeroed every runtime claim where `@FR-H-Claim` promises nothing | closed — 7e4917203: no claim zeroes; only a discarded claim (`loft_st_claim_discarded`) answers a zeroed fresh record |

## Closed on tuxedo-pr2266-next (@PLN206 P2, this stream)

| # | Divergence | Closed |
|---|---|---|
| D23 | mmap-backed stores existed only in the interpreter | closed — `src/c/loft_mapped.h` ports `Store::open_mapped` (create at 8192 bytes, reopen with the recorded length cleared and the side state rebuilt, a read surface left untouched, growth and the cut with the file, unmapped on release), POSIX and Windows; `tests/c_emit.rs` `a_store_bound_to_a_file_holds_the_same_bytes_on_both_runtimes` runs one sequence through both on a file each, reopens the other's, and compares every answer and both files byte for byte.  Binding one into a unit's store table is P4's (the call arena a C worker maps) |
| D31 | a store the interpreter frees stayed live at exit in C: (a) a default lowered into a function of its own (`__dflt_*`) was run as a test on EVERY backend — the interpreter counted "2 passed" for one test; (b) the C test harness dropped a test's returned store; (c) a failed test's `longjmp` skipped its frames' store frees | closed — on tuxedo-pr2266-next: `Definition::is_compiler_written_fn` at every runner's selection, the C test `main` frees a returned store or text and releases every store a failed test made (`loft_release_stores_since`, as the interpreter's per-test fork does); found and verified by `c_sweep.py --strict-stores` (699, 703, 2090 agree) |
| D34 | `LOFT_STRICT_STORES` makes the TOP freed slot again (`trim_free_top`): a stale read of the most recently freed top store goes unreported (loft#2272's call-twice shape), and retiring it too would hide the stale free the 1522 guard's control relies on | decided — kept as is, on both backends (owner: the strict mode is not core functionality; such bugs are found by other means).  DEBUG_STORES.md states the exception |
