// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! @PLN16.J — store change journal (two-artifact binary model).
//!
//! Records the record-level changes a debugger edit makes to the store, so the edit
//! can be **reverted** (undo) and **replayed** (the cross-store transfer that
//! finishes heap live edits).  Design: `doc/claude/plans/16-debugger/STORE_JOURNAL.md`.
//!
//! Two artifacts, split by what each is good at:
//!
//! - **blob** — a plain append-only **file** holding the variable-length payload
//!   (the `before`/`after` bytes).  No allocator: a WAL never frees mid-stream, so a
//!   store's machinery would be overhead, and a file is uniform RAM-or-disk (it is
//!   the VirtFS in the browser).
//! - **index** — a [`Store`](crate::store::Store) holding one growing fixed-stride
//!   array of [`Entry`]s.  The array is the store's only occupant, so it appends
//!   without ever relocating, and the store gives RAM-or-mmap for free (mmap is
//!   optional, never forced).
//!
//! **Commit rule:** the payload is appended to the blob *first*, the index entry
//! *last* — so on recovery the index up to its last complete entry is trusted and a
//! half-written blob tail beyond it is ignored.
//!
//! This slice records in-place field **modifies** — a pure byte restore, correct by
//! construction with no allocator interaction (it never claims, frees, or resizes a
//! *target* record), covering field / element edits.  Whole-value insert / free
//! replay is the next slice (probe #2 — replay-position determinism — is confirmed,
//! so it needs no `DbRef` remap).

use crate::database::Stores;
use crate::store::Store;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

/// `Modify` op tag — an in-place region change (payload: `before` then `after`).
const OP_MODIFY: u8 = 0;
/// `Insert` op tag — a whole record claimed at a recorded position (payload: `after`,
/// the new record's bytes).  apply = `claim_at` + write; revert = `delete`.
const OP_INSERT: u8 = 1;
/// `Free` op tag — a whole record freed (payload: `before`, snapshotted *before* the
/// `delete` repurposes the block — probe 5a).  apply = `delete`; revert = `claim_at` +
/// write.
const OP_FREE: u8 = 2;

/// Bytes per index entry — fixed stride so the array is random-access and the store
/// holds it as one flat vector.
const STRIDE: u32 = 24;

/// Byte offset of the first entry within the index record.  Leaves the 8-byte
/// allocator/header region (`fld 0` = claim size, `fld 4` = the durable entry count)
/// untouched, so the index store is self-describing.
const DATA_OFF: u32 = 8;

/// One recorded change, decoded from its 24-byte little-endian index slot.  For a
/// `Modify`, `before` is the blob bytes at `blob_at` and `after` at `blob_at + len`,
/// each `len` wide; the target region is `(store_nr, rec, off)`.
#[derive(Debug, Clone, Copy)]
struct Entry {
    op: u8,
    store_nr: u16,
    rec: u32,
    off: u32,
    len: u32,
    blob_at: u64,
}

impl Entry {
    /// Pack to the on-disk little-endian slot (portable across runs / machines).
    fn encode(self) -> [u8; STRIDE as usize] {
        let mut b = [0u8; STRIDE as usize];
        b[0] = self.op;
        b[2..4].copy_from_slice(&self.store_nr.to_le_bytes());
        b[4..8].copy_from_slice(&self.rec.to_le_bytes());
        b[8..12].copy_from_slice(&self.off.to_le_bytes());
        b[12..16].copy_from_slice(&self.len.to_le_bytes());
        b[16..24].copy_from_slice(&self.blob_at.to_le_bytes());
        b
    }

    fn decode(b: &[u8]) -> Entry {
        Entry {
            op: b[0],
            store_nr: u16::from_le_bytes([b[2], b[3]]),
            rec: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            off: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            len: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
            blob_at: u64::from_le_bytes([b[16], b[17], b[18], b[19], b[20], b[21], b[22], b[23]]),
        }
    }
}

/// The append-only payload file.  Always a file (never a store), removed on drop for
/// a transient session.
#[derive(Debug)]
struct Blob {
    file: File,
    len: u64,
    path: PathBuf,
    /// Remove the file on drop (a transient debug session); a persisted journal
    /// would keep it.
    transient: bool,
}

impl Blob {
    /// Create a fresh transient blob in the temp dir.
    fn create_temp() -> io::Result<Blob> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("loft_journal_{}_{n}.blob", std::process::id()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        Ok(Blob {
            file,
            len: 0,
            path,
            transient: true,
        })
    }

    /// Append `data`, returning its start offset (the commit-ordering first step).
    fn append(&mut self, data: &[u8]) -> io::Result<u64> {
        let at = self.len;
        self.file.seek(SeekFrom::Start(at))?;
        self.file.write_all(data)?;
        self.len += data.len() as u64;
        Ok(at)
    }

    /// Read `len` bytes at `off`.
    fn read(&mut self, off: u64, len: usize) -> io::Result<Box<[u8]>> {
        let mut buf = vec![0u8; len];
        self.file.seek(SeekFrom::Start(off))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf.into_boxed_slice())
    }
}

