<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 157 — Native within 4× of Rust on the drawing pass

## Status

Open — P0–P4d, § V (Route R), § V-c (the hoist unblock), § V-d (append in
place), § V-e (the runtime's per-allocation overhead), § V-f (the runtime's
per-record bookkeeping) and § V-g (read-only view elision at a record join)
SHIPPED (see Sub-arcs); the queue is re-ranked below, and **§ Where to
resume** is the hand-off for the next session.
Scoreboard vs the issue baseline, consumer lane on the SHIPPED tier (lean, fully
optimised — the release default since 2026-09-08, DESIGN.md § The shipped tier):
`hash` 10.9× → **9.1×** consumer / 2.7–5.4× gate row; `hair` 4.3× → **2.6×**
(under the bar); `lock` 30× → **7.8×** (6.5–7.4× gate row); `smooth` 262× →
**19×**; `fronds` 49× → **18×**; `composite` 26× → **11×**; the fills 17× →
4.5–5×; `wide_line` 17× → 8.9×.
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
- **Last touched:** 2026-09-08 (§ V-g)

## Where to resume

Written 2026-09-08 after § V-g landed: HEAD on `157-native-4x`, pushed; the
gate is the GitHub `ci.yml` dispatch on that commit (the local `make ci` was
dying under memory pressure — CI_BUDGET.md § When the local gate is
unreliable).  Nothing is in flight; the working tree is clean apart from
untracked local artefacts.

**The next unit is `fronds`' allocation class.**  § V-g removed `smooth`'s
borrow-copies (38 → 14 stores per call) and `fronds` (19×, 1.0 ms) did not move
a point, so its stores are not read-only views of a const argument.  Start
from the instrument, not a guess: `LOFT_STORES=log` on a standalone `fronds`
row (rebuild it from the consumer bench as `vr_smooth.loft` was), one call,
labels counted per variable — the labelled log names the class (a per-element
construction into a vector that § V-d's adopt route declines? a struct-field
twin `Box { s: mk(i) }`? a nullable join?).  Then the widenings § V-g listed
as its residual, each a cell of its own before any code: a LOCAL base bound
once and outliving the view (the collection twin's snapshot-witness route —
guard cell c14), a non-const source proven undisturbed between the bind and the
last read, a `CallRef` callee.  After that, in profile order: the vector append
path (~12 %: `vector_append`, `length_vector`, `get_vector`, `vector_finish`);
the default fill of a literal that writes every field (an emitter fact); the
unattributed `Parts::clone`; **P4c record scalars**.

**What § V-g taught, for the next compiler-side unit** (DESIGN.md § V-g's three
findings): count stores from the LABELLED log, not the totals — the store the
census attributed to the copy was native's slot pre-allocation; a parameter's
rebind is one `Set` and reads as single-assigned, so a witness against a
parameter needs `assigned_in`, not `multi_assigned_in`; and a fact the emitters
read is not shipped until it is a STORED variable field verified under
`LOFT_PROGRAM_CACHE=1` (a `target/` binary skips the cache, which makes the
naive warm probe vacuous).  The guard-then-bisect that found c11 (prefix mains,
then pairs, under `LOFT_POISON=1`) is the instrument for a cross-cell store
corruption; a cell that passes alone proves nothing about the frees it leaves.

**How to measure** (all verified this session):

- The engine profile: `scripts/profile.sh --engine -- --native-release
  smooth.loft --n 100000` — 100k reps gives ~800 samples (4 000 gives 40, too
  few to rank); `--calls` attributes the top eight, `--keep` leaves
  `/tmp/loft-profile/perf.data` for `perf report --no-children -G
  --symbol-filter=<sym>` on anything below them.  Timings: `--native-release`
  at 4 000 reps, medians of 3.  The standalone `smooth` row lives in the
  previous sessions' scratchpads (`vr_smooth.loft`, 61 points, hash
  `1a36fee4`); rebuild it from the consumer bench if gone.
- The consumer table: a SCRATCH clone of `loft-libs-graphics` branch
  `drawing-lock`, `python3 bench/compare.py --loft <this tree>/target/release/
  loft --repeat 3` from its `drawing/` — never the consumer's own checkout.
  ~2 min; best of 3; every hash must agree.
