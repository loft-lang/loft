<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 178 — A store's failure arm: `place = value else { … }`

## Status

**DONE — finished 2026-10-06.**  Tracker: [@PLN178](https://github.com/loft-lang/plans/issues/178)
(`status:finished`).  The ruling is
[DESIGN_DECISIONS.md C130](../DESIGN_DECISIONS_FAILURE.md#c130--a-store-carries-its-own-failure-arm-else-after-the-assignment).

## Where it lives now

| What | Home |
|---|---|
| The rules | `formal/heap.md` `(H-Write-Else)`; `formal/operational.md` `(E-Report)`'s dropped-write clause and `(E-Uncomp-Seen)` |
| The manual | LOFT.md § A store that did not take, and the narrow-slot paragraph; sample `tests/reference/store-else.loft` |
| The lowering | `src/parser/store_else.rs` — IR, so both backends and the browser share it |
| The report | the `OpSet*` templates' cold branch (`default/01_code.loft`), `DbRef::NULL_REPORTED`, `RuntimeErrorKind::WriteDropped` |
| The guards | `tests/scripts/178-store-else.loft` (the matrix, with receipt), `tests/dropped_writes.rs` (the log, both backends), `tests/scripts/a-store-in-condition-position-is-refused.loft` (phase 0) |

## What each phase delivered

| # | Phase | Delivered |
|---|---|---|
| 0 | Refuse a store in condition position | shipped with C130 |
| 1 | Rules first | `(H-Write-Else)`, the `(E-Report)` clause, `(E-Uncomp-Seen)` restated |
| 2 | `else` after a store | the arm over every write shape: setters, record copies, temps (`__place_N`, `__lift_N`), a local, a tuple element; a record variable bound as a view; the place's target bound once; the arm is a branch for the dead-store lint |
| 3 | Native | the IR lowering serves `--native` unchanged; the hoisted writers decline on a guarded store (measured: `vec_set_at` fires unguarded, the plain path runs guarded); the `(R-RefillText)` bodies carry the report |
| 4 | The Warn line | one line per unguarded dropped write — `index_out_of_bounds` where an index was found out of range, `write_dropped` for a key or a null view — and none for a guarded one; recoverable reports name the statement (interpreter) and the running function (`--native`) instead of `:0` |
| 5 | Docs | LOFT.md, CAVEATS, DIAGNOSTICS (`narrow-fallback`'s cure is now `else`), CHANGELOG |

Found and fixed on the way: a record copied into no place leaked its given-up source on both
backends (`tests/scripts/a-record-copied-into-no-place-gives-its-source-back.loft`).

The open questions closed as: the lock fault runs the arm in both run modes (the target is
nulled before the write); the log line is `write_dropped`; wasm runs the reference program
(`--native-wasm`, exit 0); a generator body and a `par` worker each run their arm in their own
frame (the matrix's last cell).
