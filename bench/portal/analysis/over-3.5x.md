<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The routines over 3.5×, and what each needs to reach about 2.5× (2026-10-23)

The owner is considering a cut at 3.5× Rust: a routine above it is handled as a bug, and the bar
comes down when that becomes feasible.  The cut takes effect only once the rows below are fixed:
until then they are this work list, not bugs.  This is that list on this laptop.

**Ratios** are `bench/stats.py` on the final L7 build (`df40d04a8`), production runtime, fastest
cores; `arguments/parse` (3.93×) is no longer on the list: loft#2135 (in the #2139 join) took it to 3.46×.
**Where the gap lives** is `scripts/gap_report.py --lean` for each routine (instructions,
callgrind; the reports were in `/var/tmp/loft-gap35/`).  An instruction share is an ESTIMATE of a
time share; only `par` is priced here (`bench/portal/hand_price.sh`).  A routine at R× needs
`1 − 2.5/R` of its time removed: 29–34 % for every row below.

| routine | now | needs | the gap is in | lever (estimate) | owner decision? |
|---|---|---|---|---|---|
| keyed `grouped_fill_find` | 3.76× | −34 % | the group APPEND's generic record path | direct group append; fast find on a borrowing table (≈ −35 %) | no |
| text2d `draw_quads` | 3.64× | −31 % | per-pixel bounds of the inlined `get_pixel`/`set_pixel` | split the `px` range once per row (≈ −45 %) | no |
| hex_terrain `terrain_relief_pass` | 3.60× | −31 % | `hex_neighbor` not inlined; checked float reads; null-int | inline the early-return leaf; reads under the proven guard (≈ −25 %); range proofs for the rest | no |
| hex_recover `forms_upto` | 3.57× | −30 % | runtime: store checks, per-iteration vector literals, clears | apart literals, store checks per record (≈ −30 %, three arcs) | lean rlib |
| graphics `draw_bezier` | 3.55× | −30 % | the two subdivision stacks: `v = v[0..k]` as a runtime call, checked pushes and reads | the stacks held apart (`(R-Apart)`, priced 2.3× in `over-9x.md`); or the prefix truncate as a length store (≈ −12 %) | no |
| hex_shape `wall_chain_walk` | 3.55× | −30 % | the emitted lines themselves (3.07× the twin) | range proofs over the integer arithmetic, which let LLVM fold `eg_dir_from` as it does the twin's (≈ −40 %) | no |
| server `header` | 3.54× | −29 % | the walked header text copied per element | borrow `h` where the body writes a store only on its exit path (≈ −30 %) | no |
| par `par` | 3.52× | −29 % | the per-item kernel, not the threads | the 50-round loop unrolled despite its NaN test (PRICED −39 % → 2.15×) | no |

Just under the cut, and the next to cross it on a bad day: `graphics/fill_polygon` 3.47×,
`slope_path_with_undo` 3.46×, `arguments/parse` 3.46×, `random/rand_indices` 3.43×,
`cbor/encode_bytes` 3.40×.

**Only `forms_upto`'s deepest store-check cut needs an owner decision.**  Two rows reach 2.5×
only by removing null-integer checks (`wall_chain_walk`, `terrain_relief_pass`).  That is PROOF
work, not a semantic change: C120 and C129 already settle that the checks are the semantics and
are dropped only where a fact before the operation shows they cannot fire (a counted range, a
declared `integer(lo, hi)`, a mask, a tested guard), so every value, null and overflow report
stays as it is.  The rewrites that do it are `(R-Range)`, `(R-GuardedChain)` and
`(R-BoundedNest)`; the work is widening what they can prove.  Dropping the checks outright
(wrapping arithmetic) would change what users see and is declined.

## Per routine

### keyed `grouped_fill_find` — 3.76×

`db.all += [G {…}]` over a vector and a hash that share one record set (`@FR-Col-Group`) is
**80 %** of the instructions.  Each append runs the generic record path —
`record_new` → `record_finish` → `insert_record` → `link_siblings` — and re-derives type facts
the compiler already has: `sub_record_type`, `nullable_field_parent` and `nullable_some_variant`
together take ≈ 4.4 M of 14.6 M instructions per call.  The find runs `get_hash_long_slow`,
because a group's hash uses the BORROWING layout (stride 0, C152), which has no fast path.

Lever: a group append emitted with its types resolved: claim in the vector, write the fields, add
to the hash through `hash::add_at` with the key read directly.  Plus a fast find for a borrowing
table.  Together ≈ −35 % → ≈ 2.5×.

### text2d `draw_quads` — 3.64×

The blit calls `atlas.a_cv.get_pixel(sx + px, sy + py)` and `cv.set_pixel(…)` per pixel.  Each
inlined call tests four bounds, multiplies the index and tests the length (lines 129, 134, 145
and 148 of `graphics.loft`: 4.0 M of 14.6 M).  The element reads are checked again inside
(`elem_index` + `get_elem_at`: 2.8 M).  The twin clamps each glyph row once to the range where
every test holds, then copies with a slice zip that LLVM vectorises.

