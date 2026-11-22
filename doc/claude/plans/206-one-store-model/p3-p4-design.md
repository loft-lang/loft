<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN206 P3 and P4 — one store layout, and a C body on the interpreter's stores

The plan is [loft-lang/plans#206](https://github.com/loft-lang/plans/issues/206).  The model both
phases rest on is [`STORE_MODEL.md`](../../STORE_MODEL.md): after P1 and P2 the two allocators
claim, delete, grow, park and free alike, event for event.  P3 makes them share one struct; P4
runs a compiled C body on the interpreter's stores through that struct.

## The invariant

**A store's allocator state is one C-layout prefix of the interpreter's `Store`, which both
allocators read and write with one meaning.**  A hosted C unit indexes the interpreter's store
table directly: its `loft_store_t` is the shared prefix padded to `sizeof(Store)`, so
`loft_stores[nr]` lands on the interpreter's `allocations[nr]`.  The host keeps only the acts
that touch state outside the prefix: a store's birth and death, and the growth of a mapped
store.

Re-assertion sites: two (the Rust struct, the C struct).  The layout test makes forgetting
either loud: it compiles `loft_store.h`, reads every shared field's offset and size, and
compares them with `std::mem::offset_of!` over `Store`.

## The shared prefix

In the C order, with one type per field.  Where the two spelled a fact differently, the
interpreter takes the C name, so the compiler lists every site the change reaches.

| field | type | Rust before | note |
|---|---|---|---|
| `buf` | `uint8_t *` | `ptr` | `malloc`/`realloc`/`free` on both sides |
| `size` | `uint32_t` | `size` | words |
| `live` | `uint8_t` | `!free` | inverted in the interpreter |
| `known` | `uint16_t` | `known_type` | the root record's type; `0xFFFF` none |
| `pinned` | `uint8_t` | `pinned` | |
| `protect` | `uint32_t` | `free_protect_depth` | the calls holding the store as an argument |
| `read_only` | `uint8_t` | `read_only` | the author's `#lock` is `user_locked`, interpreter-only |
| `serial` | `uint64_t` | `alloc_serial` | |
| `claims`, `referenced`, `removed`, `lazy_free`, `parked_set` | `loft_bits` | `Claims` (a `Vec<u64>`) | one bit per word, `malloc`ed |
| `released_root` | `uint8_t` | `released_root` | |
| `parked_heads[17]`, `parked_count` | `uint32_t` | `Parked { heads, count }` | |
| `claimed_end`, `free_root`, `wild`, `dead_words` | `uint32_t` | same | |
| `needs_coalesce`, `lazy`, `defined` | `uint8_t` | same | |
| `no_park` | `uint8_t` | `!park` | inverted in the interpreter |
| `placed`, `nplaced`, `placed_cap` | `uint32_t *`, `uint32_t` | `placed_foreign: Vec<u32>` | |
| `frame_holds` | `uint32_t` | same | |
| `orphaned` | `uint8_t` | same | |
| `mapped`, `map_file` | `size_t`, `intptr_t` | — | the interpreter sets `mapped` while a file backs the store |
| `closure_shares` | `uint32_t` | — | the interpreter keeps its side table; see the failure paths |
| `hgen` | `uint64_t` | — | the C's table-handle generation |

The policies the C fixes at build time (`LOFT_WILDERNESS`, `LOFT_CARVES`, `LOFT_PARKS`) are the
interpreter's per-store `wilderness` and `carve`, set from the same switches for a whole run.  A
hosted unit is built with the run's switches, or it is not adopted.

## The failure paths

1. **A field drifts on one side** — a type, an order, a new field.  The layout test fails.
2. **A shared field changes meaning on one side.**  The allocator differential
   (`scripts/c_alloc_diff.py`) and P4's adopted-against-interpreted suite catch it; the layout
   test cannot.
3. **The table moves under the C.**  The interpreter's `Vec<Store>` reallocates when a store is
   added.  The hosted unit reads the table's address and length through the host after every
   host call that may add a store, never from a copy it kept across one.
4. **Memory freed by the other side's allocator.**  Both use `malloc`, `realloc`, `free`.  On
   Windows a unit may link another C runtime heap, so the host lends its allocator to the unit
   (`HostTable`), and the unit allocates through it.
5. **State outside the prefix.**  `generation` (the coroutine guard, a debug build's check), the
   interpreter's `Foreign`, its `MmapStorage` and its closure-share table are not in the prefix.
   A hosted call is not made on a store that is foreign, file-backed, or a closure's: the host
   checks its arguments and interprets that call instead.  `hgen` is the C's alone: the
   interpreter never reshapes a table while a hosted call runs.
6. **A policy differs.**  A unit built under other switches than the run's is not adopted (its
   fingerprint carries them).
7. **Type numbers differ.**  A unit is adopted only when its type-layout fingerprint is the
   program's (`artifact_matches_layout`), as a rustc artifact is.

## Phases

| | Work | Compared against | Goes red when |
|---|---|---|---|
| P3a | `Claims` becomes `loft_bits`: a `#[repr(C)]` bit set in `malloc`ed memory. | The interpreter's `LOFT_ALLOC_TRACE` over the sample, byte for byte before and after. | A trace differs, or the suite does. |
| P3b | `Store` becomes `#[repr(C)]` with the shared prefix first, in the C order and types; the C retypes its flags to `uint8_t`. | The same traces; the layout test. | A trace, the suite, or an offset differs. |
| P3c | A hosted `loft_store_t` is padded to `sizeof(Store)` (`LOFT_HOST_STORE_SIZE`). | The layout test, sizes included. | `sizeof(loft_store_t)` differs from `size_of::<Store>()`. |
| P4a | The C writer emits a hosted unit: the table, birth, death and mapped growth through the host, `loft_shared_<name>` bridges over `LibArg`. | A hosted unit called from a Rust test on a live `Stores`. | Its answer or the stores after it differ from the interpreter's. |
| P4b | The interpreter adopts a hosted unit for one pure-loft library. | The library's suite interpreted, then adopted; the timings. | An answer differs, or adopted is not faster. |
