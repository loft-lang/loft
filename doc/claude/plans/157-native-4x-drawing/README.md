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
`hash` 10.9× → **2.2–2.5×** consumer / 1.2× gate row (under the bar; the spread
is the reference lane's swing); `hair` 4.3× → **2.0×** (under the bar); `lock`
30× → **4.4×** (2.7× gate row); `smooth` 262× → **14×**; `fronds` 49× →
**13.0×**; `composite` 26× → **6.3×**; the fills 17× → **3.8×** (under the bar);
`wide_line` 17× → 5.6×; `lock_curved` 11.9× → **5.7×**; `composite` 26× →
**4.4×** after § V-n + § V-o, **2.3×** after § V-p; `lock` **3.5×** and `lock_curved` **3.8×** after
§ V-q (2026-09-09, both under the bar).
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
- **Last touched:** 2026-09-09 (§ V-q)

## Where to resume

**Rebased onto `main` again on 2026-09-09 (e4c7db58, main's squash of this branch
through § V-l together with 18 issues): § V-m, P4c, § V-n and § V-o were replayed
with `git rebase --onto origin/main 91b15a44`; only the derived audit rows
conflicted and were re-measured on the joined tree; the bundle and the surface
index regenerated; the tip force-pushed with lease and the GitHub gate dispatched
on it (run 34323456806).**  The four units below are the day's work; the
scoreboard at the top is the joined tree's.

Written 2026-09-08 after § V-g landed, updated the same evening: the branch
was REBASED onto `main` (#1465 squashed this branch through § V-d together
with 55 other issues; the 13 commits after it were replayed with `git rebase
--onto`, conflicts resolved by carrying main's new `Parts::IntRaw` kind into
§ V-e's `Shape` and § V-f's facts, and main's `Output::lean` frame-naming
home into the shipped tier).  After it: queue item 1 (the hoisted loop bound)
and the leaf guard elision shipped, and the runner's death signal + the tests'
orphan reaper closed the engine-host leak.  The gate is the GitHub `ci.yml`
dispatch on the head (the local `make ci` was dying under memory pressure —
CI_BUDGET.md § When the local gate is unreliable); codegen, scopes, store and
runtime subject suites were green on the rebased tree locally, `packages` was
stopped before its verdict to free the box and is covered by the dispatch.

**Item 1 shipped** (the hoisted loop bound) with the leaf guard elision beside
it, and **§ V-h the same evening** (DESIGN.md § The out-of-line calls): the
owner's steer that LLVM would already be using the overflow flags was checked
in the disassembly and found true — what the `hash` row paid was the
un-inlined helpers BESIDE the checks, and splitting those into an inline test
plus a `#[cold]` body put the fully checked row at its plain-arithmetic floor
(`hash` 2.6× in the consumer lane).  `scripts/native_call_census.py <binary>`
is the instrument; what remains on its list is the allocation work of items 4
and 5, `get_vector` at 35 sites outside hoisted loops, and `__rust_dealloc`
in the text functions.  **The next unit is `fronds`' deep-copy class as a MOVE** (item 4 = § V-j,
designed in DESIGN.md § V-i): the sub-call's result Fronds are copied one by
one into the parent with their inner vectors and the source freed — after
§ V-i what that costs is two claims and two frees per element, not
bookkeeping.  The design: the call whose result feeds only an append loop
into `V` is delivered into `V`'s store through the `__ref` retbuf the callee
already receives, and the loop's `V += [f]` becomes a move (the element's
bytes copied shallowly, the source element marked moved, the temporary's free
shallow).  "Build into the caller's vector" is ruled OUT — `fronds` indexes
its own level with `len(fd_out)`, so sharing the vector folds the caller's
levels into the recursion.  The twelve cells are written and pass on both
backends under the copy semantics (`bytecode-comparisons/
V-j-move-append-cells.loft`); add the nested-container and enum-payload
element cells, run `LOFT_POISON=1` over the set, then the code — the
interpreter's op is the native emitter's op, because the IR is shared.  Item 3 (sentinel elision by RANGE proof — `n_seed_hash`'s
`& 0xFFFFFFFF` bounds every operand, and a bounded operand cannot overflow;
the owner ruled that null is made by ordinary arithmetic, so the declaration
is never the fact, the range is) keeps its argument but lost its measured
payoff to § V-h: rank it again when a checks-only loop shows as hot.  § V-i
took the walk out of the deep copy (`fronds` −7 %); what the class still costs
is allocation, and DESIGN.md § V-i writes the move design's cells.  **Item 4's ceiling is now measured** (DESIGN.md
§ V-j: the move −8 %, the shared arena +7 % through a `coalesce_free` cliff)
and the fresh-destination clear it exposed shipped as § V-j; the move itself
is ranked below 4b and 5.  **§ V-k came from profiling `lock` WITH CALLERS on
the § V-j runtime** rather than from the queue: the append path's bookkeeping
was 40 % of that row and reached every row that appends (`lock` 6.6× → 5.2×,
`lock_curved` 9.9× → 7.0×, `fronds` 15.5× → 13.8×) — so the next step is the
same instrument on the rows still over the bar.  Both units that ranking named
shipped the same day: **the fused scalar append** as § V-m (`lock` 5.2× → 4.6×,
`lock_curved` 6.9× → 5.8×, `wide_line` 6.3× → 5.5×) and **the in-place-only
writer** as § V-l (a setter-calling loop −21 %; `composite` hoists but did not
move — its cost is the pixel methods' own unhoisted single reads, 29 %, and the
caller's per-pixel scalar field reads, 8 %).  § V-m's full local gate (run
before its commit, 2026-09-09) found three more op-name lists the fused push
had to reach — the constant-store builder, the native fn-ref collector and the
move elision's escape test, each already guarded (DESIGN.md § V-m, the
classifier paragraph) — so a new writing op is now EIGHT lists, and
`parser::FUSED_PUSH_KINDS` is their one home.  Its GitHub gate then caught what
the local curated set had not (DESIGN.md § P4c, finding 2): a keyed collection
of ONE-FIELD records fused by shape and walked empty — the fusion now asks the
schema what the container holds.  **P4c SHIPPED the same day** (DESIGN.md
§ P4c): loop-invariant record scalar reads hoisted as `__vs_N` locals under the
header gate, keyed `(record type, offset)` with an admitted callee's written
set evicting the caller's scalars; nineteen cells, falsified by disabling the
eviction (eight red), `LOFT_NO_SCALAR_HOIST` / `LOFT_HOIST_VERIFY`.  **§ V-n
shipped after it** (DESIGN.md § V-n): a `&`-bound vector view derives its
header at the binding for the rest of its block, so the pixel methods'
element access takes one resolution instead of three (`composite` 6.3× →
6.1×, the render rows −5–6 %).  **§ V-o shipped after it** (DESIGN.md § V-o):
a stdlib one-op wrapper is emitted as its op, so `len(d)` reads the header —
`composite` 6.1× → **4.4×**, the last of that row's store resolutions.
**Next:** `composite` sits 0.4× over the bar, and its profile after § V-o (joined
tree, `--only composite`, 1.5k samples) no longer shows a runtime helper at the
top: `n_composite_layer` 37 % self, `get_pixel` 15 %, `set_pixel` 12 %, the fused
write 4.4 %, `get_elem_hoisted` 4.2 %, `addr_mut` 2.4 % — the program's own
arithmetic and the two method calls per pixel, whose bodies re-read
`self.width` / `self.height` per call (single reads, so P4c cannot hoist them
and the caller's hoisted scalars do not cross the call).  The unit that closes
that is INTERPROCEDURAL — the pixel methods inlined into the caller's loop, or a
parameter's invariant scalar fields passed across the call — an M design for the
next session (DESIGN.md § P4 item 3's caller side, one call deep).  After it:
the text readers off the hoist allow-list (P4c c17); § V-o's leaf-only rule
lifted by mirroring the pre-eval map onto the cloned operands; the 27 scalar
`OpNewRecord` sites the fusion does not reach (`parse_*`, `text.split`,
`File.lines` — S, no judged row); then the queue's 4b / 5 / the move.  The
scratch clone's `bench/bench.loft` carries a `--only <routine>` switch (scratch
only, never the consumer's tree): `scripts/profile.sh --engine --calls --
--native-release <clone>/drawing/bench/bench.loft --n 4000 --only composite`
is how each row was ranked; an all-rows run is dominated by the unjudged
`resize`.

**§ V-p SHIPPED 2026-09-09** (DESIGN.md § V-p): the interprocedural unit above,
decided by MEASURING both candidates on hand-edited emitted Rust first — a twin
of each pixel method taking the caller's hoisted values as parameters and a full
inline of the methods cost the same (rustc inlines the twin), so the rewrite passes
values and never clones IR.  `composite` 4.41× → **2.34×**, `render_lock` /
`render_marks` −8 %, everything else noise; seventeen cells, `tests/callee_inputs.rs`
and `tests/scripts/157-callee-inputs.loft`, switch `LOFT_NO_CALLEE_INPUTS`.  On the
way the rebased tree's gate turned red on a § V-o hole — a USER one-op function
inlined as a stdlib wrapper skipped the live flip (DESIGN.md § V-o, third finding) —
fixed with cell c11, and the whole native rewrite family got its formal chapter
(`doc/claude/formal/rewrites.md`, `@FR-R-…`, nine rules, the code sites citing them;
`hoist.rs` had cited none while enforcing eight).  Cell c18's INTERPRETER oracle then
answered a garbage word: re-pointing an existing `&` link (`cur = &a; … cur = &b`) was
broken on `--interpret` at every kind but a vector, and the rule was unwritten —
`(B-Ref-Repoint)` now stands in `formal/binding.md` (D-bind-30 opened and closed the
same day), `set_var`'s link branch routes the install to the link's own slot first, and
`tests/scripts/157-link-repoint.loft` guards six kinds on both backends.  **Next:** the rows still over the
bar are `smooth` 15.5× and `fronds` 12.9× (the allocation class — queue items 4b / 5
and the move, DESIGN.md § V-j's ceiling), then `lock_curved` 5.6× and `wide_line`
5.4× (profile WITH CALLERS on the § V-p runtime, as § V-k did), `lock` 4.39× a hair
over.  The text readers off the hoist allow-list (P4c c17) and § V-o's leaf-only
rule (a mirror of the pre-eval map onto the cloned operands) stay queued behind them.
GitHub gate run 34334280254 is GREEN on the § V-p tip (ce1db5bb: the unit, the interpreter
link re-point, the test-subject and walker-audit bookkeeping).

**§ V-q SHIPPED 2026-09-09** (DESIGN.md § V-q): the raster rows profiled WITH callers on the
§ V-p runtime put 30–45 % of `lock` / `lock_curved` in the scalar APPEND path — `lock_layer`'s
seven constant-fill comprehensions, one push at a time at ~12 ns.  A loop that pushes to a
vector path now keeps a PUSH header (the triple plus the capacity) the push itself keeps
current, refreshed at the growth step; aliasing decided by ownership; the ceiling measured by
hand first (−33 %).  `lock_curved` 5.64× → **3.84×**, `lock` 4.39× → **3.51×**, both under
the bar; eighteen cells, `tests/push_hoist.rs`, `tests/scripts/157-push-hoist.loft`, switch
`LOFT_NO_PUSH_HOIST`.  Two findings worth the next reader's time: a push whose VALUE reads the
pushed vector is materialised by the parser as a whole-vector COPY per iteration
(`lock_ribbons`' `lr_cum += [lr_cum[len(lr_cum)-1]? + lr_d]` is quadratic in the point
count — a unit of its own for a long path), and a rewrite that removes a path from the read
list must run after every collector that can add one (the § V-p twin inputs gave the pushed
path a second, stale header until the order was fixed; the verifier caught it).  **Next:** the
rows still over the bar are `smooth` 15.5× and `fronds` 12.9× (the allocation class, queue
items 4b / 5 / the move) and `wide_line` 5.4× (its rasteriser's own arithmetic — profile at
`--n 200000` to rank; the 4000-rep profile is half compile).  Check the GitHub gate dispatched
on the § V-q commit before building on it.

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
| **V-h** — the out-of-line calls: a runtime helper the emitted code calls per op crosses the rlib boundary as a real call unless it is `#[inline]`, so `note_format_fault` (after every float division), the two per-frame guards and `length_vector` split into an inline test and a `#[cold]` body; `scripts/native_call_census.py` ranks what still crosses | [DESIGN.md § The out-of-line calls](DESIGN.md) | the census names a fast-path symbol; `hash` above its plain-arithmetic floor | **Shipped 2026-09-08** — `hash` row 330–407k → 219–287k ns/op against a 225–272k floor with every check kept; consumer `hash` 6.5× → 2.6×, `composite` 8.9× → 7.5×, `wide_line` 9.0× → 6.5×, fills at the bar; 14/14 hashes agree |
| **V-i** — the element walk of a no-heap vector: the ownership keystone enumerated every inline element of a `vector<float>` / `vector<Pt>` for the copy and the free to visit and find nothing; the walk now yields no children when the element type owns no heap (§ V-f's fact, placed once so every consumer inherits it) | [DESIGN.md § V-i](DESIGN.md) | `fronds` not moved; a leak or a hash change on either backend | **Shipped 2026-09-08** — `fronds` 902–907k → 838–840k ns/op (−7 %) standalone, 18.1× → **15.5×** in the consumer lane (933k → 844k), hash unchanged both backends, leak checks clean, store + runtime suites green locally, codegen on the GitHub gate |
| **V-j** — the copy into a fresh element: `v += [f]` cleared a destination `OpNewRecord` had just defaulted, a walk allocating child lists to find nothing; the parser marks the copy's destination fresh (`COPY_FRESH_DEST`) and both runtimes skip the clear.  The move-append's ceiling was measured on the way (−8 %, P1) and the shared-arena variant found a runtime cliff (`coalesce_free` 29.5 %, P2) | [DESIGN.md § V-j](DESIGN.md) | a cell leaks or answers wrong on either backend | **Shipped 2026-09-09** — `fronds` 838–845k → 794–801k ns/op (−5.5 %), interpreter −7 %, hashes exact; 12 cells clean under warn/leak/poison |
| **V-k** — the append path's bookkeeping: a top-level append to a plain vector took three type lookups, the general dispatch, a `Parts::clone` per insert and a `resize` call per element (40 % of `lock`); short paths in `record_new` / `record_finish`, a copied insert kind, and `resize` on the growth step only | [DESIGN.md § V-k](DESIGN.md) | a cell leaks or answers wrong on either backend; the gate rows' hashes | **Shipped 2026-09-09** — `lock` gate row 4.4× → 3.4×; consumer `lock` 6.6× → 5.2×, `lock_curved` 9.9× → 7.0×, `fronds` 15.5× → 13.8×, `smooth` −10 %; 14/14 hashes agree |
| **V-l** — a loop calling an IN-PLACE-ONLY writer keeps its hoisted headers: `in_place_only_writer` beside § V-c's `retbuf_only_writer`, admitted under the in-place tier (`LOFT_NO_INPLACE_CALLEE_HOIST` off-switch, `LOFT_HOIST_VERIFY=1` the falsifier); nine cells, five hoist and four must not | [DESIGN.md § V-l](DESIGN.md) | a cell hoists that must not (c2/c4/c6/c9), or the verifier panics on any cell | **Shipped 2026-09-09** — the setter-loop A/B −21 %; `composite` hoists but its cost is inside the pixel methods (next: their own reads + loop-invariant scalar fields) |
| **V-m** — the fused scalar append: `v += [x]` on a plain vector was five runtime calls per element; seven typed `OpPush<Kind>` ops (one resolution, one capacity test, one write, one length bump, both backends), fused by the parser at `new_record` and the comprehension lowering; five writer classifiers taught the op | [DESIGN.md § V-m](DESIGN.md) | a cell leaks or answers wrong on either backend; `tests/fused_append.rs` | **Shipped 2026-09-09** — consumer `lock` 5.2× → 4.6×, `lock_curved` 6.9× → 5.8×, `wide_line` 6.3× → 5.5×, fills under the bar; 14/14 hashes agree |
| **P4c** — loop-invariant record scalar reads hoisted as locals: `lay.x0` read once before a loop that cannot write it, keyed `(record type, offset)` over the body's typed write set plus what an admitted callee reaches (`hoist::hoistable` / `WriteSet`); `LOFT_NO_SCALAR_HOIST` off-switch, `LOFT_HOIST_VERIFY=1` the falsifier | [DESIGN.md § P4c](DESIGN.md) | a cell hoists a field the loop writes (the eight sabotage-red cells), or the verifier panics on any cell | **Shipped 2026-09-09** — 19 cells exact on both backends; emission pinned per cell (`tests/scalar_hoist.rs`); consumer `composite` 7.6× → 6.3× (−17 %), `lock` 4.6× → 4.4×, 14/14 hashes agree |
| **V-n** — a `&`-bound vector VIEW (`d = &cv.data`) derives its header once at the binding for the rest of the block that indexes it (`hoist::view_def_header`, `Output::bind_view_header`): the pixel methods' `len(d)`-guarded element read/write take one store resolution instead of three; `LOFT_NO_VIEW_HOIST` off-switch, `LOFT_HOIST_VERIFY=1` the falsifier | [DESIGN.md § V-n](DESIGN.md) | a binding earns a header its remainder can invalidate (c18 red), or the verifier panics on any cell | **Shipped 2026-09-09** — 18 cells exact on both backends, emission pinned (`tests/view_header.rs`); consumer `composite` 6.3× → 6.1×, `render_lock` −6 %, `render_marks` −5 %, 14/14 hashes agree |
| **V-o** — a stdlib ONE-OP wrapper (`len(v)`, `sqrt(x)`, 43 of them) is emitted as its op with the caller's arguments in the operand positions (`hoist::one_op_wrapper`, `Output::wrapper_op`), so the header-aware emitters see `len(d)` after a view binding and inside a hoisted loop; `LOFT_NO_WRAPPER_INLINE` off-switch | [DESIGN.md § V-o](DESIGN.md) | a wrapper call is left, or an operand lands in the wrong position (the reversed map fails to compile; the parameter swap turns `pow` wrong) | **Shipped 2026-09-09** — 10 cells exact on both backends, emission pinned (`tests/wrapper_op.rs`); consumer `composite` 6.1× → **4.4×** (−28 %), 14/14 hashes agree |
| **V-p** — a callee's INVARIANT INPUTS cross the call: a callee admitted under `@FR-R-Callee` gets a TWIN (`<fn>__inv`) taking its record parameter's invariant scalars and vector headers as extra parameters (`hoist::callee_inputs`, transitively through pass-through callees), and a loop that hoisted them for the argument variable calls the twin (`Output::twin_call_inputs`); `LOFT_NO_CALLEE_INPUTS` off-switch, `LOFT_HOIST_VERIFY=1` the falsifier | [DESIGN.md § V-p](DESIGN.md) | a c1–c17 cell moves; `tests/callee_inputs.rs` no longer sees a twin or a twin call; a consumer hash disagrees | **Shipped 2026-09-09** — `composite` 4.41× → 2.34× (under the bar), `render_lock` / `render_marks` −8 %; the two candidate designs measured equal by hand before the code, so the cheaper machinery was built |
| **V-q** — a loop that PUSHES to a vector path keeps a PUSH header: (R-Header)'s triple plus the capacity, the push writes through it and refreshes it at a growth step, every read of the path serves from it (`hoist::FUSABLE_PUSHES`, `hoist::fused_push`, `Stores::push_hoisted`, the ownership rule in `hoist::hoistable`); `LOFT_NO_PUSH_HOIST` off-switch, `LOFT_HOIST_VERIFY=1` the falsifier | [DESIGN.md § V-q](DESIGN.md) | a c1–c18 cell moves; `tests/push_hoist.rs` no longer sees a push header; a consumer hash disagrees | **Shipped 2026-09-09** — `lock_curved` 5.64× → 3.84×, `lock` 4.39× → 3.51× (both under the bar); ceiling measured by hand first at −33 % |
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

## Phase ordering — the remaining queue (re-ranked 2026-09-08, all probe-measured)

P0–P4d, § V–V-g and the shipped tier are in; the original ordering stands as history at
the bottom.  Nothing measured puts loft at a floor: in `smooth`'s profile the program's
own arithmetic is 28 % of the row, and the simplest row, `hash` (a byte loop, 9× on the
shipped tier), shows three closable costs in its emitted Rust — DESIGN.md § The floor.
What remains, ranked by measured value per unit of work:

| # | item | evidence | expected | size |
|---|---|---|---|---|
| 1 | **bound-via-header** — the loop bound re-reads the vector length through the runtime on every iteration even where the header was hoisted (`length_vector` beside `get_elem_hoisted` in `n_fnv`) | every `for` over a vector | `hash` and every element loop | S |
| 2 | **constant shift amounts need no range check** — `x >> 8` emits a `(0..64).contains` test on a literal | `n_fnv` | XS, folds into 1 | XS |
| 3 | **range proofs for sentinel elision** — every integer op is sentinel-checked (`op_mul_int`, `op_exclusive_or_int`, `op_logical_and_int` test both operands for `i64::MIN`), and that is the SEMANTICS, not a missing declaration: null is made by ordinary arithmetic — `a/b`, `sqrt(a)`, `a+b` and `a*b` on overflow — so a non-nullable parameter is non-null only at entry and no boundary check can license trusting it inside (owner's ruling, 2026-09-08).  What CAN license eliding a check is a proof about the VALUE: a masked or bounded integer cannot overflow, a product of two bounded ones cannot, a divisor proven finite and non-zero cannot yield NaN — value-range propagation over the P3 non-sentinel facts, which already exclude arithmetic for this reason.  An opt-in to processor semantics (wrapping integers, IEEE floats as values) is DECLINED — machine-dependent behaviour is the opening loft closes (DESIGN_DECISIONS.md C67) | `n_seed_hash`: `& 0xFFFFFFFF` bounds every operand, so each following op is provably non-null — but the third of the row charged to the checks (553–584k → 370–396k ns/op) was the un-inlined fault note BESIDE them: with it inlined the fully checked row sits at its plain-arithmetic floor (219–287k vs 225–272k, DESIGN.md § The out-of-line calls), LLVM compiling each overflow test to `imul` + `jo` and each sentinel test to one `cmp`/`je` | none measured yet — re-rank when a checks-only loop with no helper call beside it shows as hot | M–L |
| 4 | **`fronds`' deep-copy class as a MOVE** — the sub-call's result Fronds are copied one by one into the parent; the move (the call's buffer placed in the parent's store, the loop's append a block move, the buffer freed as a record) is designed with its twelve cells and its ceiling MEASURED at −8 % (DESIGN.md § V-j P1); the shared-arena variant is slower (P2, a runtime cliff), and the fresh-destination clear it exposed shipped as § V-j | DESIGN.md § V-i (design), § V-j (ceilings) | `fronds` −8 % | M+ — ranked below 4b and 5, whose ceilings are of the same size at S each |
| 4b | **`fronds`' allocation class** — the sides literal, the two builders, the per-sub-array spec; CEILING MEASURED at −9 % by a source-level variant (−64 % stores, hash exact) — small emitter items, not the half the census suggested | DESIGN.md § fronds | `fronds` −9 % | S each |
| 5 | **frame-local record temporaries** — 12 of `smooth`'s 14 remaining stores are `pt(…)` results bound to locals; the scalar-tangent probe gained 11 % while DOUBLING the helper calls, so a temporary that lives in the frame takes at least that | § V-g's ceiling probe | `smooth` 19× → ~14×; the same class in every routine that names a struct temporary | M–L |
| 6 | **runtime ownership at calls** — the protect/unprotect bracket (3.3 %), the free path (`free_named` + `close_file_handle`, 5 %), the join guards; each is a decision § V-g showed can move to compile time | the post-V-g profile | per call, every row | M each |
| 7 | **the vector append path** — `vector_add`, `vector_append`, `vector_finish`, `get_vector`, `length_vector` ≈ 19 % of `smooth` | the post-V-g profile | the store's general allocator vs `Vec::push`; the bump claim (shipped, DESIGN.md § fronds) took the free-tree walks out of the fresh-store case | M |
| 8 | **LTO into the rlib** — only `#[inline]` functions cross the program/runtime boundary | P2's open probe | non-inline runtime calls per element | probe first |
| 9 | **profile the pixel rows** — `lock` 7.8×, `composite` 11×, `wide_line` 8.9× were never profiled; only `smooth` was | — | unknown until measured; do before any pixel-side unit | S |
| 10 | **P4c record scalars** — both the gate bench and the consumer's raster loop already hoist by hand | `raster_segment`'s `rs_ly0 = lay.y0` | small for this consumer; real for the next | S–M |

The § V-g widenings (a local base, a non-const source, a `CallRef` callee) sit beside 5:
each is a cell before any code.  Closing: the PR when the owner judges the branch done;
P5's checklist row.

Honest residual: `hair` (2.6×) is at the floor the design predicts (DESIGN.md § The
floor); every other row is above it for a reason in the table, not for a reason in the
design.  `lock`'s last stretch is 9 and 10 first — measure, then choose.

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