Lever: the bounds are monotone in `px`, so the range where they hold is computed once per row,
and inside it the reads and writes go unchecked.  This is an index-set split, and
`ranged-call` already looked at this loop and declined: "no call gains".  It is the same shape as
`canvas` and `fill_polygon`, so it would move those too.

**Priced 2026-10-24** (`hand_price.sh` on the production runtime, one pinned core, hash
`becf0b85` unchanged in every run; the timed loop calls `n_draw_quads__inv`, not
`n_draw_quads`, so a price on the plain function measures one call in 401):

| form, in `n_draw_quads__inv`'s row loop | µs/op | vs base |
|---|---:|---:|
| base | 650 | — |
| split, accessors without their bounds test, ONE loop with a per-pixel interval test (v4) | 840 | +29 % |
| split into three loops, the middle calling guard-true accessors (v5) | 743 | +14 % |
| split, middle loop reading and writing the elements directly: the row index computed once and stepped, both `len` limits folded into the interval's end (v3) | 266 | **−59 %** |

So widening `ranged-call` to the callee's guard is NOT the lever: LLVM already folds the chained
bounds tests well, and what remains per pixel is the checked `gy * w + gx`, the `len` test and
`get_elem_at`'s own test, which block vectorising.  The −59 % is loop bounds-check elimination
over an affine index into an invariant vector: the index strength-reduced to `row + px`, the
bounds folded into the trip count, the access direct.  ≈ 1.5× if built; a rule of its own.

### hex_terrain `terrain_relief_pass` — 3.60×

The emitted lines are 2.22× the twin on their own.  `n_hex_neighbor` (42.6 M of 210.8 M) is not
inlined: `inline-leaf` declines "a statement other than an assignment" (its early
`if dir == 0 { return (q + 1, r); }` chain).  The six neighbour reads `t.tr_h[nr * nx + nc] ?? 0.0`
pay `elem_index` + `get_elem_at` (27.8 M), though the guard on the line before proves the index
in range.  Every integer step pays the null-int checks.

Levers: inline a leaf whose body is a chain of guarded returns (≈ −12 %); read through the held
header without the per-element test where a guard dominates the read (≈ −13 %).  This gives
≈ 2.7×, and the rest is range proofs over its integer arithmetic.

### hex_recover `forms_upto` — 3.57×

The runtime holds 83 % of the instructions.  The per-access store checks (`offset_in_bounds`,
`begin_write_inner`) are priced in `store-checks.md`: removal ceiling −14.9 %, and keeping only
the bounds test −6.3 %.  `form_new(eh, [la, lb, lc], [ea, eb, ec])` builds two vector literals
per iteration (0.87 M of 9.7 M, ≈ 9 %), and `clear_vector` + claims + malloc take ≈ 1.5 M more.

Levers: the literals held apart (`(R-Apart)`, the stack-array form; ≈ −9 %); the store checks
established once per record or vector (`store-checks.md`'s second cure; −6 to −15 %); the work
buffers that keep their store (`[work-buffer] … declared inside a loop`, `… its backing is not a
__vdb_`; ≈ −5 %).  Three arcs together ≈ −30 % → ≈ 2.5×.  The −15 % end of the store checks is the
lean-rlib decision.

### graphics `draw_bezier` — 3.55×

`bz_sx = bz_sx[0..bz_top]` IS kept in place, but through `Stores::vector_keep_range`, a runtime
call costing 9.5 M instructions.  For a prefix (`lo == 0`) of a scalar vector it is a length
store.  The two lines are 16.3 M of 131 M (≈ 12 %).  The rest: the null-int family ≈ 20 M
(`unlikely`, `overflowing_add`, no-line `intrinsics`), the `set_pixel` bounds in `draw_line`
(5.8 M, Bresenham: no monotone range), and `push_hoisted` / `get_elem_hoisted` ≈ 7.6 M.

Levers: the two stacks held apart, a `Vec<i64>` no store observes (`(R-Apart)`): priced in
`over-9x.md` § draw_bezier as a 2.3× ceiling, which covers the truncate, the pushes and the reads
at once.  The narrower step is the prefix slice of a scalar vector assigned to itself as a
length store in the held header (≈ −12 % → ≈ 3.1×).

### hex_shape `wall_chain_walk` — 3.55×

This is the one routine whose emitted lines are themselves the gap: 87.4 M own instructions,
3.07× the twin's whole routine.  `eg_dir_from` loops six directions calling `nb_q`/`nb_r`
(28 M, line 1561).  The twin has the same loop, and LLVM unrolls it and folds the
`(r & 1) == 0` branches, because its arithmetic is plain.  Loft's arithmetic carries the
null-int checks (≈ 26 M directly: `unlikely`, `overflowing_add/sub`, `op_add_long`, `op_min_long`),
and they also stop that fold.

Lever: range proofs over the integer arithmetic (≈ −40 % → ≈ 2.1× if they reach all of it).
Inside one function they reach the direction index (`i` from `0..6`) and literals; the
coordinates `qa`/`ra` are parameters a caller may pass null, so closing those needs a fact about
the callers' arguments (across calls), which no rule proves yet.

### server `header` — 3.54×

`(R-FoldCompare)` already answers `h.to_lowercase().starts_with(prefix)` without building the
fold (`codegen_runtime::fold_compare` in the emitted line).  What is left is the loop:
`for h in self.headers` copies every header text out of the vector (line 312: 7.8 M of 48.6 M,
and most of the 17.6 % spent in malloc/free).  `(R-TextBorrow)` declines with "`h` — the body may
write a store", but the only store write (`parts = h.split(':')`, `value += …`) is on the path
that returns, after which `h` is never read.  The twin walks `&self.headers` and allocates
nothing.

Lever: borrow the walked text when every store write in the body is on a path that leaves the
loop and follows `h`'s last read (`over-9x.md`'s item A, ≈ −30 % → ≈ 2.5×).

