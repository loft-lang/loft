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

## Finding what the suite cannot see

Two runtimes that share a store's bytes can be compared BY those bytes.  A suite cell compares an
answer; an answer can be right while a store beside it is wrong — a claim left behind, a field
written in the wrong record, a free one side does and the other skips — and such a defect waits
for the run that reads it.  Two instruments compare the stores themselves, and each is held to a
positive control: a planted difference it must report, so a silent run is a measurement and not
a blind spot.

- **The store differential (`LOFT_STORE_DUMP=<file>`, `scripts/store_diff.py`).**  Each runtime
  writes every store's final content — as it is freed, and a store still live at the end — with
  its history name (as `c_alloc_diff.py` names it, so the two sides pair by what happened to a
  store, not by its slot), its slot, its claimed records and each record's bytes.  A dump taken
  only at the end was blind: a sound program frees everything first, which the positive control
  showed at once.  The diff lists every store live on one side only, every record claimed on one
  side only or of another size, and every run of differing bytes, each with the line that made
  the store on each side.  A store pairs with its twin by the line that made it and its root
  type, then by type alone, and among candidates by the records they hold; the C is emitted
  with `LOFT_C_TRACE_SITES`, so it records every statement's line (otherwise only one that
  calls a frame), and names each store by the line the interpreter does.  `--attribute` reruns
  with each C-only rewrite off, padding apart as in the report; `--every-rewrite-off` compares
  the two runtimes' own model.  Both sides run
  under `LOFT_POISON_CLAIM`, so a byte nothing wrote reads the same, and with one
  `LOFT_HASH_SEED`: a hash table is pseudo-random, and seeded alike its seed and its entries'
  places agree.  Two readings keep it from crying wolf: a stored reference names a store by its
  slot, so a word naming the same store by history on both sides agrees — and so does the
  `slot=<n>` line of a store image's sidecar, read back as bytes; and a byte one side
  zeroes where the other leaves it unwritten is padding no field reads, listed apart.  A second
  interpreter run is the noise control: what still varies from run to run is named, not hidden —
  over the sample, nothing does.  `--plant` flips one byte of the C's dump, which must add a
  difference.
- **The hosted verify (`LOFT_HOSTED_VERIFY=1`).**  Each call into a compiled library artifact —
  a hosted C unit or a rustc one — runs twice: compiled, on an exact copy of the world taken
  before the call (`Stores::world_copy`, `Store::exact_copy`), and the function's own bytecode
  on the world itself (`State::call_back`).  Every store that existed before the call must hold
  the same claimed bytes, and the answer must be the same (a record as `Stores::show` renders
  it); the hidden return buffer is not compared, because the compiled bridge fills the caller's
  where the call-back makes its own.  The first call that differs stops the run and names the
  function and the record; otherwise the program goes on in the compiled world, as it does
  without the switch.  `LOFT_HOSTED_PLANT` builds a unit with a planted write: with the verify the
  run stops at the call that made it, without it the wrong value surfaces statements later.

## Phases

| | Work | Compared against | Goes red when |
|---|---|---|---|
| P3a | `Claims` becomes `loft_bits`: a `#[repr(C)]` bit set in `malloc`ed memory. | The interpreter's `LOFT_ALLOC_TRACE` over the sample, byte for byte before and after. | A trace differs, or the suite does. |
| P3b | `Store` becomes `#[repr(C)]` with the shared prefix first, in the C order and types; the C retypes its flags to `uint8_t`. | The same traces; the layout test. | A trace, the suite, or an offset differs. |
| P3c | A hosted `loft_store_t` is padded to `sizeof(Store)` (`LOFT_HOST_STORE_SIZE`). | The layout test, sizes included. | `sizeof(loft_store_t)` differs from `size_of::<Store>()`. |
| P4a | The C writer emits a hosted unit: the table, birth, death and mapped growth through the host, `loft_shared_<name>` bridges over `LibArg`. | A hosted unit called from a Rust test on a live `Stores`. | Its answer or the stores after it differ from the interpreter's. |
| P4b | The interpreter adopts a hosted unit for one pure-loft library. | The library's suite interpreted, then adopted; the timings. | An answer differs, or adopted is not faster. |
| P4c | The store differential: `LOFT_STORE_DUMP` on both runtimes, `scripts/store_diff.py`. | The corpus sample, interpreted against `--native`; a planted wrong byte. | A difference no C-only rewrite explains appears, or the planted byte is not reported.  What it finds becomes `STORE_MODEL.md` rows, each converged or decided as P1's were. |
| P4d | The hosted verify (`LOFT_HOSTED_VERIFY=1`). | tween's and `hostedlib`'s programs; a planted wrong store write in a unit. | The planted write is not reported, or a real call differs. |

## Measured

- **P3** — the interpreter's allocation traces over the 140-script sample are byte-identical
  before and after (114 of 118; the rest differ only in an order that varies run to run).  The
  layout test fails on a C field retyped (`src/store/layout.rs`'s receipt).
- **P4** — tween's four test files and two probes answer alike interpreted, adopted, and adopted
  under `LOFT_POISON`.  The rustc artifact first ran 2–4 % faster (`value` 143–148 ns per item
  against the C's 148–152, `ease` 78 against 81–82; `-O3` changed nothing).  **The reason,
  measured:** each bridge call asked the host for the table by a call back — with the refresh
  struck, `value` fell by 5–6 ns and `ease` by 2.5.  **The cure:** the interpreter keeps a
  `#[repr(C)]` view of its table inside `Stores` (`host_abi::TableView`), set before each bridge
  call and after each store birth and death — the only times the table changes while a unit
  runs — and the unit reads it in place at `LOFT_STORES_VIEW_AT`.  Interleaved, three rounds:
  `value` C 143–152, rustc 146–154; `ease` C 80–84, rustc 78–84 (level in the third round); the
  per-call depth check (`LOFT_MAX_CALL_DEPTH`) changes nothing.  Against interpreting: 3.1x on
  `value`, 2.6x on `ease`.
- **P4c** — over the 122-script sample, interpreted against `--native` with `--attribute`: 59
  the same, 38 differing only by a named C-only rewrite, none unaccounted (25 have no `main` or
  are refused); `--plant` is reported in each of four programs.  What it found is STORE_MODEL
  D47 and D48, closed.
