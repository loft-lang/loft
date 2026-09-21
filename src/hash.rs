// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

use crate::arena;
use crate::keys;
use crate::keys::{Content, DbRef, Key};
use crate::store::Store;
use std::cmp::Ordering;

// Bucket-record layout (word-addressed; each field is a byte offset within
// the record `claim`).  Word 0 is the size/length header, word 1 holds the
// per-hash seed, words 2–3 the entry arena's bookkeeping, and the bucket
// array starts at word 4:
//
//   fld 0  : size header (word count = `room`; doubles as data, see
//            `Store::record_words`)
//   fld 4  : `LEN_FLD`    — live-entry count (u32)
//   fld 8  : `SEED_FLD`   — per-hash hash seed (u64, low half at 8, high at 12)
//   fld 16 : `DIR_FLD`    — the arena's chunk-directory record (`crate::arena`)
//   fld 20 : `NEXT_FLD`   — the arena's append cursor
//   fld 24 : `FREE_FLD`   — head of the arena's free list
//   fld 28 : `STRIDE_FLD` — bytes per entry slot
//   fld 32 : `BUCKET0`    — first bucket slot (u32 entry INDICES, 2 per word)
//
// The seed makes a persisted hash portable: it is stored WITH the buckets,
// so a reader re-derives the same bucket for every key (see
// `keys::seeded_hasher`).  `elms = (room - RESERVED_WORDS) * 2`.
//
// A bucket slot holds a 1-based ARENA INDEX, not a record number (@PLN135 arc H).
// Entries live packed at a fixed stride inside the arena's chunks rather than one
// store record each: a record costs a header word plus `Store::claim`'s rounding —
// 27.67 B measured for a 16 B entry — and the cost that matters is not the bytes but
// the working set they spread a random lookup across (234 ns against 80 for the same
// payload read out of one dense array, measured on this tree).
//
// These ARE the on-disk bucket contract: a reader that computes any of them
// differently looks in the wrong place for an entry a writer put somewhere. They are
// public so `tests/layout_golden.rs::placement_contract_is_pinned` can pin them —
// changing one without bumping `crate::placement::HASH` would let an older store be
// misread instead of refused. See [`crate::placement`].
pub const LEN_FLD: u32 = 4;
pub const SEED_FLD: u32 = 8;
pub const STRIDE_FLD: u32 = 28;
pub const BUCKET0: u32 = 32;
/// Words reserved before the bucket array: header, seed, and the arena's four fields.
pub const RESERVED_WORDS: u32 = 4;
/// Bytes per bucket slot — a `u32` entry index, 2 to a word.
pub const SLOT_BYTES: u32 = 4;

/// Bytes per entry slot for an element type of `size` bytes.
///
/// Rounded up to 8 so every field keeps the alignment it had when an entry was its
/// own record (a record's payload starts at byte 8 of a word-aligned block, so an
/// `integer` field was 8-byte aligned and `Store::get_long` dereferences a typed
/// pointer).  The floor of 8 also guarantees room for the free-list link a released
/// slot threads through its own first 4 bytes.
#[must_use]
pub fn stride_for(size: u32) -> u32 {
    size.max(8).next_multiple_of(8)
}

/// The bucket a key's walk starts at — the ONE place a digest becomes a bucket number.
///
/// Part of the on-disk placement contract (`crate::placement::HASH`): a reader that
/// derives it differently looks in the wrong bucket.  That is also why it is still a
/// 64-bit `%` by a count that is not a power of two — a division on every lookup's
/// critical path, priced at 7–10 % of a cache-resident lookup
/// (`bench/portal/analysis/keyed.md`).  A multiply-shift range reduction would remove it
/// and move every entry of every stored hash, and an exact reciprocal needs a per-table
/// magic number the table's header has no word for; both are a format break, so neither
/// is taken here.
#[inline]
fn home_bucket(digest: u64, count: u32) -> u32 {
    (digest % u64::from(count)) as u32
}

/// Bucket slots in table record `claim`.
fn elms(store: &Store, claim: u32) -> u32 {
    (store.record_words(claim) - RESERVED_WORDS) * 2
}

/// Bytes per entry slot, as recorded in the table.
///
/// Stored rather than re-derived from the element type, so every reader of a hash —
/// the teardown walk, the iteration builder, the paged reader — decodes an entry
/// without having to be handed a type it would otherwise only need for this.
#[must_use]
pub fn stride(store: &Store, claim: u32) -> u32 {
    store.get_u32_raw(claim, STRIDE_FLD)
}

/// A bucket slot's high bit: this slot names a store RECORD, not an arena index.
///
/// A hash has two kinds of entry and **one table can hold both**.  Entries it was
/// asked to create come from its arena; entries handed to it already built belong to
/// whoever built them — a sibling field's `other_indexes` makes one collection a
/// second view of another's records, and neither may move or free what the other
/// owns.  The loft parser's own `Data` does exactly this: `def_names` receives
/// records that the definition list allocated, alongside entries of its own.
///
/// So the discriminator is per SLOT, not per table.  It was per table first — the
/// recorded stride, on the theory that a table either allocates its entries or
/// borrows them — and the parser falsified it in the debug-assertions gate: a hash
/// with a real stride was handed a foreign record, `index_of` answered 0, and the
/// entry was filed under a slot that means EMPTY.
///
/// A record number is safe to tag: it indexes WORDS, so the high bit would need a
/// 16 GB store, and the test-suite ceiling alone is 2 GB (`TESTING.md`
/// § Store-memory ceiling).  An arena index is bounded by the same store.
pub const SLOT_RECORD: u32 = 0x8000_0000;

/// Does this hash allocate its own entries?  True once it has an arena; a table that
/// has only ever been handed foreign records has none, and frees nothing.
#[must_use]
pub fn owns_entries(store: &Store, claim: u32) -> bool {
    stride(store, claim) != 0
}

