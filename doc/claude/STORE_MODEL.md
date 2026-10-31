<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The store model

loft has one heap model, implemented twice: by the interpreter (`src/store.rs`,
`src/database/`) and by the C runtime (`src/c/loft_store.h`, `src/c/loft_rt.h`, `loft_keyed.h`,
`loft_btree.h`, `loft_hash.h`).  This page states the model once and lists where the two
implementations still differ.  Two implementations that answer differently are a defect
(@PLN206), so every row below either converges or is a decision with its reason.

**The rule for a difference.**  The C runtime's model is the standard.  Where the two differ,
the interpreter moves to the C one, unless the interpreter's is measurably better (faster,
smaller, or stricter at no cost); then the C runtime takes the interpreter's.  Each decision
names its measurement.

## Contents
- [The store](#the-store) — the record a store is, field by field
- [The store table](#the-store-table)
- [The operations](#the-operations) — where each family lives on both sides
- [Divergences](#divergences) — the numbered list, and what each one's state is
- [How a divergence is closed](#how-a-divergence-is-closed)

## The store

A store is one growable buffer of 8-byte words holding records, plus the bookkeeping that
claims, frees, grows, parks and locks them.  The buffer is the loft file layout, defined in
[formal/structures.md](formal/structures.md) `S-Live`: every block's bytes — a record's header,
a free block's header, footer and tree links, a parked block's links, a tombstone — are the
same on both sides and in a mapped file.  The bookkeeping kept BESIDE the buffer (`S-Side`) is
the shared core below.

| Concept | Interpreter (`Store`) | C runtime (`loft_store_t`) | State |
|---|---|---|---|
| the buffer | `ptr` | `buf` | same |
| its size in words | `size` | `size` | same |
| claimed blocks | `claims` | `claims` | same |
| records a stored reference named (`@FR-H-Removed`) | `referenced` | `referenced` | same |
| removed records that keep their header | `removed` | `removed` | same |
| the claimed high-water mark | `claimed_end` | `claimed_end` | same |
| the free tree's root | `free_root` | `free_root` | same |
| the wilderness end | `wild` | `wild` | same |
| words lost to fragments | `dead_words` | `dead_words` | same |
| a coalesce pending | `needs_coalesce` | `needs_coalesce` | same |
| lazy frees, and the set of them | `lazy`, `lazy_free` | `lazy`, `lazy_free` | same |
| every byte written once | `defined` | `defined` | same |
| parked blocks (`@FR-H-Park`) | `park`, `parked` | `no_park`, `parked_set`, `parked_heads`, `parked_count` | same lists, bytes proven alike (D20) |
| live or free | `free` | `live` | same, opposite sense |
| read only, and who locked it | `read_only`, `user_locked`, `lock_origin` | `read_only` | same lock; who locked it is the interpreter's report (D21) |
| a free that must wait (calls holding it) | `free_protect_depth` | `protect` | same count, two names (D22) |
| pinned (never freed) | `pinned` | `pinned` | same |
| the type of its first record | `known_type` | `known` | same |
| allocation serial | `alloc_serial` | `serial` | same |
| records placed from a foreign source | `placed_foreign` | `placed`, `nplaced`, `placed_cap` | same |
| a foreign store (`file_map`, adopted) | `foreign` | `foreign` | same (locking converged, D1) |
| backed by a mapped file (`@FR-S-Mapped`) | `file` (`MmapStorage`) | `mapped`, `map_file` (`loft_mapped.h`) | same (D23) |
| held by a live generator frame (`@FR-G-Arg-Rebind`) | — (asked elsewhere) | `frame_holds`, `orphaned` | D24 |
| allocator policy flags | `wilderness`, `carve`, `move_on_grow`, `borrowed` | compile-time switches (`-DLOFT_NO_*`) | D25 closed; `wilderness` is D35 |
| the interpreter's stack | `stack_buffer` | — (the C stack) | D26 |
| instruments | `generation`, `recording`, `tag`, `created_at`, `last_op_at`, `init_shadow`, `parse_errors`, `parse_type`, `store_nr` | — | D27 |

## The store table

| | Interpreter | C runtime |
|---|---|---|
| the stores | `Stores.allocations: Vec<Store>` | `loft_stores`, `loft_nstores` (thread-local) |
| the type table | `Stores.types`, `names` | the unit's generated tables |
| a reference | `DbRef { store_nr, rec, pos }` | `loft_ref { store_nr, rec, pos }` |

The reference is the same three parts on both sides.  The table itself is D28.

## The operations

The full operation-by-operation pairing (179 operations: 128 SAME, 44 DIVERGES, 7 C-MISSING)
is on the plan, [loft-lang/plans#206](https://github.com/loft-lang/plans/issues/206) (the two
pairing comments).  The families, and where each lives:

| Family | Interpreter | C runtime |
|---|---|---|
| claim, free, grow, coalesce, park | `store.rs`, `database/allocation.rs` | `loft_store.h` |
| records, copies, claims of a copy | `database/structures.rs`, `copy_claims` | `loft_rt.h` |
| vectors, slices, reverse | `vector.rs`, `database/structures.rs` | `loft_rt.h` |
| text fields | `database/structures.rs` | `loft_rt.h` |
| keyed: hash, index, sorted, radix | `hash.rs`, `tree.rs`, `state/io.rs` | `loft_keyed.h`, `loft_hash.h`, `loft_btree.h` |
| locks, foreign stores, images | `store.rs`, `database/` | `loft_store.h`, `loft_image.h` |
| FFI bridging (`bridge_args`) | `extensions.rs`, `database/structures.rs` | `c/mod.rs` (emitted) |

## Divergences

D1–D19 are the pairing's ranked list; their numbers are kept so the plan, the pairing and the
fixes name a row the same way.  D20 onward come from the field-by-field comparison above and
from P0b (what the C runtime lacks).  State: **open**, **fixing** (on the branch named), or
**decided** (the decision and its measurement).  A closed row moves to
[STORE_MODEL-history.md](STORE_MODEL-history.md) with its commit; D1–D8, D9a–D17,
D18–D19j, D20–D23, D25, D29, D31 and D34 are there.

### Correctness

| # | Divergence | State |
|---|---|---|

### Contract and diagnostics

| # | Divergence | State |
|---|---|---|
| D36 | a recoverable fault's Warn record (`@FR-E-Report`): the C writes none for an index out of range (`index_out_of_bounds`, `negative_index`), a value outside a field's range (`range_defaulted`), a division by zero, a cast or a shift out of range — 138 corpus programs log differently (`c_sweep.py --log`; 78 of them first at an index), so a write through an out-of-range element reports `write_dropped` in C where the interpreter reported the index | open — the reporting is D11's (`loft_fault_report`, the statement's file and line as constants); each faulting operation takes an `_at` form, its guarded `*Nullable` twin none |

### Edges

| # | Divergence | State |
|---|---|---|

### From the field comparison and P0b

| # | Divergence | State |
|---|---|---|
| D24 | a store held by a generator frame: C marks it (`frame_holds`, `orphaned`), the interpreter answers elsewhere | open |
| D26 | the interpreter's stack lives in a store; C's is the C stack | open — not a store property for a C unit; P4 decides |
| D27 | the interpreter's store instruments (generation, recording, tag, timestamps, init shadow) | P0b |
| D28 | the store table: a `Vec<Store>` beside the type table against a thread-local array | open — P3 |
| D33 | a call result rebound in a loop (`x = mul(a, x)`): native rotates the call's return buffer (`__rbb`, `@FR-O-Buffer`) and makes no store; the C adopts the result and frees the old store, so it makes and frees one store per iteration; the interpreter exchanges (`@FR-H-SwapRebind`) and also makes one, freeing the TOP slot.  Under `LOFT_STRICT_STORES` the C therefore runs out of numbers in a long loop (`c_sweep.py --strict-stores` files it as beyond the mode) | open — C moves (`@FR-R-RebindBuffer` in the C writer; the performance stream's).  Measured by hand on the emitted unit: the rotation saves about 7 % on a 3 000 000-iteration `x = mul(a, x)` loop at `-O3` (0.14 s → 0.13 s, same answer) — a C store's make and free are cheap |
| D30 | an image of another layout: the interpreter converts it (`Stores::upgrade_store`), C refuses it by name (`loft_image.h`) — except the null-first key order, which C converts by itself (`loft_rekey_walk`) | open — P0b (port), the key-order case done |

## How a divergence is closed

1. **A probe that fails on the divergence** — a program, or a store-operation trace, whose
   answer differs between the two implementations today (both backends, `LOFT_POISON` where a
   freed byte matters).
2. **The decision** — C's model, or the interpreter's with the measurement that makes it better
   (the performance agent's P1 numbers: `bench/stats.py`, the allocator counters).
3. **The change**, on the losing side, after which the probe agrees and the differential is
   green on the corpus.
4. **The row moves** to the history companion with its commit, and the probe stays as a guard.
