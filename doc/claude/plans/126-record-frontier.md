# @PLN126 — tell a bound store a record is finished

> **Status — DONE 2026-08-11.** Shipped `store_release(collection)`; declined the
> per-record release the issue proposed. Reference for the post-plan contract lives in
> [DATABASE.md § `store_release`](../DATABASE.md) and
> [STDLIB.md](../STDLIB.md); the catalogue entry was `loft-lang/features#112`, closed when
> C153 removed the bound-store family `store_release` belonged to.
> This file is a closure record: the prediction written before the instrument existed,
> the measurement that falsified half of it, and what the build then cost.

## Outcome

| the issue proposed | outcome |
|---|---|
| per-record release (`store_flush(collection)`) | **declined** — 0.0% of the pages a record touches are its own |
| automatic write-back + eviction of untouched pages | not pursued — the kernel already does this, badly, and that is the cost being removed |
| append-only / ordered-write frontier | **shipped** as `store_release`, on a premise the issue did not state |

The plan's decision rule was *"if finished records are not contiguous, this collapses
into the same granularity problem and should be re-scoped or declined."* They are not
contiguous — and taken literally that rule would have thrown away a working feature.
Contiguity was only ever the **proposed** enabling condition for a frontier hint. What a
frontier actually needs is that the region below the mark is not written again, which is
a property of the ALLOCATOR on an append-only workload, and it measured true. Hence
re-scope, not decline.

## The prediction, written before the instrument

Kept because the record of what was expected is what makes the measurement worth
anything. Four predictions from reading `src/store.rs`; three held, **P3 was wrong**.

* **P1 ✓** — a strictly sequential build never relocates: a growing vector at the top of
  the arena has the free tail as its physical successor, so `resize` absorbs it in place.
* **P2 ✓** — with `W > 1` records open, a vector has a live neighbour behind it and
  relocates; `span/live` grows with `W`.
* **P3 ✗** — predicted that best fit would backfill the freed holes with later records
  and so break the frontier. It does not: a relocating vector DOUBLES, so the block it
  frees is too small for the next request and reuse stays local in time. The frontier
  measured 87–99% clean.
* **P4 ✓** — page exclusivity fails wherever the frontier claim's cousin does, and worse.

## Results

`cargo test --release --lib database::spans -- --ignored --nocapture`. 2 000 tiles,
8 ways each, 12 vertices a way, unless a row says otherwise. `W` is how many tiles the
stream keeps open at once; `W=1` is the plan's premise taken at its word.

| | span/live mean | p95 | foreign in span | exclusive pages | droppable below frontier |
|---|---|---|---|---|---|
| W=1 | **356×** | 918× | 98.9% | **0.0%** (1/6052) | **98.7%** |
| W=2 | 364× | 935× | 98.8% | 0.0% | 96.9% |
| W=4 | 382× | 1030× | 98.4% | 0.0% | 95.4% |
| W=16 | 391× | 974× | 98.5% | 0.0% | 93.0% |
| W=64 | 540× | 1107× | 98.1% | 0.1% | 86.9% |
| W=1, 8 000 tiles | 1348× | 3555× | 99.6% | 0.0% | 99.4% |

**C1 is false by two to three orders of magnitude, and at `W=1` most of all.** The cause
is not the one the issue expected. Vector reallocation barely matters; what separates a
tile's bytes is that the outer `hash` keeps its entries in a chunked ARENA. A chunk is
claimed early and holds ~63 tiles; each of those tiles' vectors are claimed later, at
whatever the frontier was. So a tile's lowest word is in a chunk near the bottom of the
arena and its highest is near the top, with the entire store in between.

The lone-tile cell shows it with nothing else alive at all — 5 words of arena slot, then
**318 words of the collection's own spine**, then 28 words of vectors:

```
[     1..    22)   21w  spine      (the root record)
[    22..    27)    5w  tile 0     (its arena slot)
[    27..   345)  318w  spine      (the rest of the chunk, and the bucket table)
[   345..   373)   28w  tile 0     (vector<TStep> 18w + vector<TRoad> 10w)
```

**C3 is false, and it is the sharper refusal.** Of the 4 KB pages one tile touches,
**0.0%** hold only that tile — not at any window, not at any tile count. Exclusivity
appears only when a single tile grows past a page of its own (128 ways per tile → 71.7%),
which is not the shape a tile index has.

**C2 is true, and my own P3 predicted it false.** 87–99.4% of the pages below a tile's
finish-frontier hold nothing that is written afterwards. Best fit does backfill freed
holes, but a relocating vector doubles, so the block it frees is too small for the next
request and the reuse stays local in time rather than reaching back across the arena.