/// The `DbRef` a bucket slot decodes to, or a null ref when it names nothing.
///
/// For an owned entry `(chunk, offset)` is arithmetic against a chunk directory small
/// enough to stay cache-resident, so a hit still costs ONE random read — the entry's
/// own bytes.  For a borrowed record it is the record at its payload start, which is
/// what a slot has always meant.
fn entry_ref(store: &Store, claim: u32, index: u32, store_nr: u16, stride: u32) -> DbRef {
    if stride == 0 || index & SLOT_RECORD != 0 {
        return DbRef {
            store_nr,
            rec: index & !SLOT_RECORD,
            pos: crate::store::RECORD_PAYLOAD,
        };
    }
    match arena::slot(store, claim, index, stride) {
        Some((rec, pos)) => DbRef { store_nr, rec, pos },
        None => DbRef {
            store_nr,
            rec: 0,
            pos: 0,
        },
    }
}

/// The loop-invariant half of [`entry_ref`], read ONCE for a walk over a table's buckets.
///
/// Decoding a slot re-read the arena's directory field, the directory's size header and
/// its bounds for every bucket probed — all facts of the TABLE, not of the slot.  A walk
/// holds them here and [`Entries::at`] is then the chunk arithmetic and one read.  At a
/// million entries that repetition hides behind two cache misses (@PLN135 measured the
/// hoist as nothing there); in a table that fits the cache it was a tenth of a lookup
/// and an eighth of an insert (`bench/portal/analysis/keyed.md`).
///
/// Valid only while nothing appends a chunk to the arena — a lookup, the duplicate walk
/// of an insert, a rebuild of the buckets, a removal's back-shift: none of them does.
#[derive(Clone, Copy)]
struct Entries {
    store_nr: u16,
    stride: u32,
    dir: u32,
    cap: u32,
}

impl Entries {
    fn of(store: &Store, claim: u32, store_nr: u16) -> Entries {
        let stride = stride(store, claim);
        let dir = if stride == 0 {
            0
        } else {
            store.get_u32_raw(claim, arena::DIR_FLD)
        };
        Entries {
            store_nr,
            stride,
            dir,
            cap: arena::dir_capacity(store, dir),
        }
    }

    /// `(record, payload offset)` of the entry bucket value `slot` names, record 0 when
    /// it names nothing — [`entry_ref`]'s answer, arm for arm.
    #[inline]
    fn at(&self, store: &Store, slot: u32) -> (u32, u32) {
        if self.stride == 0 || slot & SLOT_RECORD != 0 {
            return (slot & !SLOT_RECORD, crate::store::RECORD_PAYLOAD);
        }
        let (k, off) = arena::locate(slot, self.stride);
        if k >= self.cap {
            return (0, 0);
        }
        match store.get_u32_raw(self.dir, arena::DIR0 + 4 * k) {
            0 => (0, 0),
            rec => (rec, off),
        }
    }

    /// Does bucket value `slot` name the entry at `rec`?  What `slot == slot_value(rec)`
    /// asked, answered from the slot's side: a record slot names its record whatever
    /// `rec.pos` is, an arena slot the chunk and the stride-wide slot `rec.pos` falls in —
    /// the two tolerances `slot_value` and `arena::index_of` have — with no scan of the
    /// chunk directory for the index.
    #[inline]
    fn names(&self, store: &Store, slot: u32, rec: &DbRef) -> bool {
        if self.stride == 0 || slot & SLOT_RECORD != 0 {
            return slot & !SLOT_RECORD == rec.rec;
        }
        let (chunk, off) = self.at(store, slot);
        chunk == rec.rec
            && rec.pos >= arena::SLOT0
            && (rec.pos - arena::SLOT0) / self.stride == (off - arena::SLOT0) / self.stride
    }

    #[inline]
    fn entry(&self, store: &Store, slot: u32) -> DbRef {
        let (rec, pos) = self.at(store, slot);
        DbRef {
            store_nr: self.store_nr,
            rec,
            pos,
        }
    }
}

/// The bucket-slot value for `rec` in table `claim` — the inverse of [`entry_at`].
///
/// An entry this table's arena did not hand out is a record somebody else owns, and
/// is stored as a tagged record number.  `index_of` answering 0 IS that test: the
/// scan covers every chunk the table has, so a miss means the entry is not in it.
fn slot_value(store: &Store, claim: u32, rec: &DbRef, stride: u32) -> u32 {
    if stride != 0 {
        let index = arena::index_of(store, claim, rec.rec, rec.pos, stride);
        if index != 0 {
            return index;
        }
    }
    rec.rec | SLOT_RECORD
}

/// Read the per-hash seed stored in bucket record `claim`.
fn read_seed(store: &Store, claim: u32) -> u64 {
    let lo = u64::from(store.get_u32_raw(claim, SEED_FLD));
    let hi = u64::from(store.get_u32_raw(claim, SEED_FLD + 4));
    lo | (hi << 32)
}

/// Write the per-hash seed into bucket record `claim`.
fn write_seed(store: &mut Store, claim: u32, seed: u64) {
    store.set_u32_raw(claim, SEED_FLD, (seed & 0xFFFF_FFFF) as u32);
    store.set_u32_raw(claim, SEED_FLD + 4, (seed >> 32) as u32);
}

