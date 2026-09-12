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
2026-09-12 are the **Apple** lane; the one directly under this paragraph is **x86-64
Linux** (this dev box).  The two do not compare row-for-row and neither is wrong: the
bar is *within 4× of the Rust reference on the same machine*, and which routines clear
it differs by target.  Label every future row with its machine.

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

**Re-measured 2026-09-12 on the § V-z tip (d80307b0)** — `compare.py --skip-interp
--repeat 5`, caches cleared, all 14 hashes agreeing: **five under the bar** — `hash`
**2.19×**, `hair` **2.76×**, `lock` **3.64×**, `composite` **3.68×**, `smooth` **3.92×**
— and `fronds` at **4.05×** (184 340 ns/op native, from 6.17× at the 09-11 close: the
two-day arc − sound placement reuse, § V-x, § V-y, § V-z − is −46 % standalone and the
reference lane's swing now decides its side of the bar).  Still over: `fill_circle`
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
| **V-j (move)** — the move-append: in `for f in call(…) { v += [f] }` with the loop var's single post-binding use that one append, the call's buffer is PLACED as a record in `v`'s own store (`Stores::place_record_in`), the append relocates the element's bytes and zeroes the source (`move_record_shallow`; store-identity dispatch, deep copy the fallback), the buffer's free record-level (`free_record_in`) (`@FR-R-MoveAppend`); declines on read-after, double append, named source, nested loop, rebound dest, non-struct element | [DESIGN.md § V-j](DESIGN.md) | the use-after sabotage turns c2 red (`3 409 45` → `3 409 0`); `tests/move_append.rs` no longer sees the pairs; a cell leaks or answers wrong under `LOFT_POISON=1` on either backend | **Shipped 2026-09-11** — sixteen cells exact both backends under poison + leak checks; ceiling re-measured by hand first (−11.3 %) and reached exactly: `fronds` 396–400k → 354–359k ns/op, consumer 8.36× → 7.88×; switch `LOFT_NO_MOVE_APPEND`.  Same-day addendum off the red gate: the placed record now dies at the LOOP's exit (scope-end freed a recycled slot — the `native_scripts` corruption), and a struct named like a stdlib typevar declines (the by-name wrapper lookup finds the generic template; loft#1519) — cells c19–c21, 21/21 exact |
| **V-w** — a self-reading vector LITERAL rides TEMPS: each destination-reading part is evaluated once before the build (`(I-Comp)`/`@FR-O-Detach` kept) instead of snapshotting the whole vector — the prefix-sum accumulator drops O(n²) → O(n); gates: literal lowering only, bare unlinked var, scalar elements, primitive/`#pure` calls; the snapshot stays for user calls, records, links, fields, comprehensions | [DESIGN.md § V-w](DESIGN.md) | the temps' Sets dropped crashes every cell; `tests/selfread_literal.rs` route pins; q8's comprehension keeps its snapshot | **Shipped 2026-09-11** — nine cells, both backends, I-Comp hand-oracle; 2 000→14 µs / 8 000→54 µs (linear); `lock_curved` within noise (63 points — the class, not the row). Full suite 4 706/4 708 (the cold-cache timeout class beside it); the one failure was the COMPOSITION improving: V-q's c3 (the accumulator that reads the pushed vector) now EARNS its push header — pre-V-w the per-iteration snapshot copy blocked the hoist — pin re-taught (0,0,0) → (1,1,0) |
| **V-v** — free-block FOOTERS: a free block carries −n at both ends (the tree node's color repacked into bit 31 of its RIGHT link to clear the half-word), so `delete` coalesces BACKWARD in O(1) — footer names the predecessor, header must agree, the free TREE must confirm (`@FR-H-FreeFooter`, formal/heap.md); one-word predecessors keep the lazy sweep; one helper (`set_free_header`) owns all eleven free-header writes; pre-footer images re-footed on open | [DESIGN.md § V-v](DESIGN.md) | the tree-confirm removed merges a freed block into a live claim (the fake-footer guard goes red); `store::tests` 50/50; a cell leaks or answers wrong under `LOFT_POISON=1` | **Shipped 2026-09-11** — `fronds` −7.7 % (288–295k ns/op standalone); retires the § V-j P2 `coalesce_free` cliff (12 % of the row); switch `LOFT_NO_FREE_FOOTER` (runtime, both backends) |
| **V-u** — a result vector ADOPTS the return buffer: a shape-A fn (separate `__retbuf` attr) whose every delivery sources ONE never-rebound result local aliases that local to the buffer (`hoist::ret_adopt`), the `one_buffer_vec_copy` pair emits as nothing while the block's frees and the entry clear stay, `OpReplaceVector` self-detects, a § V-j placement re-targets (`@FR-R-RetAdopt`); the witness-promoted ABI is already copy-free and declines | [DESIGN.md § V-u](DESIGN.md) | the un-scoped blanking corrupts the fronds probe; the whole-block collapse leaks 2 stores in the V-j corpus; `tests/retbuf_adopt.rs` no longer sees the adoption; a cell answers wrong under `LOFT_POISON=1` | **Shipped 2026-09-11** — seven cells exact both backends, leak-free under poison; `fronds` −9.5 % (350–357k), `smooth` −14.4 % (1 887 ns/op) standalone; switch `LOFT_NO_RETBUF_ADOPT` |
| **V-t** — a record append EMITS through the push header: an admitted mint whose element is a plain no-heap struct takes the header's next slot (`Stores::push_record_hoisted` — no `record_new` dispatch, no default prefill: the IR's literal lowering writes every field explicitly, a declined delivery is a whole-record copy) and the finish is the length bump (`push_record_finish`), exactly where `record_finish`'s was (`@FR-R-PushRec`); heap-owning, `__nullable` and keyed elements keep the templates | [DESIGN.md § V-t](DESIGN.md) | a growth cell answers empty (the falsified sabotage: c1 `100 0 25 9801 2475` → `0 0 0 0 0`); `tests/record_push.rs` no longer sees the fused forms; the audit accepts a template mint on a held path; a consumer hash disagrees | **Shipped 2026-09-11** — fifteen cells exact on both backends, emission pinned; standalone `smooth` −32 % (ceiling hand-measured at −37 % first); consumer `smooth` 11.0× → 4.6×, `fronds` 11.1× → 8.4× (this box), 14/14 hashes agree; switch `LOFT_NO_RECORD_PUSH` |
| **V-x** — an invariant loop-body vector LITERAL builds once per activation: the local pre-declared at fn top is its own once-flag, the declaration (one wrapped `Set`, or the flat `OpDatabase · Set · pushes` run bracketed by `hoist::flat_lit_member`, the shared predicate) guarded on it being unbound; gates: no-heap scalar elements, invariant parts (literals / pure ops / never-reassigned scalar params / scalar-getter reads of value-const record params), the `(Const-Value)`-mandated alias gate (every pd-reaching var fresh-store or const-getter-only), reads-only uses (indexed use declines — OpGetVector is context-blind) | [DESIGN.md § V-x](DESIGN.md) | the sabotage turns c5 red (`16` → `30`: iteration 0's write carried); `tests/literal_hoist.rs` no longer sees the guards; a cell answers wrong under `LOFT_POISON=1` on either backend | **Shipped 2026-09-11** — eleven cells exact both backends under poison; `fronds` −5.8 % (279–285k ns/op); switch `LOFT_NO_LITERAL_HOIST` |
| **V-y** — the complete-write prefill ELISION: a literal group the emitter proves writes every schema field position (declared defaults, sentinels, the variant tag — the parser's lowering is complete by construction) calls the no-prefill twin (`OpDatabaseNP`/`OpNewRecordNP`); a vector store's group is width-equal (the collection-field prefill is the one u32 its own `OpSetInt4` writes) and `place_record_in`'s prefill becomes one explicit len-zero (the callee entry-clear ABI covers it); the Z∩P redundancy ships as ONE argument | [DESIGN.md § V-y](DESIGN.md) | the admit-all sabotage crashes the V-j corpus at c17 under `LOFT_NO_ZERO_CLAIM=1` (the production default masks it — the first falsifier could not fail and was replaced); `tests/complete_write.rs` no longer sees the twins; a cell answers wrong under poison or the stale-arena lever | **Shipped 2026-09-12** — eight cells exact both backends under poison + the lever; `fronds` −4.8 % (264.9–272.7k ns/op, 9 twin sites); switch `LOFT_NO_COMPLETE_WRITE` |
| **V-z** — the ELEMENT-FIRST build: a local vector consumed exactly once as a record-literal field of an append is built inside the appended element — minted at the first temp's declaration (the length bump stays at the finish, so it is invisible until the append), each temp bound to the element's own field slot, the paired copies and temp stores gone; § V-u's adoption composes (a callee delivers its record straight into the element) | [DESIGN.md § V-z](DESIGN.md) | the compound sabotage (paired-offset check dropped + prefill-less prelude) crashes the corpus under `LOFT_NO_ZERO_CLAIM=1`; `tests/element_first.rs` no longer sees the early mints; a cell answers wrong under poison or the lever | **Shipped 2026-09-12** — eight cells exact both backends under poison + the lever; fronds standalone 265k → **186.4–199.7k ns/op**, the hand ceiling (−27–29 %) met exactly; switch `LOFT_NO_ELEMENT_FIRST` |
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