## What it cost to make the frontier call work

Three things the design could not have known, each found by an instrument rather than by
reading (`a_frontier_release_moves_the_resident_set`, peak RSS off `/proc/self/statm`):

1. **Reading the frontier cost the very thing the call saves.** `Store::usage` derives
   the high-water mark by walking the block chain, one header per block — which touches
   every page of the arena. Asking it for the mark faulted the whole store back in to
   decide what to drop: peak RSS became the *entire file* (80.9 MB against 44.3 MB for
   the same build with no call at all). The mark is now carried forward in
   `Store::claimed_end` at the one site where a block becomes claimed.
2. **Re-flushing from zero each time is quadratic.** Flushing "everything below the
   frontier" per record re-syncs a region that grows with the run: 208× the wall clock.
   `Store::released_bytes` bounds each call to the bytes since the last one.
3. **`MS_SYNC` is 200× and `MS_ASYNC` is free.** Both reach the same resident set;
   waiting for the writeback costs ~1.5 ms a call. Asynchronous, the identical drop
   measures 0.8–1.1× wall — occasionally faster than not calling it, because writeback
   starts early instead of arriving all at once at the end.

With all three: **44.3 MB → 2.2 MB peak RSS (20×) on an 89 MB build, at 1.0× wall**, one
call per record.

| build | file | peak RSS, no call | with a call per record | wall |
|---|---|---|---|---|
| 4 000 tiles, W=1 | 16.4 MB | 10.2 MB | **0.9 MB** (11×) | 0.6× |
| 20 000 tiles, W=1 | 89.3 MB | 44.3 MB | **2.5 MB** (17×) | 1.1× |
| 4 000 tiles, W=16 | 7.0 MB | 6.5 MB | 6.5 MB (1.00×) | 1.0× |
| 20 000 tiles, W=16 | 38.3 MB | 26.6 MB | 26.3 MB (1.01×) | 1.0× |

**It pays for an ordered build and does nothing for an interleaved one**, and the
attribution is the free-block count, not the layout: 10 free blocks at `W=1` against
3 691 at `W=16` for the same data. The free-space tree is an LLRB whose nodes live
*inside the freed blocks*, so a scattered arena gives the allocator itself a scattered
working set — and that working set is exactly the region the release is dropping. The
page-fault counter confirms the pages are dropped and read straight back rather than
never dropped.

This is why the docs say to stream in key order, and it is a stronger reason than the
plan had: ordering is not a nicety that makes the hint tidier, it is the condition under
which the hint does anything at all.

## What shipped, and what defends it

* `Store::release_resident` — `msync(MS_ASYNC)` + `madvise(MADV_DONTNEED)` over the whole
  pages below the carried mark not yet released. `Stores::release_store` and
  `store_release(collection) -> integer` (`default/02_files.loft`, `src/native.rs`).
* `Store::claimed_end` / `Store::released_bytes` — the bookkeeping without which the call
  is quadratic and self-defeating.
* **Not built**, refused on evidence rather than on a hand-wave: a per-record release.

Gates: `tests/scripts/126-store-release-keeps-everything.loft` +
`store_release_keeps_every_record_and_reference_both_backends` — content, references and
file length survive on both backends, and a reference held ACROSS a release is checked by
VALUE, because a re-faulted page returns a plausible number either way.
`database::spans::one_tile_footprint_is_the_blocks_it_owns` pins the span instrument to
arithmetic rather than to a second run of itself. The two sweeps above are `#[ignore]`.

## The axis this could not see

Every cell here is a generator that only ever APPENDS. A workload that revisits finished
records — an editor, a simulation — would re-dirty pages below the frontier, and nothing
measured says what that costs. `store_release` is documented as a hint for streaming
writers for that reason, and the residue is what the dogfood loop is for.

## See also

* [DATABASE.md § `store_release`](../DATABASE.md) — the contract, and the three
  implementation facts the next residency feature will meet again.
* [STDLIB.md](../STDLIB.md) — the call's row. Catalogue entry: `loft-lang/features#112`.
* `src/database/spans.rs` — the span instrument, and how to re-run both sweeps.
* @PLN134 / @PLN136 laid the same arena out for PAGING; this
  plan asks what a WRITER can drop from it. `store_persist_copy` moves records and is
  legal only on a copy nobody holds a reference into; `store_release` moves nothing,
  which is why it is legal on the live store.
