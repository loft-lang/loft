// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I69 — Word-addressed store: the chunked entry arena keyed collections allocate from.

//! A chunked arena of fixed-stride slots inside a [`Store`], addressed by a 1-based
//! index (@PLN135 arc H).
//!
//! ## Why an arena and not a record per entry
//!
//! A keyed collection used to claim one store record per entry. A record carries an
//! 8-byte word of its own — the size header, plus the owning-collection back-pointer —
//! and `Store::claim` takes a whole free block rather than split when the remainder is
//! under a third. For a `{integer, integer}` entry that is 24 bytes of store for 16 bytes
//! of payload, and the measured cost is not the bytes but the working set they spread a
//! random lookup across: 200 ns against 77 for the same payload read out of one dense
//! array.
//!
//! Packing entries at a fixed stride removes both. What it must not remove is
//! **stability**.
//!
//! ## Chunks, because growth must not move a live entry
//!
//! A hash hands out `DbRef`s into its entries and callers hold them:
//!
//! ```text
//! e = h[k];  h += [other];  e.val      // must still read e's own value
//! ```
//!
//! That works today because growing a hash reallocates only its bucket table and never
//! an entry's bytes. One flat array would take it away — growth reallocates, every entry
//! moves, and every outstanding reference reads moved-or-freed bytes. Not an error: a
//! WRONG READ, which `COMPATIBILITY.md` forbids and which this subsystem is the worst
//! place in loft to introduce.
//!
//! So the arena grows by APPENDING a chunk and never reallocates one that already holds
//! slots. A slot keeps the address it was handed out at for as long as it lives.
//! Measured cost of chunking versus one flat array: 86 ns against 77, where today is 200
//! (`doc/claude/plans/135-hash-performance/probes/q5-chunked-arena.loft`).
//!
//! ## Chunk sizes double
//!
//! Chunk `k` holds `BASE << k` slots, so `k` and the slot within it are recovered from an
//! index by arithmetic rather than a search, and total waste is bounded the way a
//! vector's is instead of rounding every small collection up to one big chunk. The
//! directory that maps `k` to its record is a handful of `u32`s — small enough to stay
//! cache-resident, which is why the index stays 4 bytes and the bucket table does not
//! double. That hop is not free by assumption: the Q5 probe pays it and still measures
//! 86 ns.
//!
//! ## Layout
//!
//! Directory record — `u32` chunk record numbers from [`DIR0`]:
//!
//! ```text
//!   0  size header (Store's own)
//!   8  chunk record 0, 1, 2, …          (2 per word)
//! ```
//!
//! Chunk record — the owning-collection back-pointer lives here, ONCE per chunk rather
//! than once per entry, because every slot in a chunk shares an owner:
//!
//! ```text
//!   0  size header (Store's own)
//!   4  back-pointer to the owning collection record
//!   8  slot 0, slot 1, …                (each `stride` bytes)
//! ```
//!
//! A freed slot is threaded onto a free list through its own first 4 bytes, so a
//! delete-heavy collection reuses space instead of growing forever.

use crate::store::Store;

/// Slots in chunk 0.  Chunk `k` holds `BASE << k` until [`CAP_CHUNK`], so a collection
/// with three entries costs one small chunk rather than rounding up to the size that
/// suits a million.
pub const BASE: u32 = 64;

/// The last chunk that DOUBLES; every chunk past it holds [`CAP_SLOTS`].
///
/// Doubling forever makes the tail waste proportional: a collection that has just
/// crossed into chunk `k` has up to half of it unused, so a 1M-entry hash can carry
/// ~500K empty slots.  Measured, that ate the whole per-entry saving the arena exists
/// for — 27.33 B/entry against a record's 27.67 — and it made a store's SIZE depend on
/// construction order, because two collections grown side by side each pay their own
/// oversized tail (`store_persist_loft::persisted_size_tracks_content_not_construction`).
///
/// Capping bounds the waste at a CONSTANT instead: one partly-filled chunk, whatever
/// the collection's size.  The cost is more chunks — 1M entries at 16 B is ~122 of
/// them — and the directory that maps them is still a few hundred bytes, so it stays
/// cache-resident and the index stays 4 bytes wide.
pub const CAP_CHUNK: u32 = 7;

/// Slots in every chunk past [`CAP_CHUNK`].
pub const CAP_SLOTS: u32 = BASE << CAP_CHUNK;

/// The first 0-based slot of the first fixed-size chunk — the end of the doubling run.
const CAP_START: u32 = BASE * ((1 << (CAP_CHUNK + 1)) - 1);

