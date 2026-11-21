<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 224 — hex_edge and hex_shape decide in whole numbers

Tracker: [@PLN224](https://github.com/loft-lang/plans/issues/224).  The design — the
inventory of every float decision, the whole-number form of each, the boundary rule, the
API and the tests — is [design.md](design.md).  This file is the state and the steps.

## Status

**Design written 2026-11-20, awaiting approval** (`status:need-approval`).  Nothing built.
Six answers change at 64 bits and are listed for the owner in design.md § What changes at
64 bits.

## Goal

`hex_edge` (collision as blocked hex edges) and `hex_shape` (walls, boxes, arcs) make every
discrete decision in whole numbers on a sub-lattice, so a 32-bit device, where a `float` is
a single, and a 64-bit desktop answer alike.  No 64-bit floats in a path that runs per
frame, and no accepted difference.

## Effort + design

- **Effort:** M for hex_shape (S0–S12), M for hex_edge (E0–E10), S for hex_field's addition (F1)
- **Design:** ✓ from reading the code; eight probes stand between it and belief (below)

## Steps

Every reference is `package/src/file.loft:LINE` in loft-libs-world at a18065b.  Each step
merges alone and leaves every existing test green at 64 bits.  Neither package declares
`width32` before its last step, so a test that is red under `--width 32` is mergeable.
The shape of each change: a test that fails at 32 bits first; the integer form added beside
the float one and proven equal on the lattice; the caller switched; the float path removed.

Order: hex_shape, hex_field's addition, hex_edge.  hex_shape first because its one real
defect is cured by code already in the tree, it runs when a level is laid out, and it uses
the sub-lattice in a cold path before hex_edge's hot path depends on it.

### hex_shape and hex_field

| # | Change | Code | Test | Goes red on | 64-bit answers |
|---|---|---|---|---|---|
| S0 | Test only: a wall far from the origin | — | `hex_shape/tests/01-hex-shape.loft`, new `test_a_wall_far_from_the_origin_marks_the_same_edges` (cell 300,300, 24 directions, p = 3) | red at 32 bits (predicted, probe P2) | none |
| S1 | Test only: three tolerances follow the float | — | `tests/01-hex-shape.loft:371`, `:473`; `tests/02-worked-examples.loft:182` | a tolerance left at `1e-9` | none |
| S2 | `wall_edge_agrees` beside `wall_edge_gap` | `hex_shape/src/hexwall.loft:276`, insert after `:284` | `tests/02-worked-examples.loft:339` uses it | a swapped corner or neighbour table | none |
| S3 | `box_fill` is `parts_fill` | `hex_shape/src/hexbox.loft:144-158` | `tests/01-hex-shape.loft:502`: an `f64` oracle, the sweep widened to 1..24 and to anchor (40, 41) | a wrong half-step mapping; `tests/02:431, 467` | none on the swept domain (probe P7) |
| S4 | The float fill goes | `hexbox.loft:139` (`BOX_EPS`), `:141` (`bfabs`) | — | a remaining use | none |
| F1 | The sub-lattice beside `hex_at` | `hex_field/src/hex_field.loft:253-276`, insert after `:276`: `SUB`, `sub_of_x/y`, `sub_x/y`, `cube_round_frac`, `cell_at_sub` | new `hex_field/tests/09-sub-lattice.loft`: every centre of a chunk round-trips; agreement with `hex_at` inside cells at 64 bits; the twelve boundary points of (0,0) and (300,300) | a tie rule that is not `hex_at`'s; an i32 overflow near 2²⁷ | none (addition) |
| S5 | The integer wall beside the float one | `hexwall.loft:212` (`wall_separates`), `:322` (`wall_write`); new `wall_sep_sub`, `pub fn wall_write_run` | new `test_the_whole_number_wall_marks_what_the_float_wall_marks` (24 directions × 3 vertex classes × p 1..6) | a lattice case where the float marking differs: the finding the step exists to make | none (addition) |
| S6 | `wall_write` and `wall_separates` go through the door | `hexwall.loft:212-224`, `:322-340` | S0 turns green at 32 bits | a wrong `d24` recovered from the direction; a row tie lost | only a wall not on the sub-lattice: its ends are rounded to about 1 mm |
| S7 | The float wall goes | `hexwall.loft:208` (`WALL_EPS`), `:184` (`wall_along`) | — | a remaining use | none |
| S8 | `hex_scale_round`, `nearest_vertex` on sub-steps | `hexwall.loft:875-880`, `:903-919` | a click on a hex centre; one midway between two vertices | the tie order (probe P3 first) | exact ties; targets within half a sub-step of a bisector |
| S9 | Snap by integer distance | `hexwall.loft:934`, `:952`, `:971` | a target equidistant from two legal endpoints | a product left in i32 | exact ties |
| S10 | Period count and the 24-direction table | `hexwall.loft:1014`, `:1036-1047` | the table against `atan2` in `f64`; the 24 boundary pairs | a wrong table literal | within 0.0005° of a boundary |
| S11 | `line_hexes` samples in integers | `hex_shape/src/hexbox.loft:362-383` | a segment through a vertex; one along an edge | a changed count | a sample within half a sub-step of an edge |
| S12 | Declare `width32` | `hex_shape/loft.toml`, README | the library CI runs it at 32 bits | anything still red there | none |

### hex_edge

| # | Change | Code | Test | Goes red on | 64-bit answers |
|---|---|---|---|---|---|
| E0 | Test only: thread the cell far out | — | `hex_edge/tests/02-worked-examples.loft`, new `test_threading_the_cell_holds_far_from_the_origin` | red at 32 bits (predicted, probe P2) | none |
| E1 | Test only: 7/12 follows the float | — | `tests/02-worked-examples.loft:221` | — | none |
| E2 | The integer walk, `sweep_sub`, beside the float sweep | `hex_edge/src/hex_edge.loft:554-610`, insert after `:610` | new `test_the_whole_number_sweep_agrees_with_the_float_one`; start on a bisector, end on one, through a vertex, along an edge; `num·12 == den·7` | a wrong neighbour table; an i32 product | none (addition) |
| E3 | `sweep_path` and `sweep_path_from` use it | `hex_edge.loft:518-522`, `:554-610` | E0 turns green; `tests/02:88-120` on `cell_at_sub`; `:142-171` rewritten | `tests/02:101`, `:156`, `:517`, `:447-458` | **yes** — design.md, changes 1 to 3 |
| E4 | The float sweep goes | `hex_edge.loft:570-596`, comment `:539-550` | — | — | none |
| E5 | `sight_clear` on the walk | `hex_edge.loft:687-715` | a line clipping a corner cell; one grazing a wall top | `tests/02:493` | **yes** — design.md, change 4 |
| E6 | `edge_block_arb` in 1/1024 units | `hex_edge.loft:278-284` | two surfaces equidistant from an edge, stamped in both orders | — | distances within 1/1024 tie to the lower id |
| E7 | `apply_features` interval in 1/1024 units | `hex_edge.loft:430-435` | an interval `[t, t]` at an edge's parameter | `tests/02:376` | an edge within half a unit of an interval's end |
| E8 | `edges_halfplane_surf` side in 1/1024 units | `hex_edge.loft:649-656` | `ny = 1, c = 1.5`: row 1 on the line | 21 of 21 | a centre within half a unit below the line |
| E9 | `junction_ok` threshold in integers | `hex_edge.loft:332-335` | `tests/02:552` | — | a dot within 5e-7 of 0.999 |
| E10 | Declare `width32` | `hex_edge/loft.toml` | the library CI at 32 bits | anything still red | none |

`hex_way` needs nothing to stay correct: it calls only functions that do not change, and
its own tests gate E3 to E9 at both widths.  It carries the same class of decision
(`hex_way/src/hex_way.loft:302`, `:325`, `:349`), which follows in one later step.

## Probes before belief

| # | Probe | Decides |
|---|---|---|
| P2 | Run S0 and E0 under `--width 32` before merging them | whether they are red today, as predicted |
| P3 | Print `nearest_vertex(0.0, 0.0)` at 64 bits | the integer scan order that reproduces today's winner of a six-way tie |
| P4 | For k to 4000: is `sub_of_x((k as float) * 0.8660254)` equal to `k * 768` at 32 bits? | how far from the origin a single-precision position still lands on its sub-step (estimated 680 cells) |
| P5 | `hex_edge/bench/bench.loft` before and after E3, both widths and on the device | whether the 64-bit products cost too much per step |
| P6 | `loft compat check --full` on each release; the `too-many-parameters` advice on `wall_write_run` under `LOFT_DENY_WARNINGS=1` | that every step is a drop-in and passes CI |
| P7 | The widened `box_fill` sweep; `hex_form::sign_root3` squares in i32 and reads null past 46340 | the range over which S3 holds |
| P8 | `round(x) as integer` on negative halves and `(a as i64) * (b as i64)` compared, at width 32, interpreter and C | the two idioms the design leans on |

## Cross-arc dependencies

- [@PLN217](https://github.com/loft-lang/plans/issues/217): loft refusing a silent narrowing in
  every position would turn a product left in i32 (S9, E2) into a compile error.
- `hex_field >= 0.1.13` (F1) is the floor of both libraries from S5 and E2 on.
- A game that wants movers to agree between machines holds positions in sub-steps and calls
  `sweep_sub`: a position accumulated in floats still drifts.

## Documentation

When S12 and E10 ship: each library's README (the door, the boundary rule, what `width32`
covers); hex_edge's § on the skin, which is rewritten in E3;
[LIBRARY_WIDTH32.md](../../LIBRARY_WIDTH32.md) § What to write gains the sub-lattice as the
worked example of "decide in whole numbers".

## See also

- [design.md](design.md)
- The ports that preceded it: loft-libs-world#61 (hex_field, hex_terrain), #62 (hex_body,
  hex_draw, hex_fit, hex_place, hex_roof), which left four float decisions for this plan:
  `hex_place::disk_hit` on a rim, `hex_fit::feature_fits` with `==`, `hex_draw::feature_t_num`
  on a tie, and a rig's digits when it is rewritten on a 32-bit build.