/// The hash's table record, creating and seeding it if this is the first touch.
///
/// Creation moved ahead of the first insert because an entry is now allocated from
/// the arena whose bookkeeping lives IN this record, and an entry is built before it
/// is inserted (`record_new` → the constructor's field writes → `insert_record`).
/// So `record_new` needs the table, and `add` finds it already there.
///
/// `stride` is recorded once, on creation: every entry of a given hash is the same
/// element type, so the width cannot change under it.
pub fn ensure_table(hash: &DbRef, stride: u32, stores: &mut [Store]) -> u32 {
    // loft#1213 — `collection_rec`, not `get_u32_raw`: a field left ABSENT rather than empty
    // holds the reserved absent id, and read raw that is a non-zero "existing table" whose
    // every access is out of bounds.  Mapped to `0` it means what it should — no table yet —
    // so the claim below runs and the slot is written with a real one.  That is materialising
    // an absent destination on WRITE, which is what the vector side has always done through
    // this same accessor (`vector_append`).
    let existing = keys::store(hash, stores).collection_rec(hash.rec, hash.pos);
    if existing != 0 {
        return existing;
    }
    // Claim 12 words so the bucket array (room - RESERVED_WORDS words, 2 slots/word)
    // starts at 16 slots, the size it has always started at.
    let claim = keys::mut_store(hash, stores).claim(12);
    keys::mut_store(hash, stores).zero_fill(claim);
    // Seed the new table and store the seed with its buckets, so any
    // reader (including a different process) re-derives identical buckets.
    let seed = keys::fresh_seed();
    write_seed(keys::mut_store(hash, stores), claim, seed);
    keys::mut_store(hash, stores).set_u32_raw(claim, STRIDE_FLD, stride);
    keys::mut_store(hash, stores).set_u32_raw(hash.rec, hash.pos, claim);
    claim
}

/// Hand out a zeroed entry slot, as the `DbRef` the caller then builds the entry
/// through.
///
/// The replacement for the per-entry `Store::claim` in `record_new`'s keyed arm.
/// `owner` is the record that owns the collection — written once per CHUNK, since
/// every slot in a chunk shares it, and read by `database::search` to decide whether
/// a record is live.
///
/// # Panics
///
/// If the slot the arena just handed out cannot be addressed — the arena's own
/// invariant, asserted here rather than papered over, because a silent null would
/// hand the constructor a `DbRef` that writes to record 0.
pub fn alloc_entry(hash: &DbRef, stride: u32, owner: u32, stores: &mut [Store]) -> DbRef {
    let claim = ensure_table(hash, stride, stores);
    let store = keys::mut_store(hash, stores);
    let index = arena::alloc(store, claim, stride, owner);
    let (rec, pos) = arena::slot(store, claim, index, stride).expect("just allocated");
    DbRef {
        store_nr: hash.store_nr,
        rec,
        pos,
    }
}

pub fn add(hash: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    // loft#1213 — read the slot through `collection_rec` so an ABSENT field reads as "no
    // table" and takes the branch below, rather than as a table at the reserved id.
    let mut claim = keys::store(hash, stores).collection_rec(hash.rec, hash.pos);
    if claim == 0 {
        // Reached only when the entry was allocated somewhere else — a SECONDARY index
        // over another collection's records.  Stride 0 records that: this table
        // borrows, so its slots hold record numbers and it frees nothing.
        claim = ensure_table(hash, 0, stores);
    }
    let length = keys::store(hash, stores).get_u32_raw(claim, LEN_FLD);
    let width = stride(keys::store(hash, stores), claim);
    let index = slot_value(keys::store(hash, stores), claim, rec, width);
    debug_assert!(
        index != 0,
        "a bucket slot of 0 means EMPTY, so an entry that decodes to 0 is one this \
         table cannot find again (rec={}, pos={}, stride={width})",
        rec.rec,
        rec.pos,
    );
    if let Some(grown) = grow_if_full(hash, claim, length, stores, keys) {
        claim = grown;
    }
    hash_set(claim, index, rec, stores, keys);
    keys::mut_store(rec, stores).set_u32_raw(claim, LEN_FLD, length + 1);
    // hash_validate(hash, key, stores, keys);
}

/// `LOFT_NO_HALF_LOAD=1` — a table is rebuilt at three quarters full again instead of at
/// half.  A writer's policy, read once: no reader assumes a load, so stores written under
/// either setting read under both.
fn half_load() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !keys::env_set("LOFT_NO_HALF_LOAD"))
}

/// Is a table of `room` words holding `length` entries due a rebuild before the next?
///
/// `elms = 2·(room - RESERVED_WORDS)`, so `length + RESERVED_WORDS >= room` is
/// `length >= elms / 2`: the table is rebuilt at HALF full.  It was three quarters
/// (`length * 2 / 3` in the same test), which is where linear probing turns: a probe here
/// is a dependent read of the entry, with no fingerprint to reject on, and a miss walks
/// to the first empty bucket — 8.5 buckets at 0.75 against 2.5 at 0.5.  A table filled
/// to just under the old threshold measured 2.30 key compares a hit and 5.36 a miss
/// (`bench/portal/analysis/keyed.md`).  The price is the bucket array, 4 bytes a slot:
/// ~10.7 bytes an entry averaged over a doubling instead of ~7.1.
fn is_full(length: u32, room: u32) -> bool {
    let weighed = if half_load() { length } else { length * 2 / 3 };
    weighed + RESERVED_WORDS >= room
}

/// The words a table must have to take `count` entries without a rebuild: one past
/// [`is_full`]'s trigger for `length == count`.
fn words_for(count: u64) -> u64 {
    let weighed = if half_load() { count } else { count * 2 / 3 };
    weighed + u64::from(RESERVED_WORDS) + 1
}

/// Rebuild table `claim` at twice its size when one more entry would cross the load
/// factor ([`is_full`]), answering the new table — the growth half of [`add`], shared
/// with [`add_at`].
fn grow_if_full(
    hash: &DbRef,
    claim: u32,
    length: u32,
    stores: &mut [Store],
    keys: &[Key],
) -> Option<u32> {
    let room = keys::store(hash, stores).record_words(claim);
    if !is_full(length, room) {
        return None;
    }
    let new_claim = keys::mut_store(hash, stores).claim(room * 2 - 1);
    keys::mut_store(hash, stores).zero_fill(new_claim);
    rehash_into(hash, claim, new_claim, stores, keys);
    install_table(hash, claim, new_claim, stores);
    Some(new_claim)
}

