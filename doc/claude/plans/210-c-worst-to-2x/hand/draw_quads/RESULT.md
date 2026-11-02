# text2d `draw_quads` hand-priced to 2x (C ship lane, tip 3cfb11fb9)

Driver: `pkg/` (text2d copy, both mains cut to `bench_draw_quads`), `base.c` = the ship
unit (`LOFT_RELEASE_PASS_PROBE=1 --emit-c`).  Measured on CPU 16 (E-core, `cpu_atom` PMU),
`ps.sh`: per op = (run at --n 400 − run at --n 1) / 399, `perf stat -r 3`.  Every step
prints hash `becf0b85` (= the twin's) on gcc and clang.  Steps: `step-N.py`, `mk.sh N`.

## 1. Baseline

| build | instr/op | cycles/op | ns/op | × twin (cycles) |
|---|--:|--:|--:|--:|
| Rust twin | 2.24 M | 0.935 M | 256 k | 1 |
| C ship, gcc | 9.81 M | 2.74 M | 750 k | 2.93 |
| C ship, clang | 12.72 M | 3.25 M | 864 k | 3.48 |

(stats.py on the P-core: gcc 3.10×, clang 3.46×.)  An op is 2,000 quads × 140 texels =
280 k texels: the twin runs 8 instructions a texel, gcc 35, clang 45.

## 2. Profile (gcc, `perf annotate` of the texel loop)

Frames are compiled out in the ship unit, so `get_pixel__inv`/`set_pixel__inv` wrappers cost
nothing and both accessors inline into the texel loop; gcc also drops their dead
`loft_rec_ptr(self)`.  What is left per texel (~29 instructions on the miss path):

| cost | where |
|---|---|
| 4 compares + branches | `get_pixel`'s `gx < 0 or gx >= width or gy < 0 or gy >= height` (gy invariant in the px loop, not unswitched) |
| null test + `jo` | `gy * width + gx` is a checked `loft_add` |
| 2 compares | `idx < len(d)`, then `loft_vh_index`'s own `f < 0` / `f >= len` again |
| 1 reload | the data base spilled to the stack (`-0x1e8(%rbp)`) |
| load (17 % of cycles on the compare after it) | the sheet texel |
| null compare | `gp_d[idx] ?? 0` |
| the same chain again on a hit | `set_pixel`, for the set texels |

The twin clips ONCE per quad (`lo`/`hi` columns, row tests per row) and walks two row slices.

## 3. Steps (cumulative; cycles × the twin's 0.935 M)

| step | edit in the C | gcc instr | gcc cycles | gcc × | clang instr | clang cycles | clang × |
|---|---|--:|--:|--:|--:|--:|--:|
| 0 | as emitted | 9.81 M | 2.74 M | 2.93 | 12.72 M | 3.25 M | 3.48 |
| 1 | the pixel index `gy*w + gx` as a wrapping add (both accessors) | 9.12 M | 2.61 M | 2.79 | 10.84 M | 2.84 M | 3.04 |
| 2 | the element read/write after `0 <= idx < len` is proven: `base + idx*8`, no `loft_vh_index` | 8.80 M | 2.98 M | 3.19 ¹ | 5.79 M | 1.53 M | 1.64 |
| 3 | the element's `?? 0` null test dropped (integer-null cost) | 7.68 M | 2.87 M | 3.07 ¹ | 5.34 M | 1.43 M | 1.53 |
| 4 | **per-row versioning**: if the row's two ends lie inside both canvases and both data lengths, a texel loop with no tests (`if s != 0: t[i] = colour`); else the loop as before | 2.64 M | 1.08 M | **1.15** | 2.69 M | 1.18 M | **1.27** |
| 4 alone | step 4 on the unedited unit, element null test KEPT in the fast loop (exact) | 2.99 M | 1.11 M | **1.18** | 2.45 M | 1.11 M | **1.19** |

¹ gcc placement: from step 2 on, gcc moves the in-range path out of line and spills three
loop invariants to the stack (`0x28/0x30/0x38(%rsp)`); fewer instructions, more cycles.
Layout, not work (owner: no layout fixes) — and moot once step 4 removes the loop.

ns/op at step 4 alone: gcc 302 k, clang 304 k, twin 256 k.

## 4. Mechanisms

| step | mechanism | existing rule? | C / native | safe when | shares the shape |
|---|---|---|---|---|---|
| 1 | a checked `+` whose operands are non-null and whose result the dominating compares bound | `(R-Range)` / `(R-RangedCall)` decide it by intervals | native only (`src/generation/ranged_call.rs`, `range.rs`); NOT on the C | `0 <= gx < w`, `0 <= gy < h` dominate, `w*h` fits | every accessor `y*w + x` (graphics `get/set/blend_pixel`) |
| 2 | an element access dominated by an explicit `idx < len(v)` and a `>= 0` fact needs no second range test | none for a tested index; `(R-BoundedNest)`'s raw read covers only an accumulate nest (native) | NEW on the C (small: `loft_vh_get_int`'s fold is redundant after the source's own test) | the test dominates and names the same index and the held header | the same accessors, fill_rect's `if fr_idx < fr_n` |
| 3 | integer-null cost on an element read | owner: set aside for now | — | an element bound proving no stored null (`(R-BoundedNest)`'s element bound) | every integer-vector read |
| **4** | **loop versioning of affine branch conditions**: an innermost counted loop whose body (after inlining small accessors) tests `a + k·i < L`, `a + k·i >= 0`, `idx < len` with `a`, `L` invariant — one guard at entry tests the range's two ends, the copy takes the in-range arm of every such test (and with it steps 1–2 for free) | NEW.  Closest: `(R-BoundedNest)`'s affine end test (`both ends in [0, len)`), but that rule admits only an accumulate body and proves arithmetic/reads, not an accessor's bounds BRANCHES | neither backend; `R-BoundedNest` and `R-RangedCall` are native-only | every test affine in the counter (one occurrence), limits invariant over the loop (the body writes no `w`, `h`, nor the vectors' lengths — writes to elements are fine, order is unchanged) | graphics `fill_rect` (`if fr_idx < fr_n` per pixel), `hline` under `fill_polygon` / `fill_triangle`, drawing's per-pixel `set_pixel`/`blend_pixel` loops (`fill_circle`, `fronds`) — to be checked in their own pricing |

The library alternative (the twin's algorithm: clip once per quad, walk row slices) gives the
same machine code by hand; step 4 is the general form that gets it from the per-texel
accessor source as written.

## 5. What is left

At step 4 alone: 1.18× (gcc) / 1.19× (clang).  The rest is the per-quad prologue (`%`, `/` by
`a_cols` with their null tests, the quad's record address) and the per-row guard — 2,000 quads
and 28,000 rows against 280 k texels.  Nothing further needed for 2×.
