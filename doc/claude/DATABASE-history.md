<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# DATABASE.md — history

What [DATABASE.md](DATABASE.md) no longer describes, kept for the record.

## Bound and durable stores (removed 2026-10-24)

`store_persist_bind`, `store_release`, `store_durable_check` and `store_durable_seal` were
removed together with `Store::open_durable`, the `.dmeta` sidecar and the bound-file
compaction, so that a store's memory need not be zero-filled: only a store whose raw bytes
leave the process is now written whole (`Store::defined`).  A collection is saved with
`store_persist_copy` and read back with `store_load`.  The sections below are what
DATABASE.md said about them.

### Durable stores (`Store::open_durable`) — @PLN43

A durable store is a normal mmap-backed `Store` plus a 40-byte `.dmeta`
sidecar file alongside the main store file.  The sidecar holds a signature
(`"DStoreV1"`), tier id, CRC32 over the main file, and a `last_clean_ns`
timestamp.  On clean drop the sidecar is rewritten atomically (`write tmp
→ fsync → rename`); on `kill -9` the sidecar stays stale, and the next
open detects corruption and invokes the consumer's rebuild callback.

The main store file is bit-for-bit identical to a non-durable store —
durability is a metadata layer, not a payload-layout change.  Existing
record/claim/resize code paths are untouched.

Three tiers are planned; phase 01 (the first PR slice on the
`store-durable-phase1` branch) ships **Tier 1 — `IntegrityOnly`** only:

| Tier | Mode | Hot-path cost | Loss bound | Consumer |
|---|---|---|---|---|
| 1 | `IntegrityOnly` | None (only msync on clean drop) | Everything since last clean drop | `personal/training` port (initial), `@PLN42` indexer (when phase 08 lands) |
| 2 | `SnapshotEvery(interval)` (planned, phase 02) | One msync per interval | One interval | TTT v5 multiplayer (`plans/future/32-…`) |
| 3 | `WAL` (planned, phase 03) | fsync per record, amortised by group-commit window | Zero for committed writes | @PLN6 audience demo |

API surface:

```rust
use loft::store::{Store, DurabilityMode};

let store = Store::open_durable(
    path,
    DurabilityMode::IntegrityOnly {
        on_corruption: Box::new(|p| rebuild_from_source(p)),
    },
)?;
```

**Fresh-file semantics.**  When the main file doesn't exist yet,
`on_corruption` fires with `TailMarkerMissing` and is expected to
"create empty + populate from authoritative sources" — not "repair
existing file."  After the callback returns successfully, `open_durable`
captures a fresh sidecar and retries once.

**Reading a bound collection invalidates its seal.** Iterating a keyed collection
materialises a key-sorted snapshot, and for a collection bound with
`store_persist_bind` that snapshot is claimed INSIDE the store — so the file's
bytes change and the sidecar's CRC no longer matches, with the file LENGTH
unchanged. The snapshot is released at loop exit, but a claim-then-free still
leaves different bytes than it found. Measured: a bare re-bind keeps
`store_durable_check` true; one traversal makes it false. Seal AFTER the reads
you intend to do, not before. (`store_reclaim` and compaction both refuse
outright on a store with a live sidecar, so those cannot surprise you the same
way.)

