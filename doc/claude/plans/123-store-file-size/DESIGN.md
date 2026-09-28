<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN123 — design and implementation steps

Overview + measurements: [README.md](README.md). This file is the build order.

Every step below lands on its own, is verifiable on its own, and leaves the tree
green. The first three land **inert** — analysis and mechanism with no caller — so
the behaviour change is one reviewable commit that can be reverted without taking
the machinery with it.

---

## The two invariants

**A — the capacity floor.**

> A store's capacity never falls below its **high-water mark**: the word past its
> last claimed block. Everything above the mark is free by construction, so
> lowering capacity to it cannot orphan a live record, and no `DbRef` — which is
> `(store_nr, rec, pos)`, a position, not a pointer — can name a word above it.

That is the whole safety argument for arc A, and it is why A needs no reference
tracking. It is *only* true if the mark is computed from the block chain itself
(`Store::usage`), never from a cached count.

**B — the image is the live graph.**

> A compacted image contains exactly the records reachable from the collection
> root, and loading it back yields a value-identical collection.

The check is a digest over every field, not a size: a smaller file that dropped
data satisfies every size assertion anyone would write.

---

## Failure paths (enumerate first — this is where the invariants came from)

| # | Path | Answer |
|---|---|---|
| F1 | A `DbRef` names a word above the new capacity | Impossible under invariant A — but ASSERT it (step A0), because "impossible" is the claim being made |
| F2 | The free tree still holds the truncated tail block | `fl_rebuild` **after** every shrink. A stale `free_root` hands out a block past the end |
| F3 | `set_len` down succeeds, the remap fails | `MmapStorage::resize` already restores the old mapping on error and returns `Err`; the caller must treat `Err` as "did not shrink" and leave `self.size` alone |
| F4 | The `.dmeta` durable sidecar records byte length + CRC | **Refuse** — decided in A3, and the guard had to be rewritten: it first asked `durable_meta_path.is_some()`, set only by `Store::open_durable`, which NO loft program reaches. The loft surface is path-based (`store_durable_seal(path)`), so the reachable hazard was unguarded — seal, `store_reclaim`, and a healthy store read CORRUPT (156,344 → 138,976, check true → false). It now asks whether a sidecar sits beside the store's FILE. Covered by `reclaim_and_compaction_refuse_a_sealed_store_and_a_floor_sized_one` |
| F5 | Another process has the file mapped | Out of scope: shrink only a store this process owns and has bound. Write it down rather than discovering it |
| F6 | Shrinking below `bind_path`'s 1024-word floor | Clamp; `Store::open` resizes anything smaller to 8192 bytes anyway |
| F7 | The store keeps growing after a shrink | It re-grows at 7/3. Acceptable, but measure the churn: a threshold that fires every free would thrash |
| F8 | Coalescing merges blocks the free tree indexes | `coalesce_free` already rebuilds the tree; keep that ordering |
| F9 | Arc B rebuilds indexes and the key order changes | **Not a live hazard** (B0): loft iterates a keyed collection through a KEY-SORTED SNAPSHOT, so no rebuild can reorder it — established by probing with an order-dependent fold across 3 insertion orders and 2 bucket seeds. The digest is order-independent anyway, for free |

---

## Arc A — a bound store's file shrinks to its high-water mark

### A0 — trust the mark (inert) — **DONE**

`StoreUsage::live_end_words` and the `tail%` / `inner%` report already landed with
the measurements. What was missing is the **calibration**: a blank or wrong mark
reads as "nothing to reclaim", which is indistinguishable from a healthy store.

**The assertion this step set out to add cannot fail.** `Store::usage` raises the
mark at every claimed block its walk passes, so "no claimed block begins at or
after `live_end_words`" is a restatement of the loop, not a check on it — and so
is every other phrasing against what that same walk saw (word accounting
included: `claimed + free` is built by the additions that move `pos`). Worth
writing down because it retires a whole family of assertions someone would
otherwise keep proposing here.

What the mark's trustworthiness actually rests on is **outside** the arithmetic:
whether the chain tiled the store at all. So A0 shipped a fact, not a check:

- **Code point:** `src/store.rs::Store::usage`.
- **Added:** `StoreUsage::walk_complete` — did the walk reach the store's end?
  It is false when a zero header stops the walk ("malformed / uninitialised
  tail") and when a `free` or under-one-block store is never walked. In those
  cases the mark is a **lower bound**, and a live record can sit above it.
  **A1 must refuse to shrink unless this is true.** Reporting is content with a
  lower bound; deciding is not, and that gap is the field's whole reason to exist.
- **Added:** `debug_assert!(live_end_words <= capacity_words)` — the one check
  here that *is* falsifiable: a final block whose header claims more words than
  remain drives the walk, and the mark with it, past the arena.
- **Verify:** three unit tests in `src/store.rs`'s test module — the mark's value
  on a healthy store, the malformed shape where the mark sits below a live
  record, and a `#[should_panic]` non-vacuity proof for the assertion.
- **Does not:** change any behaviour.

**The verification build matters here.** `[profile.dev.package.loft]` sets
`debug-assertions = false`, so *every* `cargo test` — `--profile dev` included —
strips the lib's `debug_assert!`s. An earlier draft of this step named that suite
as its verification, which would have been a calibration failure of its own:
green there says nothing about a DA-guarded claim. Run the
[`target-da` calibration lens](../../DEBUG.md#the-debug-assertions-calibration-run-target-da).

The non-vacuity test is `#[cfg(debug_assertions)]` for the same reason: it
compiles away together with the instrument it proves, so it can never pass by
not running.

### A1 — a shrink primitive with no caller (inert) — **DONE**

- **Code point:** `src/store.rs::resize_store` (1140) refuses `to_size <=
  self.size` and must keep doing so — it is the growth path and every caller
  relies on grow-only.
- **Add:** `Store::shrink_to(&mut self, words: u32) -> bool`, a sibling, not a
  change to `resize_store`:
  1. compute `usage()`; return `false` unless **`walk_complete`** (A0 — an
     incomplete walk reports the mark as a lower bound, and shrinking to a lower
     bound cuts live data) **and** `words >= live_end_words` (invariant A,
     checked against the chain, not against an argument);
  2. clamp to `>= 1024` (F6);
  3. mmap branch — `f.resize(words * 8)`; on `Err` return `false` and leave
     `self.size` untouched (F3); on `Ok` refresh `self.ptr` from
     `f.as_slice()`, exactly as the grow branch does;
  4. heap branch — `A.realloc` down, refresh `self.ptr`;
  5. set `self.size = words`, then **`fl_rebuild()`** (F2);
  6. bump `self.generation` (the same reason `resize` does: a suspended coroutine
     must be able to notice).
- **Verify:** a Rust unit test in `src/store.rs`'s test module — build a store,
  claim, free the tail, `shrink_to(live_end)`, assert capacity fell, assert every
  surviving record still reads its value, assert `fl_validate()` passes. And a
  negative: `shrink_to(live_end - 1)` returns `false` and changes nothing.
- **Does not:** get called from anywhere.

**Two things the sketch above did not have**, both found by building it:

*The tail has to be re-tiled, not just cut.* Truncating to somewhere ABOVE the
mark leaves free space that must become ONE block ending exactly at the new
size — otherwise the chain no longer partitions the store and every later walk
breaks on it. `retile_tail` does that, then `fl_rebuild`. Skipping the rebuild
is not a hygiene issue: with the free tree still naming words that were cut, the
unit test **SIGSEGVs**, which is F2 happening rather than being argued about.

*The 1024-word floor is a FILE fact, not a store fact.* It exists because
`Store::open` lifts a small file straight back up. A heap store has no such
floor — `Store::new` insists on two words — and clamping there would have made
the primitive refuse on every store under 8 KB, unit tests included. The floor
now has one home (`MIN_BOUND_WORDS`), which `Store::open` uses as well.

### A2 — reclaim = coalesce, then shrink (inert) — **DONE**

**Correction — the sweep does not move the mark.** This step was written on
"the tail after a mass removal is thousands of unmerged free blocks, so the mark
is far below where it could be, and shrinking alone reclaims almost nothing."
That is true of a *is the top block free* test, and this is not one:
`live_end_words` is the end of the last CLAIMED block, and merging free blocks
never moves a claimed one. The same store gives back the same tail swept or
unswept — pinned by the A2 test, which reclaims twice from identical stores.

The sweep stays, paying for a different thing: those unmerged pairs are in the
**interior**, and merging them is what decides whether the next large claim
reuses space or grows the store. An explicit, rare call is the one moment an
O(blocks) sweep is welcome — and it is the reason nothing on the free path ever
needs to walk.

- **Code point:** `src/store.rs`, next to `coalesce_free`.
- **Added:** `Store::reclaim_tail(&mut self) -> u32` — words given back:
  ```
  if self.needs_coalesce { self.coalesce_free(); }
  let before = self.size;
  let mark = self.usage().live_end_words;
  if self.shrink_to(mark) { before - self.size } else { 0 }
  ```
  Measured against `self.size` after the fact, never `before - mark`:
  `shrink_to` clamps to its floor, so the mark is not where it necessarily
  landed.
- **Verify:** two unit tests — the fragmented shape (tail returned, interior
  merged, store still usable, and the swept-first control), and a dense store
  reporting 0 while touching nothing.
- **Does not:** get called from anywhere.

**A test-shape trap worth keeping.** Freeing every OTHER record produces free
blocks that are not adjacent, so `mergeable_free_pairs` is already 0 and the
"the interior was swept" assertion says nothing. The fragmented shape has to
free in FORWARD runs of two — `delete` merges only with the block after it, and
that one is still claimed both times, which is what leaves a pair unmerged.

### A3 — expose it as an explicit call: `store_reclaim(collection)` — **DONE**

**It needed no compiler special-case, and no counter.** Two subtractions from the
sketch below, both worth keeping:

*Not a parser special-case.* `reserve` is one because it needs the element's
stored width — a static type fact available nowhere else. `store_reclaim` needs
only the reference's `store_nr`, which every call already carries, so it is a
plain stdlib declaration with a `#rust` body beside `store_verify`
(`default/02_files.loft`) plus the interpreter's registry handler
(`native.rs::n_store_reclaim`). Both backends print byte-identical numbers.

*No `freed_words_since_reclaim` counter.* The counter existed to keep `delete`
from walking — and `delete` does not walk, because A is opt-in and nothing on the
free path calls any of this. As an early-out for the explicit call it would also
be **unsound on its own**: a store that only ever grew has a tail (the 7/3
over-allocation) with no delete behind it, so "nothing freed since" does not mean
"nothing to give". Making it sound needs a second producer (growth), and a missed
producer silently under-reclaims. Dropped; the explicit call pays its one walk.

*F4 is decided and enforced at the truncation itself* (`Store::shrink_to`): a
store with a live `.dmeta` sidecar refuses, because the sidecar records byte
length + CRC and truncating behind its back reports a healthy store as corrupt.

**F7, measured.** 40 cycles of +300/−300 records over a 2,000-record hash:

| | end capacity | used | tail | resize traffic |
|---|---|---|---|---|
| left alone | 0.286 MB | 56% | 40% | — |
| `store_reclaim` every cycle | 0.173 MB | 93% | 0% | **9,537,312 bytes** |

So the thrash is real but not free-standing: per-cycle calling *does* hold the
store 1.65× denser, and pays 55× the store's own size in grow-and-shrink traffic
to save 0.11 MB. That number is the argument for opt-in, better than the
qualitative one: called once after a permanent drop it costs a single walk;
called on a cycle it buys a re-grow every time.

**The A-vs-B re-measurement** (phase ordering step 2), same bound store, before
and after one `store_reclaim`:

```
before   cap 0.262 MB  used 25%  tail 28%  inner 47%   free-blocks 2701 (2696 mergeable)
after    cap 0.188 MB  used 35%  tail  0%  inner 65%   free-blocks    4 (0 mergeable)
```

A takes the whole tail and the sweep collapses 2,701 free blocks to 4. What is
left is **65% interior** — B's target, now measured rather than assumed, on the
plan's own workload. B is justified.

#### The original step, for the record

**A is OPT-IN, and stays opt-in.** The earlier draft of this step gated it on
`used_pct()` inside `Store::delete`; that is wrong twice over, and both are worth
writing down because the second is the load-bearing one.

*It cannot be cheap enough to be automatic.* `used_pct()` comes from `usage()`,
an O(blocks) chain walk — per delete. Any always-on form needs an O(1) trigger
instead: a `freed_words_since_reclaim` counter maintained in `delete` (an add,
nothing more), and only when it crosses a threshold may anything walk. That is
the shape to use even for the explicit call's internals, so `delete` never grows
a walk.

*And the benefit is near-zero in almost every workload.* Measured, this plan's
own numbers: steady-state churn held capacity flat at 0.30 MB across 36,000
insert+remove operations, and refill (3000 → shrink to 300 → refill to 3000)
reused every byte with capacity never moving. Only a live set that drops far
below its peak **and stays there** has anything to give back. An always-on
reclaimer would walk, coalesce, and find nothing — for everyone.

*The thrash case is the one it targets.* A store that shrinks and then grows
again — a world unloading and reloading regions, the most plausible long-lived
shrink pattern there is — would be truncated and immediately re-grown at 7/3.
Whether a drop is permanent is something the program knows and the runtime
cannot infer, so the program says when.

- **Surface:** `store_reclaim(collection) -> integer` — bytes returned to the
  filesystem, 0 when there was nothing to give. ~~A compiler special-case beside
  `reserve`~~ → a stdlib declaration with a `#rust` body (see above).
- ~~**Code point for the counter:** `src/store.rs::Store::delete`~~ → dropped
  (see above).
- **Decided:** F4 — a store with a live `.dmeta` sidecar refuses to shrink.
- **Verified:** `tests/scripts/store_reclaim_123.loft` + the driver
  `store_reclaim_shrinks_a_bound_file_both_backends` — bind 300 → grow 3000 →
  drop back to 300: the file falls by exactly the bytes the call reports
  (247,944 → 124,872), the digest over every surviving record is unchanged, a
  second call returns 0 without touching the file, and the truncated store still
  takes new records and reads old ones. Both backends, byte-identical.

**The cost of opt-in, stated plainly:** a knob nobody knows about is a knob
nobody uses, so a shrunk store stays 5.8× in the wild. What makes that
acceptable is that the condition is already self-diagnosing — `store_memory()`
reports `inner%` / `tail%` — so the path is "notice the number, call the thing"
rather than loft unmapping a file under a running program. That fits *loft is
boring: noticed only in its absence* better than the automatic form does.

### A4 — document and graduate the probes

Document `store_reclaim` in [STDLIB.md](../../STDLIB.md) beside `reserve`, and in
[DATABASE.md](../../DATABASE.md) beside the high-water-mark image, each pointing
at the `inner%` reading that tells a consumer whether to bother. Probes graduate
to `tests/scripts/`. No default to flip — there is no automatic mode for A.

---

## Arc B — compact the interior when writing an image

**Do not start B until A3 has been measured on a real consumer.** A recovers the
tail; what B is worth is whatever `inner%` still reads afterwards, and that number
does not exist yet.

### B0 — the oracle first — **DONE**

`tests/scripts/store_digest_b0.loft` + `store_digest_b0_oracle_sees_loss_not_layout`.
A per-field digest over records carrying every field kind a rebuild must
relocate (integer, `i32`, variable-length `text`, variable-length `vector<i32>`,
nested struct), combined across records by sum / xor / count, plus
`store_verify` for invariant B's other half — the digest says the values are
right, `store_verify` says every internal pointer still targets a live record.

The oracle is worth exactly what its calibration is worth, so the test pins two
things and proves each can fail:

**It sees every loss.** Five injuries — a dropped record, a changed scalar, a
shortened text, a truncated vector, a changed nested field — each must move the
numbers. *This caught its own first draft*: the vector injury truncated a record
whose vector was ALREADY one element, so the digest came back identical. That
reads exactly like a blind oracle and was a blind injury. Damage that does not
damage proves nothing and looks like proof.

**It is a function of the data, not the representation.** Five stored
representations of identical records (three insertion orders × two bucket
seeds) must digest the same, since a rebuild changes where everything sits and
anything the digest picks up from layout would read as data loss.

*The controls are the load-bearing part here.* **Pin the seed** — an unseeded
hash draws a random one per process (P253), so unpinned, the five images differ
no matter what else you vary and the control passes while attributing nothing
(observed: all five reps forced identical still gave 5 distinct files). Pinned,
each row differs from the reference by one named lever, a byte-identical repeat
run proves the harness is deterministic, and each lever is asserted to actually
reach the stored bytes. Reverse insertion even changes the file SIZE
(22,944 vs 22,936).

**F9 is NOT a live hazard, and that took establishing.** The step assumed a
rebuild reorders iteration, so the digest was written order-independent. Probing
with a deliberately order-DEPENDENT fold showed that neither insertion order nor
the bucket seed changes the order records come out in: loft iterates a keyed
collection through a **key-sorted snapshot**, not a bucket walk, so a rebuild
cannot reorder it. The order-independent form stays — it costs nothing and keeps
the oracle off that implementation detail — but the *assertion* had to go, since
no available lever can make it fail. An assertion that cannot fail teaches a
later reader that something is checked when it is not.

### B1 — measure what rebuild-and-swap costs

**Presumed approach: rebuild into a fresh store and swap.** Not relocation in
place.

| | Reuses | Risk |
|---|---|---|
| **rebuild + swap** — build a fresh collection, copy each record in, swap | `copy_block_cross_store` (`src/database/structures.rs:263`) and the sibling insert path | none — a fresh store has no inbound pointers at all |
| ~~relocate~~ — move live blocks down, rewrite inbound pointers | `relocate_ptr_fields` (`src/database/allocation.rs:3334`), bound to `PagedReader` so it needs generalising | must find every inbound pointer |

Two independent reasons, and the second is why this is no longer an open choice.

*It removes the risk rather than managing it.* Relocation's whole difficulty is
finding every pointer AT a moved record; a freshly built store has none, so there
is nothing to rewrite and nothing to get wrong.

*It is what the established engines do.* InnoDB does not relocate rows to reclaim
space — `OPTIMIZE TABLE` maps to `ALTER TABLE … FORCE`, which **rebuilds the
table into a new file and swaps it**, and the space only returns to the
filesystem with `innodb_file_per_table`. The whole shape of this plan lands on
the same model: deletes free space *within* the file and never shrink it, an
explicit operation reclaims it, and a free-space number (`DATA_FREE`, our
`inner%`) tells you whether it is worth running. Prior art at that scale is worth
more than a local preference.

So B1 is no longer "which approach" but **"what does it cost"**. Measured with
`tests/scripts/store_rebuild_b1.loft`, guarded by
`store_rebuild_b1_recovers_the_interior_and_is_idempotent`, digest-checked at
every stage by B0's oracle. **DONE.**

### What it costs — the verdict is cheap and effective

300 → 6,000 surviving records, each grown 10× and dropped back, `store_reclaim`
already applied so the baseline is what arc A leaves:

| keep | post-A file | rebuilt | recovered | rebuild time (`--native`) |
|---|---|---|---|---|
| 300 | 213,016 | 48,760 | **77%** | 175 µs (583 ns/rec) |
| 1,500 | 1,715,136 | 240,896 | **86%** | 908 µs (605 ns/rec) |
| 6,000 | 6,865,056 | 969,120 | **86%** | 3,757 µs (626 ns/rec) |

Linear across a 20× range, ~0.6 µs/record native and ~1.0 µs interpreted, and
the loft-level figure is an **upper bound** — the Rust form at image-composition
time skips the interpreter and the per-record dispatch entirely. Both backends
produce byte-identical sizes.

**The standing warning does not reproduce.** A record-by-record rebuild did not
produce a larger file; it produced one 4–7× smaller. The old
634,264-vs-437,416 number came from a different path and should not be carried
forward as a caution against this one.

**It is IDEMPOTENT** — a second and third rebuild land on the same byte count, at
every vector length tested. That is the property B2's automatic mode rests on: a
compaction that grew the file a little each time would be worse than none.

### The gap B2 has to close, and where it is

A rebuild does not reach the from-scratch ceiling — 1.24× at 300 records, 1.38×
at 6,000 — and the whole gap is the **vector field**:

- shapes with only scalars, or only variable-length text, rebuild to **exactly**
  the fresh size (ratio 1.000);
- with vectors, the rebuilt size is **flat across vector lengths 1–9** (40,200
  bytes at every one) while a from-scratch build scales with them smoothly.

So the copy path claims a quantised block per vector where a fresh build claims
by length. **B2 should claim each destination vector at its LENGTH.** `reserve`
on the source side does not do it — measured, identical bytes with and without,
because it sizes the local being copied FROM, not the block copied INTO.

**Closed 2026-09-28.** `Stores::vector_add` claims an EMPTY destination at the source's
length instead of `vector_append`'s eleven-element first block, so `store_rebuild_b1` reads
rebuilt = fresh (37,256 bytes at 300 `full` records) and asserts it.  It surfaced when
@PLN157 § V-d began writing a builder's record in its element: a fresh build then filled its
vector field through the same claim and grew 28 % (34,920 → 44,616 bytes, `vec` shape), which
`store_compact_b3` caught.  `LOFT_NO_EXACT_COPY_CLAIM=1` restores the flat block.

Arc A composes on top: `store_reclaim` after a rebuild takes a further ~9%
(48,760 → 44,568), because a fresh arena still carries its own 7/3 growth slack.

### The instrument trap, worth keeping

**Iterating a bound collection grows its file.** A keyed collection iterates
through a key-sorted snapshot, and for a bound collection that snapshot is
claimed inside the store — so a digest loop inflates the very file it is
measuring (observed: 40,200 → 86,240 across two extra traversals, which read as
`store_reclaim` making the file BIGGER). Every measurement here stats the file
the instant `store_persist_bind` returns, before anything walks the collection.
It is also a fourth producer of tail, and an argument for `store_reclaim`: a
read-only traversal of a persisted collection permanently grew its file before
arc A existed.

### Prior art — checked, and it changes something

Three findings, and the third is the one the plan did not have:

1. **`OPTIMIZE TABLE` under `innodb_file_per_table` is rebuild-and-swap**, in the
   literal sense assumed here: create a new empty table, copy row by row, land it
   in a fresh `.ibd`. B's approach is the established one.
2. **In-place defragmentation exists as an option** — MariaDB 10.1.1 merged the
   Facebook/Kakao patch (`innodb_defragment`), which moves records to fill pages
   and frees the ones that end up empty. It is opt-in via a system variable and
   works at PAGE granularity. That is the relocate branch this plan ruled out,
   shipped as a switch rather than a default — consistent with ruling it out for
   the default path.
3. **MariaDB 11.2.0 shrinks the InnoDB system tablespace by reclaiming unused
   space AT STARTUP.** This is automatic reclamation, and it is arc A's
   operation — but triggered at neither of the two moments this plan considered.
   See [README.md § Open design questions](README.md) 4.

Also confirming the shared model: InnoDB deletes only mark rows, and the freed
space is never returned to the OS on its own. Same shape as loft.

Sources: [MariaDB OPTIMIZE TABLE](https://mariadb.com/docs/server/ha-and-performance/optimization-and-tuning/optimizing-tables/optimize-table) ·
[Defragmenting InnoDB Tablespaces](https://mariadb.com/docs/server/ha-and-performance/optimization-and-tuning/optimizing-tables/defragmenting-innodb-tablespaces) ·
[MariaDB 10.1.1 defragmentation](https://mariadb.org/defragmenting-unused-space-on-innodb-tablespace/) ·
[Percona: reclaiming space with file-per-table](https://www.percona.com/blog/how-to-reclaim-space-in-innodb-when-innodb_file_per_table-is-on/)

### B2 — implement the winner behind a flag, off

**The code point below is UNSAFE, and the reasoning that chose it is wrong.**
Establishing that is what B2 did first; the corrected step follows.

#### ~~The specified code point~~ — falsified

> - **Code point:** `src/database/allocation.rs::bind_path` (2514), the
>   fresh-image branch […] this is already the one place an image is composed.
> - **B runs at a moment the program already chose, once, and produces a fresh
>   file; there is no mapping to invalidate** […] because the live store is
>   untouched.

The load-bearing claim was **"nothing holds a `DbRef` into a fresh copy, so
there are no inbound pointers to fix"**. It is false, and one probe kills it:

```loft
e = h[7]                        // an interior DbRef
store_persist_bind(h, path)
println("after bind: {e.label}")   // prints label-7 — the ref is LIVE across the bind
```

`bind_path` does not leave the live store untouched: its last act is
`self.allocations[slot_idx] = new_store`, so **the fresh image IS adopted as the
live store**. Every position a compaction moved would be a dangling `DbRef` in
the running program — held in interpreter stack slots and native locals, which
nothing can enumerate or rewrite. The reason it looks safe today is exactly the
thing compaction removes: the image is a byte-for-byte copy, so every record
keeps its position.

So the property that matters is not *"is the copy fresh"* — it is **"can a
`DbRef` into this store's interior be live right now"**.

#### The corrected code point: compaction belongs at LOAD, not at write

Enumerate the moments a store's records could move, and ask that question of
each:

| moment | interior `DbRef` live? | verdict |
|---|---|---|
| `bind_path`, fresh-file branch (WRITE) | **yes** — the program just built the collection | unsafe |
| `bind_path`, existing-file branch (LOAD) | no — the collection was declared empty; its records come into existence here | **safe** |
| `store_load` / `store_load_key` (LOAD) | no — same shape, and it already replaces the slot's bytes wholesale | **safe** |
| `store_reclaim` | yes | safe only because it moves nothing (arc A) |

The load path is safe for a reason stronger than "probably nobody holds one":
**it already invalidates interior references today.** `Store::load` replaces the
slot's bytes wholesale, so any interior `DbRef` held across it is already
meaningless — compacting there adds no hazard that loft's contract does not
already carry. The write path is the opposite: it preserves every position
today, so compacting there would introduce a hazard that does not exist.

One position must still be honoured: the **root stays at `PRIMARY`** with its
layout unchanged, because the collection variable itself is a `DbRef` at the
root and it does survive. A compactor emits the root first, so this holds by
construction rather than by care.

This also explains why B1's prior-art find was pointing here all along:
**MariaDB 11.2.0 shrinks at STARTUP** — the load moment — not when writing.
What was filed as a proposed A5 is the same insight arriving from the other
direction.

#### What to build, and what is already built

The rebuild does not need new traversal code. `Stores::copy_claims` is already a
type-directed, **cross-store** deep copy that constructs the destination fresh —
re-inserting into hash/index spines, bulk-copying vector elements — over the
`for_each_owned_child` keystone. And it already does the thing B1 asked B2 to do:
`copy_claims_seq_vector` claims `1 + (size * length).div_ceil(8)`, i.e. **at the
vector's LENGTH**, not at the source block's size. So the quantised per-vector
overhead B1 measured through the loft-level insert path should not appear here.

So B2 is a composition, not an algorithm: claim a root record in a fresh store,
`copy_claims` the collection into it, swap the slot. What it must carry:

- **Refusals, fail-safe.** `copy_claims` panics on `Radix` (spatial) and the
  keystone returns an empty walk for it; a store with `known_type == u16::MAX`
  has no schema to walk. Any of these must fall back to loading exactly as
  today — never a partial compaction.
- **The gate.** Only compact when the interior is worth it, from the walk the
  load already does.
- **The oracle.** B0's digest across the load, and B1's before/after sizes.
- **Off by default**, as this step always said.

`store_load` does **not** compact today — measured, byte-identical (215,824 →
215,824), consistent with the ruled-out note about `Store::load` replacing bytes
wholesale.

#### Built — `LOFT_COMPACT_ON_LOAD=1`, off by default

`Stores::compact_slot`, called from `load_path` behind the flag. It claims the
root in a fresh store (a fresh store hands out `PRIMARY` first, so the root
cannot move), raw-copies the root block, then `copy_claims` rebuilds every heap
child into the new arena and rewrites the pointers. Measured on the plan's own
shape — 2,000 records grown, dropped to 200, tail already reclaimed by arc A:

```
source (post-arc-A)   128,744 bytes
loaded + compacted     26,992 bytes      4.8x smaller, digest unchanged, store_verify sound
```

and 215,824 → 30,648 (7.0×) on the simpler shape. Both backends identical. The
whole suite passes with the flag forced ON as well as off (3,640/3,640 each) —
though the paths that actually exercise it are the `store_load*` family, so read
that as "nothing it touches broke", not as full coverage.

**Refusals are exhaustive and say why.** `type_is_compactable` matches `Parts`
with **no catch-all**, so a variant added later is a compile error rather than a
silently-skipped shape. It declines `Radix` (`copy_claims` panics on it and the
keystone returns an empty walk) and `DbRef` (a pointer into ANOTHER store, which
a within-store rebuild cannot carry), plus a store with no recorded type, a
read-only or borrowed store, and a store already dense. Each refusal names
itself under `LOFT_LOADER_STATS`, because a refusal that reads the same as a
dense store is one nobody can test or diagnose.

**A slot leak, found and fixed.** `compact_slot` borrows a scratch slot via
`adopt_store` and returns it with `take_store` — but `take_store` leaves a freed
sentinel WITHOUT setting the slot's free bit (it is written for a store handed
out to outlive the table, the REPL session store, where the slot should stay
reserved). `find_free_slot` only returns a slot whose bit is SET, so every load
would have burned a slot number for the life of the process. Invisible from
loft — the store report counts only LIVE stores and a sentinel is not one — and
invisible in `LOFT_STORES=log`, which does not trace this allocation path. Fixed
with a named `Stores::release_slot`, and pinned by `slot_recycling_tests` so the
asymmetry is recorded rather than re-discovered.

**Verified by** `store_compact_b2_rebuilds_at_load_without_losing_anything`:
flag off ⇒ byte-identical to the source; flag on ⇒ at least halved, with B0's
digest unchanged and `store_verify` sound; the root reference read through after
the rebuild; the store still taking writes; and the refusal path declining with
its reason while still loading correctly. Both probes falsify it — skipping
`copy_claims` and accepting `DbRef` each fail the test.

**Still open for B3:** the flag defaults off, so nothing reaches a user yet;
`bind_path`'s existing-file branch is the other load moment and is not wired up
(it needs the compacted heap store written back and re-mapped); and the gate is
"did it get smaller", not the live-vs-mark ratio the step asked for.

### B3 — on by default, documented, probes graduated — **DONE**

**Default ON**, `LOFT_NO_COMPACT_ON_LOAD` opts out — the shape loft uses for its
other default-on behaviours. It can be a default because it is gated and because
it only runs where records may move.

**The gate, measured rather than guessed.** Compaction fires when the interior
free space exceeds an eighth of the high-water mark. An eighth is not an
arbitrary constant: it is the slack the image format already carries on purpose
(`bind_path` sizes an image at the mark PLUS an eighth, loft#710), so there is
one home for "an eighth is the slack we accept".

The estimate is a **lower bound**, not a prediction — recovery runs consistently
above it, because a rebuild also right-sizes live structures the metric counts as
data (a hash keeps the bucket array it grew to at its peak):

| inner | recovered |
|---|---|
| 0% | 0% (declined) |
| 2%, 9% | declined |
| 17% | 24% |
| 26% | 33% |
| 43% | 57% |

So the gate is conservative by construction: it never fires where nothing is to
be had, and declines some cases that would have paid moderately. That is the
right bias for a default — declining a marginal win costs a little space;
taking a marginal loss costs every load. A store at or below the image floor is
also declined: a bound image is padded up to the floor regardless.

**`bind_path`'s existing-file branch is wired up** — the case the plan opened
with, since a bound store is what a long-lived store normally is. `compact_slot`
leaves a dense HEAP store, so the file is rewritten and re-mapped; "the hash IS
the file" has to keep being true. The image goes to a temp file and is renamed
over, so the file is never half-written, and every failure path re-binds from
whatever the file holds rather than leaving a heap store that silently stops
persisting.

Measured across runs: written at 180,104 bytes, re-bound at **26,992** — same
count, same digest, `store_verify` sound, root reference reading, still bound.

**Two vacuous assertions caught by falsifying the test**, both worth the note:

- *"Still bound"* could not tell a bound collection from a heap copy that wrote
  the file once — every size, digest and count assertion passes either way. Only
  a THIRD process can tell: the reload run writes a record after compacting, and
  the next run must see it. Skipping the re-bind now fails the test.
- *"A dense store's file is untouched"* tested nothing about the gate, because a
  dense store rebuilds to about the size it already was. The assertion had to be
  on the **reason** the loader gives, which is why refusals name themselves.

**And an instrument trap, hit a second time.** The test's own `count`/`digest`
helpers iterate the bound collection, and that key-sorted snapshot is claimed
INSIDE the store — 2,000 records took the file 187,784 → 438,160, which reads
exactly like compaction having inflated it. Stat the file the instant the bind
returns. B1 recorded this; it still caught the next person, which is the argument
for it being written down where the next person is.

**A defect this work introduced, and the measurement that was not one.** B3
reported "re-binding a store with no slack grows its file 2.33×" as a
pre-existing thing to chase. Both halves were wrong, and the matrix said so
immediately: re-binding grows nothing, at any size, dense or fragmented, with or
without `store_reclaim` — every cell ratio 1.000.

What actually happened is the **B1 instrument trap for the third time**. The
write process printed `count`/`digest` AFTER capturing the file size; those
traversals grew the file before the process exited, and the reload process then
read a file that was already 438,160. The re-bind was innocent; my own printout
was the mutation, and I attributed it to the operation I happened to be studying.

Chasing it properly found a real defect — **mine, from arc A**:

`bind_path` sized the image `live_end + live_end/8` **`.min(src_words)`** —
"never larger than the capacity we would have written before". Safe while
capacity sat well above the mark. `store_reclaim` trims capacity TO the mark, so
the clamp collapsed the eighth to ZERO for exactly the stores someone had just
tidied — and the eighth is the thing loft#710 added to keep a bound store off the
7/3 resize cliff. Arc A disabled loft#710's protection, and the comment right
above the clamp predicts the consequence in words.

The claim that trips it is the most ordinary one there is: **reading the
collection**, because iterating a keyed collection claims its key-sorted snapshot
inside the store. Measured on a 2,000-record hash:

| | after bind | after ONE read |
|---|---|---|
| `store_reclaim` then bind | 187,784 | **438,160** |
| bind, never reclaimed | 211,256 | 211,256 |
| after the fix, either way | 211,256 | 211,256 |

So tidying up first made the file **2.07× larger** than not bothering. The clamp
is gone: the eighth is deliberate and has to survive a caller who tidied first.
Guarded by `persisted_image_keeps_its_slack_after_store_reclaim`, which fails
(187,784 → 438,160) the moment the clamp is put back.

### ~~B3 — on by default, documented, probes graduated~~ (original)

---

## Composition matrix

The done-bar for both arcs. Every cell on `--interpret` **and** `--native`, with
the digest unchanged in every one.

| Axis | Cells |
|---|---|
| binding | unbound (heap) · bound at create · bound then grown · bound then shrunk |
| shrink shape | oldest survive · newest survive · interleaved survivors |
| coalescing | lazy (never swept) · already swept |
| after reclaim | re-grown · re-bound · reloaded via `store_load` |
| durability | plain · with a `.dmeta` sidecar (F4) |
| size | below the 1024-word floor (F6) · at it · far above |

A no-output cell is vacuous: assert the digest AND the file size AND
`fl_validate()`, and prove the harness can fail by running it against a build with
the gate off.

**The collection-KIND axis was the largest gap, and it is now covered.**
Compaction accepts `Sorted`, `Array`/`Ordered`, `Index` and `ChildRec` as well
as `Hash`; every earlier test built a `hash<Rec[id]>`. Per kind — build,
fragment, persist, `store_load` back, and require the count, the digest and
`store_verify` to survive:

| kind | verdict | rebuilt |
|---|---|---|
| `hash<Rec[id]>` | compacted | 4,902 words |
| `sorted<Rec[id]>` | compacted | 6,700 words |
| `index<Rec[n,-id]>` | compacted | 19,086 words |
| nested (records owning a `vector<Sub>`) | compacted | 17,013 words |
| `spatial<Pt[x,y]>` | **declined** — `Radix` | — |

Two facts the cells taught, both about how to fragment a collection rather than
about compaction:

- **An index removed from the TOP leaves `inner 0%`** — the freed nodes coalesce
  into the tail and `store_reclaim` takes them, so compaction rightly declines
  and the cell proves nothing. Keeping every fifth instead leaves `inner 80%`.
  The removal PATTERN decides whether there is anything to compact, the same
  lesson arc A's shrink-shape axis taught.
- **A small collection lands at the image floor**, where compaction declines by
  design, so a kind's cell has to be sized above it to test anything.

**Two shapes could not be built at all** — both pre-existing, both reproducing
on the released 2026.7.2 binary, so neither is compaction's doing:

- `#remove` in a filtered loop on an `index` whose records own a `text`
  SIGSEGVs the interpreter and overflows the native stack
  ([loft#718](https://github.com/loft-lang/loft/issues/718)). Worked around with
  key assignment, which is a different path.
- The `ordered<T>` secondary shape needs a struct declaring BOTH a
  `sorted<T[..]>` and an `index<T[..]>` field — and merely DECLARING it hangs the
  interpreter and miscompiles on `--native`
  ([loft#719](https://github.com/loft-lang/loft/issues/719)). That cell is not
  skipped for convenience; the shape itself does not work, so `Array`/`Ordered`
  remains the one accepted kind with no coverage.

**The `durability` and `size` cells were the last two open, and they were open in
the worst way: the refusals were implemented, documented as shipped behaviour,
and untested.** One of them did not work at all (F4, above). Both are now covered
by `reclaim_and_compaction_refuse_a_sealed_store_and_a_floor_sized_one`, which
falsifies — restoring the old F4 subject, or removing the F6 floor, each fails
it. Carrying a refusal without a positive control is how a guard gets to look
like a feature.

**Found while testing it, pre-existing and not fixed:** *reading* a bound
collection invalidates its durable seal. A bare re-bind keeps `store_durable_check`
true; ONE traversal makes it false with the file LENGTH unchanged, because the
key-sorted snapshot is claimed inside the store and changes its bytes. So a
program that seals, then reads, then checks, sees corruption in a store nothing
wrote to. Worth its own look — it is the same snapshot-in-a-bound-store behaviour
that inflated two measurements in this plan.

## See also

- [README.md](README.md) — status, measurements, ruled-out list.
- [DATABASE.md](../../DATABASE.md) — the shipped high-water-mark image.
- [LIFETIME.md](../../LIFETIME.md) — removal frees what the element owned, without
  which none of this is measurable.
