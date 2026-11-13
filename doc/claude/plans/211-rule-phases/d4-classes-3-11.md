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
| 11 width | graphics `canvas`, `clear` | 8 bytes a pixel where the twin writes 4 | — | **BUILT** (2026-11-12, § Class 11): graphics 0.12 stores `vector<u32>`; `canvas` 2.14 → 0.98x, `clear` 2.01 → 1.00x |

The guards: `tests/scripts/a-text-walks-loop-variable-returned-is-copied-out-and-borrowed.loft`,
`tests/scripts/a-vector-of-raw-scalars-is-written-as-one-block.loft`, the `c_driver` unit test
`a_release_unit_compiles_its_shims_from_their_sources`, and the `c_emit` test
`an_integer_literal_fallback_is_unmarked_and_a_float_one_keeps_its_cold_path` — each with its
falsification receipt.

## Class 11 — the pixel width (2026-11-12)

`u32` now holds all 32 bits (`c1a1fa1cf`), so graphics' `Canvas.data` became a `vector<u32>`
(loft-libs-graphics branch `u32-pixels`: graphics 0.12, drawing 0.5, stage 0.20).  The library
change alone made every per-pixel WRITE slower on the C: a `u32` element was written through a
record reference and a store lookup, and `(R-Fill)` admitted 8-byte and `f32` elements only.
`60b40f7ff` gives the C writer the held 4-byte store (`loft_vh_set_u32*`) and `hoist::fill_loop`
the fill kind `u32` (C only; the Rust writer declines it).  C ship lane against the Rust twin,
min of 7, hashes equal:

| routine | `vector<integer>` | `vector<u32>` | + the store and the fill |
|---|--:|--:|--:|
| canvas | 2.14 | 1.00 | **0.98** |
| clear | 2.01 | 4.72 | **1.00** |
| fill_rect | 1.54 | 4.44 | **0.94** |
| fill_polygon | 2.18 | 4.88 | **2.05** |
| fill_triangle | 0.75 | 1.64 | **0.65** |
| blend_pixel | 1.22 | 1.45 | 1.35 → **1.02** (fast twin) |
| draw_line | 0.87 | 1.02 | 0.95 |
| draw_bezier | 1.95 | 1.91 | 1.94 |
| median | 1.74 | 1.78 | **0.99** |

`blend_pixel`'s last 10 % was `@FR-R-Version`: the front plan (`opt/version.rs`) admitted
8-byte, float and boolean element accesses only, so the `u32` read and write left the leaf
with no proven access (`LOFT_TRACE_VERSION=1`: "proves no element access") and every pixel
took the checked path.  The plan now admits every narrow element read (`u32`, `u8`, `i16`,
`single`, an enum: the fast twin reads them through `loft_vh_at_addr`) and the `u32` write
(`loft_vh_put_u32`).  `blend_pixel` 1.35 → **1.02x** on gcc, **0.92x** on clang; the rest of
the package unchanged.  Guard: `c_emit::a_leaf_over_narrow_elements_gets_its_fast_twin`.

`draw_line` (0.94x gcc, 1.05x clang, against 0.87x for the 8-byte build) is closed with no
change: the gap is the bench canvas's row stride, not code.  `set_pixel`'s call is declined by
`@FR-R-Version` in both builds (its coordinates step by the error term, not linearly in the
counter), and gcc emits the same hot loop for both, instruction for instruction; only the
out-of-line store differs, 4 bytes for 8.  callgrind over `bench_line`: the u32 build runs
1.7 % fewer instructions (311.7 M against 316.9 M) with the same simulated branch misses,
yet takes 6 % longer (gcc, one core: 4.44 → 4.73 ms).  The 1024-pixel row is exactly 4096
bytes as `u32`, so each pixel of a steep line lands in the same L1 set.  Widening the bench
canvas to 1056 pixels turns it round — u32 3.63 ms against 8-byte 3.91 ms, both faster than
at 1024 — and on clang the two builds already tie at 1024 (5.2 ms each).  The Rust twin pays
the same stride.  Nothing in the compiler would move it; padding the canvas rows is a library
layout choice (callers index `data` as `y * width + x`), not proposed.

`draw_bezier` (1.96x gcc, 1.79x clang) is instruction count: callgrind over the routine gives
212 M instructions against the Rust twin's 98 M, the pixel loops about equal (42 against 36
per pixel).  The rest is the subdivision stack — two `vector<integer>` cut back with
`v = v[0..top]` and refilled with `v += [a, b, c, d]` per split, where the twin pushes one
`[i64; 8]`.  Per split 575 instructions against 97; per iteration 210 against 50.  BUILT the
largest piece, `@FR-R-PushFill`'s held-run clause (`rewrites.md` § The run's held-header
clause): a run of pushes through a header the loop already holds is one room test, the
elements and one length bump, where each push tested the room, resolved the store twice and
wrote the length.  Package bench, C ship lane against the Rust twin: gcc 3.18 → 2.82 ms
(**1.96x → 1.74x**), clang 2.90 → 2.56 ms (**1.79x → 1.58x**); the other seven routines
unchanged (median 0.99x gcc, 1.01x clang).  Priced,
not built: the eight pops read through `loft_vec_get` (about 13 instructions each: the store
table and the store's base are reloaded per read) — reading through the loop's push header
gains 9 % alone, nothing on top of the run clause in the hand form.

`fill_polygon` (2.04x gcc, 1.50x clang) had two causes, both BUILT.  On gcc the edge loop
divided: `polygon_crossings` reads `pc_pts[(i + 1) % pc_n ?? 0]?`, and gcc keeps the `%` as a
64-bit division on every edge (2.78 M per callgrind run) where clang and rustc turn it into a
wrapping counter.  The wrapped-counter clause now writes a step of 1 as `i == n - 1 ? 0 : i + 1`
(`i + 1 < n ? i + 1 : 0` cost clang 5 % more instructions): gcc 485 → 325 µs.  On both, the
crossings buffer is cleared once per scanline through the release walk, which reads the type
descriptors to learn that an `integer` owns nothing; a local vector of scalars now clears with
the length reset: gcc −6 %, clang −9 %.  Package bench, C ship lane against the Rust twin:
gcc **2.04x → 1.32x**, clang **1.50x → 1.34x**; the other routines unchanged.  Left: the
callee clears its buffer twice in a row (one clear priced at a further 2 %, gcc only — not
built), and the per-scanline window derivation (41 instructions).
