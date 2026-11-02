# graphics: fill_polygon, fill_rect, canvas — hand-priced to ≤ 2x (C ship lane)

Tip 3cfb11fb9.  Drivers run one routine each (`pkg/bench/b_{poly,rect,canvas}.loft`, twins
`twin_*.rs`), CPU 0, `ps.py`: per-op counts from the difference of runs at N and 2N.  Every
step printed the baseline hash (poly 91eac327, rect 95dade18, canvas 76bd65c5).  Steps are
scripted: `step_rect.py`, `step_poly.py`, `step1_canvas.py`; outputs `s<k>_<routine>.c`.

## Baseline

| routine | twin | C gcc | C clang | instr/op twin / gcc / clang |
|---|--:|--:|--:|---|
| fill_polygon | 285 µs | 900 µs (3.1x) | 1126 µs (3.9x) | 6.6 / 19.8 / 29.3 M |
| fill_rect | 2.15 ms | 6.42 ms (3.0x) | 9.17 ms (4.2x) | 4.7 / 137 / 223 M |
| canvas | 197 µs | 582–659 µs (3.1x) | 574–682 µs (3.1x) | 0.8 / ~0 / ~0 M (rep-movs copies) |

Profiles: fill_rect and polygon are one inlined function each (the pixel loop; 92 % in
`fill_polygon__fr` on gcc); canvas is 92.7 % `memmove` (the doubling copies of `filled()`).

## Steps (cumulative per routine)

| routine | step | edit | gcc | clang | instr gcc / clang |
|---|---|---|--:|--:|---|
| fill_rect | R1 | index add `y*w + x` wraps (both operands non-null) | 6.61 ms (placement noise) | 8.92 ms | 137 / 174 M |
| fill_rect | R2 | store without `loft_vh_index`'s negative-index branch (loop versioned on width ≥ 0) | 3.55 ms | 4.79 ms | 76 / 88 M |
| fill_rect | R3 | the per-pixel `idx < n` guard becomes the row's end: one plain store loop | **3.33 ms (1.51x)** | **3.34 ms (1.51x)** | 20 / 23 M |
| fill_polygon | P1 | `hline`'s span loop as R2+R3 | 535 µs (1.83x) | 672 µs (2.29x) | 11.1 / 17.0 M |
| fill_polygon | P2a | crossings: pc_a / pc_b address `h.base + i·16`, null view + fallback kept; pc_ty / pc_by copy the address they name | 489 µs | 564 µs | 9.5 / 13.6 M |
| fill_polygon | P2 | + index proven in range: no null view, no fallback mint, no `!p` test on reads | **496 µs (1.69x)** | **376 µs (1.28x)** | 8.5 / 8.8 M |
| fill_polygon | P3, P4 | `(i+1) % n` without its null test; the double clear at entry once | 0 | 0 | — |
| canvas | C1 | `filled()`: reserve `count` once, one store loop (no doubling copies) | **406–427 µs (2.1x)** | **418–428 µs (2.1x)** | 2.6 / 1.3 M |

Library spellings of `filled()` under today's C: `reserve(fi_out, count)` before the doubling
582 µs (nothing: the copies are the cost, not regrowth); `[for _ in 0..count { value }]`
496 µs (7 instructions an element, a push each).

## What is left: the pixel width (library representation)

A twin with 8-byte pixels (`twin64_*.rs`, `Vec<u64>`): fill_rect 3.48 ms, canvas 428 µs,
fill_polygon 260 µs.  After R3 and C1 the C MATCHES or beats it (3.33 ms, 406–427 µs), so the
rest of fill_rect's and canvas' gap (1.5x, 2.1x) is `Canvas.data: vector<integer>` moving twice
the bytes of the twin's `Vec<u32>`.  fill_polygon does not depend on the width.

## The mechanisms

| # | mechanism | rule / where | routines | condition |
|---|---|---|---|---|
| 1 | **`(R-Fill)` + its guarded clause on the C** — `for i in lo..hi { t = base + i; if t < N { v[t] = c } }` runs as one range test and one fill of `lo..min(hi, N - base)` | formal/rewrites.md (R-Fill); native: `hoist::fill_loop`, `Output::fill_fast_path`, `Stores::fill_hoisted` — fires on this exact `fill_rect` loop and on `hline` (`//@FR-R-Fill guarded` in the native emission).  **Nothing in src/c emits it.** | fill_rect (R2+R3, −48 % gcc / −64 % clang), fill_polygon via `hline` (P1, −41 %); likely every span filler: graphics `clear`, `fill_triangle`, drawing `fill_circle` (2.64x gcc / 3.93x clang) — not measured | the rule's own run-time decline (negative index, past the end, overflow, span < FILL_MIN_SPAN) replaces my width ≥ 0 versioning |
| 2 | **a record element read from a held header with a base: address `h.base + i·size`, and no null view when the index is proven in range** | (R-RecPtr)'s address taken from (R-Base)'s held base instead of `loft_rec_ptr` (store lookup); (R-InRange) extended: the bound is a local bound to `len(v)` (`pc_n = len(pc_pts)`; `for i in 0..pc_n`) and `(i + 1) % pc_n ?? 0` is in range when `0 ≤ i < pc_n`.  A copy of a view (`pc_ty = if … { pc_a } else { pc_b }`) copies the address.  The earlier dropped try paid a run-time record/store test per access; here the proof is static: the loop is in the branch where the output store ≠ pts' store (already split by the push window), so pts' base holds | fill_polygon P2 (gcc −7 %, clang −44 %); the clang half of it is the in-range proof (P2a alone −12 %) | (R-Base) frame flag true; index proven in [0, len) |
| 3 | **a comprehension whose body is invariant — `[for _ in 0..n { c }]` — emits a reserve of n and one fill**; then the library's `filled()` returns to that spelling (its doubling exists only because the comprehension cost 3 bytecodes an element on the interpreter) | NEW clause (R-Fill's comprehension case; (R-PushFill) reserves runs but still pushes per element) + a loft-libs-graphics change | canvas C1 (−30 %); any `vector` built of one repeated value | trip count known before the loop; body reads nothing the loop writes |
| 4 | **library: `Canvas.data` as `vector<u32>`** | loft-libs-graphics; API change (`save_png_raw` takes `vector<integer>`; every pixel store needs `as u32 ?? …`) | fill_rect −35 % est., canvas −50 % est. (twin32 vs twin64) | — |

Not memory model / priced at zero: `%` null test, the double `loft_vec_clear_release`,
the wrapping add (R1) on gcc.

## End state (gcc / clang, × twin)

| routine | baseline | after 1+2+3 | after 4 (est. from twin32/twin64) |
|---|---|---|---|
| fill_polygon | 3.1 / 3.9 | 1.69 / 1.28 | same |
| fill_rect | 3.0 / 4.2 | 1.51 / 1.51 | ≈ 1.0 |
| canvas | 3.1 / 3.1 | 2.1 / 2.1 | ≈ 1.0 |
