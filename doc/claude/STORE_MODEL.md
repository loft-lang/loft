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

**A rewrite's difference is not a divergence.**  A rewrite the C writer applies and the
interpreter does not (`(R-PushFill)`, `(R-Apart)`, `(R-ValueRecord)`, …) may claim other room,
keep a value out of the store, or write in another place.  A program cannot observe any of it, so
it is the rewrite's, not the model's (@C122; a store trace reports the build, @C135).
`scripts/c_alloc_diff.py --attribute` runs a program that differs again with each C-only rewrite
switched off, and names the one under which it agrees; only what no switch accounts for is a row
below.  The model is per store: the order in which separate stores' events interleave (the
order a program's expressions run, a `par` region's threads) and the order a release walk
deletes one store's records are not part of it, and the tool names such a difference
`order only`.

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
| held by a live generator frame (`@FR-G-Arg-Rebind`) | `frame_holds`, `orphaned` | `frame_holds`, `orphaned` | same (D24) |
| allocator policy flags | `wilderness`, `carve`, `move_on_grow`, `borrowed` | compile-time switches (`-DLOFT_NO_*`) | D25 closed; `wilderness` is D35 |
| the interpreter's stack | `stack_buffer` | — (the C stack) | D26 |
| a parse's errors and type | `parse_errors`, `parse_type` | `loft_store_perrs` (by store: reset at a mint, exchanged with the content) | same (D27) |
| the interpreter's instruments | `generation`, `recording`, `tag`, `created_at`, `last_op_at`, `init_shadow` | the C's own: `loft_free_site`, `loft_leak_site` | decided (D27) |

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

**A third implementation measures both.**  The loft interpreter (@PLN91 1b) runs on the C
runtime's routines with the Rust interpreter's operator semantics, and
`selfhost/compare/compare.py --layers alloc` sets its store events beside the Rust
interpreter's, event by event; a difference there is a row here (or a port defect), and its
direction is decided as below ([plans/91-self-host/1b-reference.md](plans/91-self-host/1b-reference.md)).

D1–D19 are the pairing's ranked list; their numbers are kept so the plan, the pairing and the
fixes name a row the same way.  D20 onward come from the field-by-field comparison above and
from P0b (what the C runtime lacks).  State: **open**, **fixing** (on the branch named), or
**decided** (the decision and its measurement).  A closed row moves to
[STORE_MODEL-history.md](STORE_MODEL-history.md) with its commit; D1–D8, D9a–D17,
D18–D19j, D20–D25, D27, D29–D31, D33, D34 and D36–D45 are there.  (D8's second half, the gates
stated in `(H-SwapIn)`, closed with it.)

### Correctness

| # | Divergence | State |
|---|---|---|

### Contract and diagnostics

| # | Divergence | State |
|---|---|---|

### Hot paths

| # | Divergence | State |
|---|---|---|

### Edges

| # | Divergence | State |
|---|---|---|

### From the field comparison and P0b

| # | Divergence | State |
|---|---|---|
| D46 | a `par` region's machinery: the interpreter's worker result buffer (100 words) and its hand-over of a worker's stores to the parent at the join, which frees them there, against the C's results vector and its frees in the worker | decided — C's model (no measurement favours the interpreter's); the interpreter converges in P2.  Meanwhile `LOFT_ALLOC_TRACE` marks the region (`P` at `n_parallel_queue…`, `Q` at `n_parallel_buf_drop…`) and names a worker's store `<worker>.<serial>` on both backends, and `scripts/c_alloc_diff.py` holds everything outside it to the model — every event before the first region event for event, every store made after one per store; a real difference injected before or after an allocating region is reported unaccounted (the positive controls) |
| D26 | the interpreter's stack lives in a store; C's is the C stack | decided — each runtime's own: the value stack is how the interpreter executes, not a heap store of the program.  `LOFT_ALLOC_TRACE` marks it (`S`, a state's stack and constant store, a par worker's stack too) and `scripts/c_alloc_diff.py` leaves it out, under which the two agree; P4's hosted C unit meets it at the frame boundary |
| D28 | the store table: a `Vec<Store>` beside the type table against a thread-local array | decided — one model: a numbered table of stores, a `DbRef`'s `store_nr` its index, the type table beside it; only the layout of that table differs, which P3 (one layout) builds |

## How a divergence is closed

1. **A probe that fails on the divergence** — a program, or a store-operation trace, whose
   answer differs between the two implementations today (both backends, `LOFT_POISON` where a
   freed byte matters).
2. **The decision** — C's model, or the interpreter's with the measurement that makes it better
   (the performance agent's P1 numbers: `bench/stats.py`, the allocator counters).
3. **The change**, on the losing side, after which the probe agrees and the differential is
   green on the corpus.
4. **The row moves** to the history companion with its commit, and the probe stays as a guard.
