<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 157 — Native within 4× of Rust on the drawing pass

## Status

Open — P0–P4d and § V (Route R) SHIPPED (see Sub-arcs); the queue's head is
now the hoist unblock behind Route R (Phase ordering below).  Scoreboard vs
the issue baseline: `hash` 10.9× → 5.1× gate-row / 2.3× at the leaf; `hair`
4.3× → ~3.6× (under the bar); `lock` 30× → **6.2×** shipped.
Design in [DESIGN.md](DESIGN.md).  Implements
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
- **Last touched:** 2026-09-08

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
| **P1** — the per-call shadow-stack push made cheap (M1): `Cell` depth + `UnsafeCell` frame array replace the `RefCell<Vec>`, all five consumers kept in every tier, emitted prelude unchanged | [DESIGN.md § P1](DESIGN.md) | A/B min-of-runs cuts `hash` ≥ 25 %; depth-cap, stack-trace and the `i1058` balance guard (falsified) stay green | **Shipped 2026-09-07** — `hash` A/B 0.73×; consumer pass: all 14 hashes agree, `hash` 10.9→8.9× |
| **P2** — `#[inline]` on the write path; embed-bitcode in the rlib + thin LTO in `--native-release` (M4) | [DESIGN.md § P2](DESIGN.md) | the issue's by-hand rebuild table does not move; `-C lto=thin` still errors on a bitcode-free rlib | **Step 1 measured 2026-09-07**: `store_mut` `#[inline]` kept (−2 % `lock`); the other two candidates DECLINED (+1.5 % regression — cold raise paths duplicate).  LTO probe open; also the lean tier's 8.5→6.4 ns gap |
| **P3** — plain ops when operands are provably non-sentinel: float compares, counted `for`, then the `_nn` integer wiring (M3) | [DESIGN.md § P3](DESIGN.md) | `LOFT_NN_VERIFY` sweep clean; the 16-cell for-matrix byte-identical on both backends; nullable-operand cells keep the sentinel forms | **In-flight** — P3a (float compares), P3b (counted `for`, `conv_bool` 34→11) and P3c (`_nn` integer wiring, N3/N5 — 40 sites on the bench) shipped 2026-09-07, all falsified; open: interprocedural param facts |
| **P4** — element access via hoisted (base, len) incl. WRITES, field-reached vectors and loop-invariant record scalars; a two-tier gate (pure / in-place-writes-only) extending the #885 allow-list (M2) | [DESIGN.md § P4](DESIGN.md) | `lock` not ≤ 4 ms; `fill_poly`/`composite` not within 4×; `LOFT_HOIST_VERIFY=1` suite clean | **P4a/b shipped 2026-09-07** — two-tier gate + fused element write, falsified both ways; `hair` −15 % (~3.6×, under the bar); libm ops joined the allow-list (loops calling `sin`/`sqrt` never hoisted before); P4d (path keys — the raster loops: 7 fused writes + 1 read, zero per-element resolutions, `lock` −13–18 %) shipped same day; open: P4c record scalars |
| **V** — value-struct returns, Route R: a return-position struct literal builds into the caller's `__retbuf`, and the caller allocates that buffer once (loft#1426 M2's allocation half) | [DESIGN.md § V](DESIGN.md) | `lock` not ≤ ~17M on P4d's quiet-box scale; a § V matrix cell answers differently than the record form; `tests/retbuf_reuse.rs`'s positive control goes quiet | **Shipped 2026-09-08** — `lock` 25.6M → 17.9M (−30 %, the § V prediction; quiet box), hashes exact; P0 instrument `lock` 6.2× (bar 16 → 10); gated on `witness_buffer` + a single-assigned result local (the free-guard matrix found the reassignment hole the day after), controls falsified on both backends; the guard-the-free widening (§ V-b) was built and falsified the same day — the fact belongs in the return type (M–L, separate item); open: the hoist-unblock fact |
| **P5** — the pass becomes the per-library standard (LIBRARY_CHECKLIST.md row; `drawing` first) | [DESIGN.md § P5](DESIGN.md) | a library without a `bench/` passes review | Open |

