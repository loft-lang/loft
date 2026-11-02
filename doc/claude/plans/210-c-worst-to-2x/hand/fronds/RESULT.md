# drawing `fronds` and `fill_circle` — hand-priced on the C ship lane

Tip 3cfb11fb9, `LOFT_RELEASE_PASS_PROBE=1 --emit-c`, gcc/clang `-std=c99 -O3`.  Drivers: the
drawing bench with both mains cut to the one routine (`pkg/` fronds, `pkgc/` fill_circle).
Measured with `m.sh` on CPU 14 (an E-core): per-op = (run at n=N − run at n=1)/(N−1), 5 repeats,
twin on the same core.  Instructions are the primary evidence.  On this E-core the cycle ratios
read lower than stats.py's P-core table (fronds 2.31× here vs 3.29× there; fill_circle 2.43× vs
2.64×); the instruction ratio is the comparable one.  Every step: same hash.

## fronds (twin 758 k instr, 283 k cyc per op)

Profile at baseline (gcc): `n_fronds__fr` self 24 %; store mint/claim work ≈ 40 %
(`loft_st_claim_inner` 10, `loft_bits_insert` 10, `loft_database_room` 5.6, `loft_st_init` 3.4,
`loft_database` 2.9); deep copies ≈ 24 % (`loft_copy_claims` 10.6, `loft_vec_add` 8.7, memmove
2.8); `sin` 10 %.  The twin runs the same algorithm (3 mallocs a frond, two MOVES); per final
frond the C builds `pts`/`wid` in two scratch stores, deep-copies them into the `fd_out`
element (`loft_vec_add` ×2), and the parent deep-copies the whole depth-2 result again
(`loft_copy_record` per frond).

| step | edit | gcc instr | gcc cyc | × twin (instr / cyc) | Δ instr |
|---|---|--:|--:|--:|--:|
| 0 | as emitted | 2,273,817 | 655,135 | 3.00 / 2.31 | |
| S1 | `fd_pts`/`fd_wid` built INSIDE the appended `fd_out` element's fields (element minted first; no scratch stores, no `loft_vec_add`) | 1,282,564 | 390,994 | 1.69 / 1.38 | −44 % |
| S2 | `for f in fronds(…) { fd_out += [f] }` → an append twin `n_fronds__ap` writing into the caller's `fd_out` (its reads of `fd_out` offset by the entry length); no sub-result, no `loft_copy_record` | 859,709 | 275,906 | **1.13 / 0.97** | −33 % |
| S2 alone (on step 0) | | 1,850,163 | 540,799 | 2.44 / 1.91 | −19 % |

clang: 2,376,500 / 677,867 (3.13× / 2.40×) → after S1+S2 941,496 / 302,945 (1.24× / 1.07×).

Stopped at S2 (past 2×).  Not priced, still there: the per-`i` `sides` vector literal minted in
a kept store (`[1.0, -1.0]`, iterated only), the per-`k` `FrondSpec` record minted and freed
(120 bytes, `loft_database_typed`), and `cos`/`sin` of the same `phi` called twice each.

## fill_circle (twin 207 k instr, 74 k cyc per op)

Profile at baseline: 98 % in `polygon_generic`; 60 % in the inlined span writer
`pil_hline__inv` — per pixel `loft_vh_set_int` → `loft_vh_index` (negative-index map + bounds
test) and the range loop's null-range/two-exit control, so neither compiler vectorises it.  The
twin's `cv.data[base + x] = ink` has its bound check hoisted by LLVM and the fill vectorised.

| step | edit | gcc instr | gcc cyc | × twin (instr / cyc) | Δ instr |
|---|---|--:|--:|--:|--:|
| 0 | as emitted | 737,203 | 179,742 | 3.57 / 2.43 | |
| F1 | the span loop as ONE range test (`base+lo >= 0 && base+hi < len`) and a plain store loop through the held header's base; the per-element loop kept as the fallback | 306,410 | 104,126 | **1.48 / 1.40** | −58 % |

clang: 869,296 / 231,417 (4.21× / 3.12×) → 378,795 / 109,601 (1.83× / 1.48×).

What is left (gcc profile after F1): the span fill itself 18 % (the twin's too), `roundf`
through `loft_math1` 11 %, the edge table and crossings read through `loft_vh_get` /
`loft_vh_index` ≈ 18 %, `loft_rec_ptr` 8 %.  That is the routine's own algorithm with the
store's index tests; no single further lever.

## The mechanisms — all three are NATIVE rules the C writer does not emit

The analyses live in `src/opt/hoist.rs` (shared); only the rustc writer (`src/generation/`)
emits them, and `src/c` names none.  The rustc emission of this same bench admits each on
these exact functions (`LOFT_REWRITE_CENSUS_FN`: `n_fronds` R-ElemFirst 1, R-MoveAppend 1;
`n_pil_hline` R-Fill 3).

| step | rule | safe when | native | C | other rows with the shape |
|---|---|---|---|---|---|
| S1 | `(R-ElemFirst)` — a local vector consumed once as a record-literal field of an append is built inside the appended element | declaration and append top-level in one block (or the append in each arm); nothing between jumps out or names `out` | built | **missing** | any `out += [R { v: local_vec }]` build loop: graphics/text2d quads, zttext pieces, hex_* forms |
| S2 | `(R-MoveAppend)` — `for f in call(…) { V += [f] }`: the call's buffer placed in V's store and each element's bytes relocated, not deep-copied.  The hand form went one step further (an append twin, the `(R-AppendTwin)` window), which is what a recursive builder wants | the loop variable's only use is that append; V an owned local plain vector, never rebound | built | **missing** | recursive/fractal builders; `(R-AppendTwin)` is already on the C for `X += f(args)` |
| F1 | `(R-Fill)` — a counted loop `for i in lo..hi { v[base + i] = c }` as one range test and one fill | every index in `[0, len)`, range non-empty; else the per-element loop | built | **missing** | graphics `fill_rect` / `canvas` / `clear` (span fills), drawing `fill_star`, `wide_line`, `render_*` |

Order for the C: `(R-Fill)` first (smallest port, a loop-level rewrite with a fallback, and it
should reach the graphics trio too), then `(R-ElemFirst)`, then `(R-MoveAppend)`.
