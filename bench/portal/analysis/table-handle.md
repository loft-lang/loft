# The table handle (`R-TableHandle`) — design

Status: designed, not built.  The C backend only; the interpreter and the Rust writer are
untouched.

## What it removes

Every keyed operation in the C runtime re-derives its table from the record before it probes:
the collection slot, the claim, the header word and flags (count, slot width, inline or arena,
stride), the seed, and the entry directory.  A function that makes several keyed operations on
one collection, or one inside a loop, pays that each time.  `lru_mixed`'s `put` makes four on
`c.nodes`: the lookup, the length, the eviction's removal and the insert.

Three forms were priced on `lru_mixed` by hand-editing its emitted C (ship lane, gcc 15, pinned,
output hash unchanged): the runtime's typed operations re-written over a memo (C), or over a
handle `put` holds, checked (B) or not (A).  Build step 4 reproduces the B price:

| form | µs per op | Δ |
|---|--:|--:|
| the current runtime | 496 | |
| C: a runtime memo of decoded headers, no emitter change | 487 | −2 % |
| **B: a handle the emitted code holds, checked at every use** | **458** | **−7.7 %** |
| A: a handle the compiler proves valid, no check | 448 | −9.7 % |

C gains little: the memo still pays the store lookup, the slot read and its own check, and the
operations stay out-of-line calls that cannot keep the table's facts in registers.  A needs a
field-level proof that every statement between two uses leaves the collection alone, including
calls that receive its owner (`unlink(c, n)`), and is wrong silently where the proof is.  **B is
the design**: 80 % of A's gain, and its correctness does not depend on where the emitter puts
the handle.

The shape is common.  Over the 61 bench programs, 39 functions make keyed calls; 19 make two or
more on one collection (`put`, gridmesh `field_add_cell`/`field_new`, assets `build_pack`,
crawler `get_or_add`, moros `paint`, hex_recover `index_build`, word_count `tally`, the keyed
stdlib benches) and 21 make one inside a loop over a collection the loop does not replace.

## The invariant

**A handle answers for a collection place only while the place is the one it was resolved for,
the place's store has the table generation it was resolved under, and the place's collection
slot names the claim it was resolved from; any other use resolves it again.  Every runtime path
that changes a table's claim, size, slot width, entry kind, seed or entry directory, or replaces
a store's contents, gives that store a new table generation.**

The first sentence is checked in one place (`loft_hh_live`).  The second is the re-assertion
burden, counted below.

## The runtime (`src/c/loft_hash.h`, `src/c/loft_store.h`, `src/c/loft_rt.h`)

- `loft_store_t` gains `uint64_t hgen`, a C-runtime field (nothing persisted).  New values come
  from one global counter, so a value never repeats across stores or reuse of a store slot.
- `loft_hh` holds the place (`store_nr`, `rec`, `pos`), the store's `hgen` at resolve time, the
  claim, `count`, `mask`, `seed`, the entries (`loft_hents`) and `ok` (the table is tagged,
  non-inline, power-of-two).  It holds no `loft_store_t *`: the store table can move.
- `loft_hh_live(h, place)`: `h->ok`, the place equal to `h`'s, `loft_stores[nr].hgen == h->gen`,
  and the collection slot at the place equal to `h->claim`.  Two loads and a few compares.
- Typed operations take `loft_hh *` and the place as it is evaluated now: `loft_hh_find_i64`,
  `loft_hh_find_text`, `loft_hh_find_ints`, `loft_hh_len`, `loft_hh_remove_i64`,
  `loft_hh_insert` (the one-probe insert), `loft_hh_keyed_entry_ints`.  Each starts with
  `loft_hh_live`; when it fails it resolves the handle again (`loft_hh_resolve`), and when the
  table is not one `ok` covers it calls today's general function.  A handle is never wrong; a
  stale one costs one resolve.
- **The generation sites.**  One helper, `loft_h_reshaped(s)`, gives a store a new `hgen`.  It is
  called by:
  - `loft_h_set_header(s, claim, field, v)`, the only writer of `LOFT_H_STRIDE_FLD`,
    `LOFT_H_SEED_FLD` and `LOFT_AR_DIR_FLD`; today those are 12 raw `loft_set_u32` sites, all in
    `loft_hash.h` (creation, growth, inline rebuild, reserve, arena directory growth, seeding);
  - `loft_h_install_table` (the claim a place names changes: 3 callers);
  - the store-level replacements: store creation and in-place clear (`loft_store_make`, which
    keeps the serial on a clear), image load (`loft_image.h`), a mapped store's attach and
    detach (`loft_mapped.h`), `loft_swap_in` (both stores), a foreign store's copy
    (`loft_ffi_host.h`).
  A table is never resized in place (no `loft_st_resize` of a claim), and a deep copy builds its
  table through `loft_hash_reserve` and the add path, so it passes the header writes.

