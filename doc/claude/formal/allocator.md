<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/allocator.md — how one store places its blocks (strict)

**Catalogue:** @F3 (heap/store).

> **Rules then deviations** (see [README](README.md)). [heap.md](heap.md) defines what the
> store STEPS do — claim, read, write, copy, free — and what a program may observe of them.
> This chapter defines how ONE store lays out its blocks underneath: where a claim lands, when
> a freed block is merged, which index finds it again.  No VALUE a program computes depends on
> these choices (a claim's bytes are undefined, `H-Claim`; a freed record is unreachable), so
> every rule here is free to change for speed.  The one window onto them is the store report
> (`store_memory()`: free blocks, mergeable pairs, the largest hole), which describes each
> store as its pending merges will leave it.  What each rule must keep is that every free block is found
> again by exactly one index, that the block chain tiles the store, and that a store grows no
> sooner than a free block that holds the claim is out of reach.

## Rules

```
  (H-FreeFooter) inside one store, a FREE block of n words carries −n at BOTH ends: its
                 header word and the HIGH half of its LAST word (the tree node's color
                 rides bit 31 of its RIGHT link, so the half-word is clear at every
                 tracked size; a one-word block's footer shares its header's word).  A
                 record delete then coalesces BACKWARD in O(1): the footer at the freed
                 block's left edge NAMES a candidate predecessor, its header must agree,
                 and the free TREE must hold that exact block — claimed data can spell a
                 false footer and header, and the tree cannot lie (falsified: without the
                 tree confirmation, a fabricated footer+header pair merges a freed block
                 into the middle of a live claim).  A one-word predecessor is untracked
                 and unconfirmable: it is left, and the lazy O(blocks) sweep stays armed
                 for exactly that case.  Footers live in FREE space only — a persisted
                 image is unchanged, and an image written before footers existed is
                 re-footed by the open walk.
  (H-Wilderness) inside one store, the free block that ENDS the store (the wilderness)
                 is held beside the free tree rather than in it, and every tree
                 operation treats it as the node it would have been: a block of at
                 least the tree's minimum size that ends the store is recorded as the
                 wilderness instead of inserted, removing it clears the record, and a
                 best-fit take weighs it against the smallest fitting node by the
                 tree's own (size, position) order — the wilderness has the highest
                 position of any free block, so a node of equal size precedes it.
                 The block every claim takes, and so the store's layout, is therefore
                 the one the tree alone would give; what changes is that a claim from
                 the tail and a delete into it cost no tree delete, insert or
                 rebalance.  At most one wilderness exists, it never overlaps a claim,
                 and the open walk (and every re-tiling) re-derives it.
  (H-Carve)      inside one store, a best-fit claim of n words from a tree node of b
                 words with b − n ≥ n puts the (b − n)-word remainder IN THE NODE'S
                 PLACE — the node's links, its color, its parent's pointer — instead of
                 deleting the node and inserting the remainder.  The node is the
                 SMALLEST of at least n words by the tree's (size, position) order, so
                 every node before it is smaller than n: a remainder of at least n
                 words still follows that predecessor, and being smaller than b it
                 still precedes the successor.  The tree's key set is the one the
                 delete and insert leave, so every later claim takes the same block
                 and the layout is unchanged; what changes is that the claim costs no
                 tree delete, insert or rebalance.  A remainder under n words, or
                 under the tree's minimum, takes the delete and the insert.
  (H-LazyFree)   inside one store, a delete of a SMALL block (at most 64 words) during
                 the store's LAZY phase merges with its free neighbours in O(1) as the
                 exact delete does — forward by header, backward by a footer CONFIRMED
                 by the phase's record of lazily freed block starts (claimed data can
                 spell a footer, and that record cannot lie) — and inserts the merged
                 block into no tree; a block that ends the store becomes the wilderness.
                 A larger block (a vector rung, a hash table) is tracked at its delete
                 as ever, the tree's insert being nothing beside the block's own copy.
                 So a claim in the phase finds what the tree holds (the wilderness, the
                 large blocks, the remainders claims leave) or an untracked block the
                 chain walk meets, and a scratch freed each round leaves the layout as
                 it found it.  The count the phase is judged by is the untracked words
                 held NOW: a claim, a resize or a merge that consumes such a block takes
                 it off, and a tree rebuild clears the record whole.  The phase ends at the
                 first claim once the untracked words reach a floor (256) AND a fifth of
                 the extent written so far, whatever the wilderness holds: ONE sweep
                 merges every free block and rebuilds the tree, and from then on each
                 delete is tracked at its site as before; an explicit reclaim ends it
                 the same way.  A fresh or reset store starts the phase again; a store
                 bound to a file never enters it, because a file is read, reclaimed and
                 paged by its layout.  Every block is still free or claimed by its
                 header, so the usage walk, an image and the open walk read the same
                 store; what changes is WHEN the tree learns of a free.  A store built
                 once and released whole never pays for its deletes, a tree whose dead
                 words stay a small share of its live ones stays lazy however big it
                 grows, and a vector ladder — dead about half of what it wrote — sweeps
                 within its first rungs, so the working set never doubles.  Below the
                 bound a claim the wilderness cannot hold grows the store, or takes an
                 untracked block the chain walk meets on its way to growing.
  (H-ScanIndex)  a claim the tree and the wilderness cannot serve takes the FIRST free
                 block in address order that holds it, or grows the store from its tail
                 block.  The store's indexes answer it — the lowest lazily freed block
                 (`H-LazyFree`) that holds the claim, a tail whose free footer an index
                 confirms — and the block chain where they cannot: the same block.
  (H-ReadSurface) a store opened from a file only to be READ — a cached stdlib or program
                 image — writes nothing into the file: its free-block tree and footers
                 (`H-FreeFooter`, `H-Wilderness`) are left unbuilt, since a read surface
                 claims and frees nothing.  The file's mapping is shared by every process
                 that opens it, so a rebuild in place is a write another start reads.
  (H-Park)       inside one store, a delete of a block of 2 to 16 words PARKS it: the
                 block is written free (`H-FreeFooter`'s header and footer) and pushed on
                 a list for its EXACT size, doubly linked through the two half-words a
                 tree node keeps its links in, and recorded in the set of parked starts —
                 what tells a parked block from a tree node or a lazily freed one, since
                 claimed data can spell a free header and the set cannot lie.  It merges
                 with nothing: a delete beside it stops its merge there, and a backward
                 merge that finds it neither merges nor arms the sweep.  A claim of a
                 parked size pops its list before anything else is asked.  A claim the
                 tree and the wilderness cannot hold first FLUSHES every list — each block
                 freed as the exact delete frees it, merging with its tracked neighbours,
                 a still-parked neighbour merging backward at its own turn — and asks the
                 tree again before it sweeps, scans or grows; so no claim scans past a
                 parked block, and the store grows no sooner than without the lists.  A
                 consumer that takes a free block (an exact-position claim, an in-place
                 resize, the growth step) takes a parked one off its list.  Every block is
                 still free or claimed by its header, so the usage walk, an image and the
                 open walk read the same store, and the store report counts a free
                 run holding a parked block as the ONE block its flush will leave (a
                 deferred merge, not a missed one); a tree rebuild (the sweep, an open, a
                 resync) and a reset empty the lists, and a reclaim or a shrink flushes them
                 first.
                 The lists and the set are CONTENT: an exchange of two stores' contents
                 (`(H-SwapIn)`) carries them with the memory they index, so the store
                 that now holds a parked block is the one whose claim pops it.
                 Not parked: a block that ends the store (it joins the wilderness), a
                 block freed while the edit journal records, and every block of a store
                 bound to a file.  `LOFT_NO_PARK=1` turns it off.

```

## Deviations

OPEN: **0**.  The history of these rules (each one's measurement and the defect its guard
was written for) is in [heap-history.md](heap-history.md), where they were first written.

## Conformance

Every rule is checked in `src/store.rs`'s unit tests, read independently of debug
assertions (the test profile turns them off): `tree_in_order`, `chain`, `check_wilderness`
and `check_parked` re-derive the tree, the block chain, the wilderness and the parked lists
from the store's bytes after every step of a seeded churn of claims, deletes and resizes.

- `H-FreeFooter` — `a_fake_footer_in_claimed_data_never_merges`.
- `H-Wilderness` — `the_wilderness_takes_the_blocks_the_tree_takes` (the block chain equal to
  the tree-only allocator's at every step), `a_fresh_store_never_claims_its_wilderness`.
- `H-Carve` — `a_carve_in_place_takes_the_blocks_the_delete_and_insert_take`.
- `H-LazyFree` — `a_lazy_delete_is_untracked_and_the_claim_takes_the_tail`,
  `at_the_growth_bound_one_sweep_ends_the_phase`, `below_the_floor_a_lazy_store_grows_rather_than_sweeps`.
- `H-ScanIndex` — `the_indexed_scan_takes_the_block_the_walk_takes`.
- `H-ReadSurface` — `a_read_surface_open_writes_nothing_to_its_file`.
- `H-Park` — `a_parked_block_is_the_next_claim_of_its_size`,
  `a_flush_merges_parked_neighbours_into_one_block`, and
  `parking_keeps_every_live_record_and_its_bytes` (every live record's bytes intact and the
  lists consistent at every step, with the lazy phase on and off), and
  `the_report_counts_a_parked_run_as_the_block_its_flush_leaves`; the published example is
  `tests/scripts/947-feature-worked-examples.loft` (`@FTR-002`, a contiguous drop reads as a
  handful of blocks).  The rules above it pin
  their own layouts with parking off.