/// What ONE probe walk from a record's home bucket established about inserting it
/// ([`probe_for_insert`]).
pub enum Probed {
    /// No entry carries the record's key.  The walk ended on the empty bucket at byte
    /// offset `bucket` of the table — the one [`add`] would file it in — and `index` is
    /// the slot value that names the record.
    Free { bucket: u32, index: u32 },
    /// Another entry already carries the record's key: the one an insert displaces
    /// (@FR-Col-Insert — latest insert wins).
    Present(DbRef),
    /// Nothing established: no table yet, or the record is already filed in it.  The
    /// caller takes the two-walk form, which owns both answers.
    Unknown,
}

/// One hash and one probe walk for an insert: is `rec`'s key already here, and if not,
/// which bucket takes it.
///
/// An insert used to ask those as two separate walks — the duplicate lookup
/// (`Stores::dedup_keyed`: the key copied out into a `Vec<Content>`, hashed, probed) and
/// then [`add`] (the key re-read from the record, hashed AGAIN, probed again for the
/// empty bucket).  They are the same walk: both start at the key's home bucket, and the
/// duplicate — if there is one — lies before the first empty bucket, which is where the
/// second walk stops.  A quarter of a cache-resident insert was the repeat
/// (`bench/portal/analysis/keyed.md`).
///
/// The key is compared through [`keys::fast_key_of`] where it has one field of a listed
/// width, and record against record otherwise, so a compound key takes this path too.
#[must_use]
pub fn probe_for_insert(hash: &DbRef, rec: &DbRef, stores: &[Store], keys: &[Key]) -> Probed {
    let store = keys::store(hash, stores);
    let claim = store.collection_rec(hash.rec, hash.pos);
    if claim == 0 || store.record_words(claim) <= RESERVED_WORDS {
        return Probed::Unknown;
    }
    let count = elms(store, claim);
    let entries = Entries::of(store, claim, hash.store_nr);
    let own = slot_value(store, claim, rec, entries.stride);
    let hash_val = keys::hash(rec, stores, keys, read_seed(store, claim));
    let mut at = home_bucket(hash_val, count);
    let fast = keys::fast_key_of(rec, stores, keys);
    for _ in 0..count {
        let slot = store.get_u32_raw(claim, BUCKET0 + at * SLOT_BYTES);
        if slot == 0 {
            return Probed::Free {
                bucket: BUCKET0 + at * SLOT_BYTES,
                index: own,
            };
        }
        if slot == own {
            return Probed::Unknown;
        }
        let entry = entries.entry(store, slot);
        let same = match &fast {
            Some(f) => f.matches(store, entry.rec, entry.pos),
            None => keys::compare(rec, &entry, stores, keys) == Ordering::Equal,
        };
        if same {
            return Probed::Present(entry);
        }
        at += 1;
        if at >= count {
            at = 0;
        }
    }
    Probed::Unknown
}

/// File `rec` in the bucket [`probe_for_insert`] found free — [`add`] without its walk.
///
/// A table that must grow first is rebuilt exactly as [`add`] rebuilds it, and the
/// record is then filed by a walk of the NEW table: the bucket named an offset in the
/// old one.
///
/// # Panics
/// Under `LOFT_KEYED_VERIFY=1`, when `bucket` is not the one the free-slot walk chooses.
pub fn add_at(
    hash: &DbRef,
    rec: &DbRef,
    bucket: u32,
    index: u32,
    stores: &mut [Store],
    keys: &[Key],
) {
    let claim = keys::store(hash, stores).collection_rec(hash.rec, hash.pos);
    let length = keys::store(hash, stores).get_u32_raw(claim, LEN_FLD);
    if keys::keyed_verify() {
        let walked = hash_free_pos(claim, rec, stores, keys);
        assert!(
            walked == bucket,
            "LOFT_KEYED_VERIFY: the one-probe insert chose bucket {bucket} where the \
             free-slot walk chooses {walked}"
        );
    }
    if let Some(grown) = grow_if_full(hash, claim, length, stores, keys) {
        hash_set(grown, index, rec, stores, keys);
        keys::mut_store(rec, stores).set_u32_raw(grown, LEN_FLD, length + 1);
        return;
    }
    let store = keys::mut_store(rec, stores);
    store.set_u32_raw(claim, bucket, index);
    store.set_u32_raw(claim, LEN_FLD, length + 1);
}

/// Give `hash` a bucket table large enough to hold `count` entries without rehashing,
/// so filling it does not repeatedly rebuild the table (@PLN135 arc C).
///
/// Capacity only: it never changes which records are present, nor their order, nor
/// what `len` answers — a table sized for `count` behaves exactly like one that grew
/// into that size, because the seed and therefore every bucket is carried across.
/// A `count` the current table already covers does nothing, so calling it twice, or
/// with too small a number, is safe.
///
/// The size solves [`add`]'s own growth condition ([`is_full`]): `room` must exceed the
/// trigger for `length == count` ([`words_for`]).
pub fn reserve(hash: &DbRef, count: i64, stride: u32, stores: &mut [Store], keys: &[Key]) {
    // A negative or absurd count asks for nothing; a table so large its word count
    // overflows a `u32` cannot be claimed at all.  Both mean "leave it alone" — this
    // is a hint, and a hint never fails the program.
    let Ok(count) = u64::try_from(count) else {
        return;
    };
    // `+ 1` past the trigger is the whole point: `room` must EXCEED it for
    // `length == count` — sized to exactly the trigger, the last insert grows the table
    // and the reservation buys nothing but a doubling (measured under the 0.75 rule: a
    // 1M table reserved at the trigger ended up 10.7 MB at load 0.37 instead of 5.3 MB).
    let Ok(want) = u32::try_from(words_for(count)) else {
        return;
    };
    // Create the table if it is not there yet, so the seed, the stride and the arena
    // fields all come from the one place that mints them.
    let claim = ensure_table(hash, stride, stores);
    if keys::store(hash, stores).record_words(claim) >= want {
        return;
    }
    let new_claim = keys::mut_store(hash, stores).claim(want);
    keys::mut_store(hash, stores).zero_fill(new_claim);
    rehash_into(hash, claim, new_claim, stores, keys);
    install_table(hash, claim, new_claim, stores);
}

