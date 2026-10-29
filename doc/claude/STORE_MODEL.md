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
claims, frees, grows, parks and locks them.  The bytes are the same on both sides — header
words, record sizes, content — which is why a store image moves between backends.  The
bookkeeping is the shared core below.

| Concept | Interpreter (`Store`) | C runtime (`loft_store_t`) | State |
|---|---|---|---|
| the buffer | `ptr` | `buf` | same |
| its size in words | `size` | `size` | same |
| claimed blocks | `claims` | `claims` | same |
| records a stored reference named (`@FR-H-Removed`) | `referenced` | `referenced` | same |
| removed records that keep their header | `removed` | `removed` | same |
| the claimed high-water mark | `claimed_end` | `claimed_end` | same; reset on init differs — D19a |
| the free tree's root | `free_root` | `free_root` | same |
| the wilderness end | `wild` | `wild` | same |
| words lost to fragments | `dead_words` | `dead_words` | same |
| a coalesce pending | `needs_coalesce` | `needs_coalesce` | same; reset on init differs — D19a |
| lazy frees, and the set of them | `lazy`, `lazy_free` | `lazy`, `lazy_free` | same |
| every byte written once | `defined` | `defined` | same |
| parked blocks (`@FR-H-Park`) | `park`, `parked` | `no_park`, `parked_set`, `parked_heads`, `parked_count` | D20 |
| live or free | `free` | `live` | same, opposite sense |
| read only, and who locked it | `read_only`, `user_locked`, `lock_origin` | `read_only` | D21 |
| a free that must wait (calls holding it) | `free_protect_depth` | `protect` | D22 |
| pinned (never freed) | `pinned` | `pinned` | same |
| the type of its first record | `known_type` | `known` | same |
| allocation serial | `alloc_serial` | `serial` | same |
| records placed from a foreign source | `placed_foreign` | `placed`, `nplaced`, `placed_cap` | same |
| a foreign store (`file_map`, adopted) | `foreign` | `foreign` | same (locking converged, D1) |
| backed by a mapped file | `file` (`MmapStorage`) | — | D23 (decided: C gains it) |
| held by a live generator frame (`@FR-G-Arg-Rebind`) | — (asked elsewhere) | `frame_holds`, `orphaned` | D24 |
| allocator policy flags | `wilderness`, `carve`, `move_on_grow`, `borrowed` | — (always on, or absent) | D25 |
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
[STORE_MODEL-history.md](STORE_MODEL-history.md) with its commit; D1–D5, D7a and D29 are there.

### Correctness

| # | Divergence | State |
|---|---|---|
| D6 | equality through a `reference<T>` to a removed record: the interpreter follows the raw slot, C reads null (`@FR-H-Removed` says C is right) | open — the interpreter moves |
| D7b | `loft_swap_in` exchanges locked or foreign stores | fixing — laptop-c-perf |
| D7c | `loft_reset_pool_root` re-mints a locked, file-backed or foreign store | fixing — laptop-c-perf |
| D8 | the swap-in gate is narrower in C (walk scratch, constant, lazy binding), and refuses a protected destination where Rust tests the source | open |

### Leaks

| # | Divergence | State |
|---|---|---|
| D9a | `loft_keep_vector_range` releases nothing from the dropped elements | fixing — laptop-c-perf |
| D9b | `loft_vec_replace` / `_array` clear without releasing | fixing — laptop-c-perf |
| D9c | `loft_database_wrapper` zeroes the slot without `remove_claims` | fixing — laptop-c-perf |
| D9d | a dying `File` never closes its handle in C, and a copy never leases it (`@FR-H-Handle`) | fixing — laptop-c-perf |

### Contract and diagnostics

| # | Divergence | State |
|---|---|---|
| D10 | a locked write ends a production run in C; the interpreter discards it and continues | open |
| D11 | no `WriteDropped` diagnostic in C for a write through a null holder | open |
| D12 | the FFI claim callback zeroes in C and not in the interpreter | open — decided by D17 |

### Hot paths

| # | Divergence | State |
|---|---|---|
| D13 | `loft_append_text_bytes` works per byte; the interpreter grows once and copies once | open — C moves |
| D14 | `loft_vec_slice` works per element; the interpreter makes one block copy | fixing — laptop-c-perf |
| D15 | copying a text field mallocs a temporary in C | fixing — laptop-c-perf |
| D16 | `OpReplaceKeyed` rebuilds every entry in C (O(n)); the interpreter swaps stores (O(1)) | fixing — laptop-c-perf |
| D17 | `loft_claim` zeroes every runtime claim (about 27 sites) where `@FR-H-Claim` promises nothing | fixing — laptop-c-perf (owner: no zeroing of whole regions) |
| D18 | a vector copy walks elements that own no heap | open — C moves |

### Edges

| # | Divergence | State |
|---|---|---|
| D19a | `loft_st_init` resets `claimed_end` and `needs_coalesce`; the interpreter does not | open — is the interpreter's stale value intended? |
| D19b | the allocator's switches are hard-wired in C | P0b |
| D19c | no store-memory ceiling on C test runs | P0b |
| D19d | `loft_claim_grow` does not check for u32 overflow | open |
| D19e | NaN and ±0 sort in different orders | open |
| D19f | clear-release skips the store reset on an already-empty vector | open |
| D19g | slice-view and adopt make a view on a locked destination | open |
| D19h | the displaced list of a linked group is capped at 16 in C | open |
| D19i | the store minted for an FFI vector answer leaks on a null answer in C | open |
| D19j | FFI record answers are refused in C (C-MISSING) | open |

### From the field comparison and P0b

| # | Divergence | State |
|---|---|---|
| D20 | parking: one `park` switch and a `Parked` set against `no_park` and a bitset with per-size heads | open — compare under the differential |
| D21 | who locked a store: the interpreter records `user_locked` and `lock_origin` for its report | open — observability, P0b |
| D22 | free protection: a depth counter against a protect count — one meaning, two spellings | open — one name |
| D23 | mmap-backed stores exist only in the interpreter | decided (owner): C gains them, the interpreter's model kept |
| D24 | a store held by a generator frame: C marks it (`frame_holds`, `orphaned`), the interpreter answers elsewhere | open |
| D25 | allocator policy flags (`wilderness`, `carve`, `move_on_grow`, `borrowed`) are per-store in the interpreter, fixed in C | open — with D19b |
| D26 | the interpreter's stack lives in a store; C's is the C stack | open — not a store property for a C unit; P4 decides |
| D27 | the interpreter's store instruments (generation, recording, tag, timestamps, init shadow) | P0b |
| D28 | the store table: a `Vec<Store>` beside the type table against a thread-local array | open — P3 |

## How a divergence is closed

1. **A probe that fails on the divergence** — a program, or a store-operation trace, whose
   answer differs between the two implementations today (both backends, `LOFT_POISON` where a
   freed byte matters).
2. **The decision** — C's model, or the interpreter's with the measurement that makes it better
   (the performance agent's P1 numbers: `bench/stats.py`, the allocator counters).
3. **The change**, on the losing side, after which the probe agrees and the differential is
   green on the corpus.
4. **The row moves** to the history companion with its commit, and the probe stays as a guard.
