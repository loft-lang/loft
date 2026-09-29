<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Standard library reference — the record

Limitations [STDLIB.md](STDLIB.md) once stated and the library no longer has, oldest
first.  The page describes the library as it is now; this file keeps the record, each
entry naming the issue that closed and the guard that asserts the current behaviour.  A
record doc: its dates are the point (DOC_CONTRACT rule 32).  `rule_tags.py claims --issues`
(the `stale-claims` nightly, @PLN176) names a limitation still on the page whose issue has
closed, and the entry it turns into belongs here.

## 2026-08-03 — a text carrying a NUL iterated short (loft#755)

`for c in s` stopped at an embedded NUL while `len`, indexing and `byte_at` all saw the
whole text.  Iteration yields every code point, the NUL position reading as `null`.
Guard: `tests/scripts/text-nul-iteration-755.loft`.

## 2026-08-03 — `text_from_bytes` and `byte_at` were reported missing for two releases (loft#748)

Both existed; the generated reference filed them under Environment, so a keyword sweep of
the Text page came back empty and was read as a language gap.  The lesson kept from it:
check an instrument against something it should find before trusting it to report an
absence — `grep default/*.loft` answers in one call.  Guard: `tests/scripts/748-chr-code-point.loft`.

## 2026-08-07 — a text-keyed `spatial` was accepted and then answered null (loft#799)

`spatial<Word[w]>` with a `text` key compiled and every point lookup answered `null`.  It
is refused at the declaration, naming `trie<Word[w]>`.  Guard: `tests/parse_errors.rs`.

## 2026-08-07 — `store_bind_lazy` answered `true` for a kind it could not serve (loft#802)

A `sorted` / `index` / `spatial` bound to a paged IMAGE was accepted and then answered
`null` at every lookup.  The binding is refused there, and `false` is worth checking.
Guard: `tests/scripts/802-lazy-refusal-visible.loft`.

## 2026-08-07 — a spatial box query answered points outside the box (loft#800)

`xs[(x1,y1)..(x2,y2)]` answered the raw Z-order interval between the corners, a strict
superset.  It answers exactly what is inside the box.  Guard:
`tests/scripts/800-spatial-box-containment.loft`.

## 2026-08-10 — `content()` answered `""` for a non-UTF-8 file (loft#829)

Indistinguishable from an empty file, so a *write bytes, read them back, compare* gate
passed vacuously on binary data with both sides `""`.  `content()` is nullable: a missing
file, a directory and non-UTF-8 bytes are each `null`.  Guard: `tests/binary_io_matrix.rs`.

## 2026-08-17 — `reduce` with a collection accumulator read back empty (loft#956)

The refusal of a collection accumulator asked the INIT what type it was; a bare `[]` had
none and walked past it, and the fold read back empty on `--interpret` and was an internal
compiler error on `--native`.  The accumulator's type is read off the fold's first
parameter, so `[]` and a typed init are the same fold.  Guard:
`tests/scripts/956-reduce-untyped-accumulator.loft`.

## 2026-08-20 — the open spatial walk was the Morton tail (loft#1002)

`xs[(x,y)..:n]` answered only records at or past the query in Z-order, so it under-delivered
near the end of the curve.  It answers `n` from any origin.  Guard:
`tests/scripts/48b-spatial-slice.loft`.

## 2026-09-28 — four Binary Files rows named routines that do not exist

`little_endian(self: File)`, `big_endian(self: File)`, `write_bin(self: File, v)` and
`read(self: File, v)` sat in the signature table with no declaration in any `default/*.loft`;
the real forms are `f#format = LittleEndian` / `BigEndian`, `f += value` and `f#read as T`.
Found by `rule_tags.py sections`, which resolves every signature row against the stdlib
source, on the day it was written.  Guard: `tests/reference/skill-files.loft`.