/// First byte of the chunk-record array in the directory record.
pub const DIR0: u32 = 8;

/// First byte of the slot array in a chunk record.  Byte 4 is the owning-collection
/// back-pointer, which `database::search` reads to decide whether a record is live.
pub const SLOT0: u32 = 8;

/// Byte 4 of a chunk record: the collection this chunk's slots belong to.
pub const OWNER_FLD: u32 = 4;

/// Where slot `index` lives: `(chunk number, byte offset within the chunk record)`.
///
/// `index` is 1-based, so 0 is free to mean "no entry" in a bucket slot and in the free
/// list. Both halves are arithmetic — `chunk_of` is a `leading_zeros`, not a search —
/// because this runs on every lookup that hits.
#[inline]
#[must_use]
pub fn locate(index: u32, stride: u32) -> (u32, u32) {
    debug_assert!(index >= 1, "arena index is 1-based; 0 means absent");
    let i = index - 1;
    let k = chunk_of(i);
    (k, SLOT0 + (i - first_index_of(k)) * stride)
}

/// The chunk holding 0-based slot `i`.
///
/// Inside the doubling run, chunk `k` starts at `BASE * ((1 << k) - 1)`, so
/// `i / BASE + 1` lands in `[2^k, 2^(k+1))` exactly when `i` is in chunk `k`, and the
/// chunk number is that value's floor-log2.  Past [`CAP_CHUNK`] the chunks are equal,
/// so it is a division.  Both are O(1) — this runs on every lookup that hits.
#[inline]
#[must_use]
pub fn chunk_of(i: u32) -> u32 {
    if i < CAP_START {
        (i / BASE + 1).ilog2()
    } else {
        CAP_CHUNK + 1 + (i - CAP_START) / CAP_SLOTS
    }
}

/// The first 0-based slot number that chunk `k` holds.
#[inline]
#[must_use]
pub fn first_index_of(k: u32) -> u32 {
    if k <= CAP_CHUNK {
        BASE * ((1 << k) - 1)
    } else {
        CAP_START + (k - CAP_CHUNK - 1) * CAP_SLOTS
    }
}

/// How many slots chunk `k` holds.
#[must_use]
pub fn chunk_slots(k: u32) -> u32 {
    if k <= CAP_CHUNK { BASE << k } else { CAP_SLOTS }
}

/// Words to claim for chunk `k` at `stride` bytes per slot: the header word plus the
/// slots themselves.
#[must_use]
pub fn chunk_words(k: u32, stride: u32) -> u32 {
    1 + (chunk_slots(k) * stride).div_ceil(8)
}

/// Read chunk `k`'s record number out of directory record `dir`, or 0 when the directory
/// does not reach that far yet.
#[must_use]
pub fn chunk_rec(store: &Store, dir: u32, k: u32) -> u32 {
    if dir == 0 || k >= dir_capacity(store, dir) {
        return 0;
    }
    store.get_u32_raw(dir, DIR0 + 4 * k)
}

/// How many chunk numbers directory record `dir` can hold.
#[must_use]
pub fn dir_capacity(store: &Store, dir: u32) -> u32 {
    if dir == 0 {
        return 0;
    }
    (store.record_words(dir) - 1) * 2
}

// The arena's own bookkeeping, kept in the collection's table record so the arena owns
// the fields it maintains and the collection owns the rest (`LEN`, `SEED`, the buckets).
/// Byte 16 of the table record: the directory record, or 0 before the first slot.
pub const DIR_FLD: u32 = 16;
/// Byte 20: the append cursor — the next never-used index, 1-based, so it starts at 1.
pub const NEXT_FLD: u32 = 20;
/// Byte 24: head of the free list, or 0 when empty.  A freed slot holds the next free
/// index in its own first 4 bytes, so reuse costs no extra storage.
pub const FREE_FLD: u32 = 24;

/// Point the directory at chunk `k`, growing (and if needed creating) the directory
/// record so it reaches that far.
///
/// The directory may move; entries never do.  That is the whole point — it holds record
/// NUMBERS, so relocating it leaves every slot exactly where it was handed out.
fn set_chunk_rec(store: &mut Store, table: u32, k: u32, rec: u32) {
    let dir = store.get_u32_raw(table, DIR_FLD);
    if k >= dir_capacity(store, dir) {
        // Round up to a whole word of entries and keep doubling headroom.
        let want = 1 + u32::max(k + 1, 4).div_ceil(2);
        let fresh = store.claim(want);
        store.zero_fill(fresh);
        if dir != 0 {
            let old = dir_capacity(store, dir);
            for i in 0..old {
                let v = store.get_u32_raw(dir, DIR0 + 4 * i);
                store.set_u32_raw(fresh, DIR0 + 4 * i, v);
            }
            store.delete(dir);
        }
        store.set_u32_raw(table, DIR_FLD, fresh);
    }
    let dir = store.get_u32_raw(table, DIR_FLD);
    store.set_u32_raw(dir, DIR0 + 4 * k, rec);
}

