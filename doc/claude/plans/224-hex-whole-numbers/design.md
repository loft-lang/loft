<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 224 — the design: whole-number decisions in hex_edge and hex_shape

Read from loft-libs-world at a18065b; every `file.loft:LINE` is of that commit.  The steps
are in [README.md](README.md).  Nothing here is built: a statement about new code is a
design claim, and the ones that need a probe are README § Probes before belief.

## What reading the code found

- Under `--width 32` hex_edge fails 3 of 16 tests, hex_shape 5 of 31, hex_way 0 of 11.
- **Four of hex_shape's five failures are test tolerances**: two smooth float outputs
  compared within a fixed `1e-9`.  Only "608 of 3888 boxes" is a decision that flips, and
  its cure is in the tree: `parts_fill` (`hex_shape/src/hexbox.loft:197`) is exact, and the
  failing test is the proof that it equals `box_fill` at 64 bits.  `hexarc.loft` is all
  integer.
- **Two of hex_edge's three failures are about `hex_field::hex_at`**, which the tests call
  directly (`hex_edge/tests/02-worked-examples.loft:107, 192`); the third (7/12) is a
  tolerance.  The "far cell" promise is round-half-away-from-zero on cube coordinates,
  tested only at the origin cell.
- The game uses only the integer half of hex_shape (`parts_fill`, `box_part`, `BoxPart`,
  `parts_reach`) and calls no `sweep_path`, `sight_clear` or `hex_at` itself: it reaches
  hex_edge through hex_draw and hex_way.  "Per mover per frame" is hex_edge's design and
  its bench (`hex_edge/bench/bench.loft:150-184`), not this game today.

## Where a float decides

### hex_edge (`hex_edge/src/hex_edge.loft`)

| Line | Function | Decides | Compares today |
|---|---|---|---|
| 520 | `sweep_path` | the start cell | `hex_at(x0, y0)` |
| 589 | `sweep_path_from` | is this bisector ahead | `den > 0.000000001` |
| 593 | `sweep_path_from` | which edge is crossed first | `tt > t - 0.000000001 && tt < bt` |
| 597 | `sweep_path_from` | does the segment reach it | `bt > 1.0` |
| 281-284 | `edge_block_arb` | which surface owns a shared edge | `dnew < dcur - 0.000000001` |
| 435 | `apply_features` | is an edge inside a door interval | `t >= lo && t <= hi` |
| 656 | `edges_halfplane_surf` | which side of a line | `sa < 0.0 && sb >= 0.0` |
| 693-698 | `sight_clear` | how many samples; the cell of each | `(dist / 0.2) as integer + 1`; `hex_at` |
| 704, 706 | `sight_clear` | opaque enough, tall enough | `mat_opacity >= opaque_at`, `mat_height > zl` |
| 334-335 | `junction_ok` | smooth or a corner | `dot > 0.999` |
| 345 | `junction_g0` | on the seam | `da <= tol` |

Smooth outputs that stay floats: `surf_distance`, `surf_normal`, `surf_param`, `surf_point`,
`edge_point`, `collide`'s normal, every `mat_*` reader, the `t` a sweep answers.  Already
integer: `edge_k`, `edge_m`, `edge_block*`, `edge_blocked`, `passable`, `edges_cut`,
`edges_solid`.

### hex_shape

| File:line | Function | Decides | Compares today |
|---|---|---|---|
| `hexbox.loft:151` | `box_fill` | is a cell in the rectangle | `bfabs(lu) <= hw + BOX_EPS` |
| `hexbox.loft:366, 376` | `line_hexes` | sample count; the cell of each | `(ln * 3.0) as integer + 1`; `px_to_hex` |
| `hexwall.loft:215-216` | `wall_separates` | side of the wall's line | `oc > 0.0 - WALL_EPS` |
| `hexwall.loft:223` | `wall_separates` | is the crossing within the run | `al >= -wa_half - WALL_EPS` |
| `hexwall.loft:875-879` | `hex_scale_round` | nearest hex-scale point | `hex_round` on floats |
| `hexwall.loft:916` | `nearest_vertex` | nearest of six vertices | `d2 < best` |
| `hexwall.loft:943` | `project_p` | centre of the search window | `round(projection)` |
| `hexwall.loft:961, 979` | `snap_run_d24`, `snap_run_p` | nearest legal endpoint | `dd < best` |
| `hexwall.loft:1022` | `wall_nearest_d24` | which of 24 directions | degrees from `atan2` |
| `hexwall.loft:1045` | `snap_run_from_heading` | how many periods | `round(dist / per)` |