## The emitter (`src/c/body/keyed.rs`)

One decision, a speed heuristic and not a proof: **which keyed operations share a handle
variable.**  Per function body, a collection place (`OpGetField(x, f)` with `x` a variable, or a
variable that holds the collection) that two or more keyed operations name, or one inside a
loop, gets one `loft_hh hN = LOFT_HH_NONE;` at function scope.  Each operation on that place is
emitted as the handle form, with the place expression evaluated at the call as today.  Nothing
is hoisted, nothing is invalidated by the emitter, and grouping two places into one handle would
only thrash it.  A function with one keyed operation outside any loop keeps today's call.

Census name `R-TableHandle` (one admission per handle opened); switch `LOFT_NO_TABLE_HANDLE`
emits today's calls.

## How it fails, and what catches it

| failure path | caught by |
|---|---|
| the table grows between two uses (insert past half full, reserve) | install bumps; the slot check also differs |
| an inline table is rebuilt larger | install bumps |
| the arena's entry directory moves | the directory write bumps |
| the field is reassigned to another hash, or cleared and refilled | the slot check (another claim); a refill's creation bumps |
| a freed table's claim number comes back as a new table at the same place | the new table's header writes bump |
| the owner's store is freed and its slot made again (the per-op `Lru` of bench 23) | store creation or clear bumps |
| the owner's store content is swapped (`@FR-H-SwapIn`), loaded from an image, or mapped | the store-level sites bump |
| the variable that names the owner is rebound | the place compare (the current place is passed at each use) |
| a callee replaces the collection the caller holds a handle on | any of the above; the caller's next use re-checks |
| a new runtime path writes a header field raw, or replaces a store without the helper | **not caught by the check**: the guard scan and `LOFT_KEYED_VERIFY` below |

## Making omission loud

The re-assertion count is the generation sites: one helper with about 20 callers.  Forgetting
one is silent, so it is made loud twice:
- **A source guard** (`tests/`, with the other tree scans): no `loft_set_u32` of
  `LOFT_H_STRIDE_FLD`, `LOFT_H_SEED_FLD` or `LOFT_AR_DIR_FLD` outside `loft_h_set_header`, and
  every assignment of a store's `buf` or a `loft_stores[…]` slot in a function on an allow-list
  that calls `loft_h_reshaped` (or is a growth that keeps content).
- **`LOFT_KEYED_VERIFY`**: every handle use that passes `loft_hh_live` also decodes the table
  afresh and stops the run where a field differs, naming the place.  The corpus sweep's
  keyed-verify mode runs it over every program.

## Falsification

- **The guard's receipt:** remove the bump from `loft_h_install_table`; the cell where a table
  grows between two uses of one handle must stop under `LOFT_KEYED_VERIFY` and answer wrong
  without it.  Record it as `@falsified-at`.
- **Cells**, values hand-computed, interpreter and C, each also under `LOFT_KEYED_VERIFY` and
  `LOFT_POISON`: two uses around each failure path in the table above (grow; inline rebuild;
  directory growth past 64 entries; field reassigned; cleared and refilled; owner store freed
  and made again in a loop; swap-in of the owner's store; a callee that replaces the field;
  owner variable rebound), plus removals only (the handle must stay valid: one resolve); the
  text and compound-key forms; a loop whose body inserts until the table grows.

## Build steps, each tested before the next

1. `hgen`, `loft_h_reshaped`, `loft_h_set_header`; the 12 header writes, the install and the
   store-level sites through them; the source guard.  No behaviour change: c_emit, store tests,
   the corpus sweep.
2. `loft_hh` and its operations, with the verify check; a hand-written C cell per failure path.
3. The emitter grouping in `keyed.rs`, the census name and switch; the cells as suite cells;
   `make rewrite-census` blessed with the new name.
4. Reprice `lru_mixed` (expect about 458 µs) and the keyed routines in loops (gridmesh, assets,
   bench 15); the corpus sweeps in every mode.