## Joined-tree verification (2026-09-07)

The phases' ratios were measured on the plan's own branch.  A ratio measured on
one tree is not a measurement of another, and the joining tree carries loft#1437
(`Parts::IntRaw`, a new narrow-integer decode path a drawing pass exercises
heavily), loft#1433 and loft#1434.  Re-measured after the whole branch
(`4f99297e..592e51cc`) was cherry-picked onto `tuxedo-1361-tuple-copy`, on an
idle box, `scripts/native_ratio.sh --gate`:

| bench | routine | rust ns/op | native ns/op | ratio | bar |
|---|---|---:|---:|---:|---:|
| 12_drawing | `hash` | 170800 | 1216000 | **7.1** | 8 |
| 12_drawing | `lock` | 2054400 | 25895200 | **12.6** | 30 |

Exit 0, and the two lanes' output hashes agreed — the half that mattered, since
a hash mismatch is fatal even in report mode and a faster wrong answer is the
failure this join could plausibly have produced.

⚠ The join also surfaced a REGRESSION the branch carried, caught by `make ci`'s
`html_wasm::html_panic_names_itself_and_its_loft_frames`: the lean tier selected
the nameless `cr_call_push_lean` on `!emit_live`, which is false BY DEFAULT for a
production `--html` client (@PLN98 P3.4), so every browser panic lost every loft
frame name with `--lean` never passed.  Fixed by giving frame naming its own
field (`Output::lean`).  The bench is native-only, so the ratios above are
unaffected — but the phase had shipped without `make ci` green over it.

## Phase ordering

P0–P4d shipped; the original ordering below it stands as history.  What
remains, ranked by measured value:

1. ~~**Value-struct returns**~~ — **SHIPPED 2026-09-08 as Route R** (DESIGN.md
   § V): `lock` −30 %, 6.2× on the P0 instrument.  What it leaves on the
   table, now the head: **the hoist unblock** — the def-level *"this callee
   writes only into its retbuf"* fact that lets the resolve loop's committed
   P4 hoists fire across the `brush_sample` call (the −17 pts § V attributed
   to it; S–M, one attributable exception to the no-interprocedural-in-place
   rule).  The gate's declined sites (7 pts) are NOT a widening of the guards:
   § V-b built that and the matrix falsified it — three parser-level release
   sites read the dep-free return as a fresh store — so they wait on the
   type-level fact (a buffer dep the adopt lowering accepts), M–L, separate.
2. ~~**Consumer re-run**~~ — done 2026-09-07 (DESIGN.md § Consumer 14-row
   re-run); it sharpened the target rather than shrinking it, and Route R
   is the lever `smooth`/`fronds` route to as well — re-run after the hoist
   unblock lands.
3. **P4c record scalars** (S–M, ~5–10 % pixel rows) · **bound-via-header**
   (`h.len` is the bound where P4 fired; S) · **P2 thin-LTO probe** (the
   lean tier's 8.5→6.4 ns gap — the `hash`/`smooth` gate rows carry it).
4. **Closing:** the owner call on `--native-release` implying `--lean`
   (flag semantics are clean post the html lesson); P5's checklist row; the
   PR when the owner judges the branch done.

Honest residual: `lock`'s last stretch (6.2× → 4×) is not yet
probe-covered; the hoist unblock is the one measured lever left from § V,
and after it the next decomposition says whether P4c + residual per-pixel
machinery closes it.

<details>Original ordering: P0 first; P1/P2/P3 independent by cost; P4 last;
P5 closes.</details>

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
  status updates as phases land.  **N4 SHIPPED 2026-09-07** (structural
  leaf inference; `hash` −36 %, ~2.3× Rust; `LOFT_NO_LEAF_PRELUDE`
  bisects), alongside the lean-tier prelude (`cr_call_push_lean`).
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