impl Drop for Blob {
    fn drop(&mut self) {
        if self.transient {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A store change journal: an index `Store` (the fixed-stride entry array) + a blob
/// file (the payload).  Built during an edit, then kept for undo or discarded.
#[derive(Debug)]
pub struct Journal {
    index: Store,
    /// The single array record within `index`.
    rec: u32,
    /// Number of entries (authoritative; mirrored to the index header for recovery).
    count: u32,
    blob: Blob,
}

impl Journal {
    /// A fresh in-RAM journal with a transient blob file.  The index store stays in
    /// RAM (mmap is optional and not taken here); the blob is removed on drop.
    ///
    /// # Errors
    /// Returns the I/O error if the temp blob file cannot be created.
    pub fn create() -> io::Result<Journal> {
        // `new_in_use`: the journal owns this private store outright — it is
        // never registered into a `Stores` (registration is what normally
        // clears the `free` flag), and the grow path's `validate()` rejects a
        // `free`-flagged store under debug-assertions.
        let mut index = Store::new_in_use(64);
        let rec = index.claim(16); // 128 bytes — room for ~5 entries before the first grow
        index.set_u32_raw(rec, 4, 0); // durable entry count starts at 0
        Ok(Journal {
            index,
            rec,
            count: 0,
            blob: Blob::create_temp()?,
        })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.count as usize
    }

    /// The store regions this journal would write, as `(store_nr, rec, off, len)`.
    ///
    /// @PLN120 F — the debugger's undo history needs to know *where* an edit landed,
    /// not only how to revert it: a region in the stack store is frame storage, whose
    /// validity ends when the frame does or when the slot is re-handed to another
    /// local, while a heap region's address survives both.  Deciding that is what lets
    /// a step keep the entries it cannot invalidate instead of dropping all of them.
    #[must_use]
    pub fn regions(&self) -> Vec<(u16, u32, u32, u32)> {
        (0..self.count)
            .map(|i| {
                let e = self.entry(i);
                (e.store_nr, e.rec, e.off, e.len)
            })
            .collect()
    }

    /// Snapshot a record region — the `before` an edit captures *before* it writes.
    /// Pair with [`record_modify`](Self::record_modify) after the write lands.
    #[must_use]
    pub fn snapshot(stores: &Stores, store_nr: u16, rec: u32, off: u32, len: u32) -> Box<[u8]> {
        stores.allocations[store_nr as usize].read_span(rec, off, len)
    }

    /// Grow the index array record so entry `count` fits, tracking the (possibly
    /// relocated) record handle `resize` returns.
    fn ensure_capacity(&mut self) {
        let needed_words = (DATA_OFF + (self.count + 1) * STRIDE).div_ceil(8);
        let cur_words = (self.index.read::<i32>(self.rec, 0)) as u32;
        if needed_words > cur_words {
            self.rec = self.index.resize(self.rec, needed_words.max(cur_words * 2));
        }
    }

    fn push(&mut self, e: Entry) {
        self.ensure_capacity();
        self.index
            .write_span(self.rec, DATA_OFF + self.count * STRIDE, &e.encode());
        self.count += 1;
        self.index.set_u32_raw(self.rec, 4, self.count); // durable count (recovery)
    }

    fn entry(&self, i: u32) -> Entry {
        Entry::decode(
            &self
                .index
                .read_span(self.rec, DATA_OFF + i * STRIDE, STRIDE),
        )
    }

    /// Record an in-place modify whose write has just happened: `before` is the
    /// region's pre-write bytes (from [`snapshot`](Self::snapshot)); the `after` is
    /// read from the store's *current* state, so the region must still be
    /// `before.len()` bytes wide (a modify never resizes — that invariant is what
    /// keeps this allocator-free).  Commit order: blob payload first, index last.
    ///
    /// # Errors
    /// Returns the I/O error if the blob append fails.
    pub fn record_modify(
        &mut self,
        stores: &Stores,
        store_nr: u16,
        rec: u32,
        off: u32,
        before: &[u8],
    ) -> io::Result<()> {
        let len = before.len() as u32;
        let after = stores.allocations[store_nr as usize].read_span(rec, off, len);
        let mut payload = Vec::with_capacity(2 * len as usize);
        payload.extend_from_slice(before);
        payload.extend_from_slice(&after);
        let blob_at = self.blob.append(&payload)?; // FIRST — payload is durable before the entry
        self.push(Entry {
            op: OP_MODIFY,
            store_nr,
            rec,
            off,
            len,
            blob_at,
        });
        Ok(())
    }

    /// Record a whole-record **insert**: a record just claimed at `rec` (`size_words`
    /// words) and filled.  Its `after` bytes — the new record's final content for
    /// forward materialisation — are read from the store now.  apply re-claims the
    /// exact position via `claim_at` and writes them; revert frees the record.  This is
    /// the cross-store transfer half: replaying an edit's inserts into the live store.
    ///
    /// # Errors
    /// Returns the I/O error if the blob append fails.
    pub fn record_insert(
        &mut self,
        stores: &Stores,
        store_nr: u16,
        rec: u32,
        size_words: u32,
    ) -> io::Result<()> {
        let len = size_words * 8;
        let after = stores.allocations[store_nr as usize].read_span(rec, 0, len);
        let blob_at = self.blob.append(&after)?; // FIRST — payload durable before the entry
        self.push(Entry {
            op: OP_INSERT,
            store_nr,
            rec,
            off: 0,
            len,
            blob_at,
        });
        Ok(())
    }

    /// Record a whole-record **free**: `before` is the record's bytes snapshotted
    /// *before* `delete` runs (a freed block's body is repurposed instantly — probe
    /// 5a, so the snapshot must precede the delete).  apply frees the record; revert
    /// re-claims it at the same position via `claim_at` and restores `before`.
    ///
    /// # Errors
    /// Returns the I/O error if the blob append fails.
    pub fn record_free(&mut self, store_nr: u16, rec: u32, before: &[u8]) -> io::Result<()> {
        let len = before.len() as u32;
        let blob_at = self.blob.append(before)?;
        self.push(Entry {
            op: OP_FREE,
            store_nr,
            rec,
            off: 0,
            len,
            blob_at,
        });
        Ok(())
    }

    /// Replay every change **forward** (redo / cross-store apply), in record order:
    /// `Modify` writes its `after`; `Insert` re-claims the exact position (`claim_at`)
    /// and writes the new bytes; `Free` deletes the record.
    ///
    /// # Errors
    /// Returns the I/O error if a blob read fails.
    ///
    /// # Panics
    /// Panics on an unknown op tag in the index — a corrupt or forward-incompatible
    /// journal (our own writes only ever use `Modify`/`Insert`/`Free`).
    pub fn apply(&mut self, stores: &mut Stores) -> io::Result<()> {
        for i in 0..self.count {
            let e = self.entry(i);
            let store = &mut stores.allocations[e.store_nr as usize];
            match e.op {
                OP_MODIFY => {
                    let after = self
                        .blob
                        .read(e.blob_at + u64::from(e.len), e.len as usize)?;
                    store.write_span(e.rec, e.off, &after);
                }
                OP_INSERT => {
                    let after = self.blob.read(e.blob_at, e.len as usize)?;
                    store.claim_at(e.rec, e.len / 8);
                    store.write_span(e.rec, 0, &after);
                }
                OP_FREE => store.delete(e.rec),
                other => panic!("journal apply: unknown op {other}"),
            }
        }
        Ok(())
    }

    /// Replay every change **backward** (undo), in reverse order — each entry's inverse:
    /// `Modify` writes its `before`; `Insert` deletes the record; `Free` re-claims the
    /// exact position (`claim_at`) and restores the freed bytes.  Reverse order makes
    /// overlapping edits restore to the exact pre-edit bytes.
    ///
    /// # Errors
    /// Returns the I/O error if a blob read fails.
    ///
    /// # Panics
    /// Panics on an unknown op tag in the index — a corrupt or forward-incompatible
    /// journal (our own writes only ever use `Modify`/`Insert`/`Free`).
    pub fn revert(&mut self, stores: &mut Stores) -> io::Result<()> {
        for i in (0..self.count).rev() {
            let e = self.entry(i);
            let store = &mut stores.allocations[e.store_nr as usize];
            match e.op {
                OP_MODIFY => {
                    let before = self.blob.read(e.blob_at, e.len as usize)?;
                    store.write_span(e.rec, e.off, &before);
                }
                OP_INSERT => store.delete(e.rec),
                OP_FREE => {
                    let before = self.blob.read(e.blob_at, e.len as usize)?;
                    store.claim_at(e.rec, e.len / 8);
                    store.write_span(e.rec, 0, &before);
                }
                other => panic!("journal revert: unknown op {other}"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Stores` with one fresh heap store holding one claimed `words`-word record.
    fn store_with_record(words: u32) -> (Stores, u16, u32) {
        let mut stores = Stores::default();
        stores.allocations.push(Store::new_in_use(256));
        let store_nr = (stores.allocations.len() - 1) as u16;
        let rec = stores.allocations[store_nr as usize].claim(words);
        (stores, store_nr, rec)
    }

    /// @PLN16.J probe #2 — **replay-position determinism**, the invariant the
    /// whole-value insert/free replay rests on.  Replaying a constructor's inserts
    /// into the live store with **no `DbRef` remap** is sound *iff* `claim` is a pure
    /// function of allocator state: a store cloned from the live store, run through
    /// the same claims, must hand out the same positions, so the recorded inserts
    /// land where their internal `DbRef`s already point.  This drives the same
    /// claim/free/coalesce/grow sequence on two independent stores and asserts the
    /// positions match at every step.  Rust's `HashSet` is randomly seeded per
    /// construction, so if position selection ever leaked the `claims` set's
    /// iteration order, the two runs would diverge here — making this a live guard,
    /// not a tautology.  If a future allocator change breaks determinism, replay
    /// silently breaks and this test is what catches it.
    #[test]
    fn claim_is_deterministic_from_history() {
        fn claim_sequence() -> Vec<u32> {
            let mut s = Store::new_in_use(64);
            let mut pos = Vec::new();
            let a = s.claim(4);
            pos.push(a);
            let b = s.claim(2);
            pos.push(b);
            let c = s.claim(8);
            pos.push(c);
            s.delete(b); // hole in the middle
            pos.push(s.claim(2)); // reuse the hole
            s.delete(a);
            s.delete(c); // adjacent frees → coalesce
            pos.push(s.claim(6)); // reuse the coalesced region
            pos.push(s.claim(40)); // force growth past the initial 64 words
            pos.push(s.claim(1));
            pos.push(s.claim(3));
            pos
        }
        assert_eq!(
            claim_sequence(),
            claim_sequence(),
            "claim positions must be a deterministic function of allocator history",
        );
    }

    fn read_i64(stores: &Stores, sn: u16, rec: u32, off: u32) -> i64 {
        stores.allocations[sn as usize].read::<i64>(rec, off)
    }
    fn write_i64(stores: &mut Stores, sn: u16, rec: u32, off: u32, v: i64) {
        stores.allocations[sn as usize].write::<i64>(rec, off, v);
    }

    /// Undo then redo a single scalar-field edit, exact byte fidelity both ways.
    #[test]
    fn modify_reverts_then_replays() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 42);

        let before = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 99);
        let mut j = Journal::create().unwrap();
        j.record_modify(&stores, sn, rec, 8, &before).unwrap();

        assert_eq!(read_i64(&stores, sn, rec, 8), 99, "edit took");
        j.revert(&mut stores).unwrap();
        assert_eq!(
            read_i64(&stores, sn, rec, 8),
            42,
            "revert restores pre-edit"
        );
        j.apply(&mut stores).unwrap();
        assert_eq!(read_i64(&stores, sn, rec, 8), 99, "apply redoes the edit");
    }

    /// Two edits to the *same* region revert in reverse order back to the original.
    #[test]
    fn overlapping_edits_revert_in_reverse() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 1);

        let mut j = Journal::create().unwrap();
        let b1 = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 2);
        j.record_modify(&stores, sn, rec, 8, &b1).unwrap();
        let b2 = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 3);
        j.record_modify(&stores, sn, rec, 8, &b2).unwrap();

        assert_eq!(read_i64(&stores, sn, rec, 8), 3);
        j.revert(&mut stores).unwrap();
        assert_eq!(
            read_i64(&stores, sn, rec, 8),
            1,
            "reverse order restores original"
        );
    }

    /// Two fields of one record edited together revert together.
    #[test]
    fn multi_field_edit_round_trips() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 10);
        write_i64(&mut stores, sn, rec, 16, 20);

        let mut j = Journal::create().unwrap();
        let bx = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 99);
        j.record_modify(&stores, sn, rec, 8, &bx).unwrap();
        let by = Journal::snapshot(&stores, sn, rec, 16, 8);
        write_i64(&mut stores, sn, rec, 16, 88);
        j.record_modify(&stores, sn, rec, 16, &by).unwrap();

        j.revert(&mut stores).unwrap();
        assert_eq!(read_i64(&stores, sn, rec, 8), 10);
        assert_eq!(read_i64(&stores, sn, rec, 16), 20);
    }

    /// A whole-record snapshot/restore (the record-level granularity).
    #[test]
    fn whole_record_snapshot_restores() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 7);
        write_i64(&mut stores, sn, rec, 16, 8);

        let before = Journal::snapshot(&stores, sn, rec, 8, 24);
        write_i64(&mut stores, sn, rec, 8, -1);
        write_i64(&mut stores, sn, rec, 16, -2);
        let mut j = Journal::create().unwrap();
        j.record_modify(&stores, sn, rec, 8, &before).unwrap();

        j.revert(&mut stores).unwrap();
        assert_eq!(read_i64(&stores, sn, rec, 8), 7);
        assert_eq!(read_i64(&stores, sn, rec, 16), 8);
    }

    /// Changes spanning two stores apply/revert independently.
    #[test]
    fn cross_store_changes() {
        let (mut stores, sn_a, rec_a) = store_with_record(4);
        stores.allocations.push(Store::new_in_use(256));
        let sn_b = (stores.allocations.len() - 1) as u16;
        let rec_b = stores.allocations[sn_b as usize].claim(4);

        write_i64(&mut stores, sn_a, rec_a, 8, 100);
        write_i64(&mut stores, sn_b, rec_b, 8, 200);

        let mut j = Journal::create().unwrap();
        let ba = Journal::snapshot(&stores, sn_a, rec_a, 8, 8);
        write_i64(&mut stores, sn_a, rec_a, 8, 1);
        j.record_modify(&stores, sn_a, rec_a, 8, &ba).unwrap();
        let bb = Journal::snapshot(&stores, sn_b, rec_b, 8, 8);
        write_i64(&mut stores, sn_b, rec_b, 8, 2);
        j.record_modify(&stores, sn_b, rec_b, 8, &bb).unwrap();

        j.revert(&mut stores).unwrap();
        assert_eq!(read_i64(&stores, sn_a, rec_a, 8), 100);
        assert_eq!(read_i64(&stores, sn_b, rec_b, 8), 200);
    }

    /// Enough entries to force the index array to grow (and possibly relocate) past
    /// its initial capacity — every recorded change still reverts exactly.
    #[test]
    fn index_grows_past_initial_capacity() {
        let (mut stores, sn, rec) = store_with_record(34); // 33 data words: 32 i64 fields
        let n: u32 = 32;
        for i in 0..n {
            write_i64(&mut stores, sn, rec, 8 + i * 8, i64::from(i));
        }
        let mut j = Journal::create().unwrap();
        for i in 0..n {
            let off = 8 + i * 8;
            let before = Journal::snapshot(&stores, sn, rec, off, 8);
            write_i64(&mut stores, sn, rec, off, -1);
            j.record_modify(&stores, sn, rec, off, &before).unwrap();
        }
        assert_eq!(j.len(), 32, "32 entries forced a grow");
        j.revert(&mut stores).unwrap();
        for i in 0..n {
            assert_eq!(
                read_i64(&stores, sn, rec, 8 + i * 8),
                i64::from(i),
                "field {i} restored"
            );
        }
    }

    /// An empty journal is a no-op both ways.
    #[test]
    fn empty_journal_is_noop() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 5);
        let mut j = Journal::create().unwrap();
        assert!(j.is_empty());
        j.apply(&mut stores).unwrap();
        j.revert(&mut stores).unwrap();
        assert_eq!(read_i64(&stores, sn, rec, 8), 5);
    }

    /// Read a u32 region via `read_span` (works on freed records too — `read_span`
    /// bounds-checks against the whole store, not the record header).
    fn rd_u32(s: &Store, rec: u32, off: u32) -> u32 {
        let b = s.read_span(rec, off, 4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    /// @PLN16.J probe #5a — **a freed block's body is immediately repurposed.**  The
    /// allocator stores its red-black free-tree node links *inside* free blocks
    /// (`FL_LEFT` at byte offset 4, `FL_RIGHT` at offset 8 — exactly where a vector
    /// keeps its length and first element).  So `delete(rec)` overwrites the record's
    /// data the instant it runs.  This is the load-bearing reason the structural
    /// half's deferred free must mean **stay-claimed**, not "free but remember the
    /// bytes": there is no window in which a freed record's old contents are readable.
    /// (This falsified the first draft of the design, which assumed a freed record's
    /// bytes survived for undo.)
    #[test]
    fn freelist_repurposes_a_freed_blocks_body() {
        let mut s = Store::new_in_use(64);
        let rec = s.claim(4);
        s.set_u32_raw(rec, 4, 0xDEAD_BEEF); // where a vector's length lives
        s.set_u32_raw(rec, 8, 0x0BAD_F00D); // where element 0 lives
        s.delete(rec);
        assert_ne!(
            rd_u32(&s, rec, 4),
            0xDEAD_BEEF,
            "offset 4 overwritten by the free-tree node link (FL_LEFT)",
        );
        assert_ne!(
            rd_u32(&s, rec, 8),
            0x0BAD_F00D,
            "offset 8 overwritten by the free-tree node link (FL_RIGHT)",
        );
    }

    /// @PLN16.J probe #5b — **a relocating grow flips the owning u32 pointer cell.**
    /// When `resize` cannot extend in place it relocates (`claim`+`copy`+`delete`) and
    /// `vector.rs` writes the new record number into the owning cell.  That pointer
    /// write goes through `set_u32_raw`/`addr_mut` (the unhooked hot path), so the
    /// journal's structural half must capture it as an explicit **Modify** of the
    /// owning cell alongside the **Insert** of the new record — the structural ops
    /// alone do not see it.  This boxes a vector in to force a real relocation and
    /// confirms the cell flips and the new record carries the grown vector.
    #[test]
    fn relocation_flips_the_owning_pointer() {
        use crate::keys::DbRef;
        use crate::vector::{vector_append, vector_finish};

        let size = 4u32;
        let mut stores = vec![Store::new_in_use(64)];
        let container = stores[0].claim(2);
        stores[0].set_u32_raw(container, 8, 0); // empty-vector pointer cell
        let db = DbRef {
            store_nr: 0,
            rec: container,
            pos: 8,
        };

        // First append allocates the backing record; an adjacent claim pins it so the
        // next grow cannot extend in place and is forced to relocate.
        let slot = vector_append(&db, size, &mut stores);
        stores[0].set_u32_raw(slot.rec, slot.pos, 7);
        vector_finish(&db, &mut stores);
        let _blocker = stores[0].claim(8);

        let mut old_vec = stores[0].get_u32_raw(db.rec, db.pos);
        let mut hit = None;
        for i in 1u32..256 {
            let slot = vector_append(&db, size, &mut stores);
            stores[0].set_u32_raw(slot.rec, slot.pos, i * 1000 + 7);
            vector_finish(&db, &mut stores);
            let cur = stores[0].get_u32_raw(db.rec, db.pos);
            if cur != old_vec {
                hit = Some((old_vec, cur, i));
                break;
            }
            old_vec = cur;
        }
        let (old, new, n) = hit.expect("a boxed-in grow must relocate");
        assert_ne!(old, new, "the record relocated");
        assert_eq!(
            stores[0].get_u32_raw(db.rec, db.pos),
            new,
            "the owning cell flipped to the new record",
        );
        assert_eq!(
            stores[0].get_u32_raw(new, 4),
            n + 1,
            "new length = elements 0..=n"
        );
        assert_eq!(
            rd_u32(&stores[0], new, 8 + n * size),
            n * 1000 + 7,
            "last element in new"
        );
    }

    /// @PLN16.J probe #5c — **once both records exist, the owning cell alone selects
    /// which the value sees → the pointer-flip *is* the relocation switch.**  When the
    /// pre-grow and grown records both exist, flipping the owning u32 cell is the whole
    /// of redo (→ grown) and undo (→ pre-grow).  In the single model (STORE_JOURNAL.md
    /// § The model) the freed `old` is snapshotted to the blob and re-materialised on
    /// undo via `claim_at`; this probe isolates the *switch* itself — two records side
    /// by side, flip and flip back — and confirms both directions read exactly.
    #[test]
    fn deferred_free_pointer_flip_round_trips() {
        let size = 4u32;
        let mut s = Store::new_in_use(64);
        let container = s.claim(2);

        let old = s.claim(4); // pre-grow vector: len 2, elements [11, 22]
        s.set_u32_raw(old, 4, 2);
        s.set_u32_raw(old, 8, 11);
        s.set_u32_raw(old, 8 + size, 22);
        s.set_u32_raw(container, 8, old); // owning cell -> old

        let new = s.claim(8); // grown vector: len 3, elements [11, 22, 33]
        s.copy_block(old, 8, new, 8, (2 * size) as isize); // conserve the two existing elements
        s.set_u32_raw(new, 4, 3);
        s.set_u32_raw(new, 8 + 2 * size, 33);

        // Forward (redo / cross-store apply): flip the owning cell to new; old stays claimed.
        s.set_u32_raw(container, 8, new);
        let f = s.get_u32_raw(container, 8);
        assert_eq!(s.get_u32_raw(f, 4), 3, "grown length");
        assert_eq!(s.get_u32_raw(f, 8), 11, "conserved element 0");
        assert_eq!(s.get_u32_raw(f, 8 + 2 * size), 33, "appended element");

        // Backward (undo): flip the cell back to old — no snapshot, old is intact.
        s.set_u32_raw(container, 8, old);
        let b = s.get_u32_raw(container, 8);
        assert_eq!(
            s.get_u32_raw(b, 4),
            2,
            "pre-grow length restored by pointer-flip alone"
        );
        assert_eq!(s.get_u32_raw(b, 8), 11, "pre-grow element 0 intact");
        assert_eq!(s.get_u32_raw(b, 8 + size), 22, "pre-grow element 1 intact");
    }

    /// @PLN16.J probe #6a — **`claim_at` carves a record out of the MIDDLE of a free
    /// block** (the predicted falsification corner).  A slot becomes mid-block after its
    /// neighbours are freed and forward-coalesced; `Free`-revert must re-claim it at its
    /// exact recorded position regardless.  Build that exact state — free C, then B
    /// (absorbs C), then A (absorbs B+C), so `[a, d)` is one free block with `b` strictly
    /// mid-block — then re-claim `b`.  `claim_at` must split the covering block three
    /// ways (prefix free | claimed | suffix free) and leave a store that still tiles.
    #[test]
    #[allow(clippy::many_single_char_names)] // a/b/c/d are the four geometric records
    fn claim_at_carves_a_mid_block_region() {
        let mut s = Store::new_in_use(64);
        let a = s.claim(3);
        let b = s.claim(4);
        let c = s.claim(5);
        let d = s.claim(2);
        assert_eq!(
            (a + 3, b + 4, c + 5),
            (b, c, d),
            "A,B,C,D laid out adjacent"
        );

        let hdr = |s: &Store, p: u32| s.read::<i32>(p, 0);

        // Free C, then B (absorbs C forward), then A (absorbs B+C) -> one free block
        // [a, d) with b strictly in the middle.
        s.delete(c);
        s.delete(b);
        s.delete(a);
        assert_eq!(
            hdr(&s, a),
            -((d - a) as i32),
            "a..d coalesced into one free block"
        );

        // Re-claim b's exact slot. Full 3-way split: [a,b) free | [b,b+4) | [b+4,d) free.
        let got = s.claim_at(b, 4);
        assert_eq!(got, b, "claim_at returns the requested position");
        assert_eq!(hdr(&s, a), -((b - a) as i32), "prefix [a,b) left free");
        assert_eq!(hdr(&s, b), 4, "record claimed at the exact recorded size");
        assert_eq!(
            hdr(&s, b + 4),
            -((d - (b + 4)) as i32),
            "suffix [b+4,d) left free"
        );

        // The reclaimed slot is usable, and the chain still tiles [PRIMARY, size).
        s.set_u32_raw(b, 8, 0xB0B0_00FF);
        assert_eq!(
            s.get_u32_raw(b, 8),
            0xB0B0_00FF,
            "reclaimed slot is writable"
        );
        s.validate(0);
    }

    /// @PLN16.J probe #6b — **bidirectional position-exact replay** (THE keystone).
    /// The whole single model rests on this: a recorded op-log, replayed by recorded
    /// position via `claim_at`, reconstructs state exactly in *both* directions.  Run a
    /// coalescing-heavy edit (free adjacent records → they coalesce → new claims reuse
    /// the merged space, some landing exactly on freed slots), logging each structural
    /// op as an `Insert`/`Free` with its position + bytes.  Then **revert** (each entry's
    /// inverse, reverse order: `Insert`→`free`, `Free`→`claim_at`+restore) and assert the
    /// baseline returns byte-for-byte; then **re-apply forward** via `claim_at` and assert
    /// the edited records return.  The reverse pass is where coalescing could make a
    /// freed record's exact slot unrecoverable — the predicted falsification.
    #[test]
    fn bidirectional_position_exact_replay() {
        enum Op {
            Insert {
                pos: u32,
                size: u32,
                after: Box<[u8]>,
            },
            Free {
                pos: u32,
                size: u32,
                before: Box<[u8]>,
            },
        }
        let bytes = |w: u32| w * 8; // words -> bytes
        // Actual claimed size from the header (claim may hand out more than requested).
        let real_size = |s: &Store, p: u32| (s.read::<i32>(p, 0)) as u32;
        let fill = |s: &mut Store, p: u32, sz: u32, tag: u32| {
            for w in 1..sz {
                s.set_u32_raw(p, w * 8, tag | w);
                s.set_u32_raw(p, w * 8 + 4, tag | 0x8000 | w);
            }
        };

        let mut s = Store::new_in_use(64);

        // ---- baseline: five records with distinctive bytes ----
        let mut base: Vec<(u32, u32)> = Vec::new(); // (pos, actual size)
        for (i, &sz) in [2u32, 3, 2, 4, 2].iter().enumerate() {
            let p = s.claim(sz);
            let rsz = real_size(&s, p);
            fill(&mut s, p, rsz, 0xBA50_0000 + ((i as u32) << 8));
            base.push((p, rsz));
        }
        let baseline: Vec<Box<[u8]>> = base
            .iter()
            .map(|&(p, sz)| s.read_span(p, 0, bytes(sz)))
            .collect();

        // ---- the edit: free middles (coalesce), reuse the space with new claims ----
        let mut log: Vec<Op> = Vec::new();
        let free_rec = |s: &mut Store, log: &mut Vec<Op>, p: u32| {
            let sz = real_size(s, p);
            let before = s.read_span(p, 0, bytes(sz)); // BEFORE delete (probe 5a)
            s.delete(p);
            log.push(Op::Free {
                pos: p,
                size: sz,
                before,
            });
        };
        let claim_rec = |s: &mut Store, log: &mut Vec<Op>, req: u32, tag: u32| {
            let p = s.claim(req);
            let sz = real_size(s, p);
            fill(s, p, sz, tag);
            let after = s.read_span(p, 0, bytes(sz));
            log.push(Op::Insert {
                pos: p,
                size: sz,
                after,
            });
        };

        free_rec(&mut s, &mut log, base[1].0); // free R1
        free_rec(&mut s, &mut log, base[2].0); // free R2 (now two adjacent frees)
        claim_rec(&mut s, &mut log, 4, 0x4E00_0000); // N0 reuses the coalesced R1+R2 space
        free_rec(&mut s, &mut log, base[3].0); // free R3
        claim_rec(&mut s, &mut log, 1, 0x4E10_0000); // N1
        claim_rec(&mut s, &mut log, 3, 0x4E20_0000); // N2

        // ---- revert: inverse of each entry, reverse order ----
        for op in log.iter().rev() {
            match op {
                Op::Insert { pos, .. } => s.delete(*pos),
                Op::Free { pos, size, before } => {
                    s.claim_at(*pos, *size);
                    s.write_span(*pos, 0, before);
                }
            }
        }
        for (&(p, sz), want) in base.iter().zip(&baseline) {
            assert_eq!(
                s.read_span(p, 0, bytes(sz)),
                *want,
                "baseline record at {p} restored byte-for-byte after revert"
            );
        }
        s.validate(0);

        // ---- re-apply forward via claim_at, then assert the edited records return ----
        for op in &log {
            match op {
                Op::Insert { pos, size, after } => {
                    s.claim_at(*pos, *size);
                    s.write_span(*pos, 0, after);
                }
                Op::Free { pos, .. } => s.delete(*pos),
            }
        }
        for op in &log {
            if let Op::Insert { pos, size, after } = op {
                assert_eq!(
                    s.read_span(*pos, 0, bytes(*size)),
                    *after,
                    "inserted record at {pos} returns byte-for-byte after re-apply"
                );
            }
        }
        s.validate(0);
    }

    /// @PLN16.J probe #6c — **fuzzed bidirectional replay** (hardening for #6b).  A
    /// deterministic PRNG drives long, varied claim/free sequences (with heavy
    /// position-reuse) over a baseline, across several seeds; each session logs
    /// `Insert`/`Free` entries, then **revert** must restore the baseline byte-for-byte
    /// and **re-apply** must restore the edit.  This widens what 6a/6b hand-fed —
    /// hundreds of positions, sizes, and coalescing patterns — so `claim_at` meets
    /// mid-block carves and freed-slot reuse it was never explicitly given.  Fixed seeds
    /// keep any failure reproducible.
    #[test]
    #[allow(clippy::many_single_char_names, clippy::cast_possible_truncation)]
    fn bidirectional_replay_fuzz() {
        // xorshift64* — tiny, deterministic, no dependency.
        struct Rng(u64);
        impl Rng {
            fn step(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                self.0 = x;
                x.wrapping_mul(0x2545_F491_4F6C_DD1D)
            }
            fn below(&mut self, n: u32) -> u32 {
                (self.step() % u64::from(n)) as u32
            }
        }
        enum Op {
            Insert {
                pos: u32,
                size: u32,
                after: Box<[u8]>,
            },
            Free {
                pos: u32,
                size: u32,
                before: Box<[u8]>,
            },
        }

        let bytes = |w: u32| w * 8;
        let real = |s: &Store, p: u32| (s.read::<i32>(p, 0)) as u32;
        let fill = |s: &mut Store, p: u32, sz: u32, tag: &mut u32| {
            *tag = tag.wrapping_add(1);
            for w in 1..sz {
                s.set_u32_raw(p, w * 8, (*tag << 12) ^ w);
                s.set_u32_raw(p, w * 8 + 4, (*tag << 12) ^ 0x800 ^ w);
            }
        };

        for &seed in &[
            0x1234_5678_9ABC_DEF0u64,
            0xDEAD_BEEF_CAFE_0001,
            0x0F0F_0F0F_1234_5678,
            0xA5A5_5A5A_0000_0001,
            0x0000_0000_0000_0007,
            0xFFFF_FFFF_FFFF_FFFE,
            0x8000_0000_0000_0001,
            0x0123_4567_89AB_CDEF,
            0xCAFE_F00D_DEAD_BEEF,
            0x5555_AAAA_3333_CCCC,
            0x2718_2818_2845_9045,
            0x3141_5926_5358_9793,
        ] {
            let mut rng = Rng(seed);
            let mut s = Store::new_in_use(48);
            let mut tag = 0u32;

            // baseline (not logged)
            let mut base: Vec<(u32, u32)> = Vec::new();
            for _ in 0..6 {
                let p = s.claim(1 + rng.below(5));
                let sz = real(&s, p);
                fill(&mut s, p, sz, &mut tag);
                base.push((p, sz));
            }
            let baseline: Vec<Box<[u8]>> = base
                .iter()
                .map(|&(p, sz)| s.read_span(p, 0, bytes(sz)))
                .collect();

            // random edit: claims (Insert) and frees (Free), biased to keep growing.
            let mut log: Vec<Op> = Vec::new();
            let mut live: Vec<u32> = base.iter().map(|&(p, _)| p).collect();
            for _ in 0..600 {
                if live.len() < 3 || rng.below(100) < 55 {
                    let p = s.claim(1 + rng.below(6));
                    let sz = real(&s, p);
                    fill(&mut s, p, sz, &mut tag);
                    let after = s.read_span(p, 0, bytes(sz));
                    log.push(Op::Insert {
                        pos: p,
                        size: sz,
                        after,
                    });
                    live.push(p);
                } else {
                    let p = live.swap_remove(rng.below(live.len() as u32) as usize);
                    let sz = real(&s, p);
                    let before = s.read_span(p, 0, bytes(sz));
                    s.delete(p);
                    log.push(Op::Free {
                        pos: p,
                        size: sz,
                        before,
                    });
                }
            }
            s.validate(0);

            // Capture the records LIVE at end-of-edit (a position may have been
            // claimed, freed, and re-claimed at a different size — only the final
            // occupant is the re-apply target; a dead insert's bytes are gone).
            let edited: Vec<(u32, u32, Box<[u8]>)> = live
                .iter()
                .map(|&p| {
                    let sz = real(&s, p);
                    (p, sz, s.read_span(p, 0, bytes(sz)))
                })
                .collect();

            // revert -> baseline, byte-for-byte
            for op in log.iter().rev() {
                match op {
                    Op::Insert { pos, .. } => s.delete(*pos),
                    Op::Free { pos, size, before } => {
                        s.claim_at(*pos, *size);
                        s.write_span(*pos, 0, before);
                    }
                }
            }
            for (&(p, sz), want) in base.iter().zip(&baseline) {
                assert_eq!(
                    s.read_span(p, 0, bytes(sz)),
                    *want,
                    "seed {seed:#x}: baseline record at {p} restored after {} ops",
                    log.len()
                );
            }
            s.validate(0);

            // re-apply -> edited state
            for op in &log {
                match op {
                    Op::Insert { pos, size, after } => {
                        s.claim_at(*pos, *size);
                        s.write_span(*pos, 0, after);
                    }
                    Op::Free { pos, .. } => s.delete(*pos),
                }
            }
            for (p, sz, want) in &edited {
                assert_eq!(
                    s.read_span(*p, 0, bytes(*sz)),
                    *want,
                    "seed {seed:#x}: live record at {p} restored on re-apply"
                );
            }
            s.validate(0);
        }
    }

    fn rd(stores: &Stores, sn: u16, rec: u32, off: u32) -> u32 {
        stores.allocations[sn as usize].get_u32_raw(rec, off)
    }
    fn hdr(stores: &Stores, sn: u16, rec: u32) -> i32 {
        stores.allocations[sn as usize].read::<i32>(rec, 0)
    }

    /// @PLN16.J — `record_insert` / `record_free` round-trip on the `Journal` itself.
    /// `Insert`: apply re-claims the exact position via `claim_at` + restores bytes,
    /// revert frees it.  `Free`: apply frees, revert re-claims + restores the pre-delete
    /// bytes.  A neighbouring "keeper" record must stay untouched throughout.
    #[test]
    fn insert_and_free_round_trip() {
        let (mut stores, sn, keeper) = store_with_record(2);
        stores.allocations[sn as usize].set_u32_raw(keeper, 8, 0xC0FF_EE01);
        let n = stores.allocations[sn as usize].claim(3);
        stores.allocations[sn as usize].set_u32_raw(n, 8, 0x1111_2222);
        stores.allocations[sn as usize].set_u32_raw(n, 16, 0x3333_4444);

        // --- Insert(n) ---
        let mut j = Journal::create().unwrap();
        j.record_insert(&stores, sn, n, 3).unwrap();
        j.revert(&mut stores).unwrap();
        assert!(hdr(&stores, sn, n) < 0, "Insert reverted -> record freed");
        assert_eq!(rd(&stores, sn, keeper, 8), 0xC0FF_EE01, "keeper untouched");
        j.apply(&mut stores).unwrap();
        assert_eq!(hdr(&stores, sn, n), 3, "re-claimed at the exact size");
        assert_eq!(rd(&stores, sn, n, 8), 0x1111_2222, "byte 8 restored");
        assert_eq!(rd(&stores, sn, n, 16), 0x3333_4444, "byte 16 restored");

        // --- Free(n) ---
        let before = stores.allocations[sn as usize].read_span(n, 0, 24);
        let mut jf = Journal::create().unwrap();
        jf.record_free(sn, n, &before).unwrap();
        stores.allocations[sn as usize].delete(n);
        assert!(hdr(&stores, sn, n) < 0, "Free -> record freed");
        jf.revert(&mut stores).unwrap();
        assert_eq!(
            hdr(&stores, sn, n),
            3,
            "Free reverted -> re-claimed at size"
        );
        assert_eq!(
            rd(&stores, sn, n, 8),
            0x1111_2222,
            "Free revert restores bytes"
        );
        assert_eq!(
            rd(&stores, sn, keeper, 8),
            0xC0FF_EE01,
            "keeper still untouched"
        );
    }

    /// @PLN16.J — the **relocating-grow journal**: `{ Insert(new) + Modify(owning cell:
    /// old->new) + Free(old) }`.  This is the cross-store / heap-edit shape end to end.
    /// Build a vector handle (owning cell -> old), grow it (claim new, flip the cell,
    /// free old), journaling each op, then assert revert returns the pre-grow state and
    /// apply returns the grown state — owning cell + record bytes both exact.
    #[test]
    fn relocation_insert_modify_free_round_trips() {
        let (mut stores, sn, container) = store_with_record(2);

        // pre-grow: owning cell (container, 8) -> old; old is a len-2 "vector".
        let old = stores.allocations[sn as usize].claim(3);
        stores.allocations[sn as usize].set_u32_raw(old, 4, 2); // length
        stores.allocations[sn as usize].set_u32_raw(old, 8, 11);
        stores.allocations[sn as usize].set_u32_raw(old, 12, 22);
        stores.allocations[sn as usize].set_u32_raw(container, 8, old);

        // grow: claim new (len 3), conserve + append, flip the cell, free old —
        // journaling Insert(new) + Modify(cell) + Free(old) in that order.
        let mut j = Journal::create().unwrap();
        let new = stores.allocations[sn as usize].claim(5);
        stores.allocations[sn as usize].set_u32_raw(new, 4, 3);
        stores.allocations[sn as usize].set_u32_raw(new, 8, 11);
        stores.allocations[sn as usize].set_u32_raw(new, 12, 22);
        stores.allocations[sn as usize].set_u32_raw(new, 16, 33);
        j.record_insert(&stores, sn, new, 5).unwrap();

        let cell_before = stores.allocations[sn as usize].read_span(container, 8, 4);
        stores.allocations[sn as usize].set_u32_raw(container, 8, new); // the pointer-flip
        j.record_modify(&stores, sn, container, 8, &cell_before)
            .unwrap();

        let old_before = stores.allocations[sn as usize].read_span(old, 0, 24);
        j.record_free(sn, old, &old_before).unwrap();
        stores.allocations[sn as usize].delete(old);

        // revert -> pre-grow: cell -> old, old restored, new freed.
        j.revert(&mut stores).unwrap();
        assert_eq!(
            rd(&stores, sn, container, 8),
            old,
            "cell flipped back to old"
        );
        assert_eq!(rd(&stores, sn, old, 4), 2, "old length restored");
        assert_eq!(rd(&stores, sn, old, 12), 22, "old element restored");
        assert!(hdr(&stores, sn, new) < 0, "new record freed on revert");

        // apply -> grown: cell -> new, new present, old freed.
        j.apply(&mut stores).unwrap();
        assert_eq!(rd(&stores, sn, container, 8), new, "cell flipped to new");
        assert_eq!(rd(&stores, sn, new, 4), 3, "new length");
        assert_eq!(rd(&stores, sn, new, 16), 33, "appended element");
        assert!(hdr(&stores, sn, old) < 0, "old record freed on apply");
    }

    /// @PLN16.J phase 3 — the `Stores` recording drain.  Structural ops buffer per-store
    /// while recording is on (`claim` -> `Insert`, `delete` -> `Free`); `take_journal`
    /// drains them into one `Journal`, tagging each with its `store_nr`.  Records claimed
    /// *before* recording started are not journaled.  Round-trips an edit (claim `n`,
    /// free `m`): revert restores the pre-edit store, apply redoes the edit.
    #[test]
    fn stores_recording_drains_to_journal() {
        let mut stores = Stores::default();
        stores.allocations.push(Store::new_in_use(64));
        let sn = (stores.allocations.len() - 1) as u16;
        let si = sn as usize;

        // pre-existing records (claimed before recording -> not journaled).
        let keeper = stores.allocations[si].claim(2);
        stores.allocations[si].set_u32_raw(keeper, 8, 0xC0FF_EE01);
        let m = stores.allocations[si].claim(3);
        stores.allocations[si].set_u32_raw(m, 8, 0x1234_5678);
        stores.allocations[si].set_u32_raw(m, 16, 0x9ABC_DEF0);

        // record an edit: claim n (Insert), free m (Free).
        stores.start_recording();
        let n = stores.allocations[si].claim(4);
        stores.allocations[si].set_u32_raw(n, 8, 0xAAAA_0001);
        stores.allocations[si].set_u32_raw(n, 24, 0xAAAA_0002);
        stores.allocations[si].delete(m);
        let mut j = stores.take_journal().unwrap();

        // revert -> edit undone: m re-claimed + restored, n freed, keeper intact.
        j.revert(&mut stores).unwrap();
        assert!(hdr(&stores, sn, n) < 0, "n freed by revert");
        assert_eq!(rd(&stores, sn, m, 8), 0x1234_5678, "m restored");
        assert_eq!(rd(&stores, sn, m, 16), 0x9ABC_DEF0, "m restored");
        assert_eq!(rd(&stores, sn, keeper, 8), 0xC0FF_EE01, "keeper intact");

        // apply -> redo: n re-materialised with its bytes, m freed.
        j.apply(&mut stores).unwrap();
        assert_eq!(rd(&stores, sn, n, 8), 0xAAAA_0001, "n bytes back");
        assert_eq!(rd(&stores, sn, n, 24), 0xAAAA_0002, "n bytes back");
        assert!(hdr(&stores, sn, m) < 0, "m freed by apply");
        assert_eq!(
            rd(&stores, sn, keeper, 8),
            0xC0FF_EE01,
            "keeper still intact"
        );
    }
}