Smooth outputs that stay floats: a wall's angles, offsets and lengths, `tri_x`, `tri_y`,
`hex_corner_x`, `hex_corner_y`, `cell_x_w`, `cell_y_w`, `wall_edge_gap`, `box_to_local`.

## The sub-lattice

One constant, `SUB = 768`, owned by hex_field.  A point is `(X, Y)` with
`x = X·√3/(2·SUB)` and `y = Y/(2·SUB)`.

| Thing | In sub-steps |
|---|---|
| a cell centre | `(k·768, m·768)` |
| an edge midpoint | `(384·edge_k, 384·edge_m)` |
| a hex corner | a centre plus `(0, ±2)` or `(±1, ±1)` times 768 |
| a wall vertex `(a, b)` | `(256·a, 256·(a + 2b))` |

768 is 3·2⁸: the 2 makes midpoints whole, the 3 makes wall vertices whole, and the power
of two keeps `y·1536` exact in a float.  1024 fails the thirds.  A step is about 1 mm.

**At the door**: `X = round(x · 1536/√3)`, `Y = round(y · 1536)`, once per call.

**√3 cancels in everything a decision needs.**

| Quantity | Whole-number form |
|---|---|
| squared distance (× 4·SUB²) | `3·ΔX² + ΔY²` |
| dot product | `3·ΔX·ΔX' + ΔY·ΔY'` |
| sign of a cross product | `ΔX·ΔY' − ΔY·ΔX'` |

√3 survives only where an odd-heading length meets an even-heading one: `hex_form::sign_root3`
(`hex_form/src/hexform.loft:235`), which squares.

**Ranges.**  An i32 holds `abs(X), abs(Y)` to 2²⁷, about 87,000 cells.  A single-precision
position lands on its own sub-step to about 2²⁰, some 680 cells from the origin (an
estimate: probe P4); past that the caller holds positions in sub-steps.  Geometry given as
free floats (surfaces, feature intervals, half-plane normals) has no lattice form: both
sides are rounded to **1/1024 of a world unit** and compared as integers, as
`stage::facing_name` does with hundredths of a degree.

## The form of each decision

| Decision | Integer form | Width |
|---|---|---|
| point to cell | `cell_at_sub(X, Y)`: cube numerators `cx = 3X − Y`, `cz = 2Y`, `cy = −cx − cz` over `6·SUB`, rounded by the tie rule below | i32 |
| first edge crossed | per neighbour `(dk, dm)`: `den = 3·Vx·dk + Vy·dm`, `num = 3·(384·Ek − X0)·dk + (384·Em − Y0)·dm`; ahead iff `den > 0`; the smaller of `a/b`, `c/d` by `a·d < c·b`; clear iff `num > den` | `num`, `den` i32; the two products **i64** |
| the `t` a sweep answers | `num / den` as a float, once, after the walk | float |
| line of sight | the same walk over each edge crossed; `h·den > z0·den + (z1 − z0)·num`, heights in 1/1024 units | i64 |
| wall side | `sign((Cx − Ox)·sm − (Cy − Oy)·sk)` with the wall's integer step `(sk, sm)` | **i64** |
| wall within the run | segments C–N and E0–E1 intersect: `orient(C, N, E0) · orient(C, N, E1) ≤ 0`, by sign | i64 |
| box fill | `parts_fill` with one `box_part(-w, -d, w, d)` | i32, existing |
| nearest vertex, snap | compare `3ΔX² + ΔY²` | i32; **i64** for run-length deltas |
| hex-scale rounding | `cube_round_frac(2X, Y − X, 2·SUB)` | i32 |
| period count | the largest `n` with `(2n − 1)²·N36·1024² ≤ 144·D²`, `N36 = 3·ra² + (ra + 2rb)²` | i64 |
| one of 24 directions | thousandths of a degree against a table of 24 integer boundaries | i32 |
| arbitration, interval, half-plane, junction | round to 1/1024 (junction: millionths of the dot), compare | i32 |

A wall holds floats (`hexwall.loft:38-45`), so `wall_write` converts once: its origin and
ends to sub-steps, its direction to the `d24` whose step has the greatest dot with it (the
24 are 13.9° apart or more).  A hand-set direction matching none falls back to the line
through the two rounded ends.