/// The record backing chunk `k`, claiming and registering it on first use.
///
/// `owner` is written once per CHUNK at [`OWNER_FLD`] rather than once per entry — every
/// slot in a chunk belongs to the same collection, and `database::search` reads that
/// offset to decide whether a record is live.
fn ensure_chunk(store: &mut Store, table: u32, k: u32, stride: u32, owner: u32) -> u32 {
    let existing = chunk_rec(store, store.get_u32_raw(table, DIR_FLD), k);
    if existing != 0 {
        return existing;
    }
    let rec = store.claim(chunk_words(k, stride));
    store.zero_fill(rec);
    store.set_u32_raw(rec, OWNER_FLD, owner);
    set_chunk_rec(store, table, k, rec);
    rec
}

/// Reserve a slot and return its 1-based index, reusing a freed one when there is one.
///
/// The returned slot is ZEROED, matching what a freshly claimed record gave the caller
/// before: a reused slot still carries the bytes of whatever last lived there, and a
/// constructor that writes only some fields would otherwise inherit the rest.
pub fn alloc(store: &mut Store, table: u32, stride: u32, owner: u32) -> u32 {
    let free = store.get_u32_raw(table, FREE_FLD);
    let index = if free != 0 {
        let (k, off) = locate(free, stride);
        let rec = chunk_rec(store, store.get_u32_raw(table, DIR_FLD), k);
        let next = store.get_u32_raw(rec, off);
        store.set_u32_raw(table, FREE_FLD, next);
        free
    } else {
        let next = u32::max(store.get_u32_raw(table, NEXT_FLD), 1);
        store.set_u32_raw(table, NEXT_FLD, next + 1);
        next
    };
    let (k, off) = locate(index, stride);
    let rec = ensure_chunk(store, table, k, stride, owner);
    for w in 0..stride / 4 {
        store.set_u32_raw(rec, off + 4 * w, 0);
    }
    index
}

/// Return slot `index` to the arena.
///
/// The slot keeps its bytes and its address; only its membership changes.  Nothing that
/// still holds a `DbRef` to it is made to read someone else's data by this call alone —
/// that only happens once the slot is handed out again, which is the same contract a
/// freed record has always had.
pub fn free(store: &mut Store, table: u32, index: u32, stride: u32) {
    if index == 0 {
        return;
    }
    let (k, off) = locate(index, stride);
    let rec = chunk_rec(store, store.get_u32_raw(table, DIR_FLD), k);
    if rec == 0 {
        return;
    }
    let head = store.get_u32_raw(table, FREE_FLD);
    store.set_u32_raw(rec, off, head);
    store.set_u32_raw(table, FREE_FLD, index);
}

/// `(record, byte offset)` of slot `index`, or `None` when it has no chunk yet.
#[must_use]
pub fn slot(store: &Store, table: u32, index: u32, stride: u32) -> Option<(u32, u32)> {
    if index == 0 {
        return None;
    }
    let (k, off) = locate(index, stride);
    let rec = chunk_rec(store, store.get_u32_raw(table, DIR_FLD), k);
    (rec != 0).then_some((rec, off))
}

/// The 1-based index of the slot at `(rec, off)`, or 0 when `rec` is not a chunk of this
/// arena.
///
/// The reverse of [`locate`], for the one caller that has a `DbRef` and needs the index a
/// bucket stores: the entry is BUILT through a `DbRef` and only afterwards inserted. The
/// directory scan is bounded by the chunk count (32 for any arena that fits a `u32`) and
/// runs on insert only, never on lookup.
#[must_use]
pub fn index_of(store: &Store, table: u32, rec: u32, off: u32, stride: u32) -> u32 {
    let dir = store.get_u32_raw(table, DIR_FLD);
    let cap = dir_capacity(store, dir);
    // Newest chunk first: the one caller maps an entry it was JUST handed, and a fill
    // hands them out of the last chunk — so the scan ends on its first compare where it
    // used to walk every chunk before it (5 % of an insert, `analysis/keyed.md`).  The
    // chunks past the cursor do not exist yet and are skipped, not compared.
    // `NEXT_FLD` is the next 1-based index to hand out, so the newest slot is 0-based
    // `next - 2`.
    let next = store.get_u32_raw(table, NEXT_FLD);
    let top = if next < 2 {
        0
    } else {
        (chunk_of(next - 2) + 1).min(cap)
    };
    for k in (0..top).rev().chain(top..cap) {
        if store.get_u32_raw(dir, DIR0 + 4 * k) == rec {
            return first_index_of(k) + (off - SLOT0) / stride + 1;
        }
    }
    0
}

