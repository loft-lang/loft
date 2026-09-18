<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 157 — Native within 4× of Rust on the drawing pass

## Status

> **This is the one home for the scoreboard.**
> [loft#1426](https://github.com/loft-lang/loft/issues/1426) is the *surfaced report* —
> what crawler hit, and whether it is resolved for them — and it carries `status:planned`
> pointing here.  Its table is the FILED baseline, frozen as evidence and not
> maintained.  **Do not copy per-row numbers back into the issue**: they were in both places and
> drifted twice in one day (one taken on a branch, one on a join), and a wrong attribution had to
> be corrected in two places.  Report progress by editing this section; comment on the issue only
> to tell the consumer something they need — a row crossing the bar, or the class closing.

Open — P0–P4d, § V (Route R), § V-c (the hoist unblock), § V-d (append in
place), § V-e (the runtime's per-allocation overhead), § V-f (the runtime's
per-record bookkeeping) and § V-g (read-only view elision at a record join)
SHIPPED (see Sub-arcs); the queue is re-ranked below, and **§ Where to
resume** is the hand-off for the next session.
**`R-BoundedNest` SHIPPED 2026-09-17** (`formal/rewrites.md`, the queue's item 2 and C120's
admissible successor): an innermost counted loop that accumulates `?`-discharged element
products runs with PLAIN operators behind a guard evaluated once at its entry — every range end
and invariant not the sentinel, every read's element bound known (taken once at the outermost
loop that leaves the vector alone), the magnitude bound of every chain and of
`|acc| + trips × bound(term)` fitting `i64` — with the checked loop as its `else`.  Measured
on the arm64 lane (`compare.py`, 14/14 hashes): `render_marks` 6.11× → **2.69×**,
`render_lock` 4.87× → **2.76×**, `resize` 5.37× → **2.42×**, the rest within noise — **every
judged row but `lock_curved` (3.87× on this lane) is under the 3× bar, median 2.35×**.  Beside
it a runtime item the same session's `sample` profile named: a self-append (`v += v`, the
canvas fill's doubling ladder) copied byte by byte through a snapshot and, for heap-owning
elements, read the source through a freed block — one block copy now, both backends,
`render_marks` −8 % (D-heap-17).  **Step 2 the same evening:** where every read's chain is affine
in the counter the guard also proves both range ends in `[0, len)` and the arm reads RAW through
the held base with no null select — the vectorisable multiply-accumulate: `render_marks`
1 740 → **1 125 µs/op (−35 %), 1.75× its reference**; the branchless `abs_bound_i64` −4 % beside
it.  The full lane after step 2 (arm64, `compare.py --repeat 3`, two interleaved rounds against the
step-1 arm, 14/14 hashes): `render_marks` 2.79–2.86× → **1.54–1.74×**, `render_lock` 2.68× →
**2.01×**, `resize` 2.34× → **1.35–1.37×**, every other row within noise; **median 2.02×**, the
(Perf-Weight) median bar met on this lane, `lock_curved` (3.85×) the one row still over 3×.  Switches `LOFT_NO_BOUNDED_NEST`, `LOFT_NO_NEST_RAW_READS`, `LOFT_NO_SELF_APPEND_BLOCK`;
falsifier `LOFT_HOIST_VERIFY=1` (`ops::nest_verify`, and the raw read compared with the checked
one); pins `tests/bounded_nest.rs`; cells `tests/scripts/157-bounded-nest.loft` n1–n23.
**`R-LoopRecord` SHIPPED 2026-09-18** (`formal/rewrites.md`, unit A of the store-traffic
analysis in § Where to resume): a plain no-heap record local declared and minted by a literal
inside a loop keeps its store and its record across passes — declared at the loop's prelude, a
null-guarded first mint, a partial literal re-establishing its declared defaults, one free after
the loop.  Probe: 39 → 2.4 ns per pass; the lane does not move (`fd_sub` is 24 mints per `fronds` call).  Switch `LOFT_NO_LOOP_RECORD`; trace `LOFT_TRACE_LOOP_RECORD`; cells
`tests/scripts/157-loop-record.loft` l1–l15, pins `tests/loop_record.rs`.  Found on the way and
fixed first: the interpreter's last-use move firing on a name re-declared in a sibling scope
(`Function::copy_variable` started the split at `uses: 1`; guard
`a-name-reused-in-a-sibling-scope-copies.loft`).
**`R-RecPtr` SHIPPED 2026-09-18** (`formal/rewrites.md`, `(R-View)`'s record twin, out of the
`wide_line` analysis below): a plain-record VIEW (`pg_cur = pg_table[i]?`, `s = o.inner`)
carries the address of its record for the rest of its block, every scalar field read is one
load and every in-place write one store through it, and a callee twin the view is handed to
takes its scalar inputs read through the address at the call (`edge_x`); beside it `(R-Base)`
admits the null-discharge buffer's mint as growth-free, so the crossing loop holds its bases.
Standalone probe: `wide_line` **7 800 → 5 060 ns/op (−35 %), 2.9× → 1.9×** its reference.
The lane, two interleaved rounds against the switch, 14/14 hashes: `wide_line` 2.41× → **1.76×** (−27 %), `fill_circle` 1.46× → **0.89×** (−40 %), `fill_star` 1.31× → **0.94×** (−28 %) — the fills share `polygon_generic` — the rest within swing; **median 1.67×, three rows now under 1×**.  Switches `LOFT_NO_RECORD_PTR` (and `LOFT_NO_VECTOR_BASE`); falsifier
`LOFT_HOIST_VERIFY=1`; trace `LOFT_TRACE_RECPTR`; cells `tests/scripts/157-record-ptr.loft`
r1–r16, pins `tests/record_ptr.rs`.  **The `for e in v` loop variable joined the same day**: the
loop's own statements are emitted from `Value::Loop`, not `output_block`, so the hook had never seen
its `Set(e, iter-next)`; and its type is the NULLABLE element (the null ends the loop), admitted as
its record with the null address answering the sentinel.  Its probe: three fields per element over 100 000 records, −31 %; no lane row moves (the bench's only record-iterating loop grows `pg_table` and declines by design), and a second lane pass on and off confirmed every row within its swing, 14/14 hashes.
**`R-Base`'s twin clause SHIPPED 2026-09-18** (`formal/rewrites.md`, unit (1) of the
`composite` pricing below): a § V-p twin takes the element BASE of each header it is handed
(`__ib_k`), a view of the path inside it shares that base, and its fused reads and writes take
`get_elem_at` / `vec_set_at` — the caller passes its held `__vb_N` or derives one from the held
header at the call.  Measured on the arm64 lane, two interleaved rounds against the switch, 14/14 hashes: `composite` **114.5 → 91.3 µs (−20 %), 2.98× → 2.40×**; `lock` 1.83× → 1.76×, `lock_curved` 2.16× → 2.09×, `render_lock` 1.80× → 1.72×, the rest within their swing; **median 1.74×, every row under 3×**.  Switch `LOFT_NO_TWIN_BASE`; falsifier `LOFT_HOIST_VERIFY=1`;
cells `tests/scripts/157-twin-base.loft` t1–t10, pins `tests/twin_base.rs`.
**§ V-ab SHIPPED 2026-09-13** (DESIGN.md § V-ab): a counted `for` whose start is not a
literal runs a second counter seeded AT the start instead of a null-encoded one — no null
test per iteration on either backend (bare loop −32 %, a contiguous fill −18 %, the
interpreter −11 %; consumer lane `fill_circle` −3.3 %, `fill_star` −2.2 %, `wide_line`
−2.3 %, the rest noise).  Switch `LOFT_NO_NEXT_COUNTER`.  The inclusive-to-type-maximum
hang it met is pre-existing and filed as loft#1525.
**§ V-ac SHIPPED 2026-09-13** (DESIGN.md § V-ac): a header input of a § V-p twin is keyed
on the argument's PATH, so `brush_sample(br.img, …)` takes the header the resolve loop
already holds, and a return-buffer writer earns a twin — measured on **aarch64 Linux
(host `lima-default`)**, ABAB: `lock_curved` −6.3 % (3.89× → 3.63×), `lock` −8.7 % (3.73× →
3.42×), `render_lock` −2.9 %, the rest noise, 14/14 hashes.  Same switch as § V-p.
**§ V-ad SHIPPED 2026-09-13** (DESIGN.md § V-ad): the null-discharge default buffer's
allocation (`pg_cur = pg_table[i]?`) no longer declines the header hoist of the loop
around it, so `polygon_generic`'s crossing loop hoists its three vectors — aarch64 Linux
(host `lima-default`): `wide_line` 16 380 → **11 580 ns/op** (5.35× → **4.02×**),
`fill_circle` 105k → 89k (3.89× → 3.52×), `fill_star` 38.7k → 31.5k (3.81× → 3.37×);
14/14 hashes.  Switch `LOFT_NO_NULL_BUFFER_HOIST`.
**§ V-ae SHIPPED 2026-09-13** (DESIGN.md § V-ae): the FILL idiom — `pil_hline`'s
`for x in lo..=hi { d[base + x] = ink }` is one range test and one slice fill, the
per-element loop its fallback — aarch64 Linux (host `lima-default`): `wide_line` 11 580 →
**9 020 ns/op** (4.02× → **2.91×**, under the bar), `fill_circle` 89.1k → **45.9k** (3.52× →
**1.70×**), `fill_star` 31.5k → **18.0k** (3.37× → **1.79×**); 14/14 hashes.  Switch
`LOFT_NO_FILL_HOIST`.  With § V-ad the row read 17.0k ns/op this morning: **−47 % in a
day, 5.35× → 2.91×**.  Eight of the ten judged rows are under the bar here; `smooth` and
`fronds` remain, and neither is this class.
**§ V-af SHIPPED 2026-09-13** (DESIGN.md § V-af): a value branch of buffer-delivering calls
witnesses every arm's buffer, so `smooth_pts`'s `sp_ta`/`sp_tb` (an `if` of `half_chord` and
`pt`) reuse four buffers allocated once instead of minting and freeing a store per
iteration — aarch64 Linux (host `lima-default`): standalone `smooth` 1 728 → **1 286 ns/op**
(−26 %), the consumer row 2 100 → **1 620** (10.5× → **9.0×**), allocations per call 33 → 17;
14/14 hashes.  Beside it two runtime fixes measured on the same profile (−8 % on the row):
an uncached `LOFT_TRACE_CLEAR` read in `clear_vector_release` and the per-allocation
type-variable prefix compare in `enum_parent_size`.  **§ V-aa is NOT the unit for
`smooth`**: switched on, the row reads +1–3 % — `pt` already builds into the appended
element, so the tuple only adds a materialisation.  Switch `LOFT_NO_JOIN_BUFFER_WITNESS`.
**§ V-ai SHIPPED 2026-09-14** (DESIGN.md § V-ai): the § V-ag reset re-establishes the root
vector at the CAPACITY the previous fill reached instead of the fresh minimum — the buffer
is reused across calls and the store already holds the extent, so the growth ladder runs
once per buffer, and the freed rungs that took every later claim off `bump_tail` and into
the free tree are never made — x86-64 Linux (host `tuxedo`), ABAB on one binary:
`fr_only` 172.4k → **133.5k ns/op (−22.5 %)**; consumer `fronds` 175 920 → **143 180**
(**4.28× → 3.42×, under the bar** on this lane), every other row flat, 14/14 hashes.  Nine
hand-derived cells on both backends under strict stores, poison and the switch; the guard
reads the re-established capacity off `LOFT_TRACE_CLEAR`.  Switch `LOFT_NO_RESET_CAPACITY`.
**§ V-al SHIPPED 2026-09-15** (DESIGN.md § V-al): a vector local declared `[]` INSIDE a
loop keeps its per-site buffer's store and vector across iterations — the mint after
the first is a length reset that keeps the capacity, the literal's field zero is not
emitted — for elements that own no heap.  The resample's two per-pixel vectors and
`resample_coeffs`'s per-column row: the probe 109.0 → **97.6 ms/op** (−10.5 %, hash
exact, best of 3 at 20 iterations, quiet x86-64).  Seven hand-derived cells on both
backends under poison, poison-claim, strict stores and the leak check; the guard's
receipt is the value channel.  Consumer lane (500 calls per row, 14/14 hashes):
`resize` 5.66 → **5.36×**, `render_marks` 8.40 → **7.82×**, `render_lock` 5.67 →
**5.31×**, the rest within noise.  Switch `LOFT_NO_LOOP_BUFFER_REUSE`, trace
`LOFT_TRACE_LOOP_BUFFER`.
**§ V-am SHIPPED 2026-09-15** (DESIGN.md § V-am): a counted push loop RESERVES its pushes
times its trip count once before it runs, and a counted loop that is one push of an
invariant is ONE fill of the vector's tail (the per-element loop its fallback, the
counters left as the loop would).  The resample's `rl_mid` prefill is the fill and its
premultiply plane the reserve: the probe 97.5 → **95.4 ms/op** (−2.2 %, hash exact).
Eleven hand-derived cells on both backends under the falsifiers and the switch; the
guard's receipt is the value channel.  Consumer lane with § V-al and § V-am together
(500 calls per row, 14/14 hashes): `resize` 105.4 → **93.5 ms/op** (5.36×),
`render_marks` 7.46 → **6.70 ms/op** (7.57×), `render_lock` 16.26 → **14.84 ms/op**
(5.20×).  Switch `LOFT_NO_PUSH_FILL`, trace `LOFT_TRACE_PUSH_FILL`.
**§ V-an SHIPPED 2026-09-15** (DESIGN.md § V-an): the value form's PHANTOM return buffer
is a value local when every assignment to it is a value shape — which admits the parser's
`return f(…)` chain (`rb = f(…, rb); …; return rb`), a local promoted into the buffer, and
an `Object` that returns what it builds with its own frees inside.  Every reader of the
drawing package's scan module was declined by that one shape; admitted, the `parse` row's
≈194 `Scan` mints per parse are none: **77–86 → 50–54 µs/op** (10.5× → ≈ 7.2×, hash
33f6d2b8).  Nine hand-derived cells on both backends under leak check, strict stores,
poison and the switch; `tests/value_record.rs` pins the admissions.  With it the runtime
half of the queue's item 3: `link_siblings` no longer clones the parent's field list per
element append (both backends).  `LOFT_TRACE_VALUEREC` now names the refusing test.
Consumer lane after both (`compare.py --skip-interp --repeat 3 --n-ref 500 --n-native
500`, 14/14 hashes agree, this box): `parse` **47 528 ns/op** (≈ 6.6× against the
reference's 7 220), `render_lock` 14.90 ms, `render_marks` 6.69 ms, `resize` 93.3 ms —
the three resample rows flat — and the ten judged rows all within the bar (`hash` 1.12×,
`fill_circle` 1.18×, `fill_star` 1.19×, `hair` 1.92×, `lock` 1.92×, `composite` 2.00×,
`lock_curved` 2.18×, `wide_line` 2.31×, `smooth` 3.13×, `fronds` 3.32×).
**The release-pass ceiling measured 2026-09-15** (`LOFT_RELEASE_PASS_PROBE=1`, a
measurement instrument — PERFORMANCE.md § *The release-pass ceiling* has the table): the
three resample rows are bound by the checks (`resize` 5.36× → 2.17×, `render_marks`
7.57× → 3.69×, `render_lock` 5.20× → 2.98×, hashes agreeing); every other row's ceiling
sits within 3–15 % of its checked build, so `parse`, `fronds`, `smooth` and the locks are
bound by the store lifecycle and the per-record path, not by arithmetic.  The queue in
§ Where to resume follows that split.
Scoreboard vs the issue baseline, consumer lane on the SHIPPED tier (lean, fully
optimised — the release default since 2026-09-08, DESIGN.md § The shipped tier):
`hash` 10.9× → **2.2–2.5×** consumer / 1.2× gate row (under the bar; the spread
is the reference lane's swing); `hair` 4.3× → **2.0×** (under the bar); `lock`
30× → **4.4×** (2.7× gate row); `smooth` 262× → **14×**; `fronds` 49× →
**13.0×**; `composite` 26× → **6.3×**; the fills 17× → **3.8×** (under the bar);
`wide_line` 17× → 5.6×; `lock_curved` 11.9× → **5.7×**; `composite` 26× →
**4.4×** after § V-n + § V-o, **2.3×** after § V-p; `lock` **3.5×** and `lock_curved` **3.8×** after
§ V-q (2026-09-09, both under the bar).

**Which MACHINE a row was measured on is part of the row.**  The tables below through
2026-09-12 are the **Apple** lane (host `firewall02`, arm64 Darwin); the two directly
under this paragraph are **x86-64 Linux** (the 2026-09-14 one on host `tuxedo`; the
2026-09-12 one's host unrecorded — it read "this dev box",
which resolves to nothing for a later reader on another machine).  The two do not compare row-for-row and neither is wrong: the
bar is *within 4× of the Rust reference on the same machine*, and which routines clear
it differs by target.  Label every future row with its machine.

**Re-measured 2026-09-14 on x86-64 Linux (host `laptop`, the quiet perf box), tip 011687d9 —
the ten rows loft#1426 FILED are all under the bar, and the issue takes its `Fixes`
trailer with this commit.**  Release binary rebuilt at the tip, fresh scratch clone of
`drawing-lock` (250b2cd), `compare.py --skip-interp --repeat 3 --n-ref 500 --n-native 500`,
all 14 hashes agreeing: `hash` **0.90×** (110 672 ns/op native / 122 986 reference),
`fill_circle` **1.15×** (62 126), `fill_star` **1.15×** (22 386), `hair` **1.91×** (26 850),
`lock` **1.93×** (2 976 728), `composite` **2.00×** (202 214), `lock_curved` **2.23×**
(2 945 454), `wide_line` **2.25×** (13 208), `smooth` **3.26×** (1 140 / 350), `fronds`
**3.27×** (161 390 / 49 388) — against the filed 10.9 / 17 / 17 / 4.3 / 30 / 26 / 34 / 17 /
262 / 49×.  The issue is the surfaced report of these ten on this lane and closes on merge;
what stays the plan's is the four rows the bench grew since (`parse` 10.45×, `render_lock`
5.67×, `render_marks` 8.40×, `resize` 5.66× — three of them the graphics package's
resample, § *The four unjudged rows*) and the aarch64 re-measure (`smooth` read 8.44× there
before § V-ah, which took this lane from 9.5× to 3.3×).

**Re-measured 2026-09-14 on x86-64 Linux (host `tuxedo`), tip 36552c16** (the § V-ag tip
plus two commits that change no default: § V-ah stage 1 is opt-in, the last is docs) —
release lib + binary rebuilt, fresh scratch clone of `drawing-lock` (250b2cd),
`compare.py --skip-interp --repeat 5` run THREE times, each lane's fastest row kept, all
14 hashes agreeing every run: **eight of the ten judged rows under the bar** —
`fill_circle` **1.29×** (43 220 ns/op native), `fill_star` **1.51×** (19 580), `hair`
**1.73×** (21 060), `hash` **2.05×** (235 740), `wide_line` **2.36×** (10 200), `composite`
**2.49×** (151 440), `lock` **3.13×** (2 859 640), `lock_curved` **3.35×** (2 505 880) —
and two over: **`fronds` 4.28×** (175 920 / 41 080) and **`smooth` 9.50×** (1 900 / 200).
Against the 09-12 table below: `wide_line` and both fills crossed under (§ V-ad + § V-ae
landing on this lane); `fronds` 318 300 → 175 920 ns/op (−45 %, § V-ag) yet still OVER
here where the aarch64 lane reads 3.42× — the reference is faster on this box (41k vs
~46k) and native slower (176k vs 156k); the run-down is below.  Two rows are lane noise:
the `fronds` ratio spanned 2.97–4.75× over the three runs because the REFERENCE swung
41k–59k while native held 176k–195k, and `smooth`'s reference reads 200–300 ns at `--n 50`,
under its floor (§ `smooth` run down: the converged x86 figure is ≈9.6×).  ⚠ `lock`,
`lock_curved` and `composite` read 0.15–0.4× ABOVE their 09-12 ratios: native absolutes
held within 1 % across today's runs, but the 09-12 table recorded no absolutes for them
and its tip is not in this repo, so lane drift and a small regression are not yet told
apart — an A/B of the two tips on one box, cdylib cache cleared, is the check.  `hash`'s
NATIVE lane swung 235k–464k between runs (the 09-12 note put that row's spread on the
reference lane); min-of-min is what the row reads.

**`fronds` on x86-64 run down (2026-09-14, host `tuxedo`, the § V-ag tip).**  Not a
regression: the row is −45 % since the 09-12 x86 table, and § V-ag is active here
(`fr_only.loft --n 20000`, switch A/B on one binary: 256 351 → 174 239 ns/op, −32 %, hash
`ebcfd875`).  It is over the bar on this lane because the reference is ~10 % faster than
on aarch64 and native ~13 % slower, and the profile (`scripts/profile.sh --engine`, 20 000
calls) says where native's time is: store machinery ≈ 60 % of the run, the program's own
arithmetic ≈ 25 % (`n_fronds` 15.7, `__sin_fma` + `__sincos_fma` 6.1, `n_pt` 1.6, `n_hash01`
1.3).  **The free tree is still 20 % of the row after § V-ag — now on the CLAIM side**:
`claim → fl_take_ge → claim_block → fl_delete_node / fl_insert / fl_balance`, reached from
`pre_alloc_vector` for every `fd_pts` / `fd_wid` (1 296 claims per call), while the release
side is under 1 % (`fl_delete_node` 2.2 is the take, not a free).  Three one-axis probes
(depth 1, so no sub-call; the same 648 fronds) put the cause on the RESULT vector's growth
ladder.  `bump_tail` fires only while the free tree is ONE block; each growth step of
`fd_out` (`Store::resize` claims, copies and deletes, because the per-frond claims sit
right behind it: 11 → 24 → 50 → 102 → 206 → 414 → 830, six moves) frees the old block into
the store; once the freed blocks are large enough to fit a per-frond claim, `fl_find_ge`'s
best fit prefers them to the tail, and every claim in the rest of the call is a tree take,
a remainder insert and a rebalance.  Measured: free-tree share **3.0 % at 11 fronds (no
growth), 1.3 % at 22 (one step), 21.1 % at 648**; per-frond cost 217 / 212 / 202 / 201 /
241 / 251 ns at 11 / 22 / 50 / 100 / 300 / 648.  Recursion is not it (depth 1 keeps the
21 %).  **The reset re-establishes the vector at capacity 11** (`clear_vector_release` →
`pre_alloc_vector(db, 0, …)`, `count.max(11)`) although the buffer is REUSED across calls
(§ V-u) and the length it reached is read in the walk arm beside it — so every call after
the first re-runs the whole ladder inside a store that already has the extent.  Candidate
unit, unbuilt: re-establish the root vector at the capacity the previous fill reached (the
store's extent is the receipt), so the ladder runs once per buffer and `bump_tail` serves
every claim after it; ceiling from the probes ≈ −20 % of the row on this box (~176k →
~141k, ≈ 3.4×, under the bar here), and it is 3e's shape exactly — a fact the allocator
cannot have (this store is a reused return buffer) spent by loft.  The tail-bump rule is
the second question: a claim no hole fits still walks the tree to reach the tail, and a
tail fast path that survives holes would price the sub-call buffers' holes too.
**BUILT the same day as § V-ai: `fronds` 175 920 → 143 180 ns/op on the consumer table,
4.28× → 3.42× — under the bar here.**

**Re-measured 2026-09-12 on x86-64 Linux, the same § V-z tip (d3c31d82, doc-only over
d80307b0)** — rebuilt release lib + binary, fresh scratch clone of `drawing-lock`,
`compare.py --skip-interp --repeat 5` run twice, every row within 1 % of itself and all
14 hashes agreeing: **seven of the ten judged rows under the bar** — `hair` **2.03×**,
`hash` **2.24×**, `composite` **2.34×**, `fill_circle` **2.66×**, `fill_star` **2.71×**,
`lock` **2.86×**, `lock_curved` **2.98×** — and three over: `wide_line` **4.11×**,
`fronds` **6.20×** (318 300 / 322 460 ns/op native), `smooth` **8.00–8.39×**.  Note how
far the two lanes disagree on WHICH rows fail: `lock_curved` and both fills clear the
bar here and miss it on Apple, while `smooth` clears it there and is the worst row here.

The three rows that fail here were profiled (`scripts/profile.sh --engine`, a scratch-only
`--only <routine>` switch on the clone's `bench.loft`), and they fail for three DIFFERENT
reasons:

- **`fronds` — the allocation class, unchanged.**  `claim` 9.1 %, the free-list tree
  (`fl_set_red`/`fl_delete_node`/`fl_balance`/`fl_set_right`/`fl_flip_colors`) 14.8 %,
  `vector_append`+`vector_finish` 6.9 %, `record_new`/`record_finish` 3.2 %, `memset`
  3.6 % — against `n_fronds` itself at 8.4 %.  That is the ~39 % allocator+free-tree the
  09-11 profile named, so queue items 4b / 5 / the move still rank first for this row.
- **`smooth` — a record-returning CALL per output point.**  ⚠ An earlier revision of
  this section read the profile as *per-call store lifecycle*; that was WRONG and the
  size sweep below falsified it.  The gap is per-POINT: `raster::pt(x, y)` returns a
  `Pt` (two floats, no heap) and is called once per output point, where Rust inlines the
  same constructor to nothing.  `n_pt` is **18.1 %** of the loop's self time in the call
  form and **absent entirely** from the literal form.  Full working below.
- **`wide_line` — code quality in one raster loop, plus libm.**  `n_polygon_generic`
  alone is 64.1 % (real user code), and what surrounds it is small and concrete:
  `floor`+`ceil` as out-of-line libm calls 7.3 %, `n_round_down`+`n_round_up` not
  inlined 4.7 %, `get_vector`+`vec_get_or_raise_runtime` 7.0 % of unhoisted element
  reads.  Only ~7 % is store machinery, so this row is NOT the allocation class.

### `smooth` run down — the size sweep, and what the ratio actually is

Four measurements, in the order that each falsified the previous reading.  A
scratch-only `--only <routine>` switch was added to BOTH `bench.loft` and `bench.rs`
in the clone so one row could be swept alone; every cell's hash agrees across the
two lanes.

**1. `--n 50` does not measure `smooth` at all.**  The row is 360 ns/op in the full
table and **1,360 ns/op run alone** — 3.8× apart for the same work.  At `--n 50` the
reference lane runs for 18 µs *total*, so what it measures is the CPU's frequency
ramp: this box idles cores at 400 MHz and reaches 3.28 GHz, and `bench_hash` +
`bench_hair` ahead of it in the full table are what warm it up.  `smooth` is the
shortest row on the board by three orders of magnitude (`lock` runs 78 ms at the same
`--n`), which is why it and no other row is dominated by this.  **Any row whose lane
runs under ~100 ms is reporting the clock it was measured at.**  Swept to convergence
(`--n` 100 000–500 000): rust **281–285**, loft-native **2,694–2,707** ns/op — so the
honest x86 ratio is **≈9.6×**, and compare.py's 8.0–8.4× was flattering it.

**2. The reference is not hollowed out.**  The bench folds `.len()` into its sink, and
`smooth_pts`' output length is purely structural — so LLVM is free to delete every
float in the loop and keep the count.  Probed by rebuilding the reference with the sink
consuming one produced coordinate, then all of them: **281 → 282 → 333** ns/op.  No
elimination; the 281 ns is real work, and the comparison is fair.

**3. The gap is per-POINT, not per-call.**  Swept the input ring from 3 to 192 points
(output 31 → 1 921) with `n` scaled so every cell runs long:

| input pts | output pts | rust ns/op | loft ns/op | ratio |
|---|---|---|---|---|
| 3 | 31 | 160 | 1 601 | 10.01× |
| 6 | 61 | 308 | 2 878 | 9.34× |
| 12 | 121 | 529 | 5 357 | 10.13× |
| 24 | 241 | 1 083 | 10 547 | 9.74× |
| 48 | 481 | 2 152 | 20 943 | 9.73× |
| 96 | 961 | 3 797 | 41 889 | 11.03× |
| 192 | 1 921 | 7 686 | 84 457 | 10.99× |

Flat across a 64× size range.  Taking the slope off the top two rows: **rust 4.05,
loft 44.3 ns per output point**, with loft's fixed per-call cost ~173 ns against rust's
~61 ns — an excess of ~112 ns, under **4 %** of the 61-point call.  A per-call class
would have collapsed this ratio as the input grew; it does not move.  The profile at
np=192 is also within a point of the np=6 profile on every symbol, which says the same
thing a second way.

**4. What the per-point cost IS.**  `raster::pt(x, y)` is a function returning a
no-heap two-float record, called once per output point inside the innermost loop
(`sp_out += [raster::pt(...)]`), plus twice per segment through `half_chord`.  Two
copies of the same loop differing ONLY in that append — a call versus a
`raster::Pt{..}` literal, same hash — cost:

| | np=6 (out 61) | np=192 (out 1 921) |
|---|---|---|
| `raster::pt(..)` call | 1 586 ns | 53 684 ns |
| `Pt{..}` literal | 1 148 ns | 34 867 ns |
| saved | **27.6 %** | **35.1 %** (9.8 ns/point) |

and `n_pt` is 18.1 % of the call form's self time, absent from the literal's.

**The unit this names: § V-t's in-place record push does not admit a record-RETURNING
call.**  With `LOFT_NO_RECORD_PUSH=1` the two forms converge — 75 121 vs 72 755 ns, 3 %
apart — so the literal's advantage IS the push, not the arithmetic.  With it on they
are 49 955 vs 35 055.  The literal is built directly in the push header's next slot;
the call still mints its result somewhere and copies it in, so it collects the
header half of the optimisation and not the build-in-place half.  Extending the
callee-writes-into-the-slot rewrite (the § V-p twin / § V-u adoption idea, applied to a
scalar record return) is worth ~30 % of this loop.  It would NOT close the row on its
own: the literal form is still 18.1 ns/point against rust's 4.05.

### `fronds` run down — TWO memory leaks, and § V-j is currently a net negative

Run down the same way as `smooth`, on x86-64.  The reference lane was cleared first
(`.len()` sink → consume one produced coordinate → consume all of them: **48 812 →
48 518 → 51 882** ns/op, so no dead-code elimination and the comparison is fair), and
at 16 ms per timing the row is clear of the frequency ramp that contaminates `smooth`.

**1. The measured ratio was wrong, in the honest direction.**  At `--n 5000`,
median-of-10: loft **421 264** vs rust **50 126** ns/op — **8.4×**, not the 6.20× the
`--n 50` table reports.  Both lanes carry ~12–15 % run-to-run spread on this box, so
single runs at either `n` are not decisive; the medians are.

**2. `fronds` LEAKS on `--native-release`: ~357 KB per call, unbounded.**  Max RSS is
linear in the iteration count — 348 MB at n=1 000, 611 MB at 2 000, 1.42 GB at 4 000,
3.30 GB at 8 000, and **7.7 GB at 20 000** on a 14 GiB box.  This is the row a consumer
calls once per frame.

**3. The native leak is a COMPOSITION defect between two shipped units of this plan.**
Each is sound alone; together they leak:

| config | KB leaked per call |
|---|---|
| baseline (§ V-u and § V-j both on) | **356.7** |
| `LOFT_NO_RETBUF_ADOPT=1` (§ V-u off) | 0.5 |
| `LOFT_NO_MOVE_APPEND=1` (§ V-j off) | 0.2 |
| both off | −1.0 |

`fronds` is exactly the shape that meets both: it is self-recursive, its result local
adopts the return buffer (§ V-u shape A), and the recursion appends a dying temporary's
elements — `for f in fronds(..) { fd_out += [f] }` — 24 times per call (§ V-j).  The
switch documentation already anticipated this failure mode: CLAUDE.md gives
`LOFT_NO_MOVE_APPEND=1` as the bisect step for *"a wrong element, a leak or a double
free out of a loop that appends a dying temporary's elements."*

**4. What the leak costs in TIME — § V-j is currently a net negative.**  The store's
free structure degrades as the footprint grows, so the cost is not only memory.  Timing
consecutive blocks of 50 calls in one process, after a non-allocating CPU warm-up, gives
a steady state of ~257 k ns/op punctuated by deterministic spikes at blocks **0, 2, 5,
12, 29** — each interval ~2.4× the last, each spike bigger: 454 k, 568 k, 929 k,
1 938 k, **5 219 k** ns/op.  The last is **20× the steady state**: geometric growth with
a full copy, which in a game is a frame-time hitch that gets worse the longer the
process runs.  Turning the leak off is therefore FASTER, at n=5 000:

| config | ns/op | leaks |
|---|---|---|
| baseline | 387 953 | yes |
| `LOFT_NO_RETBUF_ADOPT=1` | 372 349 (−4 %) | no |
| `LOFT_NO_MOVE_APPEND=1` | **328 637 (−15 %)** | no |

**So the first `fronds` improvement is not a new optimisation — it is closing the leak**
(point 5 corrects WHERE: not in § V-j, which is sound, but in the return buffer's reuse
contract, which § V-j and § V-u each keep alive longer).  It is worth −15 % and 357 KB/call before
any new work is ranked, and it moves the row from 8.4× to ~7.1×.

**5. RUN DOWN FURTHER — it is ONE leak, it is NOT § V-j's, and § V-j is not defective.**
The switch A/B in point 3 localises the fronds growth correctly but attributes it
wrongly.  Four probes settle it (`leak/adopt.loft`, `leak/plain.loft`, kept beside this
plan):

| probe | element owns heap | adopted? | native | interpret |
|---|---|---|---|---|
| `plain.loft` — a § V-j move-append loop, no-heap element | no | — | **0.03 KB/call** | 1.2 |
| `adopt.loft` — a plain vector-returning fn, heap-owning element | yes | **no** (`LOFT_TRACE_ADOPT`: `bad=true`) | **213 KB/call** | 128 |

A § V-j move-append with a NO-HEAP element does not leak at all, so **the move-append
has no defect of its own**.  And `adopt.loft` leaks on both backends with § V-u and
§ V-j BOTH OFF, and with adoption declined outright — so the leak is neither unit's, and
it is not a @PLN157 regression.

**The root defect: a shape-A hidden return buffer is REUSED across calls, and its entry
clear is a length reset.**  `vector::clear_vector` (`src/vector.rs:527`) does one thing —
`store.set_u32_raw(v_rec, 4, 0)` — under its own standing TODOs (*"Only set size of the
vector to 0 … TODO … lower string reference counts where needed"*).  That is sound
wherever the buffer's store is freed straight afterwards, which is every use it was
written for.  It is NOT sound for the shape-A return buffer, which by ABI survives across
calls: every call therefore strands the previous call's element-owned heap in a store
that never dies.  The axis is the ELEMENT TYPE — a vector of scalars or of no-heap
structs is fine; a vector whose elements own vectors, text or records leaks the lot.
`main_vector<Frond>` is the type the store-heap ceiling names when `fronds` is driven
under `loft test` (307.7 MiB → 718.0 MiB, *"one type holding nearly all of it in ONE
store is a runaway length"*).

**What § V-u and § V-j actually do here is make the store survive LONGER, so they widen
an existing hole rather than open one.**  That is why disabling either takes fronds'
growth to ~0 while neither touches `adopt.loft`: with them off the buffer's store dies
per call and takes the strand with it.  The 357 KB/call on native and 92 KB/call on the
interpreter are the same defect reached through different lifetimes.

**So "repair § V-j" is the wrong target and was not done.**  The repair belongs at the
buffer's reuse contract, and it is a real design call with a measured cost, because the
release is work the program currently never does:

- **(a) deep-release at the entry clear** — correct everywhere, and the honest cost:
  `fronds` gets slower by whatever the release costs, which today is simply missing;
- **(b) free and reallocate the buffer's store per call** — gives up § V-u's
  across-calls backing reuse, which is most of what § V-u bought;
- **(c) decline buffer REUSE when the element type owns heap** — narrow, keeps the
  optimisation for scalar vectors, and leaves the general contract unchanged.

(c) is the smallest change that closes the class without paying (a)'s cost on the rows
that do not need it, but it needs the element-owns-heap predicate to be exact — an
under-approximation there is a leak, not a slowdown.  Owner's call; nothing has been
edited.

**Reduction status — honest:** `leak2.loft` (kept beside this plan) reproduces *a* leak
in all three shapes (move-append once / in a loop / self-recursive) on both backends,
but at 4.6–10.7 KB per call, and the switches do NOT clear it there.  So it is the
interpreter-side leak, not a faithful reduction of the native composition defect.  The
clean attribution above stands on `fronds` itself; a minimal native repro is still owed.

**What remains after the leaks, for ranking.**  The size sweep (count 6→96 at depth 2,
24→384 at depth 1; the `fr_*` cells) puts the per-frond cost at **~550–670 ns against
rust's ~70–80** with no fixed-cost term worth naming — the same per-element shape as
`smooth`.  The allocation profile (claim 9.1 %, free-list tree 14.8 %, vector
append/finish 6.9 %) is real but should be RE-TAKEN once the leaks are closed: it was
measured on a run whose store was growing without bound, so the free-tree's share is
inflated by the defect rather than by the steady-state algorithm.

**The three newest units were A/B'd on this box and all three pay here too** (each
variant a fresh `bench/.loft` — the program cache is keyed on the SOURCE, so an
env-gated emitter change is otherwise served the previous variant's binary; best of 3,
`--n 50`).  On `fronds`: base **322.9k** ns/op, `LOFT_NO_ELEMENT_FIRST=1` **488.8k**
(§ V-z is worth −34 %), `LOFT_NO_LITERAL_HOIST=1` **357.2k** (§ V-x −9.6 %),
`LOFT_NO_COMPLETE_WRITE=1` **332.7k** (§ V-y −2.9 %).  ⚠ **Read only the row the switch
targets.**  Each variant is a different binary, and code layout alone moved `hash`
between 246k and 436k across the four — wider than any of the effects above — so an
untargeted row in a switch A/B measures layout, not the switch.

**`floor`/`ceil` are a baseline-target artefact, and the note in `src/main.rs` that
`-C target-cpu=native` "moved nothing" is an APPLE measurement.**  Probed on this box
(scratch `fl.rs`, rustc 1.97): at plain `-O` `x.floor()`/`x.ceil()` emit PLT calls into
libm; at `-C target-cpu=x86-64-v2` each becomes one `roundsd`.  On aarch64 `frintm`/
`frintp` are baseline, so the flag genuinely cannot move that row there — which is
exactly why the earlier measurement found nothing.  Re-measuring the shipped tier's
flags on x86 is therefore an open, cheap unit worth ~7 % of `wide_line`; it is not yet
done, and raising the baseline is a portability decision, not just a perf one.

**§ V-aa SHIPPED — `lock_curved`'s named target is closed** (the record-returning call
per pixel): the value-record return is default-on, measured 1.65× on the call, `smooth`
−25 % on the standalone probe, `lock_curved` −3 %, with the cdylib bridge taught to
materialise so library functions qualify.  ⚠ The two row figures are from DIFFERENT
harnesses: `lock_curved`'s −3 % is the consumer lane and re-measured there as −2.9 %;
`smooth`'s −25 % is the standalone `vr_smooth.loft` probe and does NOT appear on the
consumer lane, where the same A/B moves it −0.9 % (2026-09-12 HEAD block above).  The profile that named it:

**`lock_curved` IS NOT AN ALLOCATION ROW — profiled 2026-09-12, and it names ONE target.**
77.9 % of samples are the program's own code (`n_lock_layer` 77.6 %), allocator 6.8 %.
The resolve loop is fully hoisted already (8 headers, 7 invariant scalars, 7 hoisted
element reads); the ONLY per-pixel store traffic left is `brush_sample` returning a
four-float RECORD — four writes into the return buffer, four reads back.  Measured
against the tuple return that carries the same four values: **2.1–2.4×** (29.5–33.0 ms vs
13.8–14.1 ms per 3 M calls), ~10–15 % of the row, and the SAME class the x86 lane named as
`smooth`'s cause (27–35 % there) — **one unit moves both**.  The mechanism exists for
tuples (`-> (f64, f64, f64, f64)`, in registers, no buffer); a no-heap record return
should ride it.  Scoped in four steps with its blocker (the live-dispatch fallback returns
`DbRef`) in [DESIGN.md § lock_curved profiled](DESIGN.md).  Two hypotheses killed there
too: the per-call bookkeeping costs nothing (rustc inlines same-crate calls — an earlier
"1.3 ns/call" claim from this session is retracted), and cross-library loft calls inline
normally (only a library's Rust parts are dylibs).

**ZERO-ON-CLAIM: 29 DEPENDENT → 6, AND NATIVE NEEDS IT NOT AT ALL (owner's ruling,
2026-09-12): fix the callers that rely on zeros rather than paying the memset on every
claim.**  First fix landed — both `OpDatabase` paths now initialise the record they mint
(once per store CREATION), which retired the dominant class: `clear_vector` asking a
store-root wrapper's unwritten payload whether a vector is there.  `--native` is CLEAN
across the whole corpus under the falsifier, so its flip needs nobody's permission but a
measurement; the interpreter's last six are named with their readers in DESIGN.md and
ratcheted by `tests/poison_claim.rs` (the list may only shrink).  Prize when they go:
246.2–246.8k against 250.8–253.0k on `fronds`.**  The falsifier this needed now exists — `LOFT_POISON_CLAIM=1` fills a fresh claim
with `0xDEADBEEF` (the claim-side twin of poison-on-free), so a caller relying on zero-init
fails loudly instead of inheriting recycled bytes that look like zeros.  Census on the
interpreter over every `tests/scripts`: **1 232 clean, 29 dependent**, and all seven
@PLN157 cell corpora clean on BOTH backends.  The 29 are one family — return buffers, NRVO
aliasing, fn-ref delivery, loop-local buffer lifetime — and the class is an unwritten
COLLECTION HANDLE read as a record id (first instance: `vector_replace` reading a store-root
wrapper nothing had written; both `OpDatabase` paths prefill, so the producer is some other
route and naming it is step 1).  Worth ~4 % standalone, and the § V-y matrix already showed
zeroing and the typed prefill are redundant with each other — the prefill is the one that
must survive, because zero is the wrong null for nullable integers (`i64::MIN`), booleans
(`255`) and non-null `text` (an interned empty).  Full plan, in four falsifiable steps:
[DESIGN.md § Zero-on-claim](DESIGN.md).  **Check first, it may be free:** the comment on
`zero_claim_enabled()` claims the hazard is interpreter-only while the zeroing runs on both
backends — if that holds, `--native` can stop zeroing today.

**THE STORE RESET WAS RETRIED ON THE FIXED TREE AND MEASURED AWAY (2026-09-12).**  With
the zero-on-claim fix in place it runs CLEAN — 42 corpus runs (7 corpora × 2 backends × 3
levers), guards, leak test, 1 116 lib tests — because the blocker was never a live alias:
`record 1.12` was the wrapper's un-initialised slack.  But sound, it buys nothing: 0.84–
0.87 s against the element walk's 0.78–0.94 s on a qualifying shape, and on `fronds` it
does not apply at all (§ V-j places a record in that store, so `hosts_placed` declines).
**The −36 % came from the unsound configuration** — the first probe had no gate, so
`fronds` took a reset it was not entitled to.  Reverted; do not rebuild it.  Details and
the analysis that survives: [DESIGN.md § The store reset, retried](DESIGN.md).

**The design as it was written (kept for the analysis, not as a plan):** the reset
cannot be decided inside `clear_vector` — a runtime op cannot see whether a reference into
the store is live — but the CALLER'S FRAME can, and § V-u already emits the buffer's init
there.  The reset form is that site's `if` branch verbatim (`OpDatabase`'s reuse arm IS
`clear` + re-claim), so the runtime needs NOTHING new: the unit is an analysis plus a
one-line emission choice, with four per-call-site obligations (the previous result is dead;
no live `&`-view of it or its elements; nothing § V-j-placed in that store; no escape).
Six cells listed, two of which already exist as today's failures.
[DESIGN.md § Using the store reset reliably](DESIGN.md).

**WHY THE RETURN BUFFER ACCUMULATED, AND THE 36 % THE RIGHT FIX IS WORTH (owner's
question, 2026-09-12 — measured, then found unsound at this layer).**

*The mechanic.*  A vector-returning function's hidden `__retbuf` store is minted once and
reused for every call.  The elements' OWN heap — `Heavy.hv`, `Frond.fpts` — is claimed
INSIDE that same store, because a field vector is claimed in its owner's store.  The
entry clear then resets only the outer length word.  So call 1 leaves 50 element records
plus 50 inner vector blocks claimed; call 2 appends new elements whose inner vectors
claim NEW blocks beside them, and the only route to the old ones (the outer length) was
just zeroed.  Linear growth per call.

*The owner's proposal — reset the whole store instead of walking the elements — is
right about the prize and was measured:* replacing the per-element release with
`clear` + re-claim of the root wrapper puts `fronds` at **160–167k ns/op** against the
walk's 250k and even against the LEAKING build's 186–200k, hash exact, still flat at
13 MB.  −36 %, because resetting the store also erases the free/claim churn its contents
generate (the 39.7 % allocator class, for this workload, at a stroke).

*Why it cannot ship at this layer, twice measured:* the store-root vector's store is
exclusively its own for ALLOCATION, but not for REFERENCES.
1. § V-j deliberately places the callee's buffer as a record in the destination's store,
   held by a variable in another frame; a reset frees it under that variable.  (Fixable:
   a `hosts_placed` flag set in `place_record_in` — built, and it removed the native
   failure.)
2. The one that settles it: a live VIEW into the cleared vector, or a caller's earlier
   result that § V-u aliases to the same buffer.  The length reset leaves those records
   intact (a stale view reads stale-but-valid data); a store reset FREES them, and the
   V-j corpus on `--interpret` under `LOFT_NO_ZERO_CLAIM=1` says so out loud
   (`the vector handle in record 1.12 points at record 3 … has been freed`).
`clear_vector` is a runtime op with no way to know whether a reference into the store is
live — that is the ownership question, and it is decidable at GENERATION time.

*So the unit this names* (worth −36 % on `fronds`, and it retires most of the allocator
class without touching the allocator): emit the store-reset form of the entry clear only
for a buffer the analysis proves has no live alias — no `&`-view of it or of its
elements outstanding, no adopted result local read after the next call, no placement in
its store.  Cells before code, and the two failures above are already its first two
cells.  The owner's second idea — REUSE the element records and their inner vector
allocations instead of freeing and re-claiming them — is the same prize reached from the
other side, and it needs no alias proof: nothing is freed, so nothing can dangle.  That
is probably the better unit to build first.

**THE ALLOCATOR IS THE NEXT UNIT, AND A FIRST ATTEMPT WAS REVERTED (2026-09-12) —
read this before starting it again.**  Profiled on the post-leak-fix build (8 s sample,
1 626 in-binary samples, `vr_fronds --n 40000`): the free-list TREE is **39.7 %** of the
row (`fl_set_red` 185 the hottest single symbol, `fl_balance` 115, insert/delete/rotate
the rest), `claim`/`delete`/alloc another **12.4 %**, against the program's own
**11.8 %** — the allocator is 52 % of `fronds` and 4.4× the program.  The x86 lane
measured the same class independently (fl-tree 14.8 % + claim 9.1 %), and closing the
leak RAISED it, because blocks now actually get returned.

The design that fits: a **size-class recycler** in front of the tree — per-size LIFO
lists of freed blocks (loft recycles a handful of uniform sizes: growth-doubled vectors
and same-shaped records), so a free is a push and a claim of that size is a pop, with
the tree kept for large or odd sizes and as the fallback.  `claim` today is
`bump_tail → fl_take_ge → coalesce → grow`; the recycler slots in before `bump_tail`.

**What the reverted attempt established (the next attempt starts here, not from
scratch):**
1. **Membership cannot live inside the block.**  A one-word free block's footer is
   `-1` = `0xFFFF_FFFF`, so every in-block sentinel collides with one, and a walker that
   reads a footer as a marker corrupts the heap (measured: a footer read back as a tree
   link).  Use an out-of-band position bitset — the store already has `Claims` for it.
2. **Parked blocks must stay FREE-LOOKING** (negative header + footer).  Making them
   read as claimed is safe for every walker but blinds the use-after-free detection the
   store's safety story depends on, and makes `usage`, the store-heap ceiling and
   `release_resident` all lie.
3. **Five sites must flush the recycler first**, because each reads or reshapes the heap
   as a whole: `coalesce_free`, `claim_at`, `reclaim_tail`, `fl_rebuild`,
   `release_resident`, plus `resize` and `claim_scan` — the last two because
   `claim_grow` EXTENDS THE STORE'S LAST BLOCK IN PLACE (the `radix_tree` r7 crash).
4. **`Store::init` must drop the recycler**: a reused slot's parked positions are fresh
   data a moment later.
5. **A recording store must not park** — @PLN16.J is position-addressed replay and the
   recycler changes which block a claim returns.
6. **Still unresolved at revert**: a tree corruption inside `fl_find_ge` on `fronds`
   that none of the above explained, with a double-park guard and a disturbance detector
   both silent.  The next attempt should build the invariant checker FIRST (a release-mode
   `fl_validate` + a parked-set audit callable after every claim/delete under an env
   var), because the failure surfaces far from its cause.

The unit was reverted rather than landed half-verified: it touches the heap every
program shares, and the session's evidence did not reach "provably sound".  The
measurement it exists to earn was never taken — so its ceiling is still the 39.7 %
Amdahl bound above, not a number.

**THE LEAK IS CLOSED (2026-09-12, `@FR-H-ClearRelease`) — and it re-prices the § V-z
row.**  The x86 lane's run-down attributed `fronds`' unbounded ~357 KB/call growth to
`vector::clear_vector` being a LENGTH RESET while a shape-A return buffer outlives its
call (not to § V-j or § V-u, which only make the store live longer).  Fixed at the one
chokepoint both backends call — `Stores::clear_vector_release`, routed from the native
emitter's two arms, the `#rust` template and § V-u's reuse init — gated on the SHAPE
(the store root is a one-field `main_vector<T>` wrapper, what `OpDatabase` mints) and
the ELEMENT TYPE (`owns_heap`, read from the layout, so no predicate can drift).
Measured: the `adopt` probe 60/134/310/848 MB at n=400/800/1600/3200 → **flat 12–13 MB**;
`fronds` 3.3 GB at n=8000 → **13 MB**; the interpreter's separately-reported ~92 KB/call
leak closes with it (one defect, two lifetimes); `plain` (no-heap elements) unchanged,
switch A/B identical.  **The honest price: `fronds` standalone is 250–255k ns/op, not
the 186–200k § V-z reported — that number was taken on a LEAKING build**, exactly as the
x86 hand-off warned; releasing 1 296 inner vectors per call is work the leak skipped.
Rejected alternatives, with reasons: declining buffer reuse for heap elements pays the
same release AND loses the backing; a whole-store clear cannot reach inner vectors that
live in other stores.  Guard `tests/clear_release.rs` is self-falsifying — the same
shape passes under a 64 MiB ceiling with the release on and TRIPS it under
`LOFT_NO_CLEAR_RELEASE=1`.  Two findings on the way, both caught by existing gates: a
`pos == 8` gate released on `--native` only (the interpreter's root vector sits at 12 —
ask the SHAPE, never an offset), and a trailing `//` comment in an emitted template ate
the caller's `;` (the cell corpora went red at once).

**THE FILLS AND `wide_line` PROFILED AT LAST (2026-09-12) — and they name TWO units, one
of them language-wide.**  These three rows had never had a dedicated profile; the
checkpoint instrument costs minutes now.  Both rows land on ONE function:

| | `n_pil_hline` | `n_polygon_generic` | `n_edge_x` |
|---|---:|---:|---:|
| `fill_circle` (ticks) | **59.7 %** | 32.4 % | 5.8 % |
| `wide_line` (ticks) | 32.0 % | **50.1 %** | 10.1 % |

`pil_hline`'s whole body is a contiguous fill — `for hl_x in hl_lo..=hl_hi { hl_d[hl_base +
hl_x] = ink }` (`raster.loft:121`) — and the instrument prices it at **six operators per
pixel**: `OpSetInt` + `OpAddInt` for the write, and FOUR more (`OpAddInt`, `OpLtInt`,
`OpConvBoolFromInt`, `OpNot`, 63.5 M each) for the loop itself.

**Unit 1 — the counted loop pays a null test per iteration — SHIPPED 2026-09-13 as § V-ab** (DESIGN.md § V-ab: the numbers, and why the consumer rows moved 2–3 % where the bare loop moved 32 %).  The original finding, for the record:  The emitted form initialises
the loop counter to `i64::MIN` (the null sentinel) and then, EVERY ITERATION, asks whether
it is still null in order to decide whether this is the first one:

```rust
let mut idx: i64 = i64::MIN;
loop {
    idx = if op_conv_bool_from_int(idx) as u8 != 1 { lo } else { op_add_int(idx, 1) };
    if hi < idx { break }
    …
}
```

The idiomatic lowering initialises before the loop and increments at its end, and needs
neither the sentinel nor the test.  Measured on a bare loop, an iteration costs ~0.85 ns
with a trivial body — roughly half the cost of a tight loft loop is the loop machinery.

⚠ **Scope, measured rather than assumed** (an earlier revision of this section claimed
"every `for i in a..b` in the language pays it", which is FALSE).  A ten-cell corpus plus a
three-cell discriminator says the axis is whether the range's START is a compile-time
LITERAL:

| start | sentinel test per iteration |
|---|---|
| `0..n`, `0..=n`, `2..n` — a literal | no |
| `a..b` — a parameter | **yes** |
| `z..n` — a local that HOLDS zero | **yes** |
| `(n - n)..n` — an expression | **yes** |

So it is not every loop; it is every loop whose lower bound is not written as a literal.
That is narrower than claimed and still covers the hot ones exactly: `pil_hline`'s
`hl_lo..=hl_hi` and `raster_segment`'s `rs_y0..(rs_y1 + 1)` / `rs_x0..(rs_x1 + 1)` are all
variable-start.  A `for x in v` over a vector is clean, as are `break`/`continue` bodies
and nesting — the shape alone decides it.  Corpus and the before-capture:
`bytecode-comparisons/loop-start-cells.loft`.

**Unit 2 — the FILL idiom is not recognised.**  `for i in a..=b { v[i] = <invariant> }` is a
bulk fill, and LLVM turns the reference's identical-looking Rust loop into one.  loft's
per-element `vec_set_hoisted_or_raise_runtime` cannot be recognised as contiguous, so it
stays a scalar store per element.  Measured on 204.8 M writes of the same idiom: **loft 1.27
ns/write against Rust 0.10 — 13×.**  Worth most where the fill is the program: 59.7 % of
`fill_circle`.

⚠ Both are COUNT reductions, and § "the remaining gap is operator count" says a count is not
automatically a cost — three probes there removed executions and bought ~2 %.  The
difference is what the removed work IS: those probes removed checks rustc was already
folding, while these remove a per-iteration branch LLVM cannot fold (the sentinel could
legitimately be `i64::MIN`) and a scalar-store loop it cannot vectorise.  Size each by
building it and re-measuring, not by multiplying the counts.

**THE NEXT UNIT ON `lock_curved` IS THE RECORD-FIELD RETURN DELIVERY — measured 6 %,
2026-09-12.**  A function that builds a vector into a local and returns it as a FIELD of a
record copies the whole buffer at the return.  `lock_layer` does exactly this — `ll_out =
[for _i in 0..ll_n { 0 }]` (`brush.loft:476`, one `OpAppendCopy`) filled by the resolve
loop, then `Layer { …, px: ll_out }` (`:509`, `vector_add` → `copy_claims`).  Priced by
deleting the delivery (`px: []`, which the driver's `sink` does not read, so the answer is
unchanged at `122400`): **1.02–1.04 s → 0.96–0.98 s, ~6 %**.

§ V-u already closes this for a function returning a vector DIRECTLY — the result local
adopts the hidden return buffer and the exits deliver nothing.  It does not fire here
because the vector is a FIELD of the returned record, so the local has no buffer to adopt;
§ V-z's element-first build has the right mechanism (build the vector where it must end up)
and the wrong gate (it admits an APPENDED element, not a returned record).  The unit is to
extend one of the two to *a vector local consumed exactly once as a field of the record a
function returns*.  It generalises well beyond this row: **build a buffer, return it
wrapped in a struct** is one of the most common shapes a loft library has.

⚠ **How this was found is the cautionary part.**  The first estimate off the machine
profile was "~13 % in the copy/alloc band" (`copy_claims` 6.3 %, `get_vector` 5.9 %,
`copy_block`/`vector_add`/`store_mut`/`memmove` ~9 %), and the first HYPOTHESIS — that the
seven `Lay` vectors were being deep-copied into the struct literal — was wrong: the emitted
`n_lock_layer` contains **zero** `OpCopyRecord`.  The call tree named `OpAppendCopy`, the
emitted source named the two lines, and deleting the delivery priced it.  Most of that
13 % band is allocation and initialisation the algorithm genuinely needs (7 × 38 250 floats
per call); only the delivery is removable.  Estimate from a band, and you size a unit
wrong in both directions.

**THE REMAINING GAP IS OPERATOR COUNT, NOT OPERATOR COST — four measurements, 2026-09-12,
and it redirects the queue.**  `LOFT_NATIVE_CHECKPOINTS` (PERFORMANCE.md) counts every
operator a native run executes.  On `lock_curved` at n=400: **3 295 449 657 operator
executions in 1.03 s = 0.313 ns each, about 1.25 cycles at 4 GHz.**  An operator is already
at roughly one cycle, so there is almost nothing left to win by making operators CHEAPER —
and three probes, each removing a whole class of per-operator work, say exactly that:

| probe | executions removed / made cheaper | measured |
|---|---|---|
| every integer op's null-sentinel test elided (11 sites in `ops.rs`, throwaway) | 753 M (23 % of all ops) | 1.04 → **1.02 s (~2 %)** |
| `raster_segment`'s seven field reads hand-bound to `&` views | `OpGetField` 295 M (8.9 %) **gone** | 1.04 → **1.06 s (nothing)** |
| the float non-sentinel fast path turned OFF (`LOFT_NO_NN_FAST=1`) | — | 1.02 → 1.04 s (~2 %) |

The second is the sharpest: 295 million operator executions disappeared from the mix and
the program did not get faster.  **So a count is not a cost**, and the same caution applies
to the checkpoint tick column (PERFORMANCE.md § the tick column is biased toward
operator-dense functions).  A micro-optimisation on the per-operator path is worth ~2 % on
this row; the reference runs the same work in 0.12 s, **8.6×** less, so 2 % is not the
shape of the answer.

*What this rules IN.*  The gap is that loft EXECUTES 3.3 billion operators where rustc
executes far fewer machine instructions for identical work — it vectorises, fuses and
strength-reduces across the per-operator boundary that loft's IR makes opaque.  So the
levers that can still pay are the ones that reduce the COUNT, not the price:

- **Fuse a compare with its bool conversion.**  `OpLtFloat` 154 M is followed by
  `OpConvBoolFromFloat` 149 M, and `OpConvBoolFromInt` adds 61 M — ~210 M executions (6.4 %)
  that are pure representation shuffling (`… as u8) == 1`).  A fused compare-and-test op
  removes them outright.
- **Strength-reduce a loop-invariant divide.**  `OpDivFloatNullable` 74 M, and
  `raster_segment`'s is `/ rs_l2` with `rs_l2` invariant across both loops.  ⚠ Not
  automatable behind the author's back: reciprocal-multiply is not bit-identical, and the
  bench validates by hash.  It is an ADVICE-tier diagnostic, not a rewrite.
- Anything that lets a loop body become one vector operation instead of N scalar ones.

*What this rules OUT, with the measurement beside it:* an integer non-sentinel analysis
(the float one already exists and is worth ~2 %), and a record-field-read hoist for
`raster_segment`'s shape — loft#1426's consumer comment *"a struct field read costs a store
lookup per pixel … and nothing hoists it for us"* is true about the COUNT and false about
the cost, so the hand-hoist it asks for buys nothing.  Do not build either for performance.

**Re-measured 2026-09-12 on HEAD (d954a39e + the uncommitted § V-aa tree), Apple lane,
host `firewall02` (arm64 Darwin)** — fresh scratch clone of `drawing-lock`, `compare.py
--skip-interp --repeat 5` five times, all 14 hashes agreeing every run, native columns
within 1.2 %: **four under the bar** — `hash` **2.15–2.29×**, `hair` **2.63–2.73×**,
`lock` **3.26–3.40×**, `composite` **3.54–3.59×** — and six over: `smooth`
**3.89–4.50×**, `fill_star` **4.04–4.33×**, `fill_circle` **4.06–4.11×**, `fronds`
**4.48–4.76×** (217.1–219.8k ns/op), `wide_line` **4.96–5.12×**, `lock_curved`
**5.11–5.45×** (2 229.6–2 255.7k ns/op).

Two rows read worse than the d80307b0 block below, for two unrelated reasons, and
neither is a code regression:

- **`fronds` 4.05× → 4.48–4.76× is `@FR-H-ClearRelease`, and it is the price of
  correctness.**  eeae8a27 landed after d80307b0; the row below was measured on the
  LEAKING build.  `LOFT_NO_CLEAR_RELEASE=1` on HEAD gives **184 680 ns/op / 4.05×**,
  reproducing it to 0.2 % — so the leak fix accounts for 100 % of the delta and nothing
  else moved this row.  **Do not chase it by reverting**; the route that reclaims it is
  the element/allocation REUSE unit (−36 %, 160–167k), which frees nothing and so needs
  no alias proof.
- **`smooth` 3.92× → 3.89–4.50× is the reference lane, not loft.**  Its native column
  here (2 100–2 160 ns/op) is BETTER than the 2 379 best that § V-aa records; rust came
  in at 480–540.  The row's side of the bar is decided by a lane that swings ~1.6×, so
  **`smooth` under the bar is not a reproducible claim** on this machine.

**§ V-aa A/B on the consumer lane** (cache cleared on BOTH sides — the cdylib cache is
keyed by content, so an env-var-only re-run silently re-measures the same binary and
reads as a null result): `lock_curved` **−2.9 %** (2 295.1k → 2 229.6k), confirming the
−3 % claimed.  `smooth` moves **−0.9 %**, not the −25 % recorded — that figure is the
standalone `vr_smooth.loft` probe (~3.2k ns/op), a different harness from this bench row
(~2.1k), and § V-aa's summary line gives the two numbers side by side without saying so.
Every other native column improved or held.

**Re-measured 2026-09-12 on the § V-z tip (d80307b0)** — `compare.py --skip-interp
--repeat 5`, caches cleared, all 14 hashes agreeing: **five under the bar** — `hash`
**2.19×**, `hair` **2.76×**, `lock` **3.64×**, `composite` **3.68×**, `smooth` **3.92×**
— and `fronds` at **4.05×** (184 340 ns/op native, from 6.17× at the 09-11 close: the
two-day arc − sound placement reuse, § V-x, § V-y, § V-z − is −46 % standalone and the
reference lane's swing now decides its side of the bar).  ⚠ **That `fronds` row was
taken on a LEAKING build and is not a target.**  `@FR-H-ClearRelease` (eeae8a27) landed
one commit later and makes the shape-A return buffer's entry clear release what its
elements own, closing a ~357 KB-per-call leak; the walk costs ~18 %.  Proven on ONE
binary rather than by bisect: `LOFT_NO_CLEAR_RELEASE=1` on HEAD reproduces this row at
**184 680 ns/op / 4.05×**, within 0.2 % of the 184 340 here, and the honest post-fix
row is the 2026-09-12 HEAD block above.  Still over: `fill_circle`
**4.36×**, `fill_star` **4.38×**, `wide_line` **5.36×**, `lock_curved` **5.76×** — the
fills and `wide_line` have never had a dedicated profile, and `lock_curved` owes its
with-callers profile; those are the next hand-off items.

**Re-measured 2026-09-11 on the reuse tip (5262db1b)** — `compare.py --skip-interp
--repeat 5`, cdylib caches cleared first, all 14 hashes agreeing: **five rows under the
bar** — `hash` **2.15×**, `hair` **2.60×**, `lock` **3.36×**, `composite` **3.56×**,
`smooth` **3.96×** (under, but its reference lane swung low this run) — and five over:
`fill_star` **4.07×** and `fill_circle` **4.10×** (a hair over), `wide_line` **4.85×**,
`lock_curved` **5.31×**, `fronds` **5.66×** (271 760 ns/op native, −11.6 % vs the prior
table's row: the § V-w / free-footer / sound-reuse arc landed on top of parity).

**Re-measured 2026-09-10 on the tree that JOINED this branch into
`tuxedo-1481-addr-alignment`** (`compare.py --skip-interp --repeat 5`, quiet box, every row's hash
agreeing, `make ci` green 4813/4813 and the in-tree ratio gate under bar): `hash` **2.12×**,
`hair` **1.76×**, `lock` **3.87×**, `composite` **2.65×**, `fill_circle` **3.69×**,
`fill_star` **3.96×** — ten of the fourteen judged rows under the 4.0 bar.  Four remain:
`smooth` **20.9×**, `fronds` **10.27×**, `lock_curved` **4.25×**, `wide_line` **5.23×**.
The scoreboard above reads lower for some rows because it was taken on the branch; these are the
JOINED tree's, and the delta is what carries across a join, never the endpoints.

**The 2026-09-11 arc (§ V-t · § V-j · § V-j's gate · § V-u · § V-v) is commits 2e745655 →
475ef554 on `157-native-4x`; the local full targeted suite is GREEN on each unit's tip
(the § V-v run: 4 707 tests, one real finding — the @FTR-002 worked example pinning the
pre-footer fragmentation story, re-derived and re-taught — plus the two established
cold-cache timeouts, green isolated) and GitHub gate run 34617868644 was dispatched on the tip —
check its verdict before building on it.**

**What remains in `fronds`, RE-VERIFIED 2026-09-11 on the post-gate-fix tip (595e5d98)**
— each number measured, not estimated (ABAB best-of-3, `vr_fronds --n 4000`, hash
`ebcfd875` every run; 10 s `sample` at `--n 60000`):

1. **SHIPPED same day — the host-death form**: the placed record's release rides every
   `OpFreeRef(host_vdb)` site plus the attr's null-guarded backstop, the host's
   `OpDatabase` reuse re-arms the guard (the c21 clause, falsified live at rec 472), and
   an enclosing loop reuses the placement.  Measured: `fronds` standalone 351k →
   **294.9–298.1k ns/op**, statistically identical to the hand mod-A ceiling; 22/22
   cells exact both backends under poison; pins (1,1,2) unchanged, c22 (2,2,4) added.
   The original item, for the record: **Reclaim the loop-exit-free regression (~17 %).**  The gate fix costs
   `fronds` 298–315k → 349–368k ns/op: the `k` loop now pays 24 place/free cycles per top
   call where it paid one, and the fresh profile shows it — allocator + free-tree ~39 %
   (was ~29.5 %), `remove_claims`/`owned_walk` ~6 %.  The sound reclaim: tie the placed
   record's free to the HOST STORE's death instead of the loop exit — emit
   `free_record_in` immediately before every `OpFreeRef(host_vdb)` site (early-death
   sites included; that ordering is the invariant the corruption violated), null the
   buffer var there, and let re-entries REUSE the still-placed record (the callee's
   entry clear resets its length; the reinit gate already declines the callees that
   would wipe the host).  Needs its own cells: the p12/p13 shapes must stay green, plus
   a re-entry-reuse cell asserting one placement.
2. **SHIPPED as § V-x — the invariant-literal hoist**: build 279.1–285.4k ns/op
   (−5.8 % on the reuse tip), one guard in `n_fronds`, eleven cells + pins + the
   c5 sabotage falsification.  The original item, for the record:
   **`fd_sides` loop-invariant hoist (−8.7 % MEASURED at source level).**  The `if sp.mirror { [1.0,
   -1.0] } else { [1.0] }` literal is rebuilt per `i` iteration (store clear + pre-alloc
   + appends); hoisting it above the loop in the SOURCE gives 354.7k → 323.8k best-of.
   A generation rewrite (loop-invariant vector literal, body proven read-only for it)
   earns exactly this without touching the library.
3. **Zero-on-claim skip (~9 % class: `bzero` + `memset` + `zero_range` = 701/7 700
   samples).**  A claim fully written by its group skips the memset — § V-t's
   complete-write argument one level down.
4. **SHIPPED as § V-y — the complete-write prefill elision**: `fronds` 278.3–279.3k →
   264.9–272.7k ns/op (−4.8 %, 9 twin sites); the struct-literal half alone measured
   ZERO (the samples were the vector-store creations), and the Z∩P lesson is honored —
   one argument, not two skips.  The original item, for the record: **Default-prefill
   skip (~7 % class).**
5. **SHIPPED as § V-z the same day — the build meets the hand ceiling exactly**
   (fronds standalone 186.4–199.7k ns/op; the c4 two-appends cell taught that the
   SECOND append pairs soundly).  The ceiling record, for the history:
   **Element-first temp reorder — CEILING HAND-MEASURED 2026-09-12 at −27–29 %,**
   far beyond the ~6 % class estimate: on the § V-y tip, 270.2–283.0k → **191.7–210.3k
   ns/op** (best 191 655), hash exact, ABAB.  The hand-mod (scratchpad `vrf_ef.rs`,
   rustc-first): mint the `Frond` element FIRST, bind `fd_pts`/`fd_wid` to the element's
   own FIELD SLOTS (a vector value IS the ref to its handle slot — `elm.pos + 0/+4`),
   build points and widths in place, drop both `vector_add`s and both temp stores.  The
   estimate missed three co-removals: the per-iteration temp-store reuse-clears, the
   temps' claims/frees and their free-tree traffic, and — the compounding surprise —
   § V-u's adoption composes: `n_pt(cell, …, var__elm)` delivers each point DIRECTLY
   into the just-minted element, no buffer at all.  Two wrong turns the next reader
   should skip: `OpNewRecord(elm, Frond, fld)` is NOT "create the field vector" (a
   vector-typed field's `record_new` path APPENDS an element — the probe answered 0
   points, hash `811c9dc5`), and the naive parent_tp is the VECTOR type, not the record
   (the "field 0 of 'vector<Frond>' has no storage" panic).  The rewrite to build
   (§ V-z): a pairing like V-j's at FIELD level — a local vector built then consumed
   exactly once as a record-literal field of an append retargets its declaration to the
   element's slot; cells before code.

**Every ceiling above is now HAND-MEASURED (same day, the rustc-first protocol: the
emitted `.rs` hand-edited and linked with bare `rustc -O`, plus an env-gated probe rlib
for the runtime levers; every cell hash `ebcfd875`, ABAB best-of-3, `--n 4000`):**

| stack | ns/op (best) | vs base |
|---|---:|---:|
| base (tip 6b292d21) | 351 292 | — |
| **A** — placement reused across re-entries (the loop-exit free deleted by hand; sound here because the host is the returned retbuf) | 292 814 | **−16.6 %** |
| **A+S** — plus `fd_sides` hoisted above the loop (source-level) | 273 352 | **−22.2 %** |
| **A+S+P** — plus the mint prefill skipped (`LOFT_PROBE_NO_PREFILL` gating `OpDatabase`/`OpNewRecord`/`place_record_in`'s `set_default_value` in a probe rlib) | 247 372 | **−29.6 %** |
| A+S+Z — zero-on-claim skipped instead (`LOFT_NO_ZERO_CLAIM=1`, the existing lever) | 265 556 | −24.4 % |
| A+S+Z+P — both runtime levers | 264 175 | −24.8 % |

Two verified surprises: the zero-claim class measured ~4 %, NOT the ~9 % the sample
suggested (most of the `bzero` is arena-growth `alloc_zeroed`, untouched by the lever);
and Z stacked ON TOP of P is reproducibly NEGATIVE (3/3 rounds, 247k → 264k) — the
claim-zeroing supplies the zeros the skipped prefill no longer writes, so the pair is
redundant where each alone is profitable, and the implementation should treat
complete-write as ONE argument that retires both, not two independent skips.  The
prefill ceiling (−7–8 % beyond A+S) is a GLOBAL skip; the real rewrite only skips where
the write set is provably complete, so its realizable gain is at or under the ceiling.
A+S+P lands `fronds` at ~4.3× projected consumer — element-first (item 5, ~6 % class,
ceiling not yet hand-measured) is the remaining headroom to the bar.  The pre-fix
§ V-v profile below is kept for the class history it names.

**What remained in `fronds` (6.08×), profiled on the § V-v runtime** (macOS `sample`,
9 235 samples, `coalesce_free` gone from the table): the arena is still ~41 % but now the
honest per-claim cost — `claim`+`claim_block` 15.5 %, the fl-tree ops ~14 % (partly the
backward merges' own insert/delete churn), zero-on-claim `memset`/`bzero` 7.2 %, and
`set_default` 5.5 % beside it; the append entries (`vector_append`/`pre_alloc`/`finish`/
`record_new`) 12 %; the remaining per-side temp→element copies (`vector_add` +
`copy_claims`) ~5.5 %; the program's own share is up to ~14 % as machinery falls.  Next
candidates, in that order: **skip the zero-on-claim where the claim is fully written**
(§ V-t's complete-write argument one level down, ~7–12 % ceiling with the prefill),
then the per-side temp cycle — but by the ELEMENT-FIRST reorder (build `fpts`/`fwid`
inside the just-appended element), since the two placement-shaped routes are measured
NEGATIVE (DESIGN.md § V-u's receipts).  `lock_curved` (6.24×) and `wide_line` (5.55×)
still owe their with-callers profile.

**§ V-u SHIPPED 2026-09-11, the same day, and § V-j grew its callee gate** (DESIGN.md
§ V-u, § V-j): the V-j suite run caught a REAL corruption — the sqldb fixture's
`collect_leaf` receives its buffer as the witness-promoted ABI and re-inits it with
`OpDatabase`, whose reuse arm clears the buffer's WHOLE store, under placement the
caller's half-built result (`hoist::callee_reinits_buffer` now declines such callees;
cells c17/c18).  § V-u then shipped the return-copy removal the fronds profile named as
the top inclusive chain: standalone `fronds` −9.5 % and `smooth` −14.4 % on top of
everything above, hashes exact, cells leak-free under poison.  Two negative measurements
recorded on the way (DESIGN.md § V-u): the 2026-09-08 variant-A source shape is now +7 %
(field-path appends miss the bare-variable fast paths) and placing the builder
temporaries for a field-adopt is +5 % (§ V-j P2's lesson again) — the allocation queue
item 4b is therefore RE-RANKED DOWN; what remains for `fronds` is the per-side element
machinery and the temp-store cycle, to be re-profiled on the § V-u runtime.

**§ V-j's MOVE SHIPPED 2026-09-11, the same day** (DESIGN.md § V-j, the SHIPPED addendum;
`@FR-R-MoveAppend`): the ceiling re-measured by hand on the § V-t runtime came out **−11.3 %**
on `fronds` (the copy class had grown to 23 % of a 2× faster row), and the build reached it
exactly — `fronds` 396–400k → **354–359k ns/op** standalone, 8.36× → **7.88×** consumer lane,
hash `ebcfd875` every run.  Cheaper than the sketch's "three new ops": no IR change — a
per-function pairing analysis (`hoist::move_appends`), the buffer placed as a record in the
destination's own store at the For (declaration dominance gives the store on every path),
`OpCopyRecord` from the armed loop variable dispatching on store identity, and the buffer's
free record-level.  Sixteen cells under POISON + both leak checks; falsified at the use-after
gate (c2 `3 409 45` → `3 409 0`); composes with § V-t (c16: the move lands in the push
header's slot).  Switch `LOFT_NO_MOVE_APPEND`; `LOFT_TRACE_MOVE=1` names the declining gate.
**`fronds`' next units:** the allocation class (queue 4b — the `fd_sides` literal, builders
into the appended element, the re-seeded spec; plus "skip the zero-on-claim where the claim
is fully written", § V-t's argument one level up) — the store allocator is ~41 % of the row's
profile; then Mod 3 (the result vector adopting the retbuf, −16 % measured on `smooth`'s
ceiling), which removes the per-level return copy `fronds` pays at every recursion depth.

**§ V-t SHIPPED 2026-09-11** (DESIGN.md § V-t), from re-profiling `smooth` as the § V-s
hand-off asked (macOS `sample` on the standalone probe; the branch had been fully absorbed
into `main` by #1518, so the branch was reset to `origin/main` first): the append machinery
was ~53 % of the row — per element a `record_new` dispatch, a default prefill over fields
the group was about to write, and a `record_finish` dispatch.  The ceiling was hand-measured
on the emitted Rust FIRST, in three independent mods (ABAB, hash exact): the fused record
push **−37 %**, scalarized tangent temporaries a further **−21 %**, the result vector built
in the retbuf a further **−16 %** — the full stack ~1.5× of Rust on this box.  Mod 1 shipped
as § V-t (standalone `smooth` −32 %; consumer `smooth` 11.0× → **4.57×**, `fronds` 11.1× →
**8.36×** on this box, 14/14 hashes agree).  **Next, in measured order:** mod 2 — the
frame-local record temporaries (queue item 5: `sp_ta`/`sp_tb` and `half_chord`'s locals as
scalar pairs, −21 % measured ceiling on `smooth`, the same class in every routine naming a
struct temporary — M–L, the SROA design); mod 3 — a single-assigned, append-only result
vector local ADOPTS the retbuf (`sp_out` builds in `__retbuf`, no 61-element return copy,
the backing reused across calls — M, § V's Route R for vectors); then `fronds`' remaining
allocation classes (queue 4b, −9 % source-level ceiling) and the § V-t scope left on the
table (heap-owning elements need an explicit zero of the handle fields only).  The `hair`
and `lock` rows sit under the bar on this box; re-rank `lock_curved` (6.05×), the fills
(4.3×) and `wide_line` (5.06×) by profiling WITH callers on the § V-t runtime, as § V-k did.

**That question is CLOSED — § V-s shipped 2026-09-10** (DESIGN.md § V-s): the blocker was the
record-append MINT group itself (`OpNewRecord`/`OpFinishRecord`, plus § V-d's `OpCopyRecord`
delivery), unclassified writers that declined the whole loop; the three probe cells re-run on
the joined tree had narrowed it (the fused-scalar `nopt` hoists since § V-q, both record-append
cells still declined, and a no-write variant of the same loop hoists all four reads).  The mint
is now admitted as a mover under `(R-Alias)` and a write into the FRESH element evicts nothing
(`@FR-R-Mint`, formal/rewrites.md).  Measured honestly (interleaved ABAB on the two scratch
clones, best of 3, hashes agree): **`smooth` −8 %** (3 700 → 3 420 ns/op); `fronds` and
`lock_curved` within lane noise — their appends were already § V-q scalar pushes, or the cost
sits elsewhere.  `smooth`'s row sits at ~7.5–7.9× today: most of the drop from the 20.9×
above came from the cold-half-outlining commits that joined `main` after that measurement,
and what remains is NOT the invariant reads any more — profile the row again before
choosing its next unit.

⚠ **And a switch in this family cannot A/B a LIBRARY's own code**, which is how that attribution
nearly went wrong: the switches are read at GENERATION time and a `use`d library runs as the
cdylib cached under its `native-auto/`, so `LOFT_NO_SCALAR_HOIST=1` on the consumer's run
regenerates the program and leaves the library untouched — 394 ms vs 393 ms on a probe whose whole
hot loop is inside `drawing`, which reads as "no effect" and is not one.  Rebuild the cdylib under
the switch in a SCRATCH COPY of the package.  PERFORMANCE.md § Native vs Rust carries this.
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

**And, since 2026-09-09, the FORMAL RULING of what the native rewrites assume
(owner's steer):** every rewrite the plan ships is a rule in
[formal/rewrites.md](../../formal/rewrites.md) — the hoist STATE they compose
through (`R-State`, `R-Refresh`, `R-Alias`) and one rule per rewrite, each with
its switch, its falsifier and its sites — and the emitted routines are VALIDATED
against those assumptions by two instruments: the checking forms at run time
(`LOFT_HOIST_VERIFY=1`) and the emission audit at emission time (§ V-r, queued
first).  The emitted Rust grows with every unit; a unit is not shipped until
its assumptions are written as a rule and checkable by both.

## Effort + design

- **Effort:** H total (P1 S · P2 S · P3 M · P4 L · P0/P5 XS)
- **Design:** ✓ — [DESIGN.md](DESIGN.md): per-phase invariant, code sites,
  claims + falsifying probes, predicted numbers
- **Last touched:** 2026-09-11 (§ V-t … § V-w)

## Where to resume

**2026-09-18, night — the store-traffic analysis of `fronds` (2.6×) and `parse` (2.6×), and the two
units it asks for.  START HERE.**

*Instruments that worked on this box (no `perf`; `sample` attaches to rustc or misses the
short-lived binary):* the native checkpoint census (`LOFT_NATIVE_CHECKPOINTS=count`, exact
counts per site), its `time` rollup (ticks are a HINT), the interpreter store log
(`LOFT_STORES=log`: `+ alloc`/`- free` events are FRESH stores), the release-pass ceiling, the
interpreter line profiler, and a loft micro-bench of the two runtime paths.

*What the rows are made of:*
- `fronds` (109 µs/call): 44 000 operators — 24 000 the noise hash (`seed_hash`/`seed_wave`,
  ~30 % of ticks), 3 300 trig; the object machinery at the appends `fd_out += [Frond {…}]`
  (drawing.loft:1647/1649: 654 `OpPreAllocVector` 12 %, 654 `OpNewRecord` 7 %, 654 `OpCopyRecord`
  2 %, 1 300 `OpPushFloat` 4 %) ≈ 25 % of ticks; the per-k `FrondSpec` literal 5 %; **152 fresh
  stores per call** (25 activations × ~5 hidden buffers + 24 `fd_sub`) ≈ 4 %.  The checks are
  ~1 % (`LOFT_RELEASE_PASS_PROBE`).  Rust does ~700 mallocs per call here, so the COUNT is not
  the gap; the per-append runtime calls are.
- `parse` (14 µs/call): 42 000 operators for 13 lines — the byte scanner (`word_boundary`,
  `matches_at`, `at`, `find_option`, `is_word_byte`: one loft call per byte per pass, 10 138 `at()`
  per scene) ≈ 55 % of ticks, `text.split` 8 %, `size()` 3 %; **127 fresh stores per call**, all
  temporaries dying with their activation (each `parse_*` call re-mints its return, discharge and
  vector buffers and frees them at exit) ≈ 25–30 % by the pair cost below; checks ~8 %.

*The two runtime paths, measured (native, 200 000 iterations):* a vector local declared inside
a loop (`§ V-al`: keep the store, `clear`) **10 ns**; a record literal bound inside a loop
(`s = S1 { a: k }`: `free_named` at the iteration's end, then `find_free_slot` + `reinit` +
`claim` + zero + tag) **39 ns**, 1 003 fresh stores per 1 000 iterations.  A "fresh" store is
NOT a malloc in steady state (the slot table only grows when every slot is live); the ~30 ns
pair is the free/search/init bookkeeping.  So the reset-and-reuse path is ~4× cheaper and is
taken only by `§ V-al` vector buffers and by return buffers within ONE activation.

*Owner ruling C125 bounds the design:* temporaries are REMOVED, not co-located; one store is
one object.  So "link them together" means RETAIN and re-use a site's store, never share one.

*Unit A — `R-LoopRecord` (the `§ V-al` shape for a record local):* a plain no-heap record
literal bound inside a loop (`fd_sub = FrondSpec {…}`) keeps its store across iterations —
the emitter declares the local at the loop's prelude, the per-iteration mint takes
`OpDatabase`'s clear arm, the body's `Set(v, null)` and its direct end-of-body `OpFreeRef`
are not emitted, one free follows the loop.  Admitted where every mention is the mint, a
fusable field read/write, a free, or a hand-off to a loft callee whose return does not borrow
that parameter (`(O-Borrow)`: a callee cannot keep a caller's record except by copy or through
its return deps); a copy to another local, a return, a capture, a heap-owning field, `par` or
`yield` decline.  Frees on a `return`/`continue` path stay (they are nested in an `Insert`, not
the body's tail).  Priced on the probe: 39 → ~12 ns; on the bench ~1 % (`fd_sub` is 24/call).
*Correction, later the same night — the 127 and 152 are the INTERPRETER's counts.*  The
native trace (`LOFT_TRACE_DB=1` on `--native`, `db=#65535` = a fresh mint) shows `parse`
minting **~11 fresh stores per call** (3 `vector<float>`, 3 `PointList`, 1.5 `Sketch`, a
`Paint`, a `Mark`, a `FrondSpec`, a `vector<Pt>`) and `fronds` **~51** (25 `vector<float>` —
`fd_sides`, one per activation — 24.5 `FrondSpec`, 1 `vector<Frond>`): the value-record,
element-first and buffer-adoption rewrites already remove most of what the interpreter
mints, so the per-site retention below has ~11 × 30 ns ≈ 0.3 µs to win on `parse` (2 %),
not 25–30 %.  That settles it: unit B is not worth its instrument cost, and C125 already
said so (the owner ruled against a pooled cheap store per buffer).  What remains on `parse`
is the scanner; on `fronds` the append machinery, the hash, and `fd_sides` (a per-activation
two-element literal vector — a candidate for `(R-LiteralHoist)`'s const-param branch).
*Unit B — per-site buffer retention across activations (SUPERSEDED by the correction):* the 127 fresh stores of `parse` are
hidden buffers minted and freed once per callee activation.  A per-(function, buffer) chain
that keeps the store between activations (`clear` on entry instead of `null` + `database`,
push instead of `free_named` at exit; recursion takes the next link, so a live buffer is never
re-seated) replaces the ~30 ns pair with a pop and a re-init.  Liveness is identical to
today's unconditional exit free, so the soundness question is only the LEAK instruments: a
retained chain must drain at exit so `LOFT_NATIVE_LEAK_CHECK` and `LOFT_STRICT_STORES` keep
their meaning.  Price by hand patch on `parse_circle`'s emission before building.
*The scanner* (`parse`'s 55 %) is a third unit: a leaf reading a `text` byte-wise pays a call
with a `Str` argument per byte; a twin taking the bytes once (as a header is taken) or an
inlined leaf is the shape.

**2026-09-15, afternoon — HAND-OFF after § V-ao.  START HERE.**

*What shipped.*  § V-ao (`@FR-R-Invariant`, DESIGN.md § V-ao): an invariant integer chain
is evaluated at its first use and answered from a memo after, declared at the innermost
loop that spells it — the resample probe 97.6 → 92.7 ms/op (−5 %), hash 77de7581.  With it,
**loft#1534** (silent-wrong on native, fixed in the same commit): the non-sentinel proof's
escape collector read only a BARE `Var` argument while a by-reference argument is spelled
`OpCreateStack(k)`, so a callee's overflow into a "proven" local answered a number where
the interpreter answers null.  One collector serves the proof and the memo, and the memo's
verify form is what found it.  Cells m1–m16, pins `tests/invariant_arith.rs`, guards
`tests/scripts/157-invariant-arith.loft` and `1534-by-ref-arg-escapes-the-proof.loft`.

*The table after it* (this box, `compare.py --skip-interp --repeat 3 --n-ref 500
--n-native 500`, 14/14 hashes) — resize 5.57 → 5.01×, render_lock 5.46 → 5.18×, render_marks 7.96 → 7.41×, `parse` untouched at 7.56×, the ten judged rows where they were:

| row | Rust ns/op | native ns/op | native / Rust | before |
|---|---:|---:|---:|---:|
| hash | 115 184 | 111 300 | 0.97 | 0.98 |
| fill_circle | 53 672 | 61 584 | 1.15 | 1.15 |
| fill_star | 19 212 | 23 698 | 1.23 | 1.26 |
| hair | 14 036 | 27 994 | 1.99 | 1.96 |
| composite | 100 924 | 202 008 | 2.00 | 2.00 |
| lock | 1 546 460 | 3 152 228 | 2.04 | 2.00 |
| wide_line | 5 854 | 13 638 | 2.33 | 2.34 |
| lock_curved | 1 314 394 | 3 183 058 | 2.42 | 2.34 |
| smooth | 330 | 1 158 | 3.51 | 3.82 |
| fronds | 49 088 | 178 558 | 3.64 | 3.58 |
| **resize** | 17 921 822 | 89 713 362 | **5.01** | 5.57 |
| **render_lock** | 2 863 532 | 14 844 338 | **5.18** | 5.46 |
| **render_marks** | 884 676 | 6 553 520 | **7.41** | 7.96 |
| **parse** | 6 654 | 50 294 | **7.56** | 7.59 |


*Next, by the queue:* ~~item 2, **`R-BoundedNest`**~~ — SHIPPED 2026-09-17 (§ Status); the
three resample rows are under the bar.  What remains over it is **`lock_curved`**, analysed
and PRICED 2026-09-17 (arm64, both binaries sampled with `sample`, the emission read, two hand
patches on one emission, hash `2a3aa61` throughout):

* **It is not the checks.**  `LOFT_RELEASE_PASS_PROBE=1` moves the row only 1 660 → 1 429 µs
  (−14 %; still 3.5× the reference's 404), so the lever that closed the resample rows does not
  reach this one.
* **It is two fills that are not fills.**  `ll_out = [for _i in 0..ll_n { 0 }]` (brush.loft:476)
  lowers to `OpAppendCopy`, whose runtime fills `[x; n]` with ONE `copy_block` AND ONE
  `copy_claims` call PER ELEMENT — 38 249 calls each per `lock_layer`, 14 % of the row by
  `sample` (`copy_block` 9.4, `copy_claims` 4.6); the interpreter's `State::append_copy` is its
  twin.  The seven `Lay` planes, `best: [for _i in 0..ll_n { 2.0 }]` … (brush.loft:450–453), lower
  the OTHER way — a `For comprehension` block pushing one element at a time into the record's
  field (`push_hoisted` × 7 × 38 250) — and § V-am's fill idiom never sees them, because its
  matcher admits `For loop` only; a reserve up front measured nothing (the cost is the push
  loop, not the growth ladder).  Hand-patched on the emission: the seven planes as one
  `push_fill` each, **1 635 → 1 108–1 204 µs**; plus `ll_out` as one fill, **→ 860–942 µs =
  2.2×** the reference, from 4.2×.  Both are engine units: the runtime fill in `OpAppendCopy` /
  `State::append_copy` (a block copy that doubles, the claims walk only for a heap-owning
  template — both backends), and the constant comprehension into a record-literal field taking
  the same lowering a local's does (parser, both backends) or the fill idiom admitting a
  comprehension block (emitter).  **BOTH BUILT the same evening** — `Stores::fill_from_template`
  (switch `LOFT_NO_BLOCK_REPEAT`) and the field fill (`LOFT_NO_FIELD_FILL`): `lock_curved`
  **1 635 → 895–931 µs, 2.2×**, the row under the bar on this lane; cells
  `a-repeated-element-fills-in-one-block.loft`.  The full lane, two interleaved rounds against
  both switches off, 14/14 hashes: `lock_curved` 3.89× → **2.17×**, and every other layer-building
  row moved with it — `lock` 2.38× → **1.84×**, `hair` 2.07–2.24× → **1.58–1.72×**, `render_lock`
  2.01× → **1.81×** — the rest within their swing.  **Every one of the fourteen rows is under the
  3× bar on this lane; the median is 1.73×** (2.04× before the fills, 2.35× at the start of the
  evening).  Sites: `src/codegen_runtime.rs` `OpAppendCopy`,
  `src/state/io.rs` `append_copy`, `src/parser/vectors.rs:3411` (the comprehension loop) and
  `:5389` (the `[x; n]` lowering), `hoist::fill_loop`.
* **`composite` (3.0×) analysed and PRICED 2026-09-17**, the same instruments, hash `cf852074` on
  every run.  The sample is 100 % compiled code — no store runtime at all — and the census is
  ~80 loft operators per pixel: 44 in `composite_layer`, 10 in `get_pixel`, 10 in `set_pixel`,
  7.5 in `rgba`.  Two things that LOOK expensive measured as nothing (five rounds each): the
  accessors' four range compares plus a length test, twice per pixel, and the three
  `?? 0`-discharged divisions with their fault notes — LLVM predicts them away.  What is left is
  two structural costs, independent and additive: (1) the accessor twins take a HEADER and no
  BASE, so every element read and write inside them is `get_elem_hoisted`/`vec_set_hoisted`, a
  store resolution per access where Rust's `cv.data[di]` is one load off a register — a base
  handed into the twin beside its header, **115 → 84–85 µs (−27 %)**; (2) the checked
  arithmetic, `LOFT_RELEASE_PASS_PROBE=1`, **115 → 83 µs (−28 %)**; both together **115 →
  50.7–54.5 µs = 1.32×** the reference's 38.3.  Unit (1) is a straight extension of § V-p and
  § V-ak: `CalleeInputs.headers` gains a base per header, the twin's signature `__ib_k: *const
  u8` beside `__ih_k`, its body a `vec_bases` frame so the fused read/write take
  `get_elem_at`/`vec_set_at`; a caller passes its held base, or derives one from the header at
  the call when its loop is not growth-free — sound either way, because a twin is store-free or
  in-place-only, so nothing reallocates for the call's duration.  **Unit (1) BUILT 2026-09-18**
  (`LOFT_NO_TWIN_BASE`; § Status): `composite` 114.5 → 91.3 µs (−20 %), 2.98× → 2.40×, `lock`/`lock_curved`/`render_lock` −4 % each, 14/14 hashes.  Short of the hand patch's 84–87 because `composite_layer`'s own callers grow a store and so derive the base at the call rather than passing a held one; unit (2) is what remains.  Unit (2) is C120's next range
  proof: `x & K` bounds `x` to `[0, K]`, so a chain of `+ - *` over masked leaves and literals
  whose interval fits emits plain — `rgba`, `color_*`, the alpha-over arithmetic and
  `lock_layer`'s `chan()` are all of that shape; it needs a per-function result interval for the
  small colour helpers, and a design note before it is cut.
* **`wide_line` (2.57× on the lane, 2.93× standalone) analysed and PRICED 2026-09-18**, the
  same instruments (the lean emission read, hand patches compiled through loft's own rustc
  line — the `--native-emit` default is the DEBUG tier, `--lean --native-emit` is what
  `--native-release` builds, and the first control ran 10.5 µs against loft's 7.8 for that
  reason), hash `91c48fd2` on every run.  The crossing loop `pg_cur = pg_table[i]?` paid a
  STORE RESOLUTION per record field read — six on `pg_cur`/`pg_oth` per edge per row and
  three more inside each `edge_x` — where Rust's `let cur = table[i]` reads registers, and its
  `pg_xx` reads and writes went through `get_elem_hoisted` because the `?` discharge's buffer
  mint counted as a growth and kept the loop off its bases.  Priced: bases past the mint
  7 800 → 7 300–7 600 (−5 %); record addresses for `pg_cur`/`pg_oth` → 5 800 (−25 %); `edge_x`
  reading through them → **4 650–5 050 (−40 %), 1.75×**.  **BUILT the same day** as
  `R-RecPtr` (§ Status), the emitter reproducing the hand patch's shape at 5 060 (−35 %).

  **Why LLVM does not hoist the store resolution itself — measured, because the owner would rather
  it did than have the emitter repeat its algorithm.**  Two ways of GIVING it the knowledge were
  built by hand on the same emission and moved nothing (rounds 2–4, hash `cf852074`): whole-program
  fat LTO over a runtime built with `-C embed-bitcode=yes`, so every `#[cold]` hook body was in view
  (116–118 µs, against 115–118 shipped), and the generated ABI rewritten from
  `cell: &UnsafeCell<Stores>` to `stores: &mut Stores` — a `noalias` argument — on all 42 functions
  (117.6–118 µs).  The base handed in by the emitter: 85.5–86.5.  The barrier is not the opaque
  calls and not the missing `noalias`; it is that the element write goes through a pointer LOADED
  from the store table (`allocations[k].ptr`), and a loaded pointer has no provenance LLVM can
  relate to anything — not to a `noalias` argument, not to an identified object — so alias analysis
  must let that write clobber the table it was loaded from.  The fact that makes the hoist sound is
  loft's own allocator invariant, *an element buffer never overlaps the store table*, which no
  source-level construct states; carrying the base is the one way to state it.  So `@FR-R-Base`
  and a base through a twin are not LICM re-implemented: LLVM's LICM is not ABLE to do this one.

* **After those, ~2.2× and diffuse:** five `st.*` scalar reads per resolved pixel still go
  through `store_mut` (unhoisted record scalars of a `const` parameter in the resolve loop),
  `brush_sample`/`chan`/`ramp` per pixel, the two `??`-discharged divisions per raster pixel
  with their fault notes.  The x86-64 lane read this row 2.42× before any of it.  What to read first: § V-ae's fill for the
guard-and-fallback shape, § V-al for the emitter placement, and § V-ao's placement lesson
(a memo one loop out lost its gain — the bounded nest's guard must sit where LLVM can see
it is decided).  The vertical tap's chain (counter innermost) is the case R-InvariantArith
cannot touch and R-BoundedNest must: inside a proven-bounded nest the plain form needs no
memo at all.

**2026-09-15, morning — HAND-OFF after § V-an and the rebase.**

*Branch state.*  `157-native-4x` was rebased onto `main` and EQUALS it: PR #1533
(squash 7c3f93b3) cherry-picked the four § V-an commits and re-measured QUALITY.md's two
audit rows on the joined tree, so nothing of this branch was left to replay (`git cherry`
matched none of the 44 commits by patch-id — a plain `git rebase origin/main` would have
conflicted 44 times to reach the same tree; `git rebase --onto origin/main <last>` did it
in one step).  On top of `main` the branch carries only b922d534, PERFORMANCE.md § *The
clear case first*.  No PR is open; the owner calls the wrap.

*The table on the rebased tree* (this quiet x86-64 box, `compare.py --skip-interp
--repeat 3 --n-ref 500 --n-native 500`, the scratch clone's `bench/bench.rs` with
`bench-reference-scene.rs` appended so every row is judged, 14/14 hashes agree):

| row | Rust ns/op | native ns/op | native / Rust |
|---|---:|---:|---:|
| hash | 108 116 | 106 310 | 0.98 |
| fill_circle | 53 660 | 61 700 | 1.15 |
| fill_star | 18 934 | 23 940 | 1.26 |
| hair | 14 040 | 27 456 | 1.96 |
| lock | 1 530 382 | 3 056 902 | 2.00 |
| composite | 101 050 | 202 306 | 2.00 |
| lock_curved | 1 322 634 | 3 090 188 | 2.34 |
| wide_line | 5 826 | 13 622 | 2.34 |
| fronds | 49 360 | 176 796 | 3.58 |
| smooth | 308 | 1 178 | 3.82 |
| **render_lock** | 2 858 728 | 15 599 090 | **5.46** |
| **resize** | 17 416 716 | 96 948 566 | **5.57** |
| **parse** | 6 636 | 50 356 | **7.59** |
| **render_marks** | 887 460 | 7 063 346 | **7.96** |

Ten judged rows under the bar, the four late rows over it.  `parse` came from 10.5×
(§ V-an); its ratio reads 7.6× here rather than the 6.6× in the § V-an paragraph because
this run's reference lane was faster (6.6 µs against the recorded 7.2), the native side is
within noise.

*The next unit, by the owner's rule (PERFORMANCE.md § The clear case first: the case that
shows in the statistics AND has a visible reason).*  The resample: ONE routine
(`t_6Canvas_resample`, `graphics/src/graphics.loft`) is 82 / 70 / 57 % of `resize` /
`render_marks` / `render_lock`, and the reason is read (DESIGN.md § V-aj "The tap in
machine code": 109 instructions and 35 branches per tap against the reference's 17 and
1.7 — re-tested hoisted flags, no vectorisation; the release-pass probe puts the checks at
60 % of the row).  So the queue's items 1 and 2 are the work: `R-InvariantArith` (the
invariant part of the tap's index arithmetic hoisted at loft level, sound by
construction), then `R-BoundedNest` (a bound over the nest's inputs taken once, the plain
vectorised loop under it, the checked loop as fallback — the unit that reaches the
reference's cycles per tap and takes the three rows under the bar).  The probe is
`rs_probe.loft` (the graphics `resample` lifted verbatim + the bench row; hash 77de7581;
last read 95.4 ms/op), the falsifier `LOFT_HOIST_VERIFY=1`, the ceiling
`LOFT_RELEASE_PASS_PROBE=1` (38.3 ms/op on the probe).

`parse`'s class had its own plan, **@PLN164** (`plans/164-activation-arena/`: adopt-at-bind,
build-in-place, and the elimination of the per-call temporaries a record-returning style
mints), **closed 2026-09-17**: the row is **20 740 ns against the Rust reference's 6 675 —
3.11×**, from 7.6×, and one parse mints 13 stores where it minted 31.  What is left of it is
NOT a copy class: about 1.2 of the 2.1 units the row is over Rust is the per-RECORD and
per-PUSH work every KEPT object pays (claim/free, the append pair, the append path's
`heap_facts` and `nullable_field_parent` tests, `store_mut`) — a RUNTIME lever on both
backends, priced the way every unit here is (a hand patch first, `perf` on the release
binary, all fourteen rows), and registered with the role/lifetime work in PERFORMANCE.md
§ 3e.

*Machine notes for the resample unit.*  A fresh session needs: `cargo build --release`
(bin AND lib — the native tests link the rlib), a scratch clone of `loft-libs-graphics`
at `drawing-lock` (250b2cd) — never the consumer's own tree — with the reference appended
as its header says, and the resample probe rebuilt from `graphics/src/graphics.loft:807`
(lift the function verbatim, add the bench row's call).  `scripts/emission_audit.py` on
the emission before running it; `LOFT_TRACE_VALUEREC=1` names the value form's refusing
test since § V-an.  This box (14 GiB) killed a local `find_problems.sh --subject codegen`
for memory with nothing else running; the GitHub dispatch (`gh workflow run ci.yml --ref
157-native-4x -f os=ubuntu-latest`) is the gate here, and after adding any IR-walking
function run `scripts/ir_walker_audit.py optional` and `unspan`, update QUALITY.md's two
rows and `make optional-ratchet` BEFORE dispatching — that was the one real red of the
§ V-an gate.

**2026-09-14, end of day — THE FOUR UNJUDGED ROWS, JUDGED.**  The bench's `parse`,
`render_lock`, `render_marks` and `resize` rows had no Rust reference, so they were
measured and never judged.  They have one now — `bench-reference-scene.rs` beside this
README, a port of the drawing package's `parse_scene`, `render` (its four draw paths and
the scan module) and the graphics package's Pillow-exact Lanczos resample, written to
append to the package's `bench/bench.rs` and offered to the consumer's agent (their tree
is read-only from here).  Every hash agreed with the loft native lane on the first
compile, so the four are like-for-like.  The table on this box, all fourteen rows,
`--repeat 3`, 14/14 hashes agree:

| routine | Rust ns/op | loft native ns/op | native / Rust |
|---|---:|---:|---:|
| hash | 106 660 | 289 940 | 2.72 |
| hair | 14 440 | 28 800 | 1.99 |
| smooth | 380 | 1 360 | 3.58 |
| fronds | 51 640 | 175 400 | 3.40 |
| lock | 1 545 720 | 3 832 120 | 2.48 |
| lock_curved | 1 317 820 | 3 529 380 | 2.68 |
| composite | 101 240 | 239 020 | 2.36 |
| fill_circle | 55 460 | 64 180 | 1.16 |
| fill_star | 20 500 | 26 400 | 1.29 |
| wide_line | 5 960 | 13 900 | 2.33 |
| **parse** | 7 220 | 72 340 | **10.02** |
| **render_lock** | 2 901 740 | 20 723 900 | **7.14** |
| **render_marks** | 891 260 | 9 497 760 | **10.66** |
| **resize** | 18 052 800 | 137 132 280 | **7.60** |

So the scoreboard is *ten of ten under the bar* for the rows the plan judged, and *four
of four over it* for the rows it had not — the plan's bar is not met on the whole bench
until these four are run down.  Each is a composition of routines already under the bar
(`render_marks` is `fill_poly` + `wide_line` + `smooth` + `fronds` + the resample;
`render_lock` is `lock` + the resample; `resize` IS the resample; `parse` is the scan
module over text), so what they add is the glue: text scanning by byte, the per-row
`thin_line` of the sky, the ribbon polygons, and the resample's four-channel
fixed-point loops over `vector<integer>` planes.  Profiles below.

*Profiles* (`scripts/profile.sh --engine --calls -- --native-release <row>_only.loft`, a
one-row cut of `bench/bench.loft`, this box):

| row | what carries it |
|---|---|
| `resize` (7.60×) | `t_6Canvas_resample` **81.7 %** — ONE routine: the two fixed-point passes, `rl_acc += rl_pre[(yy*iw+xmin+x)*4+ch]? * hk[rl_base+x]?` over `vector<integer>` planes with a `?`-discharged element read per tap and a guarded `rl_mid[…] =` write per channel; the reference is the same loop over `Vec<i64>` at 18.1 ms against 137 ms |
| `render_marks` (10.66×) | the same resample **69.7 %** (192×192 down to 64×64); `vector_add` 6.0 % under `n_canvas` → `filled` — the supersampled canvas built by appending its pixels one at a time; the marks themselves under 10 % |
| `render_lock` (7.14×) | the resample **57.2 %**; `n_lock_layer` 14.3 % (8.6 % of it `raster_segment__inv`, the § V-p twin); `vector_add` 5.4 % (`filled` again) |
| `parse` (10.02×) | no single routine: `matches_at` 6.9 % + `find_option` 4.2 % are the byte scan (`find_option` rescans the whole line per key, and a `Lock` or `Fronds` line reads a dozen keys), and ≈ 30 % is the store lifecycle behind the per-op records and vectors — `copy_claims` 4.7, `set_default_value_nullable` 4.7, `op_database_inner` 3.7, `database_named` 3.6, `set_free_header` 3.2, `free_named` 2.7, `fl_set_red` 2.2, `clear` 2.0, `claim` + `claim_block` 3.7, `malloc` + `memset` 3.3, `Vec<Field>::clone` 1.4 — every `Op` literal with its `pts` / `widths` / `paint.spec` vectors, and the `Mark` / `PointList` / `Scan` temporaries, mints and frees a store |

*Order.*  The resample first: one routine carries three rows, and it is a scalar element
loop of exactly the shape § V-h, § V-q and § V-ae were built for — read its emission
before anything (does the four-deep nest hoist the `rl_pre` / `hk` / `rl_mid` headers,
and is the `?` discharge on an INTEGER element the blocker § V-ad closed for records
only?).

*Done the same night (DESIGN.md § V-aj, § V-ak).*  The four-deep nest DOES hoist every
header; what each tap paid was the nullable-aware arithmetic and the per-read store
resolution.  Shipped: a counted range's counters seeded non-sentinel (`@FR-R-Counter`), a
division by a literal as one test and a divide (`@FR-R-LitDiv`), and the element BASE
beside every header of a growth-free loop (`@FR-R-Base`) — the resample probe 139 →
**111–113 ms/op**, 7.6× → 6.2×.  The C85 line — the integer closure over `+`/`-`/`*`,
worth a further −17 % as the checked non-null family and −50 % as plain operators on the
final emission — was put to the owner and **declined on 2026-09-15 (DESIGN_DECISIONS.md
C120): no random behaviour, only situations we know can be optimised.**  What remains
admissible for the taps is a RANGE proof (bounded operands that cannot overflow emit the
plain operator), which needs the library to declare its planes' bounds.  The non-tap floor —
32 ms of premultiply, prefill, growth ladder and two per-pixel vectors, 1.8× the whole
reference by itself — is the other half, unprobed beyond the profile.  Then `filled` — a `w*h` `vector<integer>` grown by `+=` per pixel, which § V-ae's
slice fill cannot reach because there is no vector yet to fill (a pre-sized fill is the
op the library wants).  Then `parse`'s store churn, which is the § V-af / § V-ah class one
level up: per-op temporaries that could be value locals or dead buffers.

*The table after § V-aj + § V-ak* (this box, `--repeat 3`, **`--n-ref 500 --n-native 500`**,
14/14 hashes agree):

| row | before | after |
|---|---:|---:|
| resize | 7.60× | **5.79×** |
| render_marks | 10.66× | **8.39×** |
| render_lock | 7.14× | **5.69×** |
| parse | 10.02× | 10.45× (untouched; noise) |
| hash | 2.72× | 1.22× (§ V-aj's counters: 289 → 130 µs) |
| lock / lock_curved / composite | 2.48 / 2.68 / 2.36× | 1.90 / 2.29 / 2.01× |
| smooth | 3.58× | 3.95× at 500 calls — see below |

⚠ **The bench's default of 50 calls per row misjudges `smooth`.**  At `--n-native 50` the
row read 4.44× today (1 600 ns), at 300 calls 1 283 ns, at 200 000 calls 1 142 ns, and
the standalone probes 1 091–1 140 — the first call's buffer growth (§ V-u/§ V-ai warm the
return buffer through its ladder once) is a fifth of a fifty-call total for a 1 µs row,
and the Rust reference has no such first call.  The 3.67× recorded above was taken at
50 calls too, so both numbers carry it; the asymptote is ≈ 3.6–3.9× against a reference
that itself moves between 304 and 400 ns/op run to run.  Judge this row at ≥ 500 calls
(the consumer's `compare.py` takes `--n-native`), and read the switch bisect as clean:
with `LOFT_NO_VECTOR_BASE=1` the standalone kernel is 1 092–1 154 against 1 131–1 140 with
bases, and `hash` 119–159 µs against 113–133 — the base unit is neutral to positive on
both, and a fifty-call table is what moved.

*The morning after (§ V-ah, two defects the gate had not seen; 2026-09-15).*  The GitHub
gate on e8101f6e was red: `tests/docs/25-generics.loft` does not compile natively — a
generic INSTANCE's selecting tail lowers as a statement join whose arms bind the join
local from a parameter's view, and the value form typed the join as its tuple while the
arms still minted and deep-copied into it (t14, `expected DbRef, found (i64,)` ×6).
Fixing it, the probe's leak check found the second: the SOURCE-level selecting tail over
parameters lifts each arm's read into a `__lift_N` copy that the oracle calls a view and
the lowering mints — one record leaked per call, on `const` parameters too (t15).  Both
closed in `hoist.rs` / `dispatch.rs`: a lift is a value local when bound from a view and
never a view leaf; a whole-record bind INTO a value local is the tuple assignment; the
admission round grows optimistically and prunes, so the join and its lifts admit each
other.  Both shapes now mint nothing at all.  `tests/value_record.rs` pins the emissions;
`tests/scripts/157-value-tail.loft` carries both receipts.  The gate's other red,
`src/generation/fnref.rs` uncatalogued, is an `@I68` tag.

*The non-tap floor, split (2026-09-15, rustc-first on the shipped emission, best of 3 at
20 iterations: shipped 108.3 ms/op).*  The `rl_mid` prefill as one reserve and a length:
**105.9** (−2 %); a reserve under the premultiply's 2.4 M-element plane: **106.8**
(−1.4 %); the two per-pixel vectors of the vertical pass kept across iterations with a
length reset instead of the clear-and-claim: **99.4** (−8 %); all three: **94.9**
(−12 %); all three with the premultiply loop removed entirely: **88.7** — the premultiply
is ≈ 6 ms of its own (three literal divides, a `?` discharge and four pushes per input
pixel).  The per-pixel vectors were the unit worth building first and are § V-al: the
row's remaining floor is the prefill and the reserve (a counted push loop is a reserve by
its trip count, a constant push loop one fill — the same recogniser, ≈ −3.5 % here and
`filled`'s whole cost in the render rows), then the premultiply's own arithmetic, which is
the C85 line again — closed by C120; a range proof over `(c >> 16) & 255` (a masked
value is bounded) is the sound form of that one.

*The queue after the ruling and the machine-code read (2026-09-15; DESIGN.md § V-aj
"The tap in machine code": 109 instructions, 35 branches, 22 cycles per tap against the
reference's 17 / 1.7 / 4.4).*  In this order:

1. **Invariant index arithmetic hoisted at loft level** (`R-InvariantArith`) — **§ V-ao
   shipped 2026-09-15 as `@FR-R-Invariant`, the lazy first-use memo:** −5 % on the
   resample probe (the horizontal tap's `yy × iw + xmin`; the vertical tap's chain has
   the counter innermost, and `base + 4x` is NOT value-preserving under overflow — that
   is R-Range's question).  Priced rustc-first (trip-guarded prelude −6 %, lazy memo
   −7.6 %); the memo declared one loop out lost the gain (DESIGN.md § V-ao).  Found and
   fixed loft#1534 on the way, a by-reference argument the non-sentinel proof could not
   see.
2. **The guarded plain nest** (`R-BoundedNest`) — a bound over the nest's inputs taken
   once per nest, the plain vectorised loop under it, the checked loop as fallback.  The
   unit that reaches the reference's 4.4 cycles per tap and takes the three resample rows
   under the bar; the largest to build.  Read § V-ae's fill for the guard-and-fallback
   shape, § V-al for the emitter placement.
3. **`parse`** (10.65×) — **§ V-an shipped 2026-09-15: ≈ 7.2×.**  The `Scan` family was
   declined by ONE shape, an explicit `return f(…)` (the phantom buffer assigned), and the
   trace's two reasons were the fixpoint's round order; the field-list clone was
   `link_siblings`, per element append, and is gone.  What the row is bound by now
   (DESIGN.md § V-an, the table): `OpCopyRecord` deep copies **18.5 %** — the FIRST bind
   of a record result copies (`bs_sk = parse_scene(…)`: the adopt-or-copy delivery adopts
   only a store the local already holds, so a fresh local copies and a loop never
   recovers), an indexed element OVERWRITE from a literal (`sc.elems[idx] = Elem{…}`) and
   a struct field assigned from a local at its last use (`paint: pp_paint`) — the
   memory-model units that remain: adopt at the first bind, move at the last use; the
   byte scan ≈ 15 % (`find_option` rescans the line per key — the library's); the `Op`
   element mint's prefill ≈ 5 % (a partial literal, so § V-y cannot elide it — a per-type
   PREFILL IMAGE would make it one block copy).
4. **The counter step's `jo`** — provable away for a counted range; one instruction per
   iteration, everywhere.
5. The aarch64 re-measure, on its own box.

Not in this plan, recorded so it is not re-proposed as a rewrite: the owner's eventual
release build pass for games (DESIGN_DECISIONS.md C120, NATIVE.md § Optimisation tiers)
— plain arithmetic for a game that has already run fault-free under the checks, compiled
from every source it uses after extended in-house testing.  Every unit here stays inside
the checked semantics.  What IS in scope, and shippable, is the owner's second half:
*"if we can clearly determine that a library cannot introduce this behaviour given
restrictions on its inputs we can also ship this; a library that allows an API with full
integer ranges we just cannot prove."*  That is the RANGE PROOF (`R-Range`, queued
under the invariant hoist and the guarded nest): a library whose public API bounds its
inputs — `integer(0, 255)` planes, a kernel declared within its fixed-point width, a
count with a declared ceiling — lets the compiler prove every product and sum inside it
cannot overflow, and the plain operator ships in the library's ordinary build with the
checks intact everywhere the proof does not reach.  A library taking full-range
integers keeps its checks; nothing can prove them away, and nothing should.  The
measurement that says how much any of this is worth per row is the release-pass PROBE
(`LOFT_RELEASE_PASS_PROBE=1`, CLAUDE.md): the fourteen rows compiled as the release pass
would compile them, the ceiling each row's checked build is measured against —
PERFORMANCE.md § *The release-pass ceiling*.

**2026-09-14, later — HAND-OFF FOR THE QUIET BOX: `smooth`, the last judged row over the
bar.**  Written on `tuxedo` (x86-64, three checkouts sharing it) for an agent on the quiet
Ubuntu laptop; everything below that is a TIMING is to be re-measured there first.

*Where things stand.*  § V-ai shipped (`fronds` under the bar on x86-64 too — nine of ten
judged rows on both lanes), `make ci` green on 0fe0878f (4 915 tests), branch pushed.
`smooth` reads 9.5–10.2× on x86-64 by `compare.py` (a reference under its 100 ns floor) and
8.44× on aarch64; the honest converged x86 ratio was ≈ 9.6× on 09-12, **44 ns per output
point against Rust's 4**, per-call cost ~4 % of the row.  Reaching 4× on x86 needs ≈ −58 %,
and no single unit measured so far reaches that: this is a stack, ranked below by what is
measured.

*Measured on the quiet box (2026-09-14, x86-64 Ubuntu laptop, load under 1.0, binary
rebuilt at 4633de05).*  Both committed probes, three runs each, every hash `520594874`; the
two ceilings the hand-off left unmeasured were taken rustc-first on the emitted Rust of
`smooth-literal-push.bench.loft` (the exact rustc line loft runs, captured with `strace`;
ABAB against the unedited emission; `emission_audit.py` clean on the edit):

| form | ns/op | vs shipped |
|---|---:|---:|
| A — shipped record return | 2 025–2 102 | — |
| A + literal push (the `n_pt` call gone) | 1 886–1 952 | −7 % |
| A + literal + direct slot write | 1 886–1 896 | −9.5 % |
| B — tuple return (§ V-ah stages 2 + 3) | 1 210–1 300 | **−41 %** |
| B + literal push | 1 078–1 091 | −48 % |
| B + literal + direct slot write | 1 042–1 049 | −50 % |
| … + no frame prelude in `ctrl_t` / `half_chord_t` | 1 006–1 034 | −51.5 % |

The § V-ah two-kernel probe agrees: A 2 035–2 078, B 1 210–1 214 (aarch64 read −40 %, so the
two lanes agree on this unit).  **The loaded-box ranking was inverted**: the `n_pt` unit is
≈ −10 % in total, not −45 %, and § V-ah is the lead.  The profile's 19 % `n_pt` share is the
cost of the SYMBOL — it includes the two guarded slot writes, which the literal form pays
identically inline — so removing the call saves the call and the delivery guard (7 %), and
removing the validation (`strict_stores`, the `.free` test, `valid`'s header read) another
3 %.  A profile share is what a symbol costs, not what its removal saves.

*What the emission says* (`--native-release --native-emit <out.rs>` of `sm_only.loft`; read
`n_smooth_pts`, `n_ctrl`, `n_half_chord`, `n_pt`).  The k-loop is already lean on READS —
the eight `sp_a/sp_b/sp_ta/sp_tb` fields are hoisted per segment (`__vs_3..10`) and the
append is `push_record_hoisted` (§ V-t) — but the element is filled by CALLING
`n_pt(cell, x, y, elm)`, then a `store_nr` compare and a conditional `OpCopyRecord`, then
`push_record_finish`.  Per segment, `pts[ia]?`, `pts[ib]?`, `flags[ia]?`, `flags[ib]?` are
UNHOISTED `get_vector` reads: the loop calls `n_half_chord`, which takes no twin (`pts` is a
plain vector parameter to a BORROW-returning callee).  Per call: four `OpDatabase` mints
for the join buffers `__ref_1..4` (§ V-af) and four `OpFreeRef` at exit, plus the reset.
`n_ctrl` and `n_half_chord` carry the frame prelude (`cr_call_push_lean`, `CallGuard`,
`FnRefBufGuard`) because `floor_mod` is a loft-bodied `t_` method and `@FR-R-Leaf` counts
that as a user call; `n_pt` is a leaf and carries none.

*Ranked by what is measured here, with ceilings*:

| # | unit | mechanism | ceiling (quiet x86-64) | state / next step |
|---|---|---|---|---|
| 1 | **§ V-ah stages 2 + 3** — `half_chord` forwards a tuple, `ctrl` selects its fields at the return | `hoist::value_records` + `scopes` (stage 3 changes who frees) | **−41 %** on both lanes | **shipped 2026-09-14** — DESIGN.md § V-ah Built: A 2 025–2 102 → 1 504–1 563 on the two-kernel probe (−27 %); consumer `smooth` 9.5–10.2× → **4.58×** on this lane (1 740 ns against Rust's 380, 14/14 hashes agree), the last row over the bar by 0.58× |
| 2 | **the push-slot write** — the `n_pt` call inlined into the literal (−7 %) and the element's fields written INTO the push header's slot without `store_mut` + `valid` per field (−3 %, the write twin of `vec_set_hoisted_or_raise_runtime`) | emission: `hoist.rs` / `Output`, both the call form and the literal form | −10 % | the call half shipped inside § V-ah (the materialise arm); the direct-write half unbuilt, ceiling measured |
| 3 | the tuple helpers' frame prelude — `ctrl_t` is not a leaf under `@FR-R-Leaf` because a loft-bodied `t_` callee counts as a user call | `is_elidable_leaf`: a `t_` callee that is itself a leaf cannot re-enter its caller | −3 % | unprobed beyond the ceiling |
| 4 | per-call store lifecycle — four join-buffer mints + frees per activation (`op_database_inner`, `database_named`, `clear`, `enum_parent_size` ≈ 6 %) | `hoist::dead_buffers`: a minted local whose every mention the value form drops emits no mint and no free | **shipped 2026-09-14**: −27 % on the probe (1 504–1 563 → 1 110–1 144), four `malloc`/`free` pairs the profile priced at 6 %; consumer `smooth` 4.58× → **3.67×**, **ten of ten judged rows under the bar on x86-64** | the remaining per-call buffers (a `?` default's, a mixed-arm branch's) are live and stay |
| 5 | the segment loop's four unhoisted element reads (`get_vector` 4.75 %) | the hoist gate declines a header when a borrow-returning callee takes the vector; admit one for a `const` parameter the callee only reads — and after #1 the callee no longer returns a borrow | ≤ 5 % | unprobed |
| 6 | the protect bracket's origin — a `Cow<'static, str>` per bracket | runtime, one function | ≤ 3 % | moot once #1 removes the brackets |

*Order.*  #1 first — it is the only unit over 10 %, its ceiling is the same on both lanes,
and #4–#6 shrink or vanish behind it.  Then #2, emission-only.  **#1 + #2 together reach
−50 % on the kernel against the ≈ −58 % the row needs for 4× on x86**, so #3–#5 are
required, not tail; the row's own per-call overhead (≈ 4 %) is outside the kernel.

*Outcome, the same day.*  #1 and the dead half of #4 shipped (DESIGN.md § V-ah Built):
the two-kernel probe's shipped form 2 025–2 102 → **1 110–1 144 ns/op** (−45 %), the
consumer `smooth` 9.5–10.2× → **3.67×**, and the table reads *ok — 10 routine(s) judged
against the reference, all within 4.0×* on this lane — for the ten rows that HAD a
reference; the four that did not were given one the same evening and all four are over
the bar (the section above this one).  #2's direct-write half, #3 and #5
are now optional margin on x86-64; the aarch64 lane (8.44× before this) is to be
re-measured on its own box before the plan is called.

*Instruments, in order of use.*  (1) `cargo build --release --lib --bin loft` — the native
lane links the rlib, `cargo build --bin loft` does not rebuild it.  (2) A SCRATCH clone of
`loft-libs-graphics` branch `drawing-lock` (at 250b2cd here), never the consumer's own
checkout; `rm -rf bench/.loft native-auto` after every rebuild of loft or any
generation-time switch.  (3) `python3 bench/compare.py --loft <tree>/target/release/loft
--skip-interp --repeat 5` from its `drawing/` — the table; 14/14 hashes must agree.
(4) `sm_only.loft` as above; profile with `--calls --keep`, then `perf report -i
~/.cache/tmp/loft-profile/perf.data --stdio --no-children -g caller,0.5,callee
--symbol-filter=<sym>` for who calls a hot runtime symbol.  (5) The two committed probes
(`loft --native-release <probe>`; every hash must agree across kernels).  (6)
`LOFT_TRACE_VALUEREC=1` on the emit names every admission and decline (the value path
is default-on; before § V-ah it said `n_ctrl` / `n_half_chord`: *tail is not an Object
build*, and now both read `-> (f64, f64)`).  (7) `scripts/emission_audit.py <emitted.rs>`
before trusting a hand-edited emission.  **Label every number with the machine** — the two
lanes already disagree on which rows fail, and a ratio from a loaded box is not a number.

**2026-09-14, end of session — state, in one place.**

*The scoreboard.*  Nine of the ten judged rows are UNDER the 4× bar on this box (aarch64
Linux, host `lima-default`), against six when the session began: `fill_circle` 1.67×,
`fill_star` 1.76×, `hair` 2.01×, `hash` 2.55×, `wide_line` 2.93×, `composite` 3.14×,
`fronds` 3.42×, `lock` 3.44×, `lock_curved` 3.67×.  **`smooth` at 8.44× is the only row
still over it.**  Ratios drift with the reference lane between runs, so compare absolute
ns/op within a session.  **On x86-64 (host `tuxedo`, 2026-09-14) it is NINE of ten as
well, after § V-ai**: `fronds` read 4.28× there at this tip (175 920 / 41 080 ns/op — the
reference ~10 % faster than aarch64's, native ~13 % slower) and reads **3.42×** with
§ V-ai (143 180); `smooth` 9.50×.  Status § *Re-measured 2026-09-14* has the table and the
three rows whose rise over 09-12 is not yet told from lane drift.

*Shipped, in order, all on `157-native-4x`:* § V-ac (a vector parameter's header crosses
the call), § V-ad (a null-discharge buffer stops blocking the hoist), § V-ae (a filling loop
is one slice fill), § V-af (a value branch witnesses every arm's buffer), § V-ag (clearing a
store-root vector resets its store).  Each has cells, an emission or IR pin, a values guard,
a switch and a falsification; each was measured against its own switch on one binary.

*The direction changed mid-session and it is written down:*
[PERFORMANCE.md § Native vs Rust 3e](../../PERFORMANCE.md) — the remaining speed is loft
knowing what a store is FOR, not what LLVM can be told.  § 3d bounds the LLVM side (four
reachable levers; the best is measured at −20 % on `composite` and is unbuilt).  Three store
rules the rewrites already stood on were missing and are now written: `@FR-H-RootExtent`,
`@FR-O-Buffer`, `@FR-R-Reuse`.

*Open work, ranked by what is known about it:*

| item | state | next step |
|---|---|---|
| `smooth` stage 1 — § V-aa default-on | **shipped 2026-09-14** (`fnref::dispatch_arms` is the one home; corpus green with the switch on) | — |
| `smooth` stage 2 — a forwarding tail | **shipped 2026-09-14** with stage 3 (`hoist::value_shape` + the positional `site_walk`; the branch-bound local is a value local) | — |
| `smooth` stage 3 — a selecting tail | **shipped 2026-09-14**: a VIEW leaf reads as a getter tuple; the ownership change is the `OpFreeRefIfDistinct` arm (always distinct against a tuple) | an OWNED tail stays declined (DESIGN.md § V-ah Built (b)) |
| `n_pt`, 19.1 % of `smooth` on x86-64 | the CALL half shipped inside § V-ah (a copy FROM a value local materialises the tuple into the push slot) | the direct-write half (−3 % rustc-first): write through the push header without `store_mut` + `valid` |
| § 3d item 1 — assert what the header proved | ceiling measured, unbuilt | validate the vector's extent once at derivation, answer `len: 0` on failure so corruption degrades to the checked path |
| § 3d items 2–4 | unprobed | item 3 (a real slice per store) wants the same rustc-first probe |
| a tail fast path that survives holes | unprobed (§ V-ai shipped the reset-capacity half: `fronds` under the bar on x86-64 too) | a claim no hole fits still walks the tree to reach the tail; the sub-call buffers' holes (§ V-j) are what it would price — measure a ceiling on `fr_only` first |
| store ROLES as rules | unnamed | nothing says what a discharge buffer, a comprehension accumulator, a worker's read-only borrow or the const store guarantees |

*Instruments, and where they live.*  The consumer table is a SCRATCH clone of
`loft-libs-graphics` branch `drawing-lock`; the per-row probes (`wl_only.loft`,
`sm_only.loft`, `fr_only.loft`) are that clone's `bench.loft` with a one-row `main`, rebuilt
in a minute (§ V-ad records the recipe, including that `perf` needs
`kernel.perf_event_paranoid ≤ 2` and an iteration count high enough that the program, not
loft's own front end, dominates).  The one probe worth keeping is committed:
`smooth-field-return.bench.loft`, the § V-ah ceiling.

*Branch state.*  `157-native-4x`, never PR'd — the owner opens PRs.  The last full local
gate (`make ci`, x86-64 host `tuxedo`) ran on 0fe0878f — the § V-ai tip — and was green:
4 915 tests, all gates passed.


**2026-09-14 — `smooth` run down, the last judged row over the bar: DESIGN.md § V-ah
(designed, ceiling measured, NOT built).**  Of the row's own time, `ctrl` and `half_chord`
carry ~26 %, and almost none of it is arithmetic: `ctrl` returns a BORROWED VIEW of an
element of a `const` parameter, so every call pays the protect bracket (9.6 %), the
adopt-or-copy delivery and a guarded buffer free — while both callers only ever read
`.ptx` and `.pty` off the result.  The record never needs to exist.  § V-aa is the
mechanism and its BODY gate is exactly what refuses them (`LOFT_TRACE_VALUEREC=1`: *tail is
not an Object build*, for both); its USE gate already passes.  **Ceiling measured in loft**
(`sm_ceiling.loft`, the kernel written twice, identical output and hash, order-independent):
returning the two floats instead of the record is **−40 %** (1 245 → 748 ns/op), of which
**−24 % comes from `half_chord` alone**.  That splits the unit into three stages, with the
blocker first: (1) flip § V-aa default-on by making its call-site gate DECLINE the three
corpus shapes that do not compile; (2) admit a FORWARDING tail — a call to another admitted
value-record fn — which is `half_chord`, needs no ownership change, and is −24 %; (3) admit
a SELECTING tail by reading the fields at the return, which is `ctrl` and the remaining
−16 %, and which DOES change who frees (a callee's guarded free declines precisely when the
buffer is the returned value; stop the result escaping and that buffer becomes nobody's).
Separately, `n_pt` is 14.9 % of the row and is a different question: § V-p's twin applied to
a record DESTINATION, and it wants its own ceiling first.
**Stage 1 started 2026-09-14 and is two-thirds done** (DESIGN.md § V-ah, the table): the
corpus with the switch on reports 376 compile errors in three classes, not the three
independent shapes the note described.  The fn-ref DISPATCH class is 218 of them — the
`match` a `CallRef` emits takes its arms from a signature scan, so all arms share one ABI —
and the LIFT-temp class is the next, a result whose set lowering reads `.store_nr` off it
where no IR walk can see.  Both are now declined and the corpus is down to **four scripts**,
all the same remainder: a lambda reaching a dispatch without a `FnRef` node or a readable fn
variable type.  **That remainder is a refactor, not a patch** — the dispatch's arm set lives
inside `Output::output_call_ref`, and the gate must read the SAME scan rather than a second
spelling of it (§ Design: P8's drifted lists are what asking twice looks like).  A blunt
"any fn-ref declines everything" was measured and rejected: the drawing bench has one.

**2026-09-14 — § V-ag SHIPPED, the first unit built under the 3e direction, and `fronds` is
UNDER THE BAR** (DESIGN.md § V-ag).  Re-profiling the row on the current tip put **27.9 %**
of it in the free tree alone, nearly all of it on one path: a recycled return buffer being
cleared, releasing the previous call's elements one at a time into a red-black tree.  The
fact loft had and the allocator did not is the one `@FR-H-ClearRelease` already tests — the
vector is its store's ROOT, so it owns the store's whole extent and every block in it is
dead at the clear.  So the release is now ONE store reset with the two records
re-established, not N deletes.  `fronds` **244k → 156k ns/op (−35.8 %)** on a switch A/B,
**4.67× → 3.42×** in the consumer table, 14/14 hashes; `smooth` is the only judged row still
over the bar.  Switch `LOFT_NO_STORE_RESET_CLEAR`.  Two findings recorded: skipping the
release entirely (`LOFT_NO_CLEAR_RELEASE=1`) measures NOTHING, because it trades deletes for
claims rather than avoiding tree work — the cost is the churn, so the cure had to reset
rather than skip; and a cleared vector must stay PRESENT, found by reading rather than by a
test, since the compiler folds `if <vector local>` to true and no program can currently ask.
**The next question this row asks is a different unit:** the free tree is still there for
the stores that genuinely need one.

**2026-09-14 — the direction, and it is not LLVM: PERFORMANCE.md § Native vs Rust 3e.**
The LLVM levers are bounded and one of them is now measured (3d below); the remaining
factor comes from loft knowing what a store is FOR and spending that itself.  Read against
the arc's own results this is what already happened: every unit that moved a row by more
than a few percent won on a fact LLVM cannot have — this loop writes no store, this callee
writes only in place, this allocation is a discharge buffer, this store is a return buffer
or an appended element or a branch arm's delivery.  A store has a ROLE, a CONTENT TYPE, a
LIFETIME and an ACCESS PATTERN, all known to the emitter and all discarded by a runtime
written for the general case.  `fronds` is the standing proof: about 39 % of the row is
allocator and free-tree machinery serving temporaries that are born and die inside one
activation, and no annotation reaches that.  The queue under this heading is
[P8](../../PERFORMANCE.md) for what an op does to a store,
[N1](../../PERFORMANCE.md) for a collection that never needed to be a store, and role and
lifetime work that has no design doc yet.  Note that step 1 of the 3d ranking converged on
the same principle by itself: its content is not a hint to LLVM but a loft-level decision,
check the extent once where the header is derived.

**2026-09-14 — the LLVM question, evaluated and written up: PERFORMANCE.md § Native vs
Rust 3d.**  Whether `rustc` can be told enough about loft's memory to hoist for us, and
which regions can carry which claim.  The short answer is that region marking is worth
doing but is not the route to the loop hoists — 3c already measured the guard, not
aliasing, as the barrier that bites — and that the unit of disjointness is the STORE, not
the `DbRef`, because two references inside one store overlap on purpose.  Two findings
worth carrying: a `par` region is the EASY case (what it shares across threads is what it
locks read-only for the duration), and the region that refuses every claim is the LAZY
store rather than the mapped file, because there a read faults and writes.  The ranked
queue it leaves, cheapest first: assert the facts a header already proved
(`std::hint::assert_unchecked`, which the tree uses nowhere today), index `allocations`
unchecked on the same proof, derive a real slice per store at the header, then LTO.  The
first is a SOUNDNESS lever and its falsifier is `LOFT_HOIST_VERIFY=1`.

**Item 1's ceiling is now MEASURED and it is the biggest number left on the board**
(PERFORMANCE.md § 3d, the table): removing the three redundant tests a hoisted element
read still pays reads `composite` **−20.4 %**, `render_marks` **−13.3 %**, `render_lock`
**−13.1 %**, `lock` **−10.3 %**, `lock_curved` **−7.8 %**, every other row inside the
lane's swing, 14/14 hashes on all four passes.  The ATTRIBUTION is what shapes the unit:
the one test that is trivially sound to drop (the store-table index) buys ~5 % on one row
and nothing else, so there is no cheap version — the win is in the position-overflow test
and the in-store bounds test, one of which is also the corruption net.  The design that
follows is the fill's shape: validate the vector's extent against the store ONCE at header
derivation, answer `len: 0` on failure so a corrupt length degrades to the existing cold
path rather than to undefined behaviour, after which `0 <= from < len` implies all three
per-element tests.  The plumbing is the element size into the derivation, which the
emitter already knows from the element-address op it collected the candidate from.  Ready
to build; items 2 to 4 are still un-probed.

**2026-09-13 (late) — § V-af shipped: the smooth profile, run down.**  `perf` on this box
(`sm_only.loft --n 2000000`, the recipe of § V-ad) read a third of the row as runtime
store lifecycle: `database_named`/`claim_block`/`op_database_inner`/`set_default_value_
nullable` under `n_pt` via `n_half_chord` (~12 %), `free_named` from the loop (3.7 %),
the borrowed-view protect bracket around `ctrl` (8 %), an uncached `getenv` in
`clear_vector_release` (3.8 %) and a per-allocation `strncmp` in `enum_parent_size` (3 %).
The two uncached costs were fixed in the runtime (−8 %); the lifecycle was the unpaired
`if`-join of `sp_ta`/`sp_tb` (§ V-af, −26 %).  `smooth` is 9.0× in the consumer table
and 1 286 ns/op standalone against Rust's ~180: **what remains is the protect bracket
around `ctrl` (8 %, a callee whose only frees are its own discharge buffers should need
no bracket), `n_pt`'s remaining per-call field writes through the buffer, and the
harness's own cold-row effect** (the compare row reads 1 620 where the standalone reads
1 286 — the `--n 50` clock, README § `smooth` run down).  § V-aa is ruled out for this
row (+1–3 % switched on).

**2026-09-13 (night) — § V-ae shipped: the FILL idiom, Unit 2 of the fills' profile and the
second unit for `wide_line`** (DESIGN.md § V-ae).  `wide_line` 2.91×, `fill_circle` 1.70×,
`fill_star` 1.79× on this box — the row the day started on at 5.35× is under the bar, and
the two fills are the best rows on the board after `hair`.  Two shapes the recogniser had
to learn from the consumer rather than the probe: the range test is a bare `Break`, not a
block, and a library module's loop body carries a `Line` marker statement the probe did
not (`LOFT_TRACE_FILL=1` found both in one emission each).  What the fill does not take,
by construction: a strided index, a value that reads the vector or depends on the loop
variable, a `??` value, a record-element field (`v[i].f = c`, a strided fill).  Next for
the two rows still over the bar: `fronds` is the allocator class (queue items 4b / 5),
`smooth` the per-point record call (§ V-aa's gate, then mods 2 and 3).  For `wide_line`
itself, `n_edge_x` as a plain call over an optional element view (§ V-p c6) is the one
named remainder, ~7 % of the row before today.

**2026-09-13 (evening) — § V-ad shipped, the first unit aimed at `wide_line`** (DESIGN.md
§ V-ad).  The row's perf profile on this box (the `wl_only.loft` recipe in DESIGN.md § V-ad)
read `n_polygon_generic` 63 % self with `pil_hline` inlined into it, the un-inlined
`n_edge_x` 7 %, the store machinery the crossing loop could not hoist 12 %, and the rounding
helpers 3.5 %.  The 12 % is gone (−29 % on the row, because the hoisted reads also let LLVM
fold the surrounding arithmetic); what remains is `pil_hline`'s fill loop — **Unit 2, the
FILL idiom**, is now the next unit for this row and for both fills: `for i in a..=b { v[i]
= c }` as one bounds test plus a slice fill.  `n_edge_x` stays a plain call because its
argument `pg_cur` is an OPTIONAL element view (§ V-p c6): a twin over an optional-view
argument is a second, smaller unit.

**2026-09-13 (later) — § V-ac shipped on the same branch** (DESIGN.md § V-ac): item 2 of
the `lock_curved` ranking below is done, and the row read 3.63× here (aarch64 Linux, host
`lima-default`; the lane's rows differ from Apple's and from the 09-12 x86-64 table, so
compare within a machine).  What the ranking leaves, re-ranked for the next session:
**Unit 2, the FILL idiom** (below: `for i in a..=b { v[i] = c }` as one bounds test plus a
slice fill — 60 % of `fill_circle`, a third of `wide_line`, the only judged rows still
over the bar on Apple; cells first, including a non-contiguous index and a value that reads
the vector), then **item 1** (the value-record gate's three declining call shapes, −4.1 %
on `lock_curved`), then **item 3** (the raster loop's checked arithmetic LLVM cannot hoist,
and the same-length fact for a struct literal's comprehension fields).  A twin now reaches
a callee through ANY pure path argument, so a profile that shows a vector-parameter reader
hot inside a hoisting loop is a cell to add, not a unit to build.  loft#1525 (the
inclusive-to-maximum hang) is fixed on the sibling branches `macos-probe` and
`tuxedo-1502-pr-followups` (b8c7d39a), not on this one — it lands with their join.

**2026-09-13 — § V-ab shipped on a fresh branch off `main` (ec4a9522, which had absorbed
the whole branch in #1524).**  The unit was the fills' Unit 1 (the variable-start counted
loop's per-iteration null test); DESIGN.md § V-ab has the numbers.  Two things for the
next session: **Unit 2, the FILL idiom**, is now the whole of what `pil_hline` costs —
`for i in a..=b { v[i] = c }` is a bounds-checked scalar store per element at 1.16 ns
against Rust's 0.10 (a `memset`/`fill` LLVM recognises), 60 % of `fill_circle` and a
third of `wide_line` by tick share; the emitter would have to recognise a loop whose only
store write is an in-place set at `base + i` of an invariant value over a hoisted header
and emit ONE bounds test plus a slice fill (M; write the cells first, including a
non-contiguous index and a value that reads the vector).  And **loft#1525** (an inclusive
range to the type maximum never terminates, both backends, pre-existing) is filed with
its design question — it should be fixed before the PR, not shipped open.  The
`compare.py` A/B recipe worked as documented (base worktree built with `cargo build
--release --lib --bin loft`, caches cleared on both sides, the bench emission checked for
the new form before trusting the numbers).

**`lock_curved` evaluated off its release-tier emission (2026-09-13; `--native-release
--native-emit` of the bench, the row's 09-12 profile beside it), ranked:**

1. **§ V-aa is OFF by default** (`hoist::value_record_disabled`: opt-in `LOFT_VALUE_RECORD=1`
   since 2026-09-12) while CLAUDE.md, DESIGN.md and this README said default-on with a
   `LOFT_NO_VALUE_RECORD` switch nothing reads — corrected the same day.  By default
   `brush_sample` returns `DbRef` through its buffer: per resolved pixel four float writes,
   four `get_float` reads behind a `rec == 0` test, a store-identity free check.  The code
   comment names why: gate (2) fails the corpus on three call shapes (result used as a
   `DbRef`, a buffer argument kept, a `match` joining a tuple and a record arm).  Unit: make
   the gate DECLINE those shapes, prove it over the corpus, flip the default.  Re-measured
   2026-09-13 on the § V-ab tree (`compare.py --repeat 5`, caches cleared, same run for both
   sides): `lock_curved` 2 293 260 → **2 198 240 ns/op (−4.1 %)**, `fill_star` and `fronds` flat,
   `smooth` +3 % (lane noise) — so the row's honest ceiling from this unit is ~4 %.
2. **`brush_sample`'s four `img[…]?` reads are each a full store resolution** —
   `get_vector` (allocations lookup + bounds) then `store(&db).get_int` then the discharge —
   while the caller holds `br` invariant.  The § V-p twin passes a RECORD parameter's
   headers; `img` is a plain vector parameter (`br.img` at the call) and never qualifies.
   Extend the twin to a vector parameter whose argument is a hoistable path: four
   header-served reads.  **CEILING HAND-MEASURED 2026-09-13: −5.7 %** (rustc-first — the
   release-tier emission of a `lock_curved`-only bench with `LOFT_VALUE_RECORD=1`, hand-edited
   so `n_brush_sample` takes `br.img`'s header derived once beside the resolve loop's other
   headers and reads through `get_elem_hoisted::<i64,_>`, linked with the same flags loft
   uses (`-C opt-level=3 -C codegen-units=1`, `--extern loft=…libloft.rlib`, the matching
   `loft_ffi` rlib): base 2 256 582–2 263 300 → **2 127 332–2 141 152 ns/op**, ABAB × 3,
   spread < 0.3 %, hash `2a3aa61` every run).  That beats the value-record unit's −4.1 %, so
   it is the next unit to BUILD: cells first (a parameter re-bound in the callee, a callee
   that pushes to its parameter, two callers passing different paths, a null vector, a
   non-hoistable argument), then the admission in `hoist::callee_inputs`.  **BUILT the
   same day as § V-ac**: −6.3 % on `lock_curved` and −8.7 % on `lock` (this box), the
   second admission it needed being the return-buffer callee, which § V-p had refused.
3. **The raster inner loop's checked integer arithmetic LLVM cannot hoist**: `rs_idx =
   op_add_int_nullable(op_mul_int(op_min_int(yy, ly0), llw), op_min_int(xx, lx0))` — the row
   term invariant across the inner loop, the column term an induction variable, and each op
   carries a guarded overflow note (a side effect), so LICM declines.  Same for the bound
   `rs_x1 + 1`, a checked add per iteration.  Plus eight bounds tests per written pixel
   against seven same-length `Lay` vectors built from one `ll_n` — a same-length fact for a
   struct literal's comprehension fields would let one test serve eight.
4. Invariant divisors (`rs_l2`, `ll_period`, `ll_crest`): the ADVICE lint already named
   above, never a rewrite (reciprocal is not bit-identical; the bench validates by hash).
   The rest of the resolve loop's machinery — literal shift-range tests, the `/255.0` zero
   test, NaN-aware compares on operands that cannot be proven non-NaN — folds under LLVM.

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
GitHub gate run 34344413805 is GREEN on the § V-r tip (c253383a: § V-p, § V-q, § V-r, the
interpreter link re-point, the formal chapter's hoist-state rules and their history); build
on that tip.

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
on the § V-q commit before building on it.  **§ V-r SHIPPED the same day** (DESIGN.md
§ V-r): `scripts/emission_audit.py` validates a `--native-emit` output against the
hoist-state rules — `R-State` (one holder per path expression per frame, every hoisted
read / write / push / `.len` / twin argument naming a live holder bound for its path),
`R-Refresh` (no template append, pre-alloc or `vector_add` on a held path), `R-Inputs` (a
twin handed only live holders) — and `tests/emission_audit.rs` runs it over the six cell
corpora, the three hoist guards and `bench/12_drawing/bench.loft` in the gate, requiring a
holder per corpus and the refusal of a doubled emission.  Proven to fail on the collector
order that produced the double holder, at emission, with no run.  **Next:** the hoist-STATE
builder (`hoistable` holds `R-State` by collector ORDER today; one `LoopState` with a single
insert path, behaviour-preserving — byte-identical emission over the corpora, and the audit
green — M), then the rows: `smooth` 14.3× / `fronds` 12.1× (allocation class) and
`wide_line` 5.3×.

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
- **The A/B trap on the consumer bench: a GENERATION-time switch needs the cdylib
  cache cleared.**  `bench/.loft/cache/` is keyed by content, so re-running
  `compare.py` with `LOFT_VALUE_RECORD=1` (or any `LOFT_NO_*` the emitter reads)
  re-measures the SAME binary and reports a flat result that reads as "the unit is
  worth nothing".  `rm -rf bench/.loft native-auto` on each side.  Runtime switches
  (`LOFT_NO_CLEAR_RELEASE`) are exempt — they live in the linked `libloft`, which is
  why one moved a row while the other did not, and that asymmetry is the tell.
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
| **V-r** — the EMISSION AUDIT: `scripts/emission_audit.py` validates a `--native-emit` output against the hoist-state rules (`R-State` one holder per path per frame, `R-Refresh` no mover on a held path, `R-Inputs` a twin handed only live holders); `tests/emission_audit.rs` runs it over every hoist corpus and the in-repo bench | [DESIGN.md § V-r](DESIGN.md) | a corpus audits with a violation; a corpus resolves no holder; the doubled emission is not refused | **Shipped 2026-09-09** — flags the § V-p × § V-q double holder at emission when the collector order is re-introduced; every corpus clean |
| **V-s** — a record-appending loop hoists its invariant scalars: the mint group (`OpPreAllocVector · OpNewRecord · sets or a retbuf callee · OpFinishRecord`, § V-d's `OpCopyRecord` delivery included) admitted as a mover under `(R-Alias)`, writes into the FRESH element evicting nothing (`@FR-R-Mint`); a keyed container's same-named ops stay blocking (admission asks the TYPE) | [DESIGN.md § V-s](DESIGN.md) | a c9/c10 cell answers the stale value (the falsified sabotage); `tests/mint_hoist.rs` no longer sees the hoists; a consumer hash disagrees | **Shipped 2026-09-10** — fifteen cells exact on both backends, emission pinned; `smooth` −8 % (interleaved ABAB best-of-3), `fronds`/`lock_curved` within noise; switch `LOFT_NO_MINT_HOIST` |
| **V-j (move)** — the move-append: in `for f in call(…) { v += [f] }` with the loop var's single post-binding use that one append, the call's buffer is PLACED as a record in `v`'s own store (`Stores::place_record_in`), the append relocates the element's bytes and zeroes the source (`move_record_shallow`; store-identity dispatch, deep copy the fallback), the buffer's free record-level (`free_record_in`) (`@FR-R-MoveAppend`); declines on read-after, double append, named source, nested loop, rebound dest, non-struct element | [DESIGN.md § V-j](DESIGN.md) | the use-after sabotage turns c2 red (`3 409 45` → `3 409 0`); `tests/move_append.rs` no longer sees the pairs; a cell leaks or answers wrong under `LOFT_POISON=1` on either backend | **Shipped 2026-09-11** — sixteen cells exact both backends under poison + leak checks; ceiling re-measured by hand first (−11.3 %) and reached exactly: `fronds` 396–400k → 354–359k ns/op, consumer 8.36× → 7.88×; switch `LOFT_NO_MOVE_APPEND`.  Same-day addendum off the red gate: the placed record now dies at the LOOP's exit (scope-end freed a recycled slot — the `native_scripts` corruption), and a struct named like a stdlib typevar declines (the by-name wrapper lookup finds the generic template; loft#1519 — root-fixed 2026-09-12, so the decline is now a net that no longer fires) — cells c19–c21, 21/21 exact |
| **V-w** — a self-reading vector LITERAL rides TEMPS: each destination-reading part is evaluated once before the build (`(I-Comp)`/`@FR-O-Detach` kept) instead of snapshotting the whole vector — the prefix-sum accumulator drops O(n²) → O(n); gates: literal lowering only, bare unlinked var, scalar elements, primitive/`#pure` calls; the snapshot stays for user calls, records, links, fields, comprehensions | [DESIGN.md § V-w](DESIGN.md) | the temps' Sets dropped crashes every cell; `tests/selfread_literal.rs` route pins; q8's comprehension keeps its snapshot | **Shipped 2026-09-11** — nine cells, both backends, I-Comp hand-oracle; 2 000→14 µs / 8 000→54 µs (linear); `lock_curved` within noise (63 points — the class, not the row). Full suite 4 706/4 708 (the cold-cache timeout class beside it); the one failure was the COMPOSITION improving: V-q's c3 (the accumulator that reads the pushed vector) now EARNS its push header — pre-V-w the per-iteration snapshot copy blocked the hoist — pin re-taught (0,0,0) → (1,1,0) |
| **V-v** — free-block FOOTERS: a free block carries −n at both ends (the tree node's color repacked into bit 31 of its RIGHT link to clear the half-word), so `delete` coalesces BACKWARD in O(1) — footer names the predecessor, header must agree, the free TREE must confirm (`@FR-H-FreeFooter`, formal/heap.md); one-word predecessors keep the lazy sweep; one helper (`set_free_header`) owns all eleven free-header writes; pre-footer images re-footed on open | [DESIGN.md § V-v](DESIGN.md) | the tree-confirm removed merges a freed block into a live claim (the fake-footer guard goes red); `store::tests` 50/50; a cell leaks or answers wrong under `LOFT_POISON=1` | **Shipped 2026-09-11** — `fronds` −7.7 % (288–295k ns/op standalone); retires the § V-j P2 `coalesce_free` cliff (12 % of the row); switch `LOFT_NO_FREE_FOOTER` (runtime, both backends) |
| **V-u** — a result vector ADOPTS the return buffer: a shape-A fn (separate `__retbuf` attr) whose every delivery sources ONE never-rebound result local aliases that local to the buffer (`hoist::ret_adopt`), the `one_buffer_vec_copy` pair emits as nothing while the block's frees and the entry clear stay, `OpReplaceVector` self-detects, a § V-j placement re-targets (`@FR-R-RetAdopt`); the witness-promoted ABI is already copy-free and declines | [DESIGN.md § V-u](DESIGN.md) | the un-scoped blanking corrupts the fronds probe; the whole-block collapse leaks 2 stores in the V-j corpus; `tests/retbuf_adopt.rs` no longer sees the adoption; a cell answers wrong under `LOFT_POISON=1` | **Shipped 2026-09-11** — seven cells exact both backends, leak-free under poison; `fronds` −9.5 % (350–357k), `smooth` −14.4 % (1 887 ns/op) standalone; switch `LOFT_NO_RETBUF_ADOPT` |
| **V-t** — a record append EMITS through the push header: an admitted mint whose element is a plain no-heap struct takes the header's next slot (`Stores::push_record_hoisted` — no `record_new` dispatch, no default prefill: the IR's literal lowering writes every field explicitly, a declined delivery is a whole-record copy) and the finish is the length bump (`push_record_finish`), exactly where `record_finish`'s was (`@FR-R-PushRec`); heap-owning, `__nullable` and keyed elements keep the templates | [DESIGN.md § V-t](DESIGN.md) | a growth cell answers empty (the falsified sabotage: c1 `100 0 25 9801 2475` → `0 0 0 0 0`); `tests/record_push.rs` no longer sees the fused forms; the audit accepts a template mint on a held path; a consumer hash disagrees | **Shipped 2026-09-11** — fifteen cells exact on both backends, emission pinned; standalone `smooth` −32 % (ceiling hand-measured at −37 % first); consumer `smooth` 11.0× → 4.6×, `fronds` 11.1× → 8.4× (this box), 14/14 hashes agree; switch `LOFT_NO_RECORD_PUSH` |
| **V-x** — an invariant loop-body vector LITERAL builds once per activation: the local pre-declared at fn top is its own once-flag, the declaration (one wrapped `Set`, or the flat `OpDatabase · Set · pushes` run bracketed by `hoist::flat_lit_member`, the shared predicate) guarded on it being unbound; gates: no-heap scalar elements, invariant parts (literals / pure ops / never-reassigned scalar params / scalar-getter reads of value-const record params), the `(Const-Value)`-mandated alias gate (every pd-reaching var fresh-store or const-getter-only), reads-only uses (indexed use declines — OpGetVector is context-blind) | [DESIGN.md § V-x](DESIGN.md) | the sabotage turns c5 red (`16` → `30`: iteration 0's write carried); `tests/literal_hoist.rs` no longer sees the guards; a cell answers wrong under `LOFT_POISON=1` on either backend | **Shipped 2026-09-11** — eleven cells exact both backends under poison; `fronds` −5.8 % (279–285k ns/op); switch `LOFT_NO_LITERAL_HOIST` |
| **V-y** — the complete-write prefill ELISION: a literal group the emitter proves writes every schema field position (declared defaults, sentinels, the variant tag — the parser's lowering is complete by construction) calls the no-prefill twin (`OpDatabaseNP`/`OpNewRecordNP`); a vector store's group is width-equal (the collection-field prefill is the one u32 its own `OpSetInt4` writes) and `place_record_in`'s prefill becomes one explicit len-zero (the callee entry-clear ABI covers it); the Z∩P redundancy ships as ONE argument | [DESIGN.md § V-y](DESIGN.md) | the admit-all sabotage crashes the V-j corpus at c17 under `LOFT_NO_ZERO_CLAIM=1` (the production default masks it — the first falsifier could not fail and was replaced); `tests/complete_write.rs` no longer sees the twins; a cell answers wrong under poison or the stale-arena lever | **Shipped 2026-09-12** — eight cells exact both backends under poison + the lever; `fronds` −4.8 % (264.9–272.7k ns/op, 9 twin sites); switch `LOFT_NO_COMPLETE_WRITE` |
| **V-z** — the ELEMENT-FIRST build: a local vector consumed exactly once as a record-literal field of an append is built inside the appended element — minted at the first temp's declaration (the length bump stays at the finish, so it is invisible until the append), each temp bound to the element's own field slot, the paired copies and temp stores gone; § V-u's adoption composes (a callee delivers its record straight into the element) | [DESIGN.md § V-z](DESIGN.md) | the compound sabotage (paired-offset check dropped + prefill-less prelude) crashes the corpus under `LOFT_NO_ZERO_CLAIM=1`; `tests/element_first.rs` no longer sees the early mints; a cell answers wrong under poison or the lever | **Shipped 2026-09-12** — eight cells exact both backends under poison + the lever; fronds standalone 265k → **186.4–199.7k ns/op**, the hand ceiling (−27–29 %) met exactly; switch `LOFT_NO_ELEMENT_FIRST` |
| **V-aa** — **default-ON since 2026-09-14** (`LOFT_NO_VALUE_RECORD=1` restores the buffer; it was opt-in from 2026-09-12 while the call-site gate did not hold over the script corpus — 376 rustc errors, 218 of them the fn-ref dispatch — until § V-ah stage 1 gave the gate the emitter's own arm scan) — the VALUE-RECORD return: a plain no-heap record of ≤6 scalar fields comes back in REGISTERS (a Rust tuple) instead of through a return buffer — the path tuples already took; three gates (shape, every call site reads fields, the body builds via `Object`), and two boundaries that keep the record contract (the live-reload arm reads the fields back; a cdylib bridge materialises into the destination it owns, so the C ABI is unchanged and in-library calls take the value path) | [DESIGN.md § V-aa](DESIGN.md) | the use gate removed = 8 rustc errors, the body gate removed = 14; `tests/value_record.rs` no longer sees the tuple returns; a cell answers wrong under poison or the switch | **Shipped 2026-09-12** — nine cells exact both backends under four levers; the call 1.65×, `smooth` −25 %, `lock_curved` −3 %; switch `LOFT_NO_VALUE_RECORD` |
| **V-ab** — the counted loop's SECOND COUNTER: a `for` whose start is not a literal seeds `next` AT the start, tests it, yields it into `i#index`, then steps it — no null-encoded "not started yet" state on either backend (P3b covered the literal start; `lo - 1` is unrepresentable at the type minimum); reverse exclusive seeds the one counter at `till`; switch `LOFT_NO_NEXT_COUNTER` | [DESIGN.md § V-ab](DESIGN.md) | `tests/next_counter.rs` (emission per cell + the switch), `tests/scripts/157-next-counter.loft` (20 value cells, sabotage-falsified), the P3b matrix byte-identical | **Shipped 2026-09-13** — bare loop 0.79 → 0.54 ns/iter, fill 1.41 → 1.16 ns/write, interpreter −11 %; consumer `fill_circle` −3.3 %, `fill_star` −2.2 %, `wide_line` −2.3 % |
| **V-ac** — a VECTOR parameter's header crosses the call: a § V-p header input is keyed on the argument's pure PATH (`br.img`, `h.cv` + the callee's offsets — `hoist::input_header_at`, the one definition the loop's candidate and the twin call ask; `hoist::substitute_path` re-spells the callee's read over the argument), a return-buffer writer is admitted with its buffer's type as its write set, and the return-buffer DELIVERY site asks the twin question too; same switch `LOFT_NO_CALLEE_INPUTS`, `LOFT_HOIST_VERIFY=1` the falsifier | [DESIGN.md § V-ac](DESIGN.md) | a k-cell moves; `tests/callee_inputs.rs`'s § V-ac predictions no longer see a twin or a twin call; the two-paths cell answers b's elements for a (the falsified rotation); a consumer hash disagrees | **Shipped 2026-09-13** — 21 cells exact on both backends under six levers; hand ceiling −5.7 % met and passed: `lock_curved` −6.3 %, `lock` −8.7 %, `render_lock` −2.9 % (aarch64 Linux, ABAB, 14/14 hashes) |
| **V-ad** — a null-discharge DEFAULT BUFFER's allocation (`e = tbl[i]?` on a vector of all-scalar records mints the absent element into a hidden `__ref_p2_N`) is admitted at the header gate and read as the record's type whole by the scalar tier (`hoist::null_buffer_alloc`, `@FR-R-InPlace`'s hidden-buffer allowance); before it the never-taken arm declined every header in `polygon_generic`'s crossing loop; switch `LOFT_NO_NULL_BUFFER_HOIST`, `LOFT_HOIST_VERIFY=1` the falsifier | [DESIGN.md § V-ad](DESIGN.md) | a d-cell moves; `tests/null_buffer_hoist.rs` no longer sees the headers, or sees one for a heap-owning record; a consumer hash disagrees | **Shipped 2026-09-13** — 12 cells exact on both backends under the verifier, the switch and poison; `wide_line` −29 % (5.35× → 4.02×), the fills −15/−19 % (aarch64 Linux, 14/14 hashes) |
| **V-ae** — the FILL idiom: a counted loop whose body is ONE in-place scalar set at `invariant + i` of an invariant value over a held header emits one guarded `Stores::fill_hoisted` (a range test, then `Store::fill` — one bounds check per end, a vectorisable store loop) with the per-element loop as the fallback for every range the fill declines and the counters left as the loop leaves them (`hoist::fill_loop`, `Output::fill_fast_path`, `@FR-R-Fill`); switch `LOFT_NO_FILL_HOIST`, `LOFT_TRACE_FILL` names a decline | [DESIGN.md § V-ae](DESIGN.md) | an f-cell moves; `tests/fill_hoist.rs` no longer sees a fill, its guard or its tail; the count sabotage turns the in-range cell red; a consumer hash disagrees | **Shipped 2026-09-13** — 16 cells exact on both backends under the verifier, the switch and poison; `wide_line` −23 % (4.02× → 2.91×), `fill_circle` −48 %, `fill_star` −43 % (aarch64 Linux, 14/14 hashes) |
| **V-af** — a value BRANCH of buffer-delivering calls (`v = if c { mk(i) } else { mk2(i) }`) witnesses every arm's hidden buffer (`scopes` pairing per tail call, `@FR-O-Complete`): the buffers are allocated once by `reuse_record_buffers` and the local's scope-exit free is the existing multi-buffer `OpDistinctStore` ladder; an IR fact, both backends; switch `LOFT_NO_JOIN_BUFFER_WITNESS`, `LOFT_STRICT_STORES=1` / `LOFT_POISON=1` the falsifiers.  Beside it: the uncached `LOFT_TRACE_CLEAR` read and the per-allocation prefix compare removed from the runtime hot path | [DESIGN.md § V-af](DESIGN.md) | a j-cell answers wrong or reports a use-after-free under strict stores; `tests/join_witness.rs` no longer sees the preamble allocations or the ladder; a consumer hash disagrees | **Shipped 2026-09-13** — 12 cells exact on both backends under strict stores, poison and the switch, store allocations 85 → 45; `smooth` −26 % standalone, 10.5× → 9.0× in the consumer table, allocations per call 33 → 17 |
| **V-ag** — clearing a store-ROOT vector RESETS its store: the shape `@FR-H-ClearRelease` already tests says the vector owns the store's whole extent, so the release is one `Store::init` plus the two records re-established (the wrapper by a claim, the vector by `pre_alloc_vector`) instead of a delete per element into the free tree; both claims bump, and `claim`'s `bump_tail` fast path is restored.  The first unit under PERFORMANCE.md § 3e.  Switch `LOFT_NO_STORE_RESET_CLEAR`; `LOFT_STRICT_STORES=1` / `LOFT_POISON=1` the falsifiers | [DESIGN.md § V-ag](DESIGN.md) | an r-cell answers wrong or reports a use-after-free under strict stores; skipping the root re-claim (the falsified sabotage) panics in `vector.rs`; a consumer hash disagrees | **Shipped 2026-09-14** — 13 cells exact on both backends under five levers; `fronds` −35.8 % on a switch A/B, **4.67× → 3.42× (under the bar)**, every other row flat |
| **V-ai** — the reset buffer keeps the capacity it reached: § V-ag's reset re-establishes the root vector at the capacity the previous fill reached (`vector::reached_capacity`, read off the record before `Store::init`; `vector::vector_capacity` the one home of the inverse formula), so the growth ladder runs once per buffer and the freed rungs that took every later claim into the free tree are never made.  Switch `LOFT_NO_RESET_CAPACITY`; `LOFT_TRACE_CLEAR=1` prints each reset's capacity | [DESIGN.md § V-ai](DESIGN.md) | a cell answers wrong under strict stores or poison; `tests/reset_capacity.rs` sees the minimum with the unit on, or a rung with it off; a consumer hash disagrees | **Shipped 2026-09-14** — nine hand-derived cells exact on both backends; `fr_only` −22.5 % on a switch A/B, consumer `fronds` **4.28× → 3.42× (under the bar on x86-64)**, every other row flat |
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
| 4 | **`fronds`' deep-copy class as a MOVE** — SHIPPED 2026-09-11 (the § V-j move row above): ceiling re-measured at −11.3 % on the § V-t runtime and reached exactly | DESIGN.md § V-j (SHIPPED addendum) | `fronds` −11 % (done) | shipped |
| 4b | **`fronds`' allocation class** — the sides literal, the two builders, the per-sub-array spec; CEILING MEASURED at −9 % by a source-level variant (−64 % stores, hash exact) — small emitter items, not the half the census suggested | DESIGN.md § fronds | `fronds` −9 % | S each |
| 5 | **frame-local record temporaries** — 12 of `smooth`'s 14 remaining stores are `pt(…)` results bound to locals; the scalar-tangent probe gained 11 % while DOUBLING the helper calls, so a temporary that lives in the frame takes at least that | § V-g's ceiling probe | `smooth` 19× → ~14×; the same class in every routine that names a struct temporary | M–L |
| 6 | **runtime ownership at calls** — the protect/unprotect bracket (3.3 %), the free path (`free_named` + `close_file_handle`, 5 %), the join guards; each is a decision § V-g showed can move to compile time | the post-V-g profile | per call, every row | M each |
| 7 | **the vector append path** — `vector_add`, `vector_append`, `vector_finish`, `get_vector`, `length_vector` ≈ 19 % of `smooth` | the post-V-g profile | the store's general allocator vs `Vec::push`; the bump claim (shipped, DESIGN.md § fronds) took the free-tree walks out of the fresh-store case | M |
| 8 | **LTO into the rlib** — only `#[inline]` functions cross the program/runtime boundary | P2's open probe | non-inline runtime calls per element | probe first |
| 9 | **profile the pixel rows** — `lock` 7.8×, `composite` 11×, `wide_line` 8.9× were never profiled; only `smooth` was | — | unknown until measured; do before any pixel-side unit | S |
| 10 | **P4c record scalars** — both the gate bench and the consumer's raster loop already hoist by hand | `raster_segment`'s `rs_ly0 = lay.y0` | small for this consumer; real for the next | S–M |
| 11 | **transitive-leaf frame elision** — N4 elides the prelude (`cr_call_push_lean`, `CallGuard`, `FnRefBufGuard`) on a LEAF; a function whose callees are all leaves cannot recurse either, so the lean tier may elide its prelude too.  `at(s, i)` fails N4 only because `size` is a loft-bodied stdlib `t_` fn, and `matches_at`, `word_boundary`, `skip_space`, `find_option` and the readers fail it the same way | hand-removed in the parse bench's release emission: 39.8–42.4 k → **34.5–40.0 k ns/op**, hash `33f6d2b8`, ABAB ×5 (@PLN164 § Re-profiled *The correction*, 2026-09-16); the rlib's `byte_at` hand-inlined beside it moved nothing | **SHIPPED 2026-09-17** (`@FR-R-LeafChain`, lean tier only, `LOFT_NO_LEAF_CHAIN=1` the bisect step): the parse emission's lean pushes 55 → 7, the row 38.8–41.7 k → **33.5–34.1 k ns/op** (−12–14 %), `perf stat` instructions −19.1 %, cycles −12.5 %, hash `33f6d2b8`, leak- and strict-store-clean; cells `bytecode-comparisons/N4b-leaf-chain-cells.loft`, pins `tests/leaf_chain.rs` | S |

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
