<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 123 — Give a store's file back

## Status

**Both arcs are complete.** Arc **A** ships `store_reclaim(collection)` — opt-in,
the free TAIL, at any moment the program chooses. Arc **B** compacts the
INTERIOR automatically **at LOAD** (`store_load`, and `store_persist_bind` on an
existing file), gated on the interior exceeding an eighth of the high-water mark;
`LOFT_NO_COMPACT_ON_LOAD` opts out. A bound store that peaked at 2,000 records
and settled at 200 came back at 180,104 bytes every run and now loads at 26,992.

The step that had to change: B2 specified compaction when WRITING the image, on
the grounds that nothing holds a `DbRef` into a fresh copy — but `bind_path`
ADOPTS the image it writes as the live store, and a program demonstrably holds
interior references across it. Compaction belongs at LOAD, where the bytes are
already replaced wholesale.

Remaining: nothing. The A5 proposal below was weighed, built and **rejected on
its own measurement** — see A5.

One defect this work introduced and has now fixed: **`store_reclaim` before a
bind used to remove the image's eighth of slack**, because the image size was
clamped to the arena's capacity and arc A trims capacity to the mark. The next
claim then paid loft#710's 7/3 cliff — and the claim that tripped it was simply
READING the collection. Measured: 187,784 bytes at bind, 438,160 after one read,
against 211,256 for never reclaiming at all. Both paths now hold at 211,256.
See DESIGN.md § The re-bind measurement that was not one.