- The P0 gate: `scripts/native_ratio.sh --gate` against
  `bench/ratio_oracle.tsv` (`lock` bar 8, `hash` bar 7).  **Its reference
  lane swings 1.6× between back-to-back runs on this box** (DESIGN.md § V-f,
  the instrument note), so read several runs before calling a ratio moved, and
  ratchet a bar only on the consumer lane's best-of-3.
- The codegen gate (loft-codegen skill): `loft introspect` on a probe with the
  inline twin beside the call form, saved under `bytecode-comparisons/`; the
  per-cell store count is a generated one-cell driver per cell under
  `LOFT_STORES=log` on BOTH backends, on and off the switch.
- Traps met this arc: `cargo build --bin loft` does not rebuild the rlib the
  native lane links (`cargo build --release --lib` or `make check-rlib`
  first); a `src/ir_schema_gen.rs` edit is a `tools/ir_schema/ir.loft` edit +
  `make ir-schema-regen` (a drift guard refuses the hand edit); launch a local
  gate with `scripts/ci-run.sh start` and poll `status` in bounded foreground
  windows, or dispatch `ci.yml` on GitHub when the box is under memory
  pressure; a `src/` edit that moves `scripts/wasm_bundle_stamp.sh` needs
  `make wasm` and the bundle committed; a memoised predicate that recurses
  must expose its HIT inline, or the call itself is the cost.

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
| **V-c** — the hoist unblock: a callee whose only store writes are scalars into its own retbuf, and a record's guarded free, no longer decline a header hoist (loft#1426 M2's last per-pixel piece) | [DESIGN.md § V-c](DESIGN.md) | `LOFT_HOIST_VERIFY=1` panics; a hash moves; the resolve loop's hoist count drops below 9 | **Shipped 2026-09-08** — `lock` 18.6M → 15.7M (−16 %, on the prediction), P0 `lock` 5.3× (bar 10 → 8), `hash` 4.5×; pinned both directions in `tests/hoist_gate.rs` |
| **V-d** — append in place: a vector-literal element that is a buffer-returning call is built IN the element's record (the twin of Route R for `v += [pt(…)]`, the two worst rows' allocation class) | [DESIGN.md § V-d](DESIGN.md) | a c1–c10 cell moves; `tests/append_in_place.rs` no longer sees the in-place shape; a consumer hash disagrees | **Shipped 2026-09-08** — `smooth` 200× → 104×, `fronds` 53× → 37× (consumer lane, hashes agree); standalone 15.2M → 3.2M ns/op; scoped to adopting callees (A2/A4 named why); the free bit follows `returns_borrowed_view` (A3) |
| **V-e** — the runtime's per-allocation overhead, found by `perf` once it could run: three uncached env reads per allocation/free, a scan of every type per allocation, field-list clones per copy, a formatted `String` per protected call | [DESIGN.md § V-e](DESIGN.md) | any consumer hash disagrees; `LOFT_STORES=log` stops reporting; the profile's `getenv` line returns | **Shipped 2026-09-08** — standalone `smooth` −50 %; consumer `smooth` 104× → 44×, `fronds` 37× → 24×, every allocation-heavy row moved; behaviour-preserving (hashes exact, both backends) |
| **V-f** — the runtime's per-record bookkeeping: the claims set as a bitset, per-type heap facts (`owns_heap`, `zero_default`) that skip the scalar walks, `strict_stores` as one atomic load, the live-store count and the `"File"` lookup off the hot path, `Store::valid` inlined | [DESIGN.md § V-f](DESIGN.md) | any consumer hash disagrees; `fl_validate` / the "Unknown record" assert stop firing in the armed build; a struct-enum's collection payload leaks; `lock` regresses on the P0 instrument | **Shipped 2026-09-08** — standalone `smooth` −48 %; consumer `smooth` 44× → 25×, `fronds` 24× → 19×, `composite` 19× → 12×, fills 6–7× → 4.5–5×, `lock` 9.0× → 8.0×; behaviour-preserving (hashes exact, both backends) |
| **V-g** — read-only view elision at a record join: a record local bound once from a call whose return borrows a value-const, never-rebound argument, and read only through projections, keeps the VIEW (the copy `(O-Move)` asks for is unobservable there) and releases the callee's minted arm by identity at scope exit — the collection join's route; the mark is a stored variable field | [DESIGN.md § V-g](DESIGN.md) | a control cell stops copying; a strict-store violation on either backend; `tests/view_elision.rs` goes quiet under the switch; a warm `LOFT_PROGRAM_CACHE=1` run copies again | **Shipped 2026-09-08** — standalone `smooth` −28 % (38 → 14 stores per call); consumer `smooth` 25× → 21×; `fronds` unmoved (not this class); 17-cell LOCK + the shape test, falsified; the c11 rebind hole found by the matrix and closed in the collection twin too |
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
   § V): `lock` −30 %, 6.2× on the P0 instrument.  ~~**The hoist unblock**~~ —
   **SHIPPED the same day as § V-c**: the retbuf-only-writer verdict plus the
   record-free admission, `lock` −16 % more, 5.3×.  What Route R still leaves
   on the table: the gate's declined sites (7 pts) are NOT a widening of the
   guards — § V-b built that and the matrix falsified it (three parser-level
   release sites read the dep-free return as a fresh store) — so they wait on
   the type-level fact (a buffer dep the adopt lowering accepts), M–L; and
   Route T's record round-trip (−13 pts) stays a separate item.
