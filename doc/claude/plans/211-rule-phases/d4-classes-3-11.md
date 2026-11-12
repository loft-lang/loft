<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN211 D4 — classes 3 to 11: priced, and built only where the win is clear

D1's classes 3–11 ([d1-classes.md](d1-classes.md)), worked through on 2026-11-11 under the
owner's rule: no optimisation without a clear win in a measured routine.  Each class: its
heaviest routine profiled by callgrind on the ship C, the mechanism read off the hot lines, a
hand form of the C priced (min of 11 interleaved runs, `bench/portal/ab.py`, hashes equal),
and built only where both compilers gain.

| class | routine | mechanism found | hand price gcc / clang | verdict |
|---|---|---|--:|---|
| 3 copies | server `header` | the loop variable `h` was promoted to the text return buffer, so every turn copied its element in and `(R-TextBorrow)` refused it | −53.5 / −58.8 % | **BUILT** (`54683568a`): a `for` loop's variable is never the return buffer; −55.1 / −56.3 % |
| 3 | pluginabi `req_state_b64` | the base64 alphabet sliced one character at a time (UTF-8 boundary tests) | −13.2 / +4.8 % | not built: clang loses |
| 3 | zttext `flow_layout_full`, arguments `parse` | text copied into records (`Run.str`, `Pos { word }`) where the twin borrows or moves | — | not priced: needs texts in records to borrow, a representation change |
| 4 record walks | moros `slope_path_with_undo` | the entry walk holds its record pointer already (the one list walker, `a881ab231`); `map_get_hex`'s chunk walk does not | −3.7 % instructions | not built: no clear win |
| 5 cold path | hex_form `boundary_loops` | `LOFT_COLD_PATH()` on an integer `?? 0` stops the select | all markers: −9 / −36 %, but `par` +49 / +23 % | **BUILT** narrow (`9340ef4b4`): an integer's `?? literal` is unmarked, a float keeps it — `boundary_loops` −9.1 / −35.8 %, `par` unchanged |
| 6 across units | random `rand` | `lr_rand` in a separately linked shim archive: `% (hi - lo + 1)` stays a hardware divide | −50.0 / −45.1 % | **BUILT** (`c3defa8ff`, `be1328a94`): the release tier compiles a `[c] shim` with the unit under `-flto`, and `stats.py`'s ship lane mirrors it; `rand` 2.06× → 1.11×, `rand_indices` → 1.06× |
| 6 | random `get` | inlined already; the rest is null and overflow tests | — | the parked null-integer cost |
| 7 recursion | 21 `build_rec` | spread: call plumbing ~36 %, store addresses 14 %, child adoption 11 % | — | not priced: a calling convention, not a rule |
| 8 keyed | gridmesh `field_add_cell` | the re-lookup is already only on the insert path; the rest is the hash runtime | — | a runtime project |
| 9 char walk | time `parse` | the size test per step, after the fast step proved it | +41.0 / −22.6 % | not built: gcc loses |
| 10 runtime | glb `save_glb` | a vector of singles serialised element by element | −43.5 / −45.2 % | **BUILT** (`67fb02fd3`): a vector whose element's written bytes are its stored bytes is one block; −43.0 / −44.3 % |
| 11 width | graphics `canvas`, `clear` | 8 bytes a pixel where the twin writes 4 | — | parked: needs a library change |

The guards: `tests/scripts/a-text-walks-loop-variable-returned-is-copied-out-and-borrowed.loft`,
`tests/scripts/a-vector-of-raw-scalars-is-written-as-one-block.loft`, the `c_driver` unit test
`a_release_unit_compiles_its_shims_from_their_sources`, and the `c_emit` test
`an_integer_literal_fallback_is_unmarked_and_a_float_one_keeps_its_cold_path` — each with its
falsification receipt.