Promoted from [loft#713](https://github.com/loft-lang/loft/issues/713) (closed in
favour of this plan), itself split out of [loft#710](https://github.com/loft-lang/loft/issues/710)
(closed — everything it reported is fixed). Every cheap alternative has been
measured and ruled out, so this is a **build** plan, not an investigation.

## Goal

A store's file size follows what it holds for the whole life of the store —
including a **bound (mmap) store**, which is what a long-lived store normally is.

## Effort + design

- **Effort:** M (A is S, B is M)
- **Design:** ✓ — A opt-in via `store_reclaim`, B a rebuild-and-swap at persist
- **Last touched:** 2026-08-01

## The measurement this plan exists for

A bound store: bind at 300 records, grow to 3000, shrink back to 300.

```
after bind (300 live)   file   177,696   used 89%   tail 11%   inner  0%
grown to 3000           file 2,257,392   used 70%   tail 30%   inner  0%
shrunk back to 300      file 2,257,392   used  8%   tail 59%   inner 33%
```

**12.7× the file for the same 300 records**, and it never comes back down:
`resize_store` returns early on `to_size <= self.size`, so a bound store's file
only ever grows.

The waste splits in two, and the split is only visible with the `tail%` / `inner%`
reading `store_memory()` now carries:

| | recovers | needs | risk | when |
|---|---|---|---|---|
| **A** — coalesce + truncate a bound store | the free TAIL above the last record | no record movement | low — everything above the last record is free by definition | **opt-in**, `store_reclaim(collection)` |
| **B** — compact when LOADING an image | the INTERIOR free space between records | a rebuild into a fresh store, then swap | low — the load path already invalidates interior refs | automatic, gated on the ratio |

**A is opt-in on purpose.** Its benefit is near-zero in every workload measured
here except one (churn and refill both hold capacity flat, so a reclaimer would
walk and find nothing), its trigger cannot be made cheap enough to sit on the
free path, and a store that shrinks then grows again would be truncated and
re-grown at 7/3 — thrashing the very case it targets. Whether a drop is permanent
is something the program knows and the runtime cannot infer. B stays automatic
because persist already walks the chain, so its gate costs nothing new.

## Sub-arcs

Build order, failure paths and code points: **[DESIGN.md](DESIGN.md)**.

| Item | Source | Status |
|---|---|---|
| **A0** — trust the high-water mark (`walk_complete`) | [DESIGN.md](DESIGN.md) | **Done** — inert |
| **A1** — `Store::shrink_to`, no caller | [DESIGN.md](DESIGN.md) | **Done** — inert |
| **A2** — `reclaim_tail` = coalesce + shrink, no caller | [DESIGN.md](DESIGN.md) | **Done** — inert |
| **A3** — expose `store_reclaim(collection)` (opt-in) | [DESIGN.md](DESIGN.md) | **Done** — the behaviour change |
| **A4** — docs + probes graduate (no default to flip) | [DESIGN.md](DESIGN.md) | **Done** |
| **B0** — the digest oracle, before any compaction code | [DESIGN.md](DESIGN.md) | **Done** — sees 5 loss modes, blind to 3 layout levers |
| **B1** — measure what rebuild-and-swap costs | [DESIGN.md](DESIGN.md) | **Done** — 77-86% back at ~0.6 µs/record, idempotent |
| **B2** — implement behind a flag, off | [DESIGN.md](DESIGN.md) | **Done** — `LOFT_COMPACT_ON_LOAD=1`, 4.8-7.0x, at LOAD not at write |
| **B3** — on by default, documented, probes graduated | [DESIGN.md](DESIGN.md) | **Done** — default ON, gated, `bind_path` wired |
| **F4/F6 refusals** — the last two done-bar cells | [DESIGN.md](DESIGN.md) | **Done** — F4's guard was on the wrong subject and did not fire |
| **Collection-kind matrix** for arc B | [DESIGN.md](DESIGN.md) | **Done** — hash/sorted/index/nested rebuild, spatial refuses; `ordered` now covered too (loft#719 fixed, `Both` declared in the kinds test) |
| **A5** — reclaim at bind time (proposed) | [README.md](README.md#a5--reclaim-at-bind-time--rejected-measured) | **Rejected** — built and measured; the premise is false and it makes the file grow (loft#727) |
| **loft#727** — the growth A5 surfaced | [README.md](README.md#a5--reclaim-at-bind-time--rejected-measured) | **Fixed** — an iteration-snapshot leak, not a slack policy; `reclaim_tail` keeps the eighth too |

### A — truncate the tail

`resize_store` keeps refusing to shrink — it is the growth path — so the shrink
is a sibling, `Store::shrink_to`. A naive "is the top block free" check would
reclaim almost nothing after a mass removal (2,696 unmerged mergeable pairs
measured), which is why the cut is made at the **high-water mark** instead;
that reads the same swept or unswept, so coalescing is about the interior, not
about how much tail comes back (DESIGN.md § A2).

Safe by construction: everything above the last claimed block is free, so no live
record and no `DbRef` is affected. Measured on the graduated probe, it halves the
file — 247,944 → 124,872 bytes for the same 300 records, digest unchanged.

Reached through `store_reclaim(collection)` — the program says when. "Bare
minimum for actual changes" taken literally: do nothing at all unless asked, and
nothing on the free path ever walks the chain. The planned freed-words tally in
`Store::delete` turned out to be unnecessary AND unsound as an early-out (a store
that only ever grew has a tail with no delete behind it) — dropped, see
DESIGN.md § A3.

### A5 — reclaim at BIND time — **REJECTED, measured**

The proposal: `store_persist_bind` on an EXISTING file is a third moment arc A
never weighed. A rejected "automatic on the free path" (a walk per delete) and
chose "the program says when"; opening a file is neither — the store is being
opened, "the program has not started allocating into it, so there is no thrash
risk and no live mapping to invalidate". MariaDB 11.2.0 reclaims unused space at
startup for that reason. It would have answered the stated cost of A being
opt-in: a knob nobody knows about is a knob nobody uses.

**The premise is false, and the measurement says so immediately.** The program
HAS started allocating by the time it matters: iterating a keyed collection
claims its key-sorted snapshot INSIDE the store, so the first read is an
allocation. Binding is not a quiescent point.

Built anyway (wired into `bind_path`'s existing-file branch, default-on with an
opt-out) and measured on a 4,000-record hash grown post-bind, re-binding ten
times:

| | ten successive binds |
|---|---|
| A5 on | 357,232 → 411,312 → 465,392 → 523,584, reads spiking to 833,536 / 959,728 / 1,085,912 |
| A5 off | **566,472, stable, all ten runs** |

So it converts a stable file into a ratchet: each cycle trims to the mark, the
next read re-grows at 7/3, and the floor settles higher than before. It would
hand every program the behaviour that today needs an explicit call — which is
F7 ("the store keeps growing after a shrink") arriving at the moment the
proposal claimed was safe from it.

**What it did surface** is a defect in what shipped, filed as
[loft#727](https://github.com/loft-lang/loft/issues/727): `store_reclaim` on a
BOUND store already ratchets, because `reclaim_tail` shrinks to the bare
high-water mark and leaves nothing for the next claim. One call costs 1.31×
(566,472 → 740,936 after a single read); calling it each run grows the file
~37 KB per run without bound. Giving `reclaim_tail` the mark's eighth — the same
slack `bind_path` writes into an image — fixes the single-call case outright
(357,232, stable) and halves the ratchet, but does not close it, so it was filed
rather than half-fixed: what remained looked like a design question about what to
reserve against a claim whose size is a function of the collection, not of the
slack.

**It was not a design question — it was a leak, and the issue's control said so
without being read.** "Rock stable at 566,472 for ten runs" was true of the FILE
and false of the store: the census grew every one of those runs, and on run 17
the file jumped 2.33× with `store_reclaim` nowhere in the program. Iterating a
keyed collection claims its key-sorted snapshot inside the store — this plan
measured that four separate times, each time as an instrument trap — and the loop
epilogue never released it, because the release only ever handled a scratch with
a store of its own. So each read left 4 bytes per element behind, permanently for
a bound store, and no reserve of any size can absorb an unbounded leak. Fixed at
the release (`Stores::free_iteration_scratch`); the eighth stays, and with the
leak gone it does close the single-call case. The reason the ratchet only
"halved" under the eighth is now plain: the eighth was absorbing half the leak.

The lesson for this plan is not about slack. It is that the same measurement got
attributed to `store_reclaim` five times — four as a trap to work around, once as
a bug to file against it — and never once to the claim that was actually doing it.
A trap you have learned to route around is an unattributed effect.

Arc A stays opt-in, and the stated cost of that stays stated.

### B — compact the image

**Compact while LOADING the image.** The earlier plan said "while writing", on
the grounds that nothing holds a `DbRef` into a fresh copy — but `bind_path`
ADOPTS the image it writes as the live store, and a program demonstrably holds
interior references across that call, so compacting at write would dangle them
(DESIGN.md § B2). The load path is safe for a stronger reason than absence of
references: it already replaces the slot's bytes wholesale, so it already
invalidates them. The root stays at `PRIMARY`, which is the one reference that
does survive.

Gated on the live-vs-mark ratio, from the walk the load does anyway — so a dense
store pays one comparison.

Ceiling is the fresh-build number: 1,042,528 → ~179,072 bytes for the same 300
records.

## Phase ordering

1. ~~**A first**, and stand-alone.~~ **Done.**
2. ~~**Re-measure** with `store_memory()`'s `tail%` / `inner%`.~~ **Done**, on
   this plan's own bound-store shape (a consumer measurement would still be worth
   having, and would only sharpen the number):

   ```
   before store_reclaim   cap 0.262 MB  used 25%  tail 28%  inner 47%   2701 free blocks
   after                  cap 0.188 MB  used 35%  tail  0%  inner 65%      4 free blocks
   ```

   A takes the whole tail; the sweep collapses 2,701 free blocks to 4. **65%
   interior remains** — that is what B is worth, now measured.
3. **B** — justified. **B0 (oracle) and B1 (cost) are done**; next is B2.

## Composition matrix — Stage A

This plan changes an *operation* (when a store's file is sized), not a value or
type, so the axes that matter are lifecycle and backend, not type-shape:

| Axis | Cells |
|---|---|
| binding | unbound (heap) · bound at create · bound then grown · bound then shrunk |
| shrink shape | oldest survive · newest survive · interleaved survivors |
| coalescing | lazy (never swept) · swept |
| backend | `--interpret` · `--native` |
| reload | file re-opened after truncation · re-bound and grown again |

The done-bar: every cell green on both backends, with the digest unchanged across
truncate/compact — a smaller file that lost data would pass every size assertion.
Probes graduate to `tests/scripts/`.

**Stage A result — every cell green.** Unbound/heap and the coalescing axis are
`src/store.rs`'s unit tests; bound-then-grown-then-shrunk, both backends, and
**reload** (a fresh process binding to the truncated file: 301 records, identical
digest) are `tests/scripts/store_reclaim_123.loft` + its driver.

The **shrink-shape** axis is the one that changes the ANSWER rather than just
passing, so it is worth having in writing. Same store, 3000 records, 300 kept:

| survivors | given back |
|---|---|
| oldest 300 (contiguous, low in the arena) | **50%** |
| newest 300 (high in the arena) | 34% |
| every 10th (spread) | 34% |

A gives back something in all three, because a bound store's 7/3 growth leaves
tail regardless of where the survivors sit. But the mark is where the LAST
survivor ends, so a workload whose survivors sit high keeps a third less. That
gap is arc B's, not a defect in A.

## Open design questions

1. ~~When does A fire?~~ **Settled: the program calls it.** Automatic was
   rejected — the benefit is near-zero in every measured workload but one, the
   trigger cannot be cheap on the free path, and a shrink-then-grow store would
   thrash. See DESIGN.md § A3.
2. **Does `MmapStorage::resize` shrink cleanly** on every target, and what happens
   to a reader that has the file mapped concurrently?
3. ~~Does B relocate, or re-insert?~~ **Settled: rebuild into a fresh store and
   swap.** It removes compaction's whole risk (a fresh store has no inbound
   pointers to rewrite), and it is what InnoDB does — `OPTIMIZE TABLE` is a
   rebuild-and-swap, not an in-place relocation. What remains is measuring its
   cost. See DESIGN.md § B1.
4. ~~**Is any of A's stance contradicted by recent MariaDB?**~~ **Checked (B1).
   Not contradicted — but incomplete.** `OPTIMIZE TABLE` under
   `innodb_file_per_table` is literally rebuild-and-swap, and in-place
   defragmentation ships only as an opt-in switch (`innodb_defragment`,
   MariaDB 10.1.1) — both consistent with what this plan chose. The finding is
   **MariaDB 11.2.0, which shrinks the InnoDB system tablespace at STARTUP**.
   That is arc A's operation made automatic at a third moment this plan never
   weighed: not on the free path (rejected — too expensive) and not "the program
   says when" (shipped), but **at a quiescent point where nothing is running
   yet**. See the A5 note below.

## Ruled out — measured, do not re-chase

- **Memory growth.** There is none. 3000 → shrink to 300 → refill to 3000 held
  capacity at 1.57 MB throughout (97% → 12% → 96%); steady-state churn held
  0.30 MB flat across 36,000 insert+remove operations. The store is a peak
  allocator, not a leaking one.
- **What looked like unbounded growth** was a leak — removal released neither the
  record nor an owned `text`/`vector`. Fixed; no compactor could have reclaimed
  it, because those blocks were still marked LIVE.
- **Coalescing as a size fix on the persist path.** Forcing the sweep took 2,700
  free blocks to 6 and left `inner` unchanged at 45%: merging free blocks never
  moves a live one. It is a prerequisite for **A** and irrelevant to **B**.
- **Serialise/deserialise as a workaround.** `store_load` → persist is
  byte-identical (437,416 → 437,416) because `Store::load` replaces the slot's
  bytes wholesale; re-inserting record-by-record gives 634,264 — *larger*.
- **In-place growth.** `Store::resize` already absorbs an adjacent free block; it
  rarely applies when every vector is followed by another, which is what `reserve`
  sidesteps.

## See also

- [DATABASE.md § What the file's SIZE and BYTES mean](../../DATABASE.md) — the
  shipped half (high-water-mark image, `LOFT_HASH_SEED`).
- [STDLIB.md § `store_memory`](../../STDLIB.md) — `tail%` / `inner%`, the reading
  this plan is triggered by, and `reserve(v, n)`.
- [LIFETIME.md § Removing from a collection](../../LIFETIME.md) — the removal leak
  that had to be fixed before any of this could be measured.
- `@PLN123` — [loft-lang/plans#123](https://github.com/loft-lang/plans/issues/123).
- Promoted from [loft#713](https://github.com/loft-lang/loft/issues/713); source
  report [loft#710](https://github.com/loft-lang/loft/issues/710) (`hit-by:routing`).
