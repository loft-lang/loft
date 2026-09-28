<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 174 — Foreign stores: read-only memory owned by somebody else, served through the store contract

## Status

**Open — ACTIVE since 2026-09-28; F0–F2, F4 and F5 shipped, F3 measured red (the row is not a copy); F6 next.**  The design is the plan issue
([loft-lang/plans#174](https://github.com/loft-lang/plans/issues/174), the one home for the
phases, the invariant and the precedents); this README is the per-phase state.

| phase | what | state |
|---|---|---|
| F0 | the probe: hand-price the claim on pluginabi's bench | **DONE 2026-09-28** — the text arm of cbor's `read_value` reading its span off the frame's element base into ONE `set_str`, the `tb` store and its byte loop gone: `check_request` **10.87 → 9.39 ms/op (−13.6 %)**, hash `ac0a4ec5` equal, two interleaved runs within 0.1 % (arm64, `bench/portal/hand_price.sh`'s rustc line plus crypto's cdylib).  The bytes arm still copies in the probe; a view removes that too.  Well outside the −4.6 % the buffers alone priced, so the plan goes on |
| F1 | a `Store` that serves foreign bytes | **DONE 2026-09-28** — `Store::make_foreign` / `FOREIGN_REC` / `ForeignOwner`; four accessors answer the id (`read`, `addr`, `valid`, `elem_base`), `bytes_of` the read-only reader; unit test reads it beside a copied store through the same accessors, byte for byte; sabotage receipts in its doc comment (the `read` intercept struck: out-of-bounds refusal; `elem_base` struck: the process dies 16 GiB past the store).  Reference: DATABASE.md § Foreign stores |
| F2 | the runtime hands one out and frees it with its handle; `file_map` | **DONE 2026-09-28** — `Stores::foreign_vector` / `free_named` releases the owner; `file_map(path)` in `02_files.loft` (a read-only `memmap::Mmap`, strace shows `mmap(…, PROT_READ, MAP_SHARED)` of the file); cells `tests/scripts/174-foreign-file.loft` c1–c10 (length, every index, an iteration, slices, `text_from_bytes` of a slice and of the whole, a hoisted loop, missing and empty files, two mappings, a rebind, a copy that is independent) pass on both backends and under `LOFT_HOIST_VERIFY`, `LOFT_STRICT_STORES`, `LOFT_POISON`, `LOFT_STORES=warn`; the write refusal is `tests/foreign_store.rs` on the spawned binary.  Two defects the phase found in the store: the lock refusal ran AFTER the bounds computation (a foreign write read as a corrupt reference), and an internal lock's write was a panic where the program's own doing deserves the runtime error |
| F3 | the crawler's `binary_read` row on it | **RED 2026-09-28, as designed** — `binary_map` (the same 150 000 i16 off `file_map`, two byte reads per element) beside `binary_read` (`bf#read(2)` per element), same hash `fa34c62d78` on all four lanes: loft 2.36 ms against 2.28 ms (idle box, two runs within 0.5 %) — the row does NOT move.  Why: this row is not a copy of the file; `bf#read(2)` is a buffered read at ~16 ns per element, and the row is the element loop — two nullable index reads, the arithmetic and the push into `out` per element — so removing the copy removes nothing.  A mapping pays where a consumer COPIES bytes out (the F0 shape, −13.6 %), not where it streams them.  The row stays in the registry as a like-for-like shape (`vector-read`); the crawler's own code keeps `f#read` |
| F4 | a slice of a read-only store is a view | **DONE 2026-09-28** — F4a: a scalar-element slice is ONE block copy (`OpSliceVector` / `Stores::vector_slice`; forty 200 000-element slices 0.13 → 0.03 s), and `(Slice-Value)` holds at EVERY vector-typed position (`Parser::iterator_as_vector`), so `text_from_bytes(bytes[a..b])` is a plain call.  F4b: `s = m[lo..hi]` bound to a local that owns its `__vdb_N` backing emits `OpSliceView` (`Parser::owns_vdb_backing`); on a foreign source the local's store becomes a foreign one over the span (`Store::foreign_span` → `Store::make_foreign`, the owner SHARED through `Arc<ForeignOwner>` — no view table, no live count, the last store serving the bytes drops them); a clear drops the view (`release_foreign`), so a rebind or a literal after it is ordinary.  Cells `174-foreign-view.loft` v1–v13 (view of a view, a field copy, a view returned past its handle, clamped/negative/empty bounds, a hoisted loop, texts, rebinds, two views outliving a rebound and emptied handle, arguments, a bind copy, 20 000 rebinds without a leak, `par` over a mapping and a view) pass on both backends, under `LOFT_HOIST_VERIFY`, `LOFT_STRICT_STORES`, `LOFT_POISON`, `LOFT_STORES=warn`, and print the same lines under `LOFT_NO_FOREIGN_VIEW=1` (the A/B, `tests/foreign_store.rs`, which also asserts the view write refusal and the emission pin).  Sabotage receipt in the cell file.  Fixed on the way: `w = m` (a bind of a mapped vector) and `par` over one read the source sixteen gigabytes past the store (`Store::block_src` is the one source address now), `write_bytes(m)` borrowed mutably, and the `@P390` self-slice rebind of a `vector<u8>` parameter appended with element row 0 (an 8-byte stride) |
| F5 | the library bridge | **DONE 2026-09-28 (loft side; the library half is on a branch, blocked on the loft-ffi 0.1.2 publish)** — `loft_ffi::LoftStore::foreign_vector_from_owned(Vec<u8>)` over a trailing `foreign_fn` callback (ABI-safe: the only host-to-cdylib crossing is the fixed `LoftBridgeFn` shape); `ForeignOwner::Extern` releases the block through the cdylib's own `release_vec`, once, with the last store; the dispatcher's return-store mint is announced (`CTX_RETURN_STORE`) and adopted into (`foreign_vector_in`).  Three mechanisms the phase found it needed, each pinned by a cell that failed without it: the BIND — `OpAdoptVector` views a foreign answer whole where the #409/#410 deliveries copied every element (the plan's gain was otherwise copied away at the only site a library uses); the ARGUMENT — a foreign vector handed to a bridge is copied into the bridge's own store (`Stores::bridge_args`, both backends; `file_map`'s result into any library bridge crashed since F2); the LOOP BUFFER — native's § V-al reset and `Stores::clear` release a view instead of writing its length.  Cells `tests/lib/native_pkg/tests/174-foreign-bridge.loft` b1–b8 on both backends under `LOFT_HOIST_VERIFY`, `LOFT_STRICT_STORES`, `LOFT_POISON`, `LOFT_STORES=warn` and the copy A/B, the two write refusals, and 2 000 rebinds of a 1 MiB answer at 17 MB RSS (`tests/foreign_bridge.rs`); unit test `a_cdylib_block_is_read_in_place_and_released_once_with_the_last_store`.  Placement needs no new refusal: `placeable_bind` places only a bare dense record, and a vector field copies |
| F6 | pluginabi's front door | open |
| F7 | the wasm side | deferred until F5 holds |

**Open questions answered** (the issue's list): (3) write policy — REFUSE, and not a
new code: the existing `write_to_locked_store` runtime error, with an advice of its own for
the foreign origin (copy first — the bind `w = v`, which copies); a foreign store sits
beside the author's `#lock`, the two locks a program can be told about.  Copy-on-write was
weighed and declined: the first write would MOVE the bytes, and a native loop that hoisted
the view's base across an element write (`v[i] = x` moves nothing on an owned vector) would
then write through the mapping — the invariant's "nothing can grow or move the block" is
what every hoist rests on.  (1) escape beyond the handle — answered by the runtime, not a
refusal and not a dep: a view holds the span's owner (`Arc<ForeignOwner>`), so it outlives
the handle for exactly as long as it is bound, in any free order; a view stored in a record
field is COPIED there (a field never views).  (2) `text` over foreign bytes — a text stays a
copy of its span, made in ONE `set_str` from `bytes_of` (the F0 shape, −13.6 %); a text VIEW
is not built, since every text op that reads the record header would need to learn a second
shape for a gain the probe did not price.  (4) the oracle — the copying path under
`LOFT_NO_FOREIGN_VIEW=1`, which both backends share, never the other backend.

## Where to resume — F6, pluginabi's front door

**F5's library half.**  crypto's `base64_to_bytes` is the first library (its contract already
reads *"read the result, never `+=` onto it"*, so a read-only foreign answer formalises it):
the native bridge answers `foreign_vector_from_owned`, the wasm bridge `Stores::foreign_vector`.
The change waits on **loft-ffi 0.1.2 on crates.io** (the release workflow's `cargo publish`
covers the `loft` crate only; loft-ffi is published by hand) — until then a library cannot
pin it, so the branch carries a `[patch.crates-io]` to the in-tree crate for the testbed run,
to be dropped at publish (the imaging fixture's precedent).  Measure the crypto bench's
`base64_to_bytes` row on the testbed after the publish.

**Side-finding, not F5's:** on `--native` a loop of 2 000 OWNED 1 MiB bridge answers
(`ext_make_bytes`, the copying path) sits at 246 MB RSS where the interpreter sits at 21 MB
and the foreign path at 17 MB — the minted return store's growth, retained across frees.
Filed separately.

## F5 as designed, for the record

**F4b as shipped differs from the design first written here** (a view-id table inside the
handle's store, two synthetic ids per view, a live-view count with an orphaned owner).  The
re-cut, measured on the first probe: a local whose `DbRef` LEFT its own `__vdb_N` store for
the handle's could not be cleared or rebound (`s = m[a..b]; s = [1]` clears `s` — a write
into the handle's store — before the parser could restore the local's own store), and the
table never shrank (a loop of 20 000 rebinds would have exhausted its 500 000 ids in
seconds).  Keeping the local's store as the view's home — `make_foreign` over a span whose
owner is shared — needs no op that answers a handle, no count, no orphan and no parser
restore; the clear that every rebind already emits drops the view.  The unit test and the
cells are the receipt.

**Pinned gap found by the cells:** `len(v[a..b])` — a METHOD call whose receiver is a slice
expression — still reports *"Unknown function len"*: method resolution reads the receiver's
type before the `(Slice-Value)` coercion sees the position.  `len(s)` on a bound local, and
every function-argument position, are fine.  Route: the receiver of a stdlib method is a
vector-typed position like any other.

**F5 next:** `loft-ffi`'s `foreign_vector_from_owned(Vec<u8>)` beside
`alloc_vector_from_bytes` (`src/vector.rs:146` the runtime side, `Stores::foreign_vector`
already takes a `ForeignOwner::Bytes`); one library first; placement refuses the result as
a View (`src/place_result.rs`).  Then F6, pluginabi's frame as a foreign store and cbor's two
arms on views, re-measured on the portal row.

## Goal

A loft program reads bytes that belong to somebody else — a library's buffer, a system file, a
host frame — through the ordinary `vector<u8>` / `text` surface with no copy into a store, and a
slice of such a value is a view; no new syntax, no new op.

## Effort + design

Effort per phase on the issue (XS–M).  Design: the issue's § The one invariant — *a foreign
store is read-only, its bytes are described by the same header contract every vector op
reads, and it outlives every `DbRef` into it* — and the precedent table there.  Open
questions are answered on the issue as each phase settles them.

## Composition matrix — Stage A

The cells land with F2 (`tests/scripts/<N>-foreign-file.loft`): length, an indexed read, an
iteration, a slice (a copy until F4, a view after), `text_from_bytes` of a slice, a hoisted loop
under `LOFT_HOIST_VERIFY=1`, and a write that must be refused.  Each cell runs the copying
`f#read` path beside the foreign one and compares bytes.

## See also

`bench/portal/analysis/alloc-temp.md` § check_request priced (the pricing this plan answers) ·
`formal/heap.md` (H-Alloc) · `formal/collections.md` (Slice-Value) · `REMOTE_STORES.md` ·
`LAZY_STORES.md` · `PLACEMENT.md` § views · @PLN158.