The hot loop is 32-bit adds and at most six 32×32→64 multiplies per lattice step, with no
64-bit divide.  Everything else that needs i64 runs at layout or in an editor.

## The boundary rule: closed, first, away, crossed

| Clause | Rule | Applies to |
|---|---|---|
| Closed | a boundary belongs to the region: `≤` and `≥` are in, a zero offset is on the positive side, a run's two ends are in | fills, the wall side and run ends, feature intervals, half-planes |
| First | among exact equals the first in the documented order wins: lower surface id, lower direction, earlier `d24` then `p` | arbitration, a corner hit in the walk, snap ties |
| Away | a resting point on a cell's edge or corner belongs to the cell `cell_at_sub` names | a sweep's start, `line_hexes` samples |
| Crossed | a moving point that reaches a bisector has crossed it (`num ≤ den`): the edge is tested and, if open, the mover is in the far cell | the walk |

**Away, in full**: round each cube coordinate to the nearest whole number, an exact half
going away from zero; if the three no longer sum to zero, recompute x if its residual is
strictly the largest, else y if strictly larger than z's, else z.  That is `hex_at`
(`hex_field.loft:259-271`) in integers.  Worked by hand for the six edge midpoints of cell
(0,0): all six go to the neighbour, so the test's "far cell in all six directions" holds,
at 32 bits too.  It is not translation-invariant: for a general edge it is the cell further
from the lattice origin.

**The cell a sweep was handed is authoritative**: a start on or beyond one of its bisectors,
moving toward it, crosses at `t = 0`.  That replaces both `1e-9` tolerances with no number.

## API

No signature and no record changes.  A float function keeps its signature and converts at
the door.

| Package | Added |
|---|---|
| hex_field | `pub const SUB = 768`; `sub_of_x`, `sub_of_y` (in); `sub_x`, `sub_y` (out); `cube_round_frac`; `cell_at_sub` |
| hex_edge | `sweep_sub(e, sq, sr, x0, y0, x1, y1)` answering `(num, den, q, r, dir)` |
| hex_shape | `wall_write_run(d24, a0, b0, p, mat, edges, q0, r0, wq, hq)`; `wall_edge_agrees(q, r, dir)` |

Left alone on purpose: `hex_field::hex_at` and `hex_grid::px_to_hex` (making `hex_at` follow
`cell_at_sub` changes a third package's published answers near a boundary, and is its own
decision); the records `Wall`, `Surfaces`, `Box` (a field added is a declared break, which is
why a wall's `d24` is recovered and not stored).

## What changes at 64 bits

1. **A sweep's positions are rounded to sub-steps** (E3): `t` moves by up to a sub-step
   over the segment, and a path within a sub-step of a corner, or ending within one of an
   edge, can name another cell or direction.
2. **A start outside the cell handed to `sweep_path_from`**, moving outward, crosses at
   `t = 0`.  Today it is ignored once it is more than `1e-9` out.
3. **The skin.**  With a fixed sub-step a constant skin exists: one sub-step.  The test at
   `hex_edge/tests/02-worked-examples.loft:161-170` and the comment at
   `hex_edge.loft:539-550` argue that none does, and are rewritten.
4. **Sight is blocked more often** (E5): the walk sees a cell the 0.2 sampling steps over at
   a corner, and takes a height at the crossing where a sample takes it up to 0.2 further
   on.  The fallback, integer sampling at today's count, needs a 64-bit divide per sample.
5. **Arbitration, feature intervals, half-planes** (E6–E8): values within 1/1024 tie.
6. **hex_shape**: a free `wall_new` wall has its ends rounded (S6); exact ties in S8–S10;
   rounded samples in S11.

## The limit of the claim

"The same answer on both" holds as a function of the integers.  A float input gives the
same integers on both machines when it is born on the lattice, or is representable in a
single and not within float error of half a sub-step.  A position accumulated in floats
has no single twin and still drifts between a desktop and a device: full agreement needs
the consumer to hold positions in sub-steps and call `sweep_sub`.

## Not determined

- Whether the game's house fill ever runs per frame (`hexhouse.loft:67`, called from
  `hextown.loft:562, 889` and `marbles.loft:219`; by name these build a town).
- Whether a double `hex_at` follows the transliterated tie rule away from the origin; it
  matters only if `hex_at` is later made to delegate.
- The same class of decision in `hex_way` (`hex_way/src/hex_way.loft:302, 325, 349`).
