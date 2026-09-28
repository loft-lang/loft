<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 174 — Foreign stores: read-only memory owned by somebody else, served through the store contract

## Status

**Open — ACTIVE since 2026-09-28; F0–F2 and F4a shipped, F3 measured red (the row is not a copy).**  The design is the plan issue
([loft-lang/plans#174](https://github.com/loft-lang/plans/issues/174), the one home for the
phases, the invariant and the precedents); this README is the per-phase state.

| phase | what | state |
|---|---|---|
| F0 | the probe: hand-price the claim on pluginabi's bench | **DONE 2026-09-28** — the text arm of cbor's `read_value` reading its span off the frame's element base into ONE `set_str`, the `tb` store and its byte loop gone: `check_request` **10.87 → 9.39 ms/op (−13.6 %)**, hash `ac0a4ec5` equal, two interleaved runs within 0.1 % (arm64, `bench/portal/hand_price.sh`'s rustc line plus crypto's cdylib).  The bytes arm still copies in the probe; a view removes that too.  Well outside the −4.6 % the buffers alone priced, so the plan goes on |
| F1 | a `Store` that serves foreign bytes | **DONE 2026-09-28** — `Store::make_foreign` / `FOREIGN_REC` / `ForeignOwner`; four accessors answer the id (`read`, `addr`, `valid`, `elem_base`), `bytes_of` the read-only reader; unit test reads it beside a copied store through the same accessors, byte for byte; sabotage receipts in its doc comment (the `read` intercept struck: out-of-bounds refusal; `elem_base` struck: the process dies 16 GiB past the store).  Reference: DATABASE.md § Foreign stores |
| F2 | the runtime hands one out and frees it with its handle; `file_map` | **DONE 2026-09-28** — `Stores::foreign_vector` / `free_named` releases the owner; `file_map(path)` in `02_files.loft` (a read-only `memmap::Mmap`, strace shows `mmap(…, PROT_READ, MAP_SHARED)` of the file); cells `tests/scripts/174-foreign-file.loft` c1–c10 (length, every index, an iteration, slices, `text_from_bytes` of a slice and of the whole, a hoisted loop, missing and empty files, two mappings, a rebind, a copy that is independent) pass on both backends and under `LOFT_HOIST_VERIFY`, `LOFT_STRICT_STORES`, `LOFT_POISON`, `LOFT_STORES=warn`; the write refusal is `tests/foreign_store.rs` on the spawned binary.  Two defects the phase found in the store: the lock refusal ran AFTER the bounds computation (a foreign write read as a corrupt reference), and an internal lock's write was a panic where the program's own doing deserves the runtime error |
| F3 | the crawler's `binary_read` row on it | **RED 2026-09-28, as designed** — `binary_map` (the same 150 000 i16 off `file_map`, two byte reads per element) beside `binary_read` (`bf#read(2)` per element), same hash `fa34c62d78` on all four lanes: loft 2.36 ms against 2.28 ms (idle box, two runs within 0.5 %) — the row does NOT move.  Why: this row is not a copy of the file; `bf#read(2)` is a buffered read at ~16 ns per element, and the row is the element loop — two nullable index reads, the arithmetic and the push into `out` per element — so removing the copy removes nothing.  A mapping pays where a consumer COPIES bytes out (the F0 shape, −13.6 %), not where it streams them.  The row stays in the registry as a like-for-like shape (`vector-read`); the crawler's own code keeps `f#read` |
| F4 | a slice of a read-only store is a view | **F4a DONE 2026-09-28** — a scalar-element slice is ONE block copy (`OpSliceVector` / `Stores::vector_slice`, the span read through `bytes_of`, so a mapped file's slice copies without a record per byte): forty 200 000-element slices 0.13 → 0.03 s; sabotage receipt in the cell file (the copy's start struck: c1 reads 18 36 189 for 21 39 210).  **And the language half the owner asked for on the way:** `(Slice-Value)` holds at EVERY vector-typed position now — an argument, a return, an `if` arm, a field value, a literal element — through one home (`Parser::iterator_as_vector`); `text_from_bytes(bytes[a..b])` is a plain call.  Gap pinned: a generic's instance keeps the loop.  **F4b (the view) and F4c open** — see § Where to resume |
| F5 | the library bridge | open |
| F6 | pluginabi's front door | open |
| F7 | the wasm side | deferred until F5 holds |

**Open questions answered so far** (the issue's list): (3) write policy — REFUSE, and not a
new code: the existing `write_to_locked_store` runtime error, with an advice of its own for
the foreign origin (copy first); a foreign store sits beside the author's `#lock`, the two
locks a program can be told about.  (1), (2) and (4) are F4's.

## Where to resume — F4, re-cut on what F2 found (2026-09-28)

**The fact F4 starts from** (`loft introspect` on `s = m[7..14]`): a vector slice bound to a
local lowers to a `Slice materialise` loop — the local's own hidden store (`__vdb_N`) is
minted, the bounds are clamped, and then, per element, `OpGetByte(OpGetVectorNullable(m, …))`
feeds an `OpNewRecord` / `OpSetByte` / `OpFinishRecord` group into it: a record mint per
BYTE.  A slice used only as an iterator (`for b in m[a..b]`) never materialises.  So the
copy F4 elides sits at the BIND, and it is dearer than a `memcpy` by two orders.

**The cut, in three units that can each go red:**

1. **F4a — a slice of a scalar vector is ONE bulk copy** (S, both backends, no foreign
   store involved): the materialise loop becomes a runtime `Stores::slice_into(dst, src, lo,
   hi)` that reserves once and copies the span (`alloc_vector_from_bytes` is the shape).
   Validation: the loop and the call run side by side under `LOFT_NO_SLICE_COPY=1`, same
   bytes, on both backends; `make speed` on the slice-heavy corpus.  Goes red on any
   element kind the bulk copy cannot take (a text element, a linked vector): those keep
   the loop, and the cell says which.
2. **F4b — on a read-only store the copy is a VIEW** (M).  **Runtime half DONE 2026-09-28:**
   `Store::add_view(lo, hi)` registers `(offset, len)` on the foreign bytes and answers a
   SLOT id whose word at `+8` names the view's RECORD id; both ids go through the same four
   accessors (`is_foreign_rec`, one test), so a `DbRef { rec: <slot>, pos: 8 }` reads exactly
   as a minted vector's handle — the unit test reads a view beside the copied slice byte for
   byte, clamped and empty views included.  **Left:** (i) the PARSER binds the slice's local
   from the op — `s = OpSliceVector(s, src, lo, hi, tp)` answering a view's handle when the
   source is foreign and the bind is fresh (`=` on an empty local), the copied `db` otherwise
   (`+=`, a non-foreign source), with the native template the same expression; (ii) the
   LIFETIME — the local's hidden store records `view_of: Some(store_nr)`, the foreign store
   counts live views, a handle freed while views live orphans the owner until the count
   reaches zero, and a view's hidden-store free decrements; (iii) `LOFT_NO_FOREIGN_VIEW=1`
   and the cells below.  The design as first written follows.  The same `slice_into` answers,
   when `src`'s store is foreign, a DbRef into THAT store under a view id from a small
   `(offset, len)` table in `Foreign` (ids above `FOREIGN_REC`, the four accessors index the
   table; a slice of a view sums the offset).  Lifetime: the local's hidden store `__vdb_N`
   records `view_of: Some(foreign store_nr)` and the foreign store counts its views; a
   handle freed while views live ORPHANS the owner (kept until the count reaches zero), and
   a view's hidden-store free decrements it — so no view outlives the bytes, whatever order
   the program frees in, and the scope pass needs no new dep (the plan's open question 1 is
   answered by the runtime, not by a refusal).  `LOFT_NO_FOREIGN_VIEW=1` restores the copy;
   the A/B is the falsifier; cells: slice of a slice, a slice stored in a record field read
   after the handle is rebound, negative and clamped bounds equal to the copy's, a view
   under `LOFT_HOIST_VERIFY=1`, `text_from_bytes(view)` (reads through `bytes_of`, which
   must learn the view ids).  `(Slice-Value)` in `formal/collections.md` gains the clause
   and cites `slice_into`.
3. **F4c — DONE with F4a (2026-09-28), and wider than planned**: a slice is a vector at every
   vector-typed position (`(Slice-Value)`, `Parser::iterator_as_vector`), so
   `text_from_bytes(bytes[a..b])` is a plain call today; with F4b that call reads a view.
   F6 then rewrites cbor's two arms exactly so.

**Not to do:** a static dep from a slice-bound local to its source (it would forbid every
program that rebinds the source while a COPY lives — a contract change for no gain); a
compile-time "is this store foreign" (a runtime property).

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
