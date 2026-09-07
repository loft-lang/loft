<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 157 — Native within 4× of Rust on the drawing pass

## Status

Open — design in [DESIGN.md](DESIGN.md), no implementation yet.  Implements
[loft#1426](https://github.com/loft-lang/loft/issues/1426): loft-native runs
10–50× behind plain Rust on the drawing library's routines, measured by a
byte-identical reference (`loft-libs-graphics/drawing/bench/`, branch
`drawing-lock` — 14 routines, FNV-1a-32 output hashes equal across the
interpreter, `--native-release` and `rustc -O` Rust).  The mechanisms and their
shares are already measured on the issue (M0 ruled out; M1–M4 attributed), so
this plan starts at fix design, not investigation.

**Owner's ruling (2026-09-07, on the issue):** the lessons become work on loft
itself.  A library must stay readable loft and be efficient *because loft is*;
native exports of pixel routines and any "write it in Rust" path for library
authors are non-goals.  The Rust reference is the **instrument**, not an
implementation path.

## Goal

loft-native within **4×** of plain Rust on every judged row of the drawing
performance pass, with the library's loft source unchanged and every row's
hash unchanged.

## Effort + design

- **Effort:** H total (P1 S · P2 S · P3 M · P4 L · P0/P5 XS)
- **Design:** ✓ — [DESIGN.md](DESIGN.md): per-phase invariant, code sites,
  claims + falsifying probes, predicted numbers
- **Last touched:** 2026-09-07

## Composition matrix — Stage A

No new composition surface: every phase changes what the native backend
*emits or links* for programs that already run, never what they compute.  The
constitutive gate is therefore identity, not a new matrix: the drawing pass's
14 output hashes (P0) plus the full both-backend suite must be unchanged by
every phase, and each phase adds `--emit`-level probes (counts of the removed
form in the generated Rust) per the loft-codegen skill's byte-comparison
discipline.  P3/P4 change emitted forms of existing ops — their per-phase
sections in DESIGN.md name the cells (nullable × non-null operands, read ×
write, local × field-reached vectors) that must stay green on both backends.

## Sub-arcs

`Verify` names the red condition, all through the same command (the P0 gate)
unless said otherwise.

| Item | Source | Verify | Status |
|---|---|---|---|
| **P0** — the pass runs from this tree; baseline table lands in PERFORMANCE.md | [DESIGN.md § P0](DESIGN.md) | `make native-ratio`: a hash mismatch always fails; `--gate` fails a ratio over `bench/ratio_oracle.tsv`'s bar | **Shipped 2026-09-07** — `bench/12_drawing/` (`hash`+`lock`, hashes asserted, all three lanes agree), `scripts/native_ratio.sh` (median-of-runs; both red arms falsified), PERFORMANCE.md § The drawing-pass baseline |
| **P1** — release tier drops the per-call shadow-stack push (M1) | [DESIGN.md § P1](DESIGN.md) | `hash` row not ≤ 0.6 ms/100k calls; `smooth` not ≥ 5× faster; recursion cap still fires (its own test) | Open |
| **P2** — `#[inline]` on the write path; embed-bitcode in the rlib + thin LTO in `--native-release` (M4) | [DESIGN.md § P2](DESIGN.md) | the issue's by-hand rebuild table does not move; `-C lto=thin` still errors on a bitcode-free rlib | Open |
| **P3** — plain ops when operands are statically non-null: float `<`/`<=`, int `+`/`*`/`min`, counted `for`, `sqrt` intrinsic (M3) | [DESIGN.md § P3](DESIGN.md) | `is_nan()` count in the standalone's emitted file not ~0; `ops::op_*` count not small; nullable-operand cells keep the sentinel forms | Open |
| **P4** — element access via hoisted (base, len) incl. WRITES, field-reached vectors and loop-invariant record scalars; a two-tier gate (pure / in-place-writes-only) extending the #885 allow-list (M2) | [DESIGN.md § P4](DESIGN.md) | `lock` not ≤ 4 ms; `fill_poly`/`composite` not within 4×; `LOFT_HOIST_VERIFY=1` suite clean | Open |
| **P5** — the pass becomes the per-library standard (LIBRARY_CHECKLIST.md row; `drawing` first) | [DESIGN.md § P5](DESIGN.md) | a library without a `bench/` passes review | Open |

## Phase ordering

1. **P0 first** — every later phase reports through it; without it no phase
   can go red.
2. **P1, P2, P3 are independent** of each other; order by cost (P1, P2, P3).
   Each is measurable the day it lands.
3. **P4 last** — the bulk (~90 % of `lock`/`composite`/`fill_poly`), and it
   consumes P8's written-set query (PERFORMANCE.md § Design: P8) plus the
   #885 hoist machinery it extends.
4. P5 closes the loop once the numbers hold.

## Open design questions

The sketch's questions are settled in DESIGN.md; what remains is decided by
measurement, not prose:

1. P1 — cheap fixed-array push (keeps all five shadow-stack consumers, every
   tier) vs a lean depth-counter tier: the hand-timed probe decides
   (≤ 2 ns/call keeps the diagnostics everywhere).
2. P2 — `[profile.release] lto="thin"` (one chokepoint; changes the pinned
   release sha + build time) vs a dedicated rlib profile: owner's call, and
   the thin-LTO benefit is probed by hand before either is wired.
3. P3 — the leaf-trust set for "provably non-sentinel" starts conservative
   (literals, closed ops, proven loop vars); widening any leaf needs a
   measured reason and a clean `LOFT_NN_VERIFY` run.
4. P4 — scope settled: extend the #885 hoist's own allow-list (two-tier gate),
   NOT the old N1 direct-`Vec` emit (recorded mis-scoped in PERFORMANCE.md,
   2026-06-25) and NOT the parser's deny-list `find_written_vars` (holes:
   `CallRef`/`Parallel`/`Yield` fall through `_ => {}`).

## Cross-arc dependencies

- **PERFORMANCE.md § Design: P8** (store-effect classifier) — a sibling, not
  a blocker: P4 extends hoist.rs's own allow-list classification (the doctrine
  P8 endorses) rather than importing the parser's deny-list; P8's "one home
  for the leaf set" remains the longer-term convergence point.
- **PERFORMANCE.md § Design: N1/N2/N3/N4/N5** — this plan implements the
  N-class from a measured consumer workload; those design entries get
  status updates as phases land.
- **loft#885 hoist** (`src/generation/hoist.rs`, `LOFT_HOIST_VERIFY`,
  `LOFT_NO_VECTOR_HOIST`, `LOFT_NO_ELEM_FUSE`) — P4 extends it; its
  verify/bisect switches are the safety instrument.
- **@PLN85** (finished) — the ownership/representation fact N1's old design
  waits on; P4's hoist-shaped scope is chosen to not need it.
- **@PLN140** profiler — `make profile PROFILE_FLAGS=--engine` attributes
  native time when a phase's number does not move as predicted.

## See also

- [loft#1426](https://github.com/loft-lang/loft/issues/1426) — the source
  issue; its comments carry the measured attribution (M0–M4) and baseline.
- [`@PLN157`](https://github.com/loft-lang/plans/issues/157) — the tracker
  issue for this plan.
- [PERFORMANCE.md](../../PERFORMANCE.md) — N-class designs, P8, `make speed`.
- [NATIVE.md](../../NATIVE.md) — the native backend's architecture.
- [LIBRARY_CHECKLIST.md](../../LIBRARY_CHECKLIST.md) — P5's home.
- `formal/draw.md` D-draw-2 (consumer side, loft-libs-graphics) — the
  deviation this closes.
