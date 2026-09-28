<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 174 — Foreign stores: read-only memory owned by somebody else, served through the store contract

## Status

**Open — ACTIVE since 2026-09-28; F0–F2 shipped, F3 measured red (the row is not a copy).**  The design is the plan issue
([loft-lang/plans#174](https://github.com/loft-lang/plans/issues/174), the one home for the
phases, the invariant and the precedents); this README is the per-phase state.

| phase | what | state |
|---|---|---|
| F0 | the probe: hand-price the claim on pluginabi's bench | **DONE 2026-09-28** — the text arm of cbor's `read_value` reading its span off the frame's element base into ONE `set_str`, the `tb` store and its byte loop gone: `check_request` **10.87 → 9.39 ms/op (−13.6 %)**, hash `ac0a4ec5` equal, two interleaved runs within 0.1 % (arm64, `bench/portal/hand_price.sh`'s rustc line plus crypto's cdylib).  The bytes arm still copies in the probe; a view removes that too.  Well outside the −4.6 % the buffers alone priced, so the plan goes on |
| F1 | a `Store` that serves foreign bytes | **DONE 2026-09-28** — `Store::make_foreign` / `FOREIGN_REC` / `ForeignOwner`; four accessors answer the id (`read`, `addr`, `valid`, `elem_base`), `bytes_of` the read-only reader; unit test reads it beside a copied store through the same accessors, byte for byte; sabotage receipts in its doc comment (the `read` intercept struck: out-of-bounds refusal; `elem_base` struck: the process dies 16 GiB past the store).  Reference: DATABASE.md § Foreign stores |
| F2 | the runtime hands one out and frees it with its handle; `file_map` | **DONE 2026-09-28** — `Stores::foreign_vector` / `free_named` releases the owner; `file_map(path)` in `02_files.loft` (a read-only `memmap::Mmap`, strace shows `mmap(…, PROT_READ, MAP_SHARED)` of the file); cells `tests/scripts/174-foreign-file.loft` c1–c10 (length, every index, an iteration, slices, `text_from_bytes` of a slice and of the whole, a hoisted loop, missing and empty files, two mappings, a rebind, a copy that is independent) pass on both backends and under `LOFT_HOIST_VERIFY`, `LOFT_STRICT_STORES`, `LOFT_POISON`, `LOFT_STORES=warn`; the write refusal is `tests/foreign_store.rs` on the spawned binary.  Two defects the phase found in the store: the lock refusal ran AFTER the bounds computation (a foreign write read as a corrupt reference), and an internal lock's write was a panic where the program's own doing deserves the runtime error |
| F3 | the crawler's `binary_read` row on it | **RED 2026-09-28, as designed** — `binary_map` (the same 150 000 i16 off `file_map`, two byte reads per element) beside `binary_read` (`bf#read(2)` per element), same hash `fa34c62d78` on all four lanes: loft 2.54 ms against 2.48 ms in one run — the row does NOT move.  Why: this row is not a copy of the file; `bf#read(2)` is a buffered read at ~16 ns per element, and the row is the element loop — two nullable index reads, the arithmetic and the push into `out` per element — so removing the copy removes nothing.  A mapping pays where a consumer COPIES bytes out (the F0 shape, −13.6 %), not where it streams them.  The row stays in the registry as a like-for-like shape (`vector-read`); the crawler's own code keeps `f#read` |
| F4 | a slice of a read-only store is a view | open |
| F5 | the library bridge | open |
| F6 | pluginabi's front door | open |
| F7 | the wasm side | deferred until F5 holds |

**Open questions answered so far** (the issue's list): (3) write policy — REFUSE, and not a
new code: the existing `write_to_locked_store` runtime error, with an advice of its own for
the foreign origin (copy first); a foreign store sits beside the author's `#lock`, the two
locks a program can be told about.  (1), (2) and (4) are F4's.

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