### par `par` — 3.52×

Not the threads.  Pinned to one core with one worker, loft takes 22.9 ms per pass against the
twin's 6.5 ms (3.5×).  Unpinned on all cores it is 1.3×, because the twin then lands on
efficiency cores too.  The kernel is `newton_sqrt`: 50 rounds of `guess = (guess + (x / guess)?) / 2.0`.
The `?` turns a NaN quotient into 0 (`if !v.is_nan() { v } else { 0.0 }`).  With that test in the
body LLVM keeps the 50-round loop (10 `divsd` in the binary).  Without it, LLVM unrolls the loop
fully (58 `divsd`).

**Priced** (`hand_price.sh`, one pinned core, hash unchanged, two runs agree): the test removed
13.96 ms (**−39 %**, ≈ 2.15×).  The same test as a branch-free mask: 22.9 ms, unchanged, so the
cost is the lost unroll, not the select.  Forcing `n_work` and `n_newton_sqrt` inline: no change.
The test cannot simply go: `x` is 0 for `value == -1`, and then `0 / 0` is NaN.

Lever: unroll a constant-count loop with a small float body in the emitter, so LLVM's cost model
is not what decides (to price next), or move the test off the loop-carried chain.  The
remaining 2.15× is the row loop (`get_vector`, store lookup and `push` per row, no two-lane divide
across rows).

**Priced 2026-10-24: unrolling is not the lever.**  All 50 rounds unrolled by hand, each keeping
its NaN test: 230 ms against 230 ms, unchanged (hash `141af99`).  The test removed still gives
140 ms (−39 %).  The test compiles branch-free (`divsd`, `cmpordsd`, `andpd`, `addsd`, `mulsd`
per round against `divsd`, `addsd`, `mulsd`): two µops on a 50-round chain, and the measured
1.65× is more than their latency, so the cost is the out-of-order window — fewer µops per item
let the processor overlap the next items' independent chains.  The test is the program's:
`x / guess` is `float?` and the source must discharge it into the non-null `float` (without
the `?` loft warns and `newton_sqrt(0.0)` answers null), while the twin's natural Rust has no
such step.  No general rewrite removes it; a proof that the quotient is never NaN is
algorithm-specific (`x > 0` finite keeps `guess > 0` by induction).  **An owner question**:
what a float division owes a non-null float.

### hex_roof `roof_match` — 3.60× (new on 2026-10-24)

`gap_report.py --fn match_op` (the bench's wrapper; `roof_match` itself is inlined into it):
139.5 M instructions per call against the twin's 33.6 M.  `n_track_distance__inv` is 65 M and
its caller `n_ridge_at` 56.6 M: every cell walks the track's segments, `seg_distance` reading
`t.tk_p[i * 6 + j]` through `pv`.  Each read pays the checked `i * 6 + j` (`op_mul_long`,
`op_add_long`, the overflow intrinsics, ≈ 25 M) and the element read's own bounds test
(`elem_index` + `get_elem_at`, 17 M); `op_min_long_nullable` is another 9 M.  `ranged-call`
declines every loop: "no call gains".  `tk_n` is a field nothing bounds, so no range proof
reaches `i * 6`: the lever is a fact about the vector itself — `tk_p` holds `6 · tk_n`
elements — which loft has no rule to state.  Not yet priced.

## In order

1. **No decision needed, largest reach:** `draw_quads`' range split (also moves `canvas`,
   `fill_polygon`), `header`'s borrowed walk, `grouped_fill_find`'s direct group append,
   `draw_bezier`'s apart stacks.  Each is one rule clause with its guard.
2. `par`: price the emitter-side unroll before building anything.
3. `forms_upto`: the apart literals, then the per-record store checks.
4. **Range proofs** (`wall_chain_walk`, then the remainder of `terrain_relief_pass`): first
   what one function can prove (counted ranges, guards, literals), then facts across calls.

Each lever is hand-priced before it is built (`feedback: hand-price and falsify`), and this
table is re-measured with `make perf-portal` after each.
