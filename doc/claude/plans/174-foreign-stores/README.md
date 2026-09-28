<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 174 — Foreign stores: read-only memory owned by somebody else, served through the store contract

## Status

**CLOSED — DONE 2026-09-28 (the owner's call).**  The design is the plan issue
([loft-lang/plans#174](https://github.com/loft-lang/plans/issues/174)); the reference
content lives in **[DATABASE.md § Foreign stores](../../DATABASE.md)** (the store, the
runtime, the views, the bridge, the argument marshal), `formal/collections.md` `(Slice-Value)`
(the view clause), [PACKAGES.md § loft-ffi](../../PACKAGES.md) (`foreign_vector_from_owned`),
[BOTH_BACKEND_SWITCHES.md](../../BOTH_BACKEND_SWITCHES.md) (`LOFT_NO_FOREIGN_VIEW`) and the
pricing in `bench/portal/analysis/alloc-temp.md` § `check_request` priced / § F6.  The
feature is catalogued as [features#123](https://github.com/loft-lang/features/issues/123).
The two tails are open-work rows, not phases: [PACKAGES.md § Open work](../../PACKAGES.md)
**PKG.FOREIGN-HOST** (which host hands pluginabi's frame over — F6's host half — and the
browser typed array, F7).

## Closure record

| phase | outcome |
|---|---|
| F0 probe | DONE — the text span off the frame priced −13.6 % on `check_request`, past the −4.6 % floor |
| F1 store | DONE — `Store::make_foreign`, `FOREIGN_REC`, four accessors, `bytes_of` |
| F2 runtime + `file_map` | DONE — `Stores::foreign_vector`, cells `tests/scripts/174-foreign-file.loft` |
| F3 crawler `binary_read` | RED as designed — the row streams bytes and never copied them; the crawler keeps `f#read` |
| F4 a slice is a view | DONE — `OpSliceVector` (a block copy) / `OpSliceView` (a view on a read-only source), cells `174-foreign-view.loft`, the copy A/B |
| F5 library bridge | DONE — loft-ffi 0.1.2 (`foreign_vector_from_owned`, published), `OpAdoptVector` at the bind, `Stores::bridge_args` for a foreign argument, cells `tests/lib/native_pkg/tests/174-foreign-bridge.loft`; crypto on `loft-libs-core` branch `174-f5-crypto-foreign` (row unchanged: the decode is the cost) |
| F6 pluginabi's front door | LIBRARY HALF DONE — cbor's payload arms as block spans (`174-f6-cbor-spans`): `check_request` −4 % owned / −11 % on a foreign frame, `req_state_b64` −59 %; the HOST half is PKG.FOREIGN-HOST |
| F7 wasm typed array | NOT STARTED — PKG.FOREIGN-HOST |

**Open questions, as answered:** write policy — REFUSE (`write_to_locked_store` with the
foreign advice; copy-on-write declined, since a first write would move the block under a
hoisted base); escape beyond the handle — a view holds the span's owner (`Arc<ForeignOwner>`),
so it outlives the handle for as long as it is bound, and a record field copies; `text` over
foreign bytes — a text stays a copy of its span in one `set_str`; the oracle — the copying path
under `LOFT_NO_FOREIGN_VIEW=1`, never the other backend.

**Found and fixed on the way:** a bind and a `par` over a mapping read past the store
(`Store::block_src`); a foreign vector handed to any library bridge crashed (`Stores::bridge_args`);
native's loop-buffer reset wrote a view's length; a package's loft floor was skipped when the
package was adopted as a sibling first (`Parser::loft_floor_holds`).  **Filed:** loft#1723
(native RSS on owned bridge answers), loft#1728 (`len(v[a..b])`).

**Levers priced and not taken** (`alloc-temp.md` § F6): fusing `text_from_bytes(v[lo..hi])`
into one `set_str`; the record-tree copies of `pa_decode` (placement).