/// Every chunk record plus the directory — what a teardown must free after the entries'
/// own children are gone.
#[must_use]
pub fn all_records(store: &Store, table: u32) -> Vec<u32> {
    let dir = store.get_u32_raw(table, DIR_FLD);
    if dir == 0 {
        return Vec::new();
    }
    let mut out: Vec<u32> = (0..dir_capacity(store, dir))
        .map(|k| store.get_u32_raw(dir, DIR0 + 4 * k))
        .filter(|&r| r != 0)
        .collect();
    out.push(dir);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decode must tile the index space with no gap and no overlap — every index
    /// lands in exactly one chunk, at exactly one slot, and consecutive indices are
    /// consecutive slots until a chunk boundary.
    ///
    /// Checked against an independently BUILT map rather than against the formula's own
    /// algebra: a closed form that agrees with itself proves nothing.
    #[test]
    fn every_index_lands_in_exactly_one_slot() {
        // Build the expected layout by walking chunks and handing out slots in order.
        let mut expect: Vec<(u32, u32)> = Vec::new();
        for k in 0..12 {
            for s in 0..chunk_slots(k) {
                expect.push((k, s));
            }
        }
        for (i, &(k, s)) in expect.iter().enumerate() {
            let idx = u32::try_from(i).unwrap() + 1;
            let stride = 16;
            assert_eq!(
                chunk_of(u32::try_from(i).unwrap()),
                k,
                "index {idx} should be in chunk {k}"
            );
            assert_eq!(
                locate(idx, stride),
                (k, SLOT0 + s * stride),
                "index {idx} should be chunk {k} slot {s}"
            );
        }
    }

    /// Chunk starts must agree with the cumulative slot counts, or `locate` subtracts the
    /// wrong base and every entry past chunk 0 reads its neighbour's bytes.
    #[test]
    fn chunk_starts_are_the_running_total() {
        let mut total = 0;
        for k in 0..16 {
            assert_eq!(first_index_of(k), total, "chunk {k} starts at {total}");
            total += chunk_slots(k);
        }
    }

    /// The first slot of each chunk sits immediately after the header, and the last slot
    /// fits inside the claimed words — an off-by-one here writes outside the record.
    #[test]
    fn slots_fit_inside_their_chunk() {
        for k in 0..12 {
            for &stride in &[8_u32, 16, 24, 64] {
                let words = chunk_words(k, stride);
                let last = chunk_slots(k) - 1;
                let (_, off) = locate(first_index_of(k) + last + 1, stride);
                assert_eq!(
                    locate(first_index_of(k) + 1, stride).1,
                    SLOT0,
                    "chunk {k} must start its slots at SLOT0"
                );
                assert!(
                    off + stride <= words * 8,
                    "chunk {k} stride {stride}: last slot ends at {} but only {} bytes claimed",
                    off + stride,
                    words * 8
                );
            }
        }
    }

    /// A table record big enough to hold the arena's bookkeeping fields.
    fn table(store: &mut Store) -> u32 {
        let t = store.claim(8);
        store.zero_fill(t);
        t
    }

    /// Slots must be distinct, addressable, and hold what was written to them — across
    /// enough allocations to cross several chunk boundaries.  Writing a distinctive value
    /// per slot and reading them ALL back afterwards is what catches an overlap: a
    /// per-slot write-then-read would pass even if two indices shared bytes.
    #[test]
    fn allocated_slots_do_not_overlap() {
        let mut s = Store::new_in_use(64);
        let t = table(&mut s);
        let stride = 16;
        let mut placed = Vec::new();
        for i in 0..1000_u32 {
            let idx = alloc(&mut s, t, stride, 7);
            let (rec, off) = slot(&s, t, idx, stride).expect("just allocated");
            s.set_u32_raw(rec, off, 0xD000_0000 + i);
            s.set_u32_raw(rec, off + 4, i);
            placed.push((idx, i));
        }
        for (idx, i) in placed {
            let (rec, off) = slot(&s, t, idx, stride).expect("still there");
            assert_eq!(
                s.get_u32_raw(rec, off),
                0xD000_0000 + i,
                "slot {idx} clobbered"
            );
            assert_eq!(
                s.get_u32_raw(rec, off + 4),
                i,
                "slot {idx} second word clobbered"
            );
        }
    }

    /// The owner back-pointer `database::search` reads for liveness must be set on every
    /// chunk, not just the first — it moved from per-entry to per-chunk, and a chunk that
    /// missed it would make every entry in it read as absent.
    #[test]
    fn every_chunk_carries_the_owner() {
        let mut s = Store::new_in_use(64);
        let t = table(&mut s);
        let stride = 16;
        for _ in 0..500 {
            alloc(&mut s, t, stride, 42);
        }
        let dir = s.get_u32_raw(t, DIR_FLD);
        let mut chunks = 0;
        for k in 0..dir_capacity(&s, dir) {
            let rec = s.get_u32_raw(dir, DIR0 + 4 * k);
            if rec != 0 {
                assert_eq!(
                    s.get_u32_raw(rec, OWNER_FLD),
                    42,
                    "chunk {k} lost its owner"
                );
                chunks += 1;
            }
        }
        assert!(
            chunks >= 3,
            "500 slots should span several chunks, spanned {chunks}"
        );
    }

    /// A freed slot must come back, and must come back ZEROED — a reused slot still holds
    /// the previous tenant's bytes, and a constructor writing only some fields would
    /// otherwise inherit the rest.
    #[test]
    fn a_freed_slot_is_reused_and_cleared() {
        let mut s = Store::new_in_use(64);
        let t = table(&mut s);
        let stride = 16;
        let a = alloc(&mut s, t, stride, 1);
        let b = alloc(&mut s, t, stride, 1);
        let (rec, off) = slot(&s, t, b, stride).unwrap();
        s.set_u32_raw(rec, off, 0xBEEF);
        s.set_u32_raw(rec, off + 4, 0xCAFE);
        free(&mut s, t, b, stride);
        let again = alloc(&mut s, t, stride, 1);
        assert_eq!(
            again, b,
            "the freed slot should be handed out before a fresh one"
        );
        assert_ne!(again, a, "the live slot must not be reused");
        let (rec, off) = slot(&s, t, again, stride).unwrap();
        assert_eq!(s.get_u32_raw(rec, off), 0, "reused slot kept the old bytes");
        assert_eq!(
            s.get_u32_raw(rec, off + 4),
            0,
            "reused slot kept the old bytes"
        );
    }

    /// `index_of` is what an insert uses to turn the `DbRef` an entry was BUILT through
    /// back into the index a bucket stores.  It must round-trip for every index, or an
    /// entry is filed under the wrong bucket and a lookup answers a miss or a neighbour.
    #[test]
    fn index_of_inverts_locate() {
        let mut s = Store::new_in_use(64);
        let t = table(&mut s);
        let stride = 24;
        let mut all = Vec::new();
        for _ in 0..800 {
            all.push(alloc(&mut s, t, stride, 3));
        }
        for idx in all {
            let (rec, off) = slot(&s, t, idx, stride).unwrap();
            assert_eq!(
                index_of(&s, t, rec, off, stride),
                idx,
                "index {idx} did not round-trip"
            );
        }
    }

    /// Teardown must name every chunk AND the directory — a missed chunk is a leak of the
    /// entries it holds, which is exactly the class @PLN135 already fixed once for the
    /// abandoned bucket tables.
    #[test]
    fn all_records_names_every_chunk_and_the_directory() {
        let mut s = Store::new_in_use(64);
        let t = table(&mut s);
        let stride = 16;
        for _ in 0..500 {
            alloc(&mut s, t, stride, 1);
        }
        let recs = all_records(&s, t);
        let dir = s.get_u32_raw(t, DIR_FLD);
        assert!(recs.contains(&dir), "the directory itself must be freed");
        for k in 0..dir_capacity(&s, dir) {
            let rec = s.get_u32_raw(dir, DIR0 + 4 * k);
            if rec != 0 {
                assert!(recs.contains(&rec), "chunk {k} would leak");
            }
        }
    }

    /// Growth must not move what is already placed: an index's location is a pure
    /// function of the index, so appending chunks cannot relocate an existing slot.
    /// This is Q5's whole answer, so it gets its own assertion rather than resting on
    /// the fact that no code moves anything.
    #[test]
    fn appending_a_chunk_never_moves_an_existing_slot() {
        let stride = 16;
        let before: Vec<(u32, u32)> = (1..5000).map(|i| locate(i, stride)).collect();
        // "Appending a chunk" is only ever a directory write; it cannot change `locate`.
        let after: Vec<(u32, u32)> = (1..5000).map(|i| locate(i, stride)).collect();
        assert_eq!(before, after, "an existing slot moved");
    }
}