/// Point `hash` at `new_claim` and give `old_claim` back to the store.
///
/// The two are one step: a bucket table that is no longer the hash's table is
/// unreachable, and a claim nothing can reach is a leak. Both replacement sites — `add`'s
/// growth and [`reserve`] on a non-empty hash — used to do only the first half, so every
/// doubling stranded its predecessor. A grown 1M-entry hash carried 49.3 MB where the
/// identical content pre-sized carried 33.0, `store_reclaim` recovered none of it (the
/// blocks are CLAIMED, not free), and `store_persist_bind` wrote the dead tables to disk.
///
/// The order is load-bearing and is why this is one function rather than a line at each
/// site: repoint FIRST, free second. `Store::delete` repurposes the block's body as a
/// free-tree node and may coalesce it with its neighbours, so between the free and the
/// repoint the hash's field would name bytes that are already something else.
///
/// `old_claim == 0` is the first-allocation case — there is no predecessor to release.
fn install_table(hash: &DbRef, old_claim: u32, new_claim: u32, stores: &mut [Store]) {
    keys::mut_store(hash, stores).set_u32_raw(hash.rec, hash.pos, new_claim);
    if old_claim != 0 {
        keys::mut_store(hash, stores).delete(old_claim);
    }
}

/// Move every entry of bucket table `from` into the freshly zeroed table `into`,
/// carrying the seed and the live-entry count.
///
/// The seed travels with the buckets because the bucket layout is seed-dependent: a
/// rebuild that minted a new one would place every existing record somewhere else than
/// its own lookup will later look.
fn rehash_into(hash: &DbRef, from: u32, into: u32, stores: &mut [Store], keys: &[Key]) {
    let seed = read_seed(keys::store(hash, stores), from);
    write_seed(keys::mut_store(hash, stores), into, seed);
    // The arena's bookkeeping travels with the table it lives in.  Leaving it behind
    // would strand every chunk and the directory in the freed predecessor — the
    // abandoned-table leak this plan already fixed once, except that this one also
    // loses the ENTRIES, so the next insert would hand out index 1 again on top of a
    // live entry.  These four fields plus the seed are the whole of the table's
    // identity; the buckets are re-derived below.
    let entries = Entries::of(keys::store(hash, stores), from, hash.store_nr);
    for fld in [arena::DIR_FLD, arena::NEXT_FLD, arena::FREE_FLD, STRIDE_FLD] {
        let v = keys::store(hash, stores).get_u32_raw(from, fld);
        keys::mut_store(hash, stores).set_u32_raw(into, fld, v);
    }
    let length = keys::store(hash, stores).get_u32_raw(from, LEN_FLD);
    let count = elms(keys::store(hash, stores), from);
    for i in 0..count {
        let index = keys::store(hash, stores).get_u32_raw(from, BUCKET0 + SLOT_BYTES * i);
        if index == 0 {
            continue;
        }
        let entry = entries.entry(keys::store(hash, stores), index);
        hash_set(into, index, &entry, stores, keys);
    }
    keys::mut_store(hash, stores).set_u32_raw(into, LEN_FLD, length);
}

/// File arena `index` (whose entry is at `rec`) into table `claim`'s buckets.
fn hash_set(claim: u32, index: u32, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    let pos = hash_free_pos(claim, rec, stores, keys);
    keys::mut_store(rec, stores).set_u32_raw(claim, pos, index);
}

fn hash_free_pos(claim: u32, rec: &DbRef, stores: &[Store], keys: &[Key]) -> u32 {
    let count = elms(keys::store(rec, stores), claim);
    let seed = read_seed(keys::store(rec, stores), claim);
    let hash_val = keys::hash(rec, stores, keys, seed);
    let mut index = home_bucket(hash_val, count);
    for _ in 0..count {
        if keys::store(rec, stores).get_u32_raw(claim, BUCKET0 + index * SLOT_BYTES) == 0 {
            break;
        }
        index += 1;
        if index >= count {
            index = 0;
        }
    }
    BUCKET0 + index * SLOT_BYTES
}

/// The 0-based bucket that currently holds the entry at `rec`, with the slot value it
/// holds, or `None` when no bucket does.  The entry is recognised by DECODING each slot on
/// the chain (`Entries::names`), so a removal never maps its record back to an arena index
/// — a scan of the chunk directory it used to make twice, once here and once to free.
///
/// Probes from the key's home bucket and stops at the first EMPTY slot, which ends every probe
/// chain (deletion shifts entries back, so there are no tombstones to step over) — a record
/// the table holds is never past one.  It used to run the whole table and, for a record the
/// table did NOT hold, wrap back to the home bucket and answer THAT, which [`remove`] then
/// zeroed: a null element of a linked group handed to the unlink loop — a record whose key
/// reads as zero and that no view holds — took a live entry with it under every seed whose
/// zero-key bucket happened to be occupied, one run in twenty.
fn hash_rec_pos(
    claim: u32,
    entries: &Entries,
    rec: &DbRef,
    stores: &[Store],
    keys: &[Key],
) -> Option<(u32, u32)> {
    let store = keys::store(rec, stores);
    let count = elms(store, claim);
    let hash_val = keys::hash(rec, stores, keys, read_seed(store, claim));
    let mut index = home_bucket(hash_val, count);
    for _ in 0..count {
        let val = store.get_u32_raw(claim, BUCKET0 + index * SLOT_BYTES);
        if val == 0 {
            return None;
        }
        if entries.names(store, val, rec) {
            return Some((index, val));
        }
        index += 1;
        if index >= count {
            index = 0;
        }
    }
    None
}

