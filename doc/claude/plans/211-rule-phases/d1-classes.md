<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN211 D1 — the routines over 2×, by the rule that refuses them (measured 2026-11-10)

The input to D2's order.  Produced by

```bash
python3 bench/stats.py --lanes rust,cship --no-local --cc gcc|clang <portal.library_packages()>
```

on tip a0da7073f (gcc 15.3, clang 22.1.8): 220 routines, median 1.33× (gcc) / 1.31× (clang),
none over 3× on gcc, one on clang (`boundary_loops`, 3.01×, layout: below).  33 routines are
over 2× on either compiler.  Each was profiled from a `clang -O3 -g` build of its package's
ship C (`perf record`, pinned), and its hottest emitted lines were read beside the Rust twin
and the refusing rule's trace (`LOFT_TRACE_VERSION`, `LOFT_TRACE_TEXT_BORROW`, …).  A class is
a decline reason seen at the hot site.

## The classes, by the time they carry

| # | class: the rule and why it declines | routines (gcc / clang) |
|---|---|---|
| 1 | **`(R-Range)`'s INDEX clause proves only the lower bound.**  An index the loop or a guard keeps below `len(v)` still pays `LOFT_VH_OUT` (and the wrap, where the lower bound is unproved).  Spellings seen: a counter of `0..n` with `n = len(v)`; a guard `i < len(v)` / `i + k < n` (the parser records it as `index_bounded`, a typing fact the writers never read); `i % n` with `n = len(v)`; a stepped `while` variable under `pos > 0`. | hex_form boundary_loops (1.86 / 3.01), random indices (2.22 / 2.21), graphics fill_polygon (2.23 / 2.49), drawing smooth (1.88 / 2.49), drawing render_marks (1.70 / 2.34), hex_recover forms_upto (2.17 / 1.82), pluginabi request (2.26 / 2.19), zttext materialise (1.58 / 2.35), zttext flow_layout_full (2.58 / 2.55) |
| 2 | **`(R-Version)` takes only a LEAF**: an accessor that calls an index helper is refused ("no leaf: calls hs_index", "calls hz_index"), so every cell read pays checked `loft_sub`, the window tests, the bounds test and the null read. | hex_place field_union (2.13 / 2.34), hex_roof roof_match (2.66 / 1.88), hex_roof roof_cone (2.22 / 1.40); boundary_loops' scan half |
| 3 | **A copy where the twin borrows or moves.**  `(R-TextBorrow)`: a loop variable reached through a field ("the loop variable is not a text").  `(R-MoveLast)`: a parameter moved into a field (`Doc { buf: d.buf, … }` copies the buffer).  View return: `pa_get` answers `OpCopyRecord`.  `(R-Place)`/`(R-Destination)`: a text result appended to a place is built, copied out, then appended. | server header (2.04 / 2.27), zttext delete_range (1.47 / 2.13), pluginabi req_state_b64 (2.87 / 1.63), zttext materialise, flow_layout_full, arguments parse (`Pos { word: arg }`) |
| 4 | **A record walk holds no record pointer on C**: `(R-RecPtr)`'s `/nested` clause is native only, and `/text` does not reach a field read inside a walk; a `&`-slot cursor is decoded on every read. | 17 slope_path_with_undo (2.30 / 1.78), arguments parse (1.95 / 2.36), 21_tree tree_query (1.03 / 2.13) |
| 5 | **A null discharge is a branch behind `LOFT_COLD_PATH()`** (`asm volatile("")`), which forbids the select: `?? 0` and a float `?`.  A writer heuristic, not a rule. | boundary_loops (clang), 11_par par (2.63 / 2.73, the `?` in `newton_sqrt`) |
| 6 | **A call not inlined, or across a unit**: a multi-statement callee on constant arguments (`RandStream::get`'s `% (hi - lo + 1)` stays a divide), a `[c]` shim archive (`lr_rand`), `__inv` callees. | random get (2.07 / 2.54), random rand (2.11 / 2.05), stage pack_instances (2.04 / 2.04) |
| 7 | **Recursion pays the frame and store plumbing per level**: `(R-Leaf)` refuses a self-call; the recursive-builder class of `(R-Place)`/`(R-Destination)`. | 21_tree build_rec (2.08 / 1.98) |
| 8 | **Keyed runtime per call**: no rule declines in build_index (the out-of-line `loft_keyed_entry_ints`; its gcc gap is inlining and layout); field_add_cell looks the key up again after a conditional insert — `(R-SameRead)`'s filed clause wants a `Var` key in the next statement (here `k.ck`, the insert in a branch arm). | gridmesh build_index (2.57 / 1.40), gridmesh field_add_cell (2.30 / 2.03) |
| 9 | **`(R-CharWalk)` keeps the size test per step** "because a body may grow T", where no statement writes T. | time parse (1.51 / 2.06) |
| 10 | **Runtime, no rewrite rule**: the generic JSON lexer and printer (an owned token per peek), a per-element cursor decode (`(R-StreamWalk)` is native only and deferred), a per-element serialiser (`loft_ser2` per `single`), a single-byte `find`. | 17 map_json (2.21 / 1.94), 18 binary_read (2.00 / 2.29), glb save_glb (2.51 / 2.53) |
| 11 | **Element width**: a `vector<integer>` canvas writes 8 bytes a pixel where the twin writes 4; the fill is memory-bound. | graphics canvas (2.11 / 2.10), graphics clear (2.01 / 2.01) |

Not classed: 23_lru lru_mixed (set aside), drawing parse (no samples at its size).  The
null-integer conversions in drawing `brush_sample` and stage `i2s` are the owner's deferred
focus.

## Class 1 priced by hand: boundary_loops

The label propagation reads `ea[pi] ?? 0` and its siblings in an `ne × ne` nest, `ne =
len(ea)`.  Hand edits of the emitted C, in this one function ( every hash
`6c9375d9`; twin 6.05 ms):

| form | gcc | clang |
|---|--:|--:|
| as emitted | 11.18 ms | 18.24 ms |
| `?? 0` as a select (class 5) | 11.17 | 14.61 |
| `ea`'s reads without the bounds test, plus the select (the provable part) | 9.29 | 9.28 |
| every read without the bounds test, plus the select | 6.72 | 6.68 |
| without the bounds and the null tests | 4.13 | 4.13 |

The provable part is the reads of `ea` (the counters run below `len(ea)`, and nothing resizes
`ea` in the nest).  `eb` and `comp` have the same length, but that is a fact of their
construction loops, which this clause does not see.  The shape is common: 190 `for i in
0..len(v)` loops and 69 `n = len(v)` locals across the libraries.

## Layout, noted

`boundary_loops` on clang moved 14.9 → 18.2 ms across the rebase onto main dbf8b05ae with its
own C byte-identical: the ship C differs only in the hash runtime (main's `loft_h_filed`),
clang's `n_boundary_loops` is the same 2022 instructions, and its start moved from a 64-byte
boundary (0x36280) to 0x362b0.  `build_index` and `req_state_b64` did not move (interleaved
A/B of the two binaries, two rounds, 11 samples).