2. ~~**Consumer re-run**~~ — done twice (DESIGN.md § Consumer 14-row re-run,
   2026-09-07 and 2026-09-08).  The second one CORRECTS the first: `lock`
   17.2× → 9.2× in the consumer lane, but `smooth` (200×) and `fronds` (53×)
   did not move a point — they are NOT Route R's class.  Their allocation is
   per-element RECORD CONSTRUCTION into a vector (`pts += [Pt{…}]`: an
   `OpNewRecord` + writes + `OpFinishRecord` per element) — **SHIPPED the same
   day as § V-d** (`smooth` −48 %, `fronds` −31 %).  What it leaves: NRVO-shaped
   constructors keep the copy (the same missing type-level fact as § V-b), the
   struct-field twin (`Box { s: mk(i) }`) is untouched.  `smooth`'s other half
   turned out to be the RUNTIME's per-allocation overhead, not arithmetic —
   **§ V-e shipped the same day** (perf, five chokepoint fixes, `smooth` 104× →
   44×).  The head now is what the profile leaves: the claims bookkeeping
   (~16 %, a hasher and an iteration-order check), the per-field walks on
   all-scalar records (~15 %), and the allocation COUNT (borrow-copies of live
   elements, the tangent joins) — DESIGN.md § V-e's residual list, in order.
   **The first two shipped the same day as § V-f, with `Store::valid`
   inlined** (`smooth` 44× → 25×, `fronds` 24× → 19×, `composite` 19× →
   12×).  **The allocation COUNT's first class — the read-only borrow-copies
   — shipped as § V-g** (`smooth` 25× → 21×, 38 → 14 stores per call);
   `fronds` did not move, so its allocations are a different class and the
   next unit starts from its own labelled census — § Where to resume.
3. **P4c record scalars** (S–M, ~5–10 % pixel rows) · **bound-via-header**
   (`h.len` is the bound where P4 fired; S) · **P2 thin-LTO probe** (the
   lean tier's 8.5→6.4 ns gap — the `hash`/`smooth` gate rows carry it).
4. **Closing:** the owner call on `--native-release` implying `--lean`
   (flag semantics are clean post the html lesson); P5's checklist row; the
   PR when the owner judges the branch done.

Honest residual: `lock`'s last stretch (5.3× → 4×) is not yet
probe-covered; § V-e's perf decomposition was of `smooth`, not `lock`, and the
same instrument on the `lock` standalone says whether P4c record scalars + the
remaining per-pixel machinery close it, or whether the type-level buffer fact
(§ V-b) is needed first.  `smooth` (25×) and `fronds` (19×) are the rows
furthest from the bar; after § V-f their remaining cost is the NUMBER of stores
a call makes (the borrow-copies, the tangent joins) more than what each costs —
the queue above.

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