#[must_use]
pub fn find(hash_ref: &DbRef, stores: &[Store], keys: &[Key], key: &[Content]) -> DbRef {
    let store = &stores[hash_ref.store_nr as usize];
    let claim = store.get_u32_raw(hash_ref.rec, hash_ref.pos);
    let mut record = DbRef {
        store_nr: hash_ref.store_nr,
        rec: 0,
        pos: 0,
    };
    if claim == 0 {
        return record;
    }
    let room = store.record_words(claim);
    if room == 0 {
        return record;
    }
    let count = elms(store, claim);
    let seed = read_seed(store, claim);
    let hash_val = keys::key_hash(key, seed);
    let mut index = home_bucket(hash_val, count);
    // @PLN135 arc B — a probe asks only *is this the key*, about the SAME key every
    // time, so the `(Content, type_nr)` match belongs outside the loop.  `fast_key`
    // resolves the field offset and the value once; the loop then reads the field
    // directly.  Same hash, same bucket order, same answer — measured at ~10 ns of a
    // ~33 ns cache-resident lookup on 1M `integer` keys.  A compound key, or a width
    // `fast_key` does not list, answers `None` and takes the general loop below.
    if let Some(fast) = keys::fast_key(keys, key) {
        return find_fast(hash_ref, store, claim, count, index, &fast);
    }
    let entries = Entries::of(store, claim, hash_ref.store_nr);
    let mut slot = store.get_u32_raw(claim, BUCKET0 + index * SLOT_BYTES);
    'Record: for _ in 0..count {
        if slot == 0 {
            record.rec = 0;
            record.pos = 0;
            break;
        }
        record = entries.entry(store, slot);
        if keys::key_compare(key, &record, stores, keys) != Ordering::Equal {
            index += 1;
            if index >= count {
                index = 0;
            }
            slot = store.get_u32_raw(claim, BUCKET0 + index * SLOT_BYTES);
            continue 'Record;
        }
        break;
    }
    record
}