**What it does NOT do any more is grow the file.** The snapshot used to be left
behind — the loop epilogue released it only when it had a store of its own, and
a writable collection's is co-located, on the reasoning that its records go back
when the store dies. A collection outlives the loops that read it, and a bound
collection's store is a file that outlives the process, so every read leaked 4
bytes per element into it permanently: sixteen runs of a program that only READ
a 4,000-record hash took its file from 566,472 to 1,321,768 bytes, with no
writes anywhere (loft#727). Reading is free now, in memory and on disk — a
40-pass traversal loop leaves a store's census byte-identical. The old note
"stat the file the instant `store_persist_bind` returns, before anything walks
the collection" no longer applies.

**Drop-on-panic is by design.**  A panic between open and clean drop
skips the sidecar write → next open detects corruption → callback fires.
This is what makes Tier 1 cheap.  Do not use Tier 1 for data that cannot
be re-derived from authoritative sources; use Tier 2 or Tier 3 instead.

Full design + implementation history:
[`doc/claude/plans/43-loft-store-durable/`](plans/43-loft-store-durable/README.md).


#### What the file's SIZE and BYTES mean (loft#710)

A persisted store used to be the arena's whole **capacity**, so its size said how
the store was BUILT, not what it holds: filling each record's vector whole before
inserting gave 1.84× what growing them interleaved gave for byte-identical data,
and 160,000 coordinates persisted to the same byte count as 290,000. The image is
now sized from the **high-water mark** — the end of the last live record — plus an
eighth. The eighth is not slack for its own sake: a bound store stays live and the
arena grows by 7/3, so persisting with no room left costs a 2.33× file resize on
the very next claim, which is worse than the tail it removed.

**Interior free space stays while the store is live.** Reclaiming it means
relocating records, which no live `DbRef` may see, so it happens only when the store
is next loaded (the INTERIOR paragraph below, @PLN123 arc B). Until then the size
follows the content, but two construction orders can differ by the fragmentation
each leaves.

**The bind-first path reaches the same answer at RELEASE (loft#752).** That #710
fix is on the IMAGE WRITE, so it covers `store_persist_bind` LAST and nothing
else. Bind a store FIRST and its file IS the live arena: while the program runs,
the size is the arena's **capacity**, which grows by 7/3 and never shrinks on its
own. So the file used to be quantized to a ladder and could sit up to **57%**
(`1 − 3/7`) above its content. Measured on one generator shape, varying only the
feature count:

| features | file (bytes), before | after |
|---|---|---|
| 150 000 | 39,179,744 | content-sized |
| 200 000 – 400 000 | **91,419,400** (unchanged across a 2× data increase) | one size per count |
| 500 000 – 700 000 | 213,311,928 | content-sized |

Freeing the collection's store now hands the tail back before the slot is marked
free — the same `reclaim_tail` `store_reclaim` calls, at the one moment the
runtime can tell a permanent drop from a lull, because there is no next claim to
pay 7/3 for. `store_reclaim` at the end of a build is therefore no longer needed
and finds nothing left: measured over 40 000 and 60 000 features, with and
without it, the files are byte-identical per count and differ between counts.

⚠ **MID-RUN, a bound store's file size still compares nothing.** Between the bind
and the release it is capacity: two points a rung apart differ by 133% with
byte-identical content. Call `store_reclaim(collection)` before reading a size in
the middle of a run, or do not read it.

This is not a footnote: a consumer measured two insertion orders, saw 2.3×, and
concluded that feeding keys in order — the thing that bounds a generator's working
set — was the worse strategy on every axis. Both numbers were rungs (loft#747).
Right-sized, the same shape leaves a 1.30× spread, which IS the fragmentation the
two orders genuinely differ by.

### Binding FIRST is the low-memory choice, and its pages are reclaimable

**`store_persist_bind` FIRST uses far LESS memory than binding last**, which is the
opposite of what a reader expects and of what loft#747 was filed claiming. Measured
across two generator shapes and two boxes:

| features / tiles | RSS, bind FIRST | RSS, bind LAST |
|---|---|---|
| 400 000 / 40 000 | **12 MB** | 29 MB |
| 1 600 000 / 160 000 | **31 MB** | 125 MB |
| 4 000 000 / 400 000 | **65 MB** | 289 MB |

4.4× lower here, 2.8–5.3× lower on a heavier consumer shape. Bind LAST builds in
anonymous heap and then writes an image; bind FIRST makes the file the arena, and
**file-backed pages are reclaimable while anonymous heap is not**. So the dataset
size does not set a hard memory requirement — it sets a working-set/throughput
tradeoff. Under a hard cgroup cap with swap disabled:

| | result |
|---|---|
| 4 M features, `MemoryMax=32M` | **completes**, 34 MB peak, 271 s, correct read-back |
| 1.6 M, cap 96 MB, bind FIRST | **completes**, 88 MB peak, 27.6 s (12.7 s uncapped) |
| 1.6 M, cap 96 MB, bind LAST | **OOM-killed** |

⚠ **`MemoryMax` alone proves nothing on a box with swap.** A first attempt at the
table above had BOTH orders passing under 96 MB, because the unbound heap simply
paged out to 8 GB of swap. `MemorySwapMax=0` is what makes a cap a cap; without it
the measurement is vacuous in the direction that looks like success.

The cost of capping is that the kernel's LRU only learns the working set by evicting
the wrong pages first — ~2× wall at a modest cap, 271 s at an aggressive one.
`store_release(collection)` lets the program say so instead, and turns that cliff
into a curve.

### `store_release` — the working set follows the program, not the eviction (@PLN126)

`store_release(collection)` starts writing everything below the arena's high-water
mark out to the bound file and drops it from the resident set, answering the bytes
dropped. **Content is untouched and every reference stays valid**: nothing moves and
nothing is freed, the mapping is `MAP_SHARED`, and reading a released record simply
re-reads it from the file one page fault later. It is a HINT — calling it too often,
or on a collection that is not bound, costs a little speed and can never cost an
answer. Not `store_reclaim`, which changes the FILE's length; this never does. Not a
durability barrier either — it asks for writeback to START, which is
`store_durable_seal`'s job to promise.

Measured on a 20 000-record generator, one call per record: **peak RSS 44.3 MB → 2.2
MB (20×) at 1.0× wall clock.**

**It pays for a build in KEY ORDER and does nothing for an interleaved one** (1.01× on
the same data), and the reason is the allocator rather than the layout: a generator
that keeps many records open at once relocates its growing vectors and leaves the
arena scattered with free blocks — 3 691 against 10 for the same data written in
order. The LLRB free-space tree's nodes live *inside* those freed blocks, so a
scattered arena gives the allocator its own scattered working set, which is exactly
the region the release just dropped. Stream in cell order and this is close to free.

Three implementation facts, each of which cost 200× before it was found, and each
recorded because the next residency feature meets all three again:

* **Deriving the frontier costs the whole point.** `Store::usage` walks the block
  chain, one header per block, touching every page of the arena — so asking it what to
  drop faulted the entire store back in first (peak RSS became the whole file, 80.9 MB
  against 44.3 MB for making no call at all). `Store::claimed_end` carries the mark
  forward at `claim_block` instead. It is a monotone UPPER bound on `live_end_words`,
  which is the safe direction here (flushing a few free pages costs a fault) and the
  wrong one for `shrink_to` / `reclaim_tail` / `bind_path`, which still read the chain
  because truncating to an upper bound cuts live data.
* **Each call must be bounded to what is new.** Flushing from zero every time re-syncs
  a region that grows with the run: 208× wall. `Store::released_bytes` is the
  watermark.
* **`MS_ASYNC`, never `MS_SYNC`.** Both reach the same resident set; waiting costs
  ~1.5 ms a call, which is the difference between a call a generator can make per
  record and one it cannot make at all.

**Why there is no per-RECORD release, and it is not a matter of taste.** `MADV_DONTNEED`
works on pages, and `database::spans` measured what one record of a real generator's
shape (`hash<TTile[tkey]>` with two grown-by-append vectors) actually occupies:
**0.0% of the pages a record touches hold only that record**, at every window and every
scale tried. The span between a record's lowest and highest word is 356–1348× the bytes
it owns, because the hash keeps its entries in a chunked arena claimed early while the
record's vectors are claimed at the frontier much later — so a record's own bytes sit
either side of the whole store. A frontier release needs none of that: it needs the
region below the mark not to be written again, and 87–99% of its pages are not.
Full workings: [plans/126-record-frontier.md](plans/126-record-frontier.md).

**A BOUND store's file only ever grew, until `store_reclaim`.** The sizing above
happens when the image is WRITTEN; after that `resize_store` returns early on any
request at or below the current size, so a bound store that grew ten-fold and
dropped back to its original live set kept the ten-fold file for the rest of the
run (12.7× measured). `store_reclaim(collection)` (@PLN123 arc A) truncates the
file to the store's high-water mark **plus an eighth** and answers with the bytes
it gave back — half the file on that shape, with every surviving record
bit-for-bit unchanged.

The eighth is not a rounding: it is the same slack the image format gives a
freshly-bound store, and for the same reason. The store stays live, growth
multiplies by 7/3, so one trimmed to the byte pays a 2.33× resize on its very
next claim — on the FILE. Trimming to the bare mark made `store_reclaim` hand
back 40% of a file and take 133% back on the next read (loft#727). A store that
came straight from `bind_path` is therefore already the right size and answers
`0`; a tail only appears once the store has GROWN while bound.

Safe without any reference tracking, and the reason is worth knowing: everything
above the high-water mark is free by construction, and a `DbRef` is a POSITION
(`store_nr, rec, pos`), not a pointer — so nothing can name a word above the
mark, and no record moves. What it will NOT do is touch the interior; read
`store_memory()`'s `tail%` / `inner%` to see which of the two you have. It is
opt-in for a reason ([STDLIB_RUNTIME.md § Memory diagnostics](STDLIB_RUNTIME.md)): on a churning
store, calling it per cycle buys density with 55× the store's size in resize
traffic. And it refuses outright on a store carrying a `store_durable_seal`
sidecar — that sidecar records the file's byte length and CRC, so truncating
behind its back would report a healthy store as corrupt.

**The INTERIOR is taken automatically, when a store is LOADED** (@PLN123 arc B).
The space *between* surviving records needs the collection rebuilt somewhere
dense, which moves records — so it happens only where an interior `DbRef` cannot
be live: `store_load`, and `store_persist_bind` on an **existing** file. Both are
loads, and both already replace the slot's bytes wholesale, so a reference held
across them was already meaningless. Binding to a NEW file is a *write* and is
deliberately untouched: a program keeps element references across it, and the
byte-for-byte image is what makes that work.

A bound store that peaked at 2,000 records and settled at 200 came back at
180,104 bytes every run; it now loads at 26,992 — the same records, the same
digest, and still bound. The one position that never moves is the collection
root, because the collection variable itself is a `DbRef` at it.

It is **gated**, which is what lets it be a default: a store whose interior free
space is under an eighth of its high-water mark is measured and left alone. An
eighth is the slack the image format already carries on purpose (the mark plus
an eighth, above), and the estimate is a *lower bound* on what a rebuild returns
— a rebuild also right-sizes live structures the metric counts as data, such as
a hash's bucket array still sized for its peak.

It declines, and says why under `LOFT_LOADER_STATS`: a spatial (`Radix`)
collection, a record holding a `reference<T>` into another store, an untyped,
read-only or borrowed store, a store at or below the image floor, and one
carrying a durable sidecar. `LOFT_NO_COMPACT_ON_LOAD` turns the whole thing off.

**The eighth of slack survives `store_reclaim`** — and for a while it did not.
The image size used to be clamped to the arena's current capacity ("never larger
than we would have written before"), which was safe while capacity sat well above
the mark. `store_reclaim` trims capacity TO the mark, so the clamp collapsed the
eighth to zero for exactly the stores someone had just tidied, and the next claim
paid the 7/3 ladder the eighth exists to prevent. The claim that tripped it was
the most ordinary one there is: READING the collection, because iterating a keyed
collection claims its key-sorted snapshot inside the store. A 2,000-record hash
wrote 187,784 bytes and one read took it to 438,160 — **2.07× larger than never
reclaiming at all**. The clamp is gone; both paths now land on 211,256 and stay
there. Guarded by `persisted_image_keeps_its_slack_after_store_reclaim`.