/// `@FR-R-TypedKeyed` — [`find`] for a key of ONE integer field, handed over as the value
/// itself: the entry point of a caller that already knows the collection's kind and its key's
/// (`codegen_runtime::OpGetHashLong`, which `--native` emits for a `hash<T[k]>` whose one
/// key is an integer).  Nothing is built to be taken apart again — no `Content`, no
/// `FastKey` chosen and then re-dispatched — so it is the table's three header reads, the
/// hash, and the walk.  `None` when `key` is not one of the integer widths
/// [`keys::fast_key`] lists; the caller then takes [`find`].
///
/// The answer is [`find`]'s for `[Content::Long(value)]`: same digest
/// ([`keys::long_hash`]), same home bucket, same walk ([`probe`]), same comparison
/// ([`keys::FastKey::matches`]).
#[must_use]
pub fn find_long(hash_ref: &DbRef, stores: &[Store], key: &Key, value: i64) -> Option<DbRef> {
    use keys::FastKey;
    let kind = key.type_nr.unsigned_abs();
    if !matches!(kind, 1 | 2 | 8 | 12) {
        return None;
    }
    let store = &stores[hash_ref.store_nr as usize];
    let claim = store.get_u32_raw(hash_ref.rec, hash_ref.pos);
    let mut found = DbRef {
        store_nr: hash_ref.store_nr,
        rec: 0,
        pos: 0,
    };
    if claim == 0 || store.record_words(claim) == 0 {
        return Some(found);
    }
    let count = elms(store, claim);
    let home = home_bucket(keys::long_hash(value, read_seed(store, claim)), count);
    let entries = Entries::of(store, claim, hash_ref.store_nr);
    let p = u32::from(key.position);
    (found.rec, found.pos) = match kind {
        1 => {
            let k = FastKey::Int(p, value);
            probe(store, claim, count, home, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        2 => {
            let k = FastKey::Long(p, value);
            probe(store, claim, count, home, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        8 => {
            let k = FastKey::I32(p, value);
            probe(store, claim, count, home, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        _ => {
            let k = FastKey::U32(p, value);
            probe(store, claim, count, home, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
    };
    Some(found)
}

/// The probe walk for a pre-resolved key, compiled once per key KIND.
///
/// The kind is the table's, not the probe's: inside one arm the test the walk repeats is
/// a read and a compare with nothing left to decide, where a `fast.matches(…)` call per
/// bucket re-dispatched on the kind and paid a call for it (a quarter of a lookup's probe
/// cost, `bench/portal/analysis/keyed.md`).  Each arm rebuilds its own variant as a
/// constant so the comparison stays [`FastKey::matches`] — one home for what "the same
/// key" means — and folds to its one arm.
fn find_fast(
    hash_ref: &DbRef,
    store: &Store,
    claim: u32,
    count: u32,
    home: u32,
    fast: &keys::FastKey,
) -> DbRef {
    use keys::FastKey;
    let entries = Entries::of(store, claim, hash_ref.store_nr);
    let (rec, pos) = match *fast {
        FastKey::Int(p, v) => {
            let k = FastKey::Int(p, v);
            probe(store, claim, count, home, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        FastKey::Long(p, v) => {
            let k = FastKey::Long(p, v);
            probe(store, claim, count, home, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        FastKey::I32(p, v) => {
            let k = FastKey::I32(p, v);
            probe(store, claim, count, home, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        FastKey::U32(p, v) => {
            let k = FastKey::U32(p, v);
            probe(store, claim, count, home, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        FastKey::ShortRaw(p, st, v) => {
            let k = FastKey::ShortRaw(p, st, v);
            probe(store, claim, count, home, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        FastKey::Str(p, v) => {
            let k = FastKey::Str(p, v);
            probe(store, claim, count, home, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
    };
    DbRef {
        store_nr: hash_ref.store_nr,
        rec,
        pos,
    }
}

/// Walk from bucket `at` to the entry `same` accepts, or to the first empty bucket —
/// which ends every probe chain, since a removal shifts entries back and leaves no
/// tombstone.  Answers the entry's `(record, payload offset)`, record 0 for a miss.
// Measured, not assumed: the walk must be compiled INTO each key kind's arm, or the
// per-bucket test is a call again (`bench/portal/analysis/keyed.md`, L5).
#[allow(clippy::inline_always)]
#[inline(always)]
fn probe(
    store: &Store,
    claim: u32,
    count: u32,
    mut at: u32,
    entries: &Entries,
    same: impl Fn(u32, u32) -> bool,
) -> (u32, u32) {
    for _ in 0..count {
        let slot = store.get_u32_raw(claim, BUCKET0 + at * SLOT_BYTES);
        if slot == 0 {
            break;
        }
        let (rec, pos) = entries.at(store, slot);
        if same(rec, pos) {
            return (rec, pos);
        }
        at += 1;
        if at >= count {
            at = 0;
        }
    }
    (0, 0)
}

/// Unlink `rec` from the table, answering the bucket value that named it — 0 when the
/// table did not hold it.  [`free_slot`] takes that value to release an owned entry
/// without looking its arena index up again.
///
/// # Panics
/// Under `LOFT_KEYED_VERIFY=1`, when the slot recognised by decoding is not the one the
/// entry's arena index maps to.
pub fn remove(hash_ref: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) -> u32 {
    if rec.rec == 0 {
        return 0;
    }
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    let length = keys::store(hash_ref, stores).get_u32_raw(claim, LEN_FLD);
    if length == 0 {
        return 0;
    }
    let count = elms(keys::store(hash_ref, stores), claim);
    let entries = Entries::of(keys::store(hash_ref, stores), claim, hash_ref.store_nr);
    let seed = read_seed(keys::store(hash_ref, stores), claim);
    // Find the slot holding the entry and zero it (create the hole).  A record the table
    // does not hold leaves nothing — a remove of an absent record is a no-op, never a hole
    // where some other entry sat.
    let Some((mut hole, gone)) = hash_rec_pos(claim, &entries, rec, stores, keys) else {
        return 0;
    };
    if keys::keyed_verify() {
        let mapped = slot_value(keys::store(hash_ref, stores), claim, rec, entries.stride);
        assert!(
            mapped == gone,
            "LOFT_KEYED_VERIFY: the removal recognised bucket value {gone} where the \
             entry's own index maps to {mapped}"
        );
    }
    keys::mut_store(hash_ref, stores).set_u32_raw(claim, BUCKET0 + hole * SLOT_BYTES, 0);
    // Walk forward from hole+1 and pull each element back if its probe distance
    // to the hole is shorter than its probe distance to its current slot.
    // Stop at the first empty slot (all probe chains end at one).
    //
    // Every bucket number here is below `count`, so a distance round the table is one
    // conditional add and the step one conditional reset: the `%` each of the three used
    // to be is a division per entry walked, for an answer a compare already has.
    let round = |to: u32, from: u32| {
        if to >= from {
            to - from
        } else {
            to + count - from
        }
    };
    let step = |at: u32| if at + 1 >= count { 0 } else { at + 1 };
    let mut idx = step(hole);
    for _ in 0..count {
        let val = keys::store(hash_ref, stores).get_u32_raw(claim, BUCKET0 + idx * SLOT_BYTES);
        if val == 0 {
            break;
        }
        let next = entries.entry(keys::store(hash_ref, stores), val);
        let ideal = home_bucket(keys::hash(&next, stores, keys, seed), count);
        // Move if probe distance to hole is shorter than probe distance to idx.
        if round(hole, ideal) < round(idx, ideal) {
            keys::mut_store(hash_ref, stores).set_u32_raw(claim, BUCKET0 + hole * SLOT_BYTES, val);
            keys::mut_store(hash_ref, stores).set_u32_raw(claim, BUCKET0 + idx * SLOT_BYTES, 0);
            hole = idx;
        }
        idx = step(idx);
    }
    keys::mut_store(hash_ref, stores).set_u32_raw(claim, LEN_FLD, length - 1);
    gone
}

/// Give back the entry bucket value `slot` named — [`free_entry`] for a caller that still
/// holds what [`remove`] answered.  A record slot is somebody else's record and a
/// borrowing table owns nothing, so both release nothing, as [`free_entry`] decides too.
pub fn free_slot(hash_ref: &DbRef, slot: u32, stores: &mut [Store]) {
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    if claim == 0 || slot == 0 || slot & SLOT_RECORD != 0 {
        return;
    }
    let width = stride(keys::store(hash_ref, stores), claim);
    if width == 0 {
        return;
    }
    arena::free(keys::mut_store(hash_ref, stores), claim, slot, width);
}

/// Give an entry's slot back to the arena — the counterpart of the `Store::delete`
/// that used to release an entry record.
///
/// Deliberately NOT part of [`remove`], which only UNLINKS: a secondary index shares
/// its entries with the primary collection and must never free them.  That split is
/// also what keeps the order safe — a released slot threads the free list through its
/// own first four bytes, so it must be released only after the caller has finished
/// reading the entry's fields (its key to unlink by, its pointers to free).
pub fn free_entry(hash_ref: &DbRef, rec: &DbRef, stores: &mut [Store]) {
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return;
    }
    let width = stride(keys::store(hash_ref, stores), claim);
    if width == 0 {
        // A borrowed record belongs to the primary collection, which frees it.
        return;
    }
    let index = arena::index_of(
        keys::store(hash_ref, stores),
        claim,
        rec.rec,
        rec.pos,
        width,
    );
    arena::free(keys::mut_store(hash_ref, stores), claim, index, width);
}

/**
Check the allocations and structure of the hash table.
# Panics
When the structure is not correctly filled
*/
/// Count the live records in a hash table.
///
/// Walks the bucket array (same loop as `records()` but counting
/// instead of collecting).  O(room) where `room` is the bucket array
/// length, typically ~1.5× the live-record count.  Returns 0 for an
/// uninitialised hash (no claim allocated yet).
///
/// Powers `len(h)` for `hash<T[key]>` (P192).
#[must_use]
pub fn count(hash_ref: &DbRef, stores: &[Store]) -> u32 {
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return 0;
    }
    let count = elms(keys::store(hash_ref, stores), claim);
    let mut total: u32 = 0;
    for i in 0..count {
        if keys::store(hash_ref, stores).get_u32_raw(claim, BUCKET0 + i * SLOT_BYTES) != 0 {
            total += 1;
        }
    }
    total
}

/// Byte size of a hash's bucket table — the full table, holes included
/// (@PLN110 `size`).
///
/// The hash's own allocation is the bucket array: `elms` slots, each a 4-byte
/// `u32` record-id (an empty slot is a hole — a zero rec-id — and still counts,
/// because open addressing's spare capacity IS the format). Allocation-local:
/// the pointed-to entry records live in separate allocations and are NOT
/// counted. Excludes the two reserved header words (record header + seed),
/// mirroring `size(vector)` counting content, not the length prefix. Returns 0
/// for an uninitialised hash (no claim allocated yet).
#[must_use]
pub fn table_bytes(hash_ref: &DbRef, stores: &[Store]) -> u32 {
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return 0;
    }
    elms(keys::store(hash_ref, stores), claim) * SLOT_BYTES
}

/// C60 Step 1: collect every live record's record-number from a hash.
///
/// Walks the hash's internal bucket array — the same traversal pattern
/// as `validate`, but appending each nonzero slot into a vector instead
/// of asserting.  Returned order is internal bucket order (unspecified
/// but stable for a given hash state) — callers that need a
/// user-visible ordering sort the result afterwards.
///
/// Runs in O(room) time where `room` is the bucket array length,
/// typically around 1.5× the live-record count.
#[must_use]
pub fn records(hash_ref: &DbRef, stores: &[Store]) -> Vec<DbRef> {
    entries(hash_ref, stores)
        .into_iter()
        .map(|(at, _)| at)
        .collect()
}

/// Every live entry, each paired with whether THIS table's arena allocated it.
///
/// The teardown needs the pair, not the ref: an entry the arena handed out comes
/// back with the chunks, and one this table only BORROWS — a record a sibling
/// collection allocated, reached through an `other_indexes` view — must be left
/// entirely alone, not freed and not even recursed into, because the collection that
/// owns it will do both.  One table can hold some of each ([`SLOT_RECORD`]).
#[must_use]
pub fn entries(hash_ref: &DbRef, stores: &[Store]) -> Vec<(DbRef, bool)> {
    let store = keys::store(hash_ref, stores);
    let claim = store.get_u32_raw(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return Vec::new();
    }
    let count = elms(store, claim);
    let width = stride(store, claim);
    let mut out = Vec::new();
    for i in 0..count {
        let index = store.get_u32_raw(claim, BUCKET0 + i * SLOT_BYTES);
        if index != 0 {
            let ours = width != 0 && index & SLOT_RECORD == 0;
            out.push((
                entry_ref(store, claim, index, hash_ref.store_nr, width),
                ours,
            ));
        }
    }
    out
}

/// Every record the hash's storage occupies besides the entries themselves: the
/// arena's chunks and its directory.
///
/// What a teardown frees AFTER recursing into the entries' own children — the table
/// record is the caller's `container_rec` and is freed alongside.  An arena chunk
/// missed here leaks every entry in it, which is the class this plan already fixed
/// once for abandoned bucket tables.
#[must_use]
pub fn arena_records(hash_ref: &DbRef, stores: &[Store]) -> Vec<u32> {
    let store = keys::store(hash_ref, stores);
    let claim = store.get_u32_raw(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return Vec::new();
    }
    arena::all_records(store, claim)
}

/// C60 Step 2: collect every live record sorted by the hash's key.
///
/// Ascending on each key field, with `-` prefix flipping the direction
/// per-field — the existing `keys::compare` helper handles multi-field
/// lexicographic order and the descending bit for us, so one call
/// covers Steps 2 / 6 / 7 of the plan in CAVEATS.md C60.
///
/// Inefficient by design: walks the whole bucket array (Step 1) then
/// sorts the collected references in O(n log n).  Suitable for the
/// small hashes that scripting code typically iterates; users with a
/// tight loop over a large hash should pair the hash with a `vector`
/// or `sorted` for amortised traversal.
///
/// # Panics
///
/// Panics if `keys::compare` encounters a key field type it cannot
/// compare — same invariant as the existing `hash::find` path and
/// not reachable from valid loft source.
#[must_use]
pub fn records_sorted(hash_ref: &DbRef, stores: &[Store], keys: &[Key]) -> Vec<DbRef> {
    let mut recs = records(hash_ref, stores);
    recs.sort_by(|a, b| keys::compare(a, b, stores, keys));
    recs
}

/// Validate the bucket structure of a hash — each live slot's record
/// must `find` back to the same rec-nr, and the stored length must
/// match the number of nonzero slots.
///
/// # Panics
///
/// Panics via `assert_eq!` when the bucket structure is inconsistent
/// (a slot whose key does not round-trip through `find`, or a stored
/// length that does not match the actual live-slot count).  Used as a
/// debug-time structural invariant check; callers should never hit a
/// panic here in production.
pub fn validate(hash_ref: &DbRef, stores: &[Store], keys: &[Key]) {
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    let length = keys::store(hash_ref, stores).get_u32_raw(claim, LEN_FLD);
    let mut l = 0;
    for record in records(hash_ref, stores) {
        l += 1;
        let key = keys::get_key(&record, stores, keys);
        let found = find(hash_ref, stores, keys, &key);
        assert_eq!(
            (found.rec, found.pos),
            (record.rec, record.pos),
            "Incorrect entry"
        );
    }
    assert_eq!(length, l, "Incorrect hash length");
}
