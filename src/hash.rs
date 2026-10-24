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
//   fld 32 : `BUCKET0`    — first bucket slot: a u32 entry INDEX (2 per word), or with the
//            `TAGGED` flag that index and the key digest's low 32 bits (`(Col-Tag)`)
//
// An INLINE table (`(Col-Inline)`, the `INLINE` flag) keeps the same seed, insertion cursor
// and tombstone count, and differs in three words: field 4 names the record that owns the
// collection (where an arena chunk keeps it), the live count moves to field 16
// (`INLINE_LEN_FLD`, the chunk directory an inline table has no use for), and `STRIDE_FLD`
// carries the bucket count's exponent in bits 16–20.  Its slots are tagged; its entries
// follow them, one per bucket (`ent0`).
//
// The seed makes a persisted hash portable: it is stored WITH the buckets,
// so a reader re-derives the same bucket for every key (see
// `keys::seeded_hasher`).  `elms = (room - RESERVED_WORDS) * 2`.
//
// An arena table's bucket slot holds a 1-based ARENA INDEX, not a record number.
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
/// Bytes per UNTAGGED bucket slot — a `u32` entry index, 2 to a word — the width a table
/// without the [`TAGGED`] flag has; a tagged slot is 8 (`(Col-Tag)`, @C105).
pub const SLOT_BYTES: u32 = 4;

/// `(Col-Tag)` — `STRIDE_FLD`'s high bit: this table's slots are TAGGED, 8 bytes each — the
/// entry index, then the low 32 bits of its key's digest.  A table without it (one a store
/// written before carries) has [`SLOT_BYTES`]-wide slots and is read as it was written.
pub const TAGGED: u32 = 0x8000_0000;

/// `(Col-Inline)` — `STRIDE_FLD`'s next bit: this table keeps its entries IN the record, an
/// entries array of `count × stride` bytes after the `count` tagged slots, an entry at its
/// bucket's position.  A slot's first word is the entry's insertion number (`arena::NEXT_FLD`
/// counts them, from 1), not an arena index: a walk in slot order sorts by it, so it follows
/// the inserts and removes alone, never the seed.  A removal leaves [`TOMB`] (a keyed removal
/// moves no other entry, `(Col-Remove)`); only a rebuild moves entries, and drops the
/// tombstones, which `arena::FREE_FLD` counts.  Only tables that own their entries are made
/// inline: a type a stored reference or a group shares is `linked`, and gets per-record entries
/// in a borrowing table (`Stores::record_new`).
pub const INLINE: u32 = 0x4000_0000;

/// A removed inline entry's slot: live for a probe walk, empty for a walk over the entries.
pub const TOMB: u32 = u32::MAX;

/// The bytes per entry in `STRIDE_FLD`: an element type's size is a `u16`, so the word's high
/// half is free for the flags above and, on an inline table, its bucket count.
pub const STRIDE_MASK: u32 = 0xFFFF;

/// `(Col-Inline)` — an inline table's bucket count is a power of two (`@FR-Col-Bucket`), kept
/// as its exponent in `STRIDE_FLD` bits 16–20: read with the flags, no division and no field.
const COUNT_SHIFT: u32 = 16;

/// `(Col-Inline)` — an inline table's field 4 names its OWNER, as an arena chunk's does
/// (`arena::OWNER_FLD`): the record that holds an entry names the record that holds the
/// collection, whichever record that is.  Its live count moves to field 16, which an inline
/// table has no chunk directory for.
pub const INLINE_LEN_FLD: u32 = arena::DIR_FLD;

/// Bytes per entry, from a table's `STRIDE_FLD` word.
#[inline]
#[must_use]
pub fn stride_of(flags: u32) -> u32 {
    flags & STRIDE_MASK
}

/// An inline table's bucket count, from its `STRIDE_FLD` word.
#[inline]
#[must_use]
pub fn inline_count_of(flags: u32) -> u32 {
    1 << ((flags >> COUNT_SHIFT) & 31)
}

/// The field a table keeps its live count in, from its `STRIDE_FLD` word.
#[inline]
fn len_fld_of(flags: u32) -> u32 {
    if flags & INLINE != 0 {
        INLINE_LEN_FLD
    } else {
        LEN_FLD
    }
}

/// `LOFT_NO_HASH_INLINE=1` — `(Col-Inline)` off: tables that own their entries keep them in
/// the arena.  A writer's policy, read once: a reader reads either layout (`@FR-Col-Layouts`,
/// @C152).
pub fn hash_inline() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !keys::env_set("LOFT_NO_HASH_INLINE") && hash_tag())
}

/// `LOFT_TRACE_HASH_LAYOUT=1` — each table made, grown or demoted names its layout on stderr,
/// with the record that holds it: what a store holds is otherwise visible only in its bytes.
fn trace_layout(event: &str, hash: &DbRef, from: Option<u32>, flags: u32) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| keys::env_set("LOFT_TRACE_HASH_LAYOUT")) {
        return;
    }
    let name = |f: u32| {
        let kind = if f & INLINE != 0 {
            "inline"
        } else if f & TAGGED != 0 {
            "tagged"
        } else {
            "plain"
        };
        let own = if stride_of(f) == 0 { " borrowing" } else { "" };
        format!("{kind}{own}")
    };
    let from = from.map_or(String::new(), |f| format!("{} -> ", name(f)));
    eprintln!(
        "hash layout: {event} {from}{} at {}:{}+{}",
        name(flags),
        hash.store_nr,
        hash.rec,
        hash.pos
    );
}

/// A table's header word `STRIDE_FLD` read once, with what it decides.
#[derive(Clone, Copy)]
struct Hdr {
    /// Bucket slots.
    count: u32,
    /// Bytes per slot.
    sb: u32,
    /// Bytes per entry, 0 for a table that borrows.
    stride: u32,
    inline: bool,
}

/// The buckets of an inline table of `words` words past the reserved ones: the largest power
/// of two whose slots and entries fit.
#[inline]
#[must_use]
pub fn inline_count(words: u32, stride: u32) -> u32 {
    let n = (u64::from(words) * 8 / u64::from(8 + stride)) as u32;
    if n == 0 { 0 } else { 1 << n.ilog2() }
}

/// The words an inline table of `count` buckets of `stride`-byte entries takes.
#[inline]
fn inline_words(count: u32, stride: u32) -> u32 {
    RESERVED_WORDS + count * (8 + stride) / 8
}

/// Byte offset of an inline table's entries array.
#[inline]
#[must_use]
pub fn ent0(count: u32) -> u32 {
    BUCKET0 + count * 8
}

#[inline(always)]
#[allow(clippy::inline_always)]
fn hdr(store: &Store, claim: u32) -> Hdr {
    let words = store.record_words(claim).saturating_sub(RESERVED_WORDS);
    let flags = store.get_u32_raw(claim, STRIDE_FLD);
    let stride = stride_of(flags);
    if flags & INLINE != 0 {
        // The count is recorded when the table is made (`inline_create`, `inline_rebuild`):
        // deriving it from the words is a division per access.
        Hdr {
            count: inline_count_of(flags),
            sb: 8,
            stride,
            inline: true,
        }
    } else {
        let sb = if flags & TAGGED != 0 { 8 } else { SLOT_BYTES };
        Hdr {
            count: slots_in(words, sb),
            sb,
            stride,
            inline: false,
        }
    }
}

/// Is table `claim` inline?
#[inline]
#[must_use]
pub fn is_inline(store: &Store, claim: u32) -> bool {
    store.get_u32_raw(claim, STRIDE_FLD) & INLINE != 0
}

/// Bytes per bucket slot of table `claim`: 8 when it is tagged, [`SLOT_BYTES`] when not.
#[inline]
#[must_use]
pub fn slot_bytes(store: &Store, claim: u32) -> u32 {
    if store.get_u32_raw(claim, STRIDE_FLD) & TAGGED != 0 {
        8
    } else {
        SLOT_BYTES
    }
}

/// The bucket slots `words` words hold at `sb` bytes a slot — a shift, not a division: the
/// width is one of two.  A tagged table is only ever made at a power-of-two count, and the
/// allocator may hand it a block a few words larger: its count is the largest power of two
/// the words hold, so a home bucket stays a mask and the words past it stay empty slots.
#[inline]
#[must_use]
pub fn slots_in(words: u32, sb: u32) -> u32 {
    if sb == 8 {
        if words == 0 { 0 } else { 1 << words.ilog2() }
    } else {
        words * 2
    }
}

/// The tag a key's digest leaves in a tagged slot: its low 32 bits, which hold the home
/// bucket of every power-of-two table up to 2^32 buckets (`home_bucket` masks them).
#[inline]
#[must_use]
pub fn tag_of(digest: u64) -> u32 {
    digest as u32
}

/// Fill slot `at` of a table whose slots are `sb` bytes: the index, and on a tagged table
/// the tag.
#[inline(always)]
#[allow(clippy::inline_always)]
fn put_slot(store: &mut Store, claim: u32, sb: u32, at: u32, index: u32, tag: u32) {
    store.set_u32_raw(claim, BUCKET0 + at * sb, index);
    if sb == 8 {
        store.set_u32_raw(claim, BUCKET0 + at * 8 + 4, tag);
    }
}

/// Does slot `at` of a table whose slots are `sb` bytes carry a tag other than `want`?  An
/// untagged table answers no, so its walk reads every entry as it always did.
#[inline]
fn tag_differs(store: &Store, claim: u32, sb: u32, at: u32, want: u32) -> bool {
    sb == 8 && store.get_u32_raw(claim, BUCKET0 + at * 8 + 4) != want
}

/// `LOFT_NO_HASH_TAG=1` — `(Col-Tag)` off: tables are created and rebuilt with untagged
/// 4-byte slots, as before.  A writer's policy, read once: a reader reads either width.
fn hash_tag() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !keys::env_set("LOFT_NO_HASH_TAG") && pow2_buckets())
}

/// The width a table created or rebuilt now takes.
fn new_slot_bytes() -> u32 {
    if hash_tag() { 8 } else { SLOT_BYTES }
}

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

/// The bucket a key's walk starts at — the ONE place a digest becomes a bucket number,
/// for the writer here and the paged reader alike.
///
/// Part of the on-disk placement contract (`crate::placement::HASH`): the bucket is
/// `digest mod count`, and a reader that derives it differently looks in the wrong bucket.
/// Enforces `@FR-Col-Bucket` (@C146): every table this program sizes has a POWER-OF-TWO
/// count, for which the remainder is a mask — the 64-bit division on every lookup's
/// critical path priced at 7–10 % of a lookup (`bench/portal/analysis/keyed.md`) is gone,
/// and the bucket is the same number.  A table the allocator handed a few words more, or
/// one a store written before carries, keeps the division: same formula, so no stored hash
/// moves and none is refused.
#[inline]
#[must_use]
pub fn home_bucket(digest: u64, count: u32) -> u32 {
    if count.is_power_of_two() && pow2_buckets() {
        (digest & u64::from(count - 1)) as u32
    } else {
        (digest % u64::from(count)) as u32
    }
}

/// `LOFT_NO_POW2_BUCKETS=1` — `@FR-Col-Bucket` off: tables grow and reserve to the sizes
/// they had before (a count that is not a power of two) and every bucket is a division.
/// The A/B for the rule, and the way to write a table the rule's reader must still place.
fn pow2_buckets() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !keys::env_set("LOFT_NO_POW2_BUCKETS"))
}

/// The words a table record needs for a POWER-OF-TWO bucket count of at least `elms`
/// slots (`@FR-Col-Bucket`, @C146): the bucket array is `RESERVED_WORDS` behind the
/// header, two slots a word, so the count is `2·(room - RESERVED_WORDS)`.
fn room_for_buckets(elms: u64, sb: u32) -> u64 {
    u64::from(RESERVED_WORDS) + elms.max(2).next_power_of_two() * u64::from(sb) / 8
}

/// Bucket slots in table record `claim`.
fn elms(store: &Store, claim: u32) -> u32 {
    hdr(store, claim).count
}

/// Bytes per entry slot, as recorded in the table.
///
/// Stored rather than re-derived from the element type, so every reader of a hash —
/// the teardown walk, the iteration builder, the paged reader — decodes an entry
/// without having to be handed a type it would otherwise only need for this.
#[must_use]
pub fn stride(store: &Store, claim: u32) -> u32 {
    stride_of(store.get_u32_raw(claim, STRIDE_FLD))
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
/// 16 GB store, and the test-suite ceiling alone is 2 GB (`RUN_BOUNDS.md`
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
    /// `(Col-Inline)`: entries sit in the table `claim` from byte `ent0`, at their bucket.
    inline: bool,
    claim: u32,
    ent0: u32,
}

impl Entries {
    fn of(store: &Store, claim: u32, store_nr: u16) -> Entries {
        let h = hdr(store, claim);
        if h.inline {
            return Entries::inline(claim, store_nr, &h);
        }
        Entries::with_stride(store, claim, store_nr, h.stride)
    }

    /// The entries of table `claim`, its header already read.
    fn from_hdr(store: &Store, claim: u32, store_nr: u16, h: &Hdr) -> Entries {
        if h.inline {
            Entries::inline(claim, store_nr, h)
        } else {
            Entries::with_stride(store, claim, store_nr, h.stride)
        }
    }

    /// The entries of inline table `claim`.
    fn inline(claim: u32, store_nr: u16, h: &Hdr) -> Entries {
        Entries {
            store_nr,
            stride: h.stride,
            dir: 0,
            cap: 0,
            inline: true,
            claim,
            ent0: ent0(h.count),
        }
    }

    /// The entry a probe at bucket `at` holding slot value `slot` compares: in place for an
    /// inline table, through the arena otherwise.
    #[inline]
    fn at_bucket(&self, store: &Store, slot: u32, at: u32) -> (u32, u32) {
        if self.inline {
            (self.claim, self.ent0 + at * self.stride)
        } else {
            self.at(store, slot)
        }
    }

    /// [`Entries::of`] for a caller that has read the stride already.
    fn with_stride(store: &Store, claim: u32, store_nr: u16, stride: u32) -> Entries {
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
            inline: false,
            claim: 0,
            ent0: 0,
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
#[inline]
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
    // starts at 16 slots, the size it has always started at — a power of two
    // (`@FR-Col-Bucket`).
    let sb = new_slot_bytes();
    // `(Col-Inline)` — a table that owns its entries keeps them in its record.
    let claim = if stride != 0 && hash_inline() {
        inline_create(keys::mut_store(hash, stores), 16, stride, hash.rec)
    } else {
        arena_table(keys::mut_store(hash, stores), sb, stride)
    };
    // Seed the new table and store the seed with its buckets, so any
    // reader (including a different process) re-derives identical buckets.
    let seed = keys::fresh_seed();
    write_seed(keys::mut_store(hash, stores), claim, seed);
    keys::mut_store(hash, stores).set_u32_raw(hash.rec, hash.pos, claim);
    trace_layout(
        "made",
        hash,
        None,
        keys::store(hash, stores).get_u32_raw(claim, STRIDE_FLD),
    );
    claim
}

/// A new empty arena table of 16 `sb`-byte slots for `stride`-byte entries, unseeded.
fn arena_table(store: &mut Store, sb: u32, stride: u32) -> u32 {
    let claim = store.claim(RESERVED_WORDS + 16 * sb / 8);
    store.zero_fill(claim);
    let flag = if sb == 8 { TAGGED } else { 0 };
    store.set_u32_raw(claim, STRIDE_FLD, stride | flag);
    claim
}

/// `(Col-Inline)` — field 4 of a record [`alloc_entry`] staged for an inline table.  A record
/// handed to a table from elsewhere carries its back-pointer there, a record NUMBER, and a
/// record number never reaches this (it indexes words: the top bits would need a 16 GB store,
/// as [`SLOT_RECORD`] says), so the mark tells the table's own stage from a record it must
/// not consume.
const STAGE_MARK: u32 = 0xFFFF_FFF1;

/// Hand out a zeroed entry slot, as the `DbRef` the caller then builds the entry
/// through.
///
/// The replacement for the per-entry `Store::claim` in `record_new`'s keyed arm.
/// `owner` is the record that owns the collection — written once per CHUNK, since
/// every slot in a chunk shares it, and read by `database::search` to decide whether
/// a record is live.  The address is the one the arena's claim resolved
/// ([`arena::alloc_at`]): a chunk it just ensured, never record 0.
pub fn alloc_entry(hash: &DbRef, stride: u32, owner: u32, stores: &mut [Store]) -> DbRef {
    let claim = ensure_table(hash, stride, stores);
    let store = keys::mut_store(hash, stores);
    if is_inline(store, claim) {
        // `(Col-Inline)` — the entry is built in a stage of its own and copied into its
        // bucket when it is filed ([`add`]), since its key decides the bucket.
        let rec = store.claim(1 + stride.div_ceil(8));
        store.zero_fill(rec);
        store.set_u32_raw(rec, 4, STAGE_MARK);
        return DbRef {
            store_nr: hash.store_nr,
            rec,
            pos: 8,
        };
    }
    let (_, rec, pos) = arena::alloc_at(store, claim, stride, owner);
    DbRef {
        store_nr: hash.store_nr,
        rec,
        pos,
    }
}

/// `@FR-R-KeyedEntry` — the entry an insert of a record keyed `value` writes its fields into:
/// [`entry_ints`] for a key of one `integer` or `long` part.
pub fn entry_long(
    hash: &DbRef,
    stores: &mut [Store],
    keys: &[Key],
    value: i64,
    owner: u32,
) -> Option<DbRef> {
    entry_ints(hash, stores, keys, &[value], owner)
}

/// `@FR-R-KeyedEntry` — the entry an insert of a record keyed `values` writes its fields into:
/// the one already under that key, or a fresh one filed where the probe for it ended.
///
/// ONE walk from the key's home bucket answers both, as [`probe_for_insert`]'s does, but over
/// the key VALUES: the record is not built yet.  A hit answers the entry itself — the caller
/// overwrites its fields, which is what replacing it with an identical-keyed record leaves,
/// with no slot claimed and none released.  A miss claims an arena slot, writes every key part
/// into it, and files it in the empty bucket the walk stopped at, under the arena index the
/// claim answered (no `arena::index_of` scan to recover it).  The digest is
/// [`keys::ints_hash`], which is [`keys::hash`]'s over a record carrying those parts, so a
/// later lookup by either route walks the same buckets.
///
/// `None` when the table cannot be walked this way — no table yet, a table that borrows its
/// entries, a key that is not one to four `integer` / `long` parts, one value per part — and
/// nothing was changed: the caller takes the general insert.
pub fn entry_ints(
    hash: &DbRef,
    stores: &mut [Store],
    keys: &[Key],
    values: &[i64],
    owner: u32,
) -> Option<DbRef> {
    // An `integer` and a `long` part are the same 8-byte read and write (`Store::get_int`,
    // `get_long`), so one compare serves both.
    if keys.is_empty()
        || keys.len() > 4
        || keys.len() != values.len()
        || keys
            .iter()
            .any(|k| !matches!(k.type_nr.unsigned_abs(), 1 | 2))
    {
        return None;
    }
    let store = keys::store(hash, stores);
    let claim = store.collection_rec(hash.rec, hash.pos);
    if claim == 0 || store.record_words(claim) <= RESERVED_WORDS {
        return None;
    }
    let entries = Entries::of(store, claim, hash.store_nr);
    if entries.stride == 0 {
        return None;
    }
    let count = elms(store, claim);
    let sb = slot_bytes(store, claim);
    let digest = keys::ints_hash(values, read_seed(store, claim));
    let tag = tag_of(digest);
    let mut at = home_bucket(digest, count);
    let mut bucket = None;
    for _ in 0..count {
        let slot = store.get_u32_raw(claim, BUCKET0 + at * sb);
        if slot == 0 {
            bucket = Some(BUCKET0 + at * sb);
            break;
        }
        if (entries.inline && slot == TOMB) || tag_differs(store, claim, sb, at, tag) {
            at += 1;
            if at >= count {
                at = 0;
            }
            continue;
        }
        let (rec, pos) = entries.at_bucket(store, slot, at);
        if keys
            .iter()
            .zip(values)
            .all(|(k, v)| store.get_int(rec, pos + u32::from(k.position)) == *v)
        {
            return Some(DbRef {
                store_nr: hash.store_nr,
                rec,
                pos,
            });
        }
        at += 1;
        if at >= count {
            at = 0;
        }
    }
    // A full walk with no empty bucket cannot happen below the load factor; leave it to the
    // general insert, which rebuilds the table first.
    let bucket = bucket?;
    if entries.inline {
        // A locked store's refusal is the general insert's to make (`@FR-H-WriteLocked`).
        if keys::store(hash, stores).read_only {
            return None;
        }
        let (claim, at) = inline_slot_for(hash, claim, tag, stores);
        let store = keys::mut_store(hash, stores);
        let entry = inline_file(store, hash.store_nr, claim, at, tag, true);
        for (k, v) in keys.iter().zip(values) {
            store.set_int(entry.rec, entry.pos + u32::from(k.position), *v);
        }
        return Some(entry);
    }
    let store = keys::mut_store(hash, stores);
    let (index, rec, pos) = arena::alloc_at(store, claim, entries.stride, owner);
    for (k, v) in keys.iter().zip(values) {
        store.set_int(rec, pos + u32::from(k.position), *v);
    }
    let rec = DbRef {
        store_nr: hash.store_nr,
        rec,
        pos,
    };
    add_at(hash, &rec, bucket, index, tag, stores, keys);
    Some(rec)
}

/// `@FR-R-KeyedEntry` for ONE `integer` / `long` key — [`entry_ints`] with the table's header
/// read once (its record words, its width-and-stride word, its seed) and the insert written in
/// place when the table does not grow: the shape `--native` emits for every `hash<T[k]>`
/// entry whose one key is an integer (`Stores::keyed_entry_long_at`).  Same digest, same walk,
/// same answer as [`entry_ints`]; `None` where it would answer `None`.
pub fn entry_one_int(
    hash: &DbRef,
    stores: &mut [Store],
    keys: &[Key],
    value: i64,
    owner: u32,
) -> Option<DbRef> {
    let [key] = keys else { return None };
    if !matches!(key.type_nr.unsigned_abs(), 1 | 2) {
        return None;
    }
    let kp = u32::from(key.position);
    let store = keys::store(hash, stores);
    let claim = store.collection_rec(hash.rec, hash.pos);
    if claim == 0 {
        return None;
    }
    if let Some(entry) = inline_entry_int(hash, claim, kp, value, stores) {
        return Some(entry);
    }
    let store = keys::store(hash, stores);
    if store.record_words(claim) <= RESERVED_WORDS {
        return None;
    }
    let h = hdr(store, claim);
    let (count, sb, width) = (h.count, h.sb, h.stride);
    if width == 0 {
        return None;
    }
    let entries = Entries::from_hdr(store, claim, hash.store_nr, &h);
    let digest = keys::ints_hash(&[value], read_seed(store, claim));
    let tag = tag_of(digest);
    let mut at = home_bucket(digest, count);
    let mut bucket = None;
    for _ in 0..count {
        let slot = store.get_u32_raw(claim, BUCKET0 + at * sb);
        if slot == 0 {
            bucket = Some(at);
            break;
        }
        if (!h.inline || slot != TOMB) && !tag_differs(store, claim, sb, at, tag) {
            let (rec, pos) = entries.at_bucket(store, slot, at);
            if store.get_int(rec, pos + kp) == value {
                return Some(DbRef {
                    store_nr: hash.store_nr,
                    rec,
                    pos,
                });
            }
        }
        at += 1;
        if at >= count {
            at = 0;
        }
    }
    let at = bucket?;
    if h.inline {
        if store.read_only {
            return None;
        }
        let used =
            store.get_u32_raw(claim, INLINE_LEN_FLD) + store.get_u32_raw(claim, arena::FREE_FLD);
        let entry = if is_full(used, count) {
            let (claim, at) = inline_slot_for(hash, claim, tag, stores);
            inline_file(
                keys::mut_store(hash, stores),
                hash.store_nr,
                claim,
                at,
                tag,
                true,
            )
        } else {
            // The walk above ended at this empty bucket: it is the one the entry takes.
            inline_file_at(
                keys::mut_store(hash, stores),
                hash.store_nr,
                claim,
                count,
                width,
                at,
                tag,
                true,
            )
        };
        keys::mut_store(hash, stores).set_int(entry.rec, entry.pos + kp, value);
        return Some(entry);
    }
    let length = store.get_u32_raw(claim, LEN_FLD);
    let store = keys::mut_store(hash, stores);
    let (index, rec, pos) = arena::alloc_at(store, claim, width, owner);
    store.set_int(rec, pos + kp, value);
    let entry = DbRef {
        store_nr: hash.store_nr,
        rec,
        pos,
    };
    if is_full(length, count) || keys::keyed_verify() {
        add_at(hash, &entry, BUCKET0 + at * sb, index, tag, stores, keys);
    } else {
        put_slot(store, claim, sb, at, index, tag);
        store.set_u32_raw(claim, LEN_FLD, length + 1);
    }
    Some(entry)
}

/// [`entry_ints`] for a table whose ONE key is a `text` (`@FR-R-KeyedEntry`'s text clause):
/// one walk over the key text answers the entry already under it, or a fresh one with its key
/// text written, filed where the walk ended.  The digest is [`keys::str_hash`], which is
/// [`keys::hash`]'s over a record carrying that text.  `None` when the table cannot be walked
/// this way, and nothing was changed.
pub fn entry_str(
    hash: &DbRef,
    stores: &mut [Store],
    keys: &[Key],
    value: &str,
    owner: u32,
) -> Option<DbRef> {
    let [key] = keys else { return None };
    if key.type_nr.unsigned_abs() != 6 {
        return None;
    }
    let store = keys::store(hash, stores);
    let claim = store.collection_rec(hash.rec, hash.pos);
    if claim == 0 || store.record_words(claim) <= RESERVED_WORDS {
        return None;
    }
    let entries = Entries::of(store, claim, hash.store_nr);
    if entries.stride == 0 {
        return None;
    }
    let count = elms(store, claim);
    let sb = slot_bytes(store, claim);
    let fast = keys::FastKey::Str(u32::from(key.position), value);
    let digest = keys::str_hash(value, read_seed(store, claim));
    let tag = tag_of(digest);
    let mut at = home_bucket(digest, count);
    let mut bucket = None;
    for _ in 0..count {
        let slot = store.get_u32_raw(claim, BUCKET0 + at * sb);
        if slot == 0 {
            bucket = Some(BUCKET0 + at * sb);
            break;
        }
        if (entries.inline && slot == TOMB) || tag_differs(store, claim, sb, at, tag) {
            at += 1;
            if at >= count {
                at = 0;
            }
            continue;
        }
        let (rec, pos) = entries.at_bucket(store, slot, at);
        if fast.matches(store, rec, pos) {
            return Some(DbRef {
                store_nr: hash.store_nr,
                rec,
                pos,
            });
        }
        at += 1;
        if at >= count {
            at = 0;
        }
    }
    let bucket = bucket?;
    if entries.inline {
        if keys::store(hash, stores).read_only {
            return None;
        }
        let (claim, at) = inline_slot_for(hash, claim, tag, stores);
        let store = keys::mut_store(hash, stores);
        let entry = inline_file(store, hash.store_nr, claim, at, tag, true);
        let text = store.set_str(value);
        store.set_u32_raw(entry.rec, entry.pos + u32::from(key.position), text);
        return Some(entry);
    }
    let store = keys::mut_store(hash, stores);
    let (index, rec, pos) = arena::alloc_at(store, claim, entries.stride, owner);
    let text = store.set_str(value);
    store.set_u32_raw(rec, pos + u32::from(key.position), text);
    let rec = DbRef {
        store_nr: hash.store_nr,
        rec,
        pos,
    };
    add_at(hash, &rec, bucket, index, tag, stores, keys);
    Some(rec)
}

/// File `rec` in the hash: answers the entry as filed — `rec` itself, or for an inline table
/// (`(Col-Inline)`) the bucket's entry its staged bytes were copied into, the stage freed.
pub fn add(hash: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) -> DbRef {
    // loft#1213 — read the slot through `collection_rec` so an ABSENT field reads as "no
    // table" and takes the branch below, rather than as a table at the reserved id.
    let mut claim = keys::store(hash, stores).collection_rec(hash.rec, hash.pos);
    if claim != 0 && is_inline(keys::store(hash, stores), claim) {
        return inline_add(hash, claim, rec, stores, keys);
    }
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
    *rec
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
fn is_full(length: u32, elms: u32) -> bool {
    if half_load() {
        length * 2 >= elms
    } else {
        length * 4 >= elms * 3
    }
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
    let count = elms(keys::store(hash, stores), claim);
    if !is_full(length, count) {
        return None;
    }
    // Twice the buckets, at a power-of-two count (`@FR-Col-Bucket`): a table that was not
    // one — a store written before, a claim the allocator rounded up — reaches it here, and
    // an untagged one is rebuilt tagged (`(Col-Tag)`).
    let sb = new_slot_bytes();
    let want = if pow2_buckets() {
        let Ok(want) = u32::try_from(room_for_buckets(u64::from(count) * 2, sb)) else {
            return None;
        };
        want
    } else {
        room * 2 - 1
    };
    let new_claim = keys::mut_store(hash, stores).claim(want);
    keys::mut_store(hash, stores).zero_fill(new_claim);
    mark_width(keys::mut_store(hash, stores), new_claim, sb);
    rehash_into(hash, claim, new_claim, stores, keys);
    install_table(hash, claim, new_claim, stores);
    Some(new_claim)
}

/// Mark freshly zeroed table `claim` as having `sb`-byte slots, before anything reads its
/// count — [`elms`] answers from the mark.
fn mark_width(store: &mut Store, claim: u32, sb: u32) {
    store.set_u32_raw(claim, STRIDE_FLD, if sb == 8 { TAGGED } else { 0 });
}

/// What ONE probe walk from a record's home bucket established about inserting it
/// ([`probe_for_insert`]).
pub enum Probed {
    /// No entry carries the record's key.  The walk ended on the empty bucket at byte
    /// offset `bucket` of the table — the one [`add`] would file it in — and `index` is
    /// the slot value that names the record.
    Free { bucket: u32, index: u32, tag: u32 },
    /// Another entry already carries the record's key: the one an insert displaces
    /// (@FR-Col-Insert — latest insert wins).  It sits in the bucket at byte offset
    /// `bucket`, and `index` is the slot value that names the inserted record — so the
    /// insert can take that bucket over ([`replace_at`]).
    Present {
        entry: DbRef,
        bucket: u32,
        index: u32,
    },
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
    if is_inline(store, claim) {
        // An inline table files a staged record by copying it ([`add`]): the two-walk form
        // owns both answers.
        return Probed::Unknown;
    }
    let count = elms(store, claim);
    let sb = slot_bytes(store, claim);
    let entries = Entries::of(store, claim, hash.store_nr);
    let own = slot_value(store, claim, rec, entries.stride);
    let hash_val = keys::hash(rec, stores, keys, read_seed(store, claim));
    let tag = tag_of(hash_val);
    let mut at = home_bucket(hash_val, count);
    let fast = keys::fast_key_of(rec, stores, keys);
    for _ in 0..count {
        let slot = store.get_u32_raw(claim, BUCKET0 + at * sb);
        if slot == 0 {
            return Probed::Free {
                bucket: BUCKET0 + at * sb,
                index: own,
                tag,
            };
        }
        if slot == own {
            return Probed::Unknown;
        }
        if (entries.inline && slot == TOMB) || tag_differs(store, claim, sb, at, tag) {
            at += 1;
            if at >= count {
                at = 0;
            }
            continue;
        }
        let entry = entries.entry(store, slot);
        let same = match &fast {
            Some(f) => f.matches(store, entry.rec, entry.pos),
            None => keys::compare(rec, &entry, stores, keys) == Ordering::Equal,
        };
        if same {
            return Probed::Present {
                entry,
                bucket: BUCKET0 + at * sb,
                index: own,
            };
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
    tag: u32,
    stores: &mut [Store],
    keys: &[Key],
) {
    let claim = keys::store(hash, stores).collection_rec(hash.rec, hash.pos);
    if is_inline(keys::store(hash, stores), claim) {
        inline_add(hash, claim, rec, stores, keys);
        return;
    }
    let length = keys::store(hash, stores).get_u32_raw(claim, LEN_FLD);
    if keys::keyed_verify() {
        let (at, _) = hash_free_pos(claim, rec, stores, keys);
        let walked = BUCKET0 + at * slot_bytes(keys::store(hash, stores), claim);
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
    if slot_bytes(store, claim) == 8 {
        store.set_u32_raw(claim, bucket + 4, tag);
    }
    store.set_u32_raw(claim, LEN_FLD, length + 1);
}

/// Put the record slot value `index` in the bucket at byte offset `bucket`, where
/// [`probe_for_insert`] found an entry with the same key, and answer the value it held.
///
/// The record carries the displaced entry's key, so its home bucket and its probe chain are
/// the displaced entry's: a lookup walks the same buckets and stops here, and no other
/// entry's chain passes through anything that changed.  The count stays what it was — one
/// entry out, one in — so the table never grows here.  That is the whole of what [`remove`]
/// and [`add`] together did to the table, without the back-shift and the second walk.
pub fn replace_at(hash: &DbRef, bucket: u32, index: u32, stores: &mut [Store]) -> u32 {
    let claim = keys::store(hash, stores).collection_rec(hash.rec, hash.pos);
    let store = keys::mut_store(hash, stores);
    let gone = store.get_u32_raw(claim, bucket);
    store.set_u32_raw(claim, bucket, index);
    gone
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
    // At a power-of-two bucket count (`@FR-Col-Bucket`): the words past the trigger,
    // rounded up to the next table of that shape.
    let need = words_for(count).saturating_sub(u64::from(RESERVED_WORDS)) * 2;
    let sb = new_slot_bytes();
    let words = if pow2_buckets() {
        room_for_buckets(need, sb)
    } else {
        words_for(count)
    };
    let Ok(want) = u32::try_from(words) else {
        return;
    };
    // Create the table if it is not there yet, so the seed, the stride and the arena
    // fields all come from the one place that mints them.
    let claim = ensure_table(hash, stride, stores);
    if is_inline(keys::store(hash, stores), claim) {
        // `(Col-Inline)` — rebuilt in place at the bucket count the fill needs.
        let target = need.max(2).next_power_of_two();
        if u64::from(elms(keys::store(hash, stores), claim)) < target
            && let Ok(target) = u32::try_from(target)
        {
            inline_rebuild(hash, claim, target, stores);
        }
        return;
    }
    let have = u64::from(elms(keys::store(hash, stores), claim));
    if have >= need.max(2).next_power_of_two()
        || (!pow2_buckets() && keys::store(hash, stores).record_words(claim) >= want)
    {
        return;
    }
    let new_claim = keys::mut_store(hash, stores).claim(want);
    keys::mut_store(hash, stores).zero_fill(new_claim);
    mark_width(keys::mut_store(hash, stores), new_claim, sb);
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
/// blocks are CLAIMED, not free), and an image of the store wrote the dead tables to disk.
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
        let store = keys::store(hash, stores);
        let (was, now) = (
            store.get_u32_raw(old_claim, STRIDE_FLD),
            store.get_u32_raw(new_claim, STRIDE_FLD),
        );
        trace_layout("grown", hash, Some(was), now);
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
    // The width is the new table's own (`mark_width`); the stride travels.
    let width = keys::store(hash, stores).get_u32_raw(into, STRIDE_FLD) & TAGGED;
    for fld in [arena::DIR_FLD, arena::NEXT_FLD, arena::FREE_FLD] {
        let v = keys::store(hash, stores).get_u32_raw(from, fld);
        keys::mut_store(hash, stores).set_u32_raw(into, fld, v);
    }
    let from_stride = stride(keys::store(hash, stores), from);
    keys::mut_store(hash, stores).set_u32_raw(into, STRIDE_FLD, from_stride | width);
    let length = keys::store(hash, stores).get_u32_raw(from, LEN_FLD);
    let count = elms(keys::store(hash, stores), from);
    let from_sb = slot_bytes(keys::store(hash, stores), from);
    let into_count = elms(keys::store(hash, stores), into);
    if from_sb == 8 && into_count.is_power_of_two() && pow2_buckets() {
        // `(Col-Tag)` — every entry placed from its tag: no key read, nothing hashed.
        let store = keys::mut_store(hash, stores);
        let into_sb = slot_bytes(store, into);
        let mask = into_count - 1;
        // Both tables are claims sized from their own headers (`count`, `into_count`), so
        // the walk reads and writes their slots directly: per slot, the accessor's bounds
        // test was most of a rebuild's instructions.
        let base = store.base_ptr();
        // Every access through these is `read_unaligned` / `write_unaligned`.
        #[allow(clippy::cast_ptr_alignment)]
        let src = |i: u32| unsafe {
            let p = base
                .add(from as usize * 8 + (BUCKET0 + 8 * i) as usize)
                .cast::<u32>();
            (p.read_unaligned(), p.add(1).read_unaligned())
        };
        #[allow(clippy::cast_ptr_alignment)]
        let dst = |at: u32| unsafe {
            base.add(into as usize * 8 + (BUCKET0 + at * into_sb) as usize)
                .cast::<u32>()
        };
        for i in 0..count {
            let (index, tag) = src(i);
            if index == 0 {
                continue;
            }
            let mut at = tag & mask;
            while unsafe { dst(at).read_unaligned() } != 0 {
                at = (at + 1) & mask;
            }
            unsafe {
                dst(at).write_unaligned(index);
                if into_sb == 8 {
                    dst(at).add(1).write_unaligned(tag);
                }
            }
        }
    } else if let Some(parts) = int_parts(keys) {
        rehash_ints(
            hash,
            from,
            into,
            count,
            &entries,
            &parts[..keys.len()],
            seed,
            stores,
        );
    } else {
        for i in 0..count {
            let index = keys::store(hash, stores).get_u32_raw(from, BUCKET0 + from_sb * i);
            if index == 0 {
                continue;
            }
            let entry = entries.entry(keys::store(hash, stores), index);
            hash_set(into, index, &entry, stores, keys);
        }
    }
    keys::mut_store(hash, stores).set_u32_raw(into, LEN_FLD, length);
}

/// The field offsets of a key whose every part is an `integer` or a `long` (one to four of
/// them), or `None` for any other key — the shape [`rehash_ints`] files without the key
/// description.
fn int_parts(keys: &[Key]) -> Option<[u32; 4]> {
    if keys.is_empty() || keys.len() > 4 {
        return None;
    }
    let mut parts = [0; 4];
    for (n, k) in keys.iter().enumerate() {
        if !matches!(k.type_nr.unsigned_abs(), 1 | 2) {
            return None;
        }
        parts[n] = u32::from(k.position);
    }
    Some(parts)
}

/// [`rehash_into`]'s loop for a key of integer parts (`int_parts`): each entry's parts read
/// straight from it and digested by [`keys::ints_hash`] — the digest [`keys::hash`] answers
/// for that entry, so every bucket is the one the general loop chooses — with the new table's
/// count and seed read once rather than per entry.
#[allow(clippy::too_many_arguments)]
fn rehash_ints(
    hash: &DbRef,
    from: u32,
    into: u32,
    count: u32,
    entries: &Entries,
    parts: &[u32],
    seed: u64,
    stores: &mut [Store],
) {
    let store = keys::mut_store(hash, stores);
    let into_count = elms(store, into);
    let from_sb = slot_bytes(store, from);
    let into_sb = slot_bytes(store, into);
    let mut values = [0_i64; 4];
    for i in 0..count {
        let index = store.get_u32_raw(from, BUCKET0 + from_sb * i);
        if index == 0 {
            continue;
        }
        let (rec, pos) = entries.at(store, index);
        for (v, p) in values.iter_mut().zip(parts) {
            *v = store.get_int(rec, pos + p);
        }
        let digest = keys::ints_hash(&values[..parts.len()], seed);
        let mut at = home_bucket(digest, into_count);
        for _ in 0..into_count {
            if store.get_u32_raw(into, BUCKET0 + at * into_sb) == 0 {
                break;
            }
            at += 1;
            if at >= into_count {
                at = 0;
            }
        }
        put_slot(store, into, into_sb, at, index, tag_of(digest));
    }
}

/// File arena `index` (whose entry is at `rec`) into table `claim`'s buckets.
fn hash_set(claim: u32, index: u32, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    let (at, tag) = hash_free_pos(claim, rec, stores, keys);
    let store = keys::mut_store(rec, stores);
    let sb = slot_bytes(store, claim);
    put_slot(store, claim, sb, at, index, tag);
}

/// The empty bucket a walk from `rec`'s home ends at, with the tag `rec`'s key leaves.
fn hash_free_pos(claim: u32, rec: &DbRef, stores: &[Store], keys: &[Key]) -> (u32, u32) {
    let count = elms(keys::store(rec, stores), claim);
    let sb = slot_bytes(keys::store(rec, stores), claim);
    let seed = read_seed(keys::store(rec, stores), claim);
    let hash_val = keys::hash(rec, stores, keys, seed);
    let mut index = home_bucket(hash_val, count);
    for _ in 0..count {
        if keys::store(rec, stores).get_u32_raw(claim, BUCKET0 + index * sb) == 0 {
            break;
        }
        index += 1;
        if index >= count {
            index = 0;
        }
    }
    (index, tag_of(hash_val))
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
    let sb = slot_bytes(store, claim);
    let hash_val = keys::hash(rec, stores, keys, read_seed(store, claim));
    let tag = tag_of(hash_val);
    let mut index = home_bucket(hash_val, count);
    for _ in 0..count {
        let val = store.get_u32_raw(claim, BUCKET0 + index * sb);
        if val == 0 {
            return None;
        }
        if !tag_differs(store, claim, sb, index, tag) && entries.names(store, val, rec) {
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
    let claim = store.collection_rec(hash_ref.rec, hash_ref.pos);
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
    let sb = slot_bytes(store, claim);
    let seed = read_seed(store, claim);
    let hash_val = keys::key_hash(key, seed);
    let tag = tag_of(hash_val);
    let mut index = home_bucket(hash_val, count);
    // @PLN135 arc B — a probe asks only *is this the key*, about the SAME key every
    // time, so the `(Content, type_nr)` match belongs outside the loop.  `fast_key`
    // resolves the field offset and the value once; the loop then reads the field
    // directly.  Same hash, same bucket order, same answer — measured at ~10 ns of a
    // ~33 ns cache-resident lookup on 1M `integer` keys.  A compound key, or a width
    // `fast_key` does not list, answers `None` and takes the general loop below.
    if let Some(fast) = keys::fast_key(keys, key) {
        return find_fast(hash_ref, store, claim, count, index, tag, &fast);
    }
    let entries = Entries::of(store, claim, hash_ref.store_nr);
    let mut slot = store.get_u32_raw(claim, BUCKET0 + index * sb);
    'Record: for _ in 0..count {
        if slot == 0 {
            record.rec = 0;
            record.pos = 0;
            break;
        }
        let skip = (entries.inline && slot == TOMB) || tag_differs(store, claim, sb, index, tag);
        if !skip {
            let (r, p) = entries.at_bucket(store, slot, index);
            record = DbRef {
                store_nr: hash_ref.store_nr,
                rec: r,
                pos: p,
            };
        }
        if skip || keys::key_compare(key, &record, stores, keys) != Ordering::Equal {
            index += 1;
            if index >= count {
                index = 0;
            }
            slot = store.get_u32_raw(claim, BUCKET0 + index * sb);
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
    let claim = store.collection_rec(hash_ref.rec, hash_ref.pos);
    let mut found = DbRef {
        store_nr: hash_ref.store_nr,
        rec: 0,
        pos: 0,
    };
    if claim == 0 || store.record_words(claim) == 0 {
        return Some(found);
    }
    if matches!(kind, 1 | 2)
        && let Some(entry) = inline_find_int(
            store,
            hash_ref.store_nr,
            claim,
            u32::from(key.position),
            value,
        )
    {
        return Some(entry);
    }
    let h = hdr(store, claim);
    let (count, sb) = (h.count, h.sb);
    let digest = keys::long_hash(value, read_seed(store, claim));
    let tag = tag_of(digest);
    let home = home_bucket(digest, count);
    let entries = Entries::from_hdr(store, claim, hash_ref.store_nr, &h);
    let p = u32::from(key.position);
    (found.rec, found.pos) = match kind {
        1 => {
            let k = FastKey::Int(p, value);
            probe(store, claim, count, sb, home, tag, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        2 => {
            let k = FastKey::Long(p, value);
            probe(store, claim, count, sb, home, tag, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        8 => {
            let k = FastKey::I32(p, value);
            probe(store, claim, count, sb, home, tag, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        _ => {
            let k = FastKey::U32(p, value);
            probe(store, claim, count, sb, home, tag, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
    };
    Some(found)
}

/// [`find_long`] for a hash keyed by two to four `integer` / `long` parts, the key handed
/// over as the values (`@FR-R-TypedKeyed`'s compound clause): the digest is
/// [`keys::ints_hash`], the one [`keys::key_hash`] computes for those `Content::Long`s, and
/// the probe compares every part in place.  `None` for any other key shape.
#[must_use]
pub fn find_ints(
    hash_ref: &DbRef,
    stores: &[Store],
    keys: &[Key],
    values: &[i64],
) -> Option<DbRef> {
    if keys.len() < 2
        || keys.len() > 4
        || keys.len() != values.len()
        || keys
            .iter()
            .any(|k| !matches!(k.type_nr.unsigned_abs(), 1 | 2))
    {
        return None;
    }
    let store = &stores[hash_ref.store_nr as usize];
    let claim = store.collection_rec(hash_ref.rec, hash_ref.pos);
    let mut found = DbRef {
        store_nr: hash_ref.store_nr,
        rec: 0,
        pos: 0,
    };
    if claim == 0 || store.record_words(claim) == 0 {
        return Some(found);
    }
    let h = hdr(store, claim);
    let (count, sb) = (h.count, h.sb);
    let digest = keys::ints_hash(values, read_seed(store, claim));
    let home = home_bucket(digest, count);
    let entries = Entries::from_hdr(store, claim, hash_ref.store_nr, &h);
    (found.rec, found.pos) = probe(
        store,
        claim,
        count,
        sb,
        home,
        tag_of(digest),
        &entries,
        |r, b| {
            keys.iter()
                .zip(values)
                .all(|(k, v)| store.get_int(r, b + u32::from(k.position)) == *v)
        },
    );
    Some(found)
}

/// [`find_long`] for a hash whose ONE key is a `text` (type 6), the key handed over as the
/// text itself (`@FR-R-TypedKeyed`'s text clause): the digest is [`keys::str_hash`], the
/// one [`keys::key_hash`] computes for a `Content::Str`, and the probe compares each
/// entry's key text in place.  `None` for any other key kind, which leaves the lookup to
/// the general entry.
#[must_use]
pub fn find_str(hash_ref: &DbRef, stores: &[Store], key: &Key, value: &str) -> Option<DbRef> {
    if key.type_nr.unsigned_abs() != 6 {
        return None;
    }
    let store = &stores[hash_ref.store_nr as usize];
    let claim = store.collection_rec(hash_ref.rec, hash_ref.pos);
    let mut found = DbRef {
        store_nr: hash_ref.store_nr,
        rec: 0,
        pos: 0,
    };
    if claim == 0 || store.record_words(claim) == 0 {
        return Some(found);
    }
    let h = hdr(store, claim);
    let (count, sb) = (h.count, h.sb);
    let digest = keys::str_hash(value, read_seed(store, claim));
    let home = home_bucket(digest, count);
    let entries = Entries::from_hdr(store, claim, hash_ref.store_nr, &h);
    let k = keys::FastKey::Str(u32::from(key.position), value);
    (found.rec, found.pos) = probe(
        store,
        claim,
        count,
        sb,
        home,
        tag_of(digest),
        &entries,
        |r, b| k.matches(store, r, b),
    );
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
    tag: u32,
    fast: &keys::FastKey,
) -> DbRef {
    use keys::FastKey;
    let entries = Entries::of(store, claim, hash_ref.store_nr);
    let sb = slot_bytes(store, claim);
    let (rec, pos) = match *fast {
        FastKey::Int(p, v) => {
            let k = FastKey::Int(p, v);
            probe(store, claim, count, sb, home, tag, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        FastKey::Long(p, v) => {
            let k = FastKey::Long(p, v);
            probe(store, claim, count, sb, home, tag, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        FastKey::I32(p, v) => {
            let k = FastKey::I32(p, v);
            probe(store, claim, count, sb, home, tag, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        FastKey::U32(p, v) => {
            let k = FastKey::U32(p, v);
            probe(store, claim, count, sb, home, tag, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        FastKey::ShortRaw(p, st, v) => {
            let k = FastKey::ShortRaw(p, st, v);
            probe(store, claim, count, sb, home, tag, &entries, |r, b| {
                k.matches(store, r, b)
            })
        }
        FastKey::Str(p, v) => {
            let k = FastKey::Str(p, v);
            probe(store, claim, count, sb, home, tag, &entries, |r, b| {
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
#[allow(clippy::too_many_arguments)]
fn probe(
    store: &Store,
    claim: u32,
    count: u32,
    sb: u32,
    mut at: u32,
    tag: u32,
    entries: &Entries,
    same: impl Fn(u32, u32) -> bool,
) -> (u32, u32) {
    for _ in 0..count {
        let slot = store.get_u32_raw(claim, BUCKET0 + at * sb);
        if slot == 0 {
            break;
        }
        if (entries.inline && slot == TOMB) || tag_differs(store, claim, sb, at, tag) {
            at += 1;
            if at >= count {
                at = 0;
            }
            continue;
        }
        let (rec, pos) = entries.at_bucket(store, slot, at);
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
    let claim = keys::store(hash_ref, stores).collection_rec(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return 0;
    }
    if is_inline(keys::store(hash_ref, stores), claim) {
        return inline_remove(hash_ref, claim, rec, stores);
    }
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
    let sb = slot_bytes(keys::store(hash_ref, stores), claim);
    let by_tag = sb == 8 && count.is_power_of_two() && pow2_buckets();
    keys::mut_store(hash_ref, stores).set_u32_raw(claim, BUCKET0 + hole * sb, 0);
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
        let val = keys::store(hash_ref, stores).get_u32_raw(claim, BUCKET0 + idx * sb);
        if val == 0 {
            break;
        }
        let tag = if sb == 8 {
            keys::store(hash_ref, stores).get_u32_raw(claim, BUCKET0 + idx * 8 + 4)
        } else {
            0
        };
        // `(Col-Tag)` — the home bucket from the tag where the table is a tagged power of
        // two; the key re-hashed otherwise.
        let ideal = if by_tag {
            tag & (count - 1)
        } else {
            let next = entries.entry(keys::store(hash_ref, stores), val);
            home_bucket(keys::hash(&next, stores, keys, seed), count)
        };
        // Move if probe distance to hole is shorter than probe distance to idx.
        if round(hole, ideal) < round(idx, ideal) {
            let store = keys::mut_store(hash_ref, stores);
            put_slot(store, claim, sb, hole, val, tag);
            store.set_u32_raw(claim, BUCKET0 + idx * sb, 0);
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
    let claim = keys::store(hash_ref, stores).collection_rec(hash_ref.rec, hash_ref.pos);
    if claim == 0
        || slot == 0
        || slot & SLOT_RECORD != 0
        || is_inline(keys::store(hash_ref, stores), claim)
    {
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
    let claim = keys::store(hash_ref, stores).collection_rec(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return;
    }
    if is_inline(keys::store(hash_ref, stores), claim) {
        // A removed inline entry is a tombstone in the table, which frees it with itself; a
        // stage never filed is a record of its own.
        if rec.rec != claim {
            keys::mut_store(hash_ref, stores).delete(rec.rec);
        }
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
/// Count the live records in a hash table: the header's live count (`LEN_FLD`), which every
/// insert, removal and rebuild maintains and [`validate`] checks against the occupied slots.
/// Constant time.  Returns 0 for an absent or uninitialised hash.
///
/// Powers `len(h)` for `hash<T[key]>` (P192).
#[must_use]
pub fn count(hash_ref: &DbRef, stores: &[Store]) -> u32 {
    // An absent hash has no elements, as an absent vector has none (`length_vector`): the
    // null is tested before any store is indexed.
    if hash_ref.is_null() {
        return 0;
    }
    let claim = keys::store(hash_ref, stores).collection_rec(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return 0;
    }
    live_len(keys::store(hash_ref, stores), claim)
}

/// Table `claim`'s live count, from the field its kind keeps it in.
fn live_len(store: &Store, claim: u32) -> u32 {
    store.get_u32_raw(claim, len_fld_of(store.get_u32_raw(claim, STRIDE_FLD)))
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
    let claim = keys::store(hash_ref, stores).collection_rec(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return 0;
    }
    let h = hdr(keys::store(hash_ref, stores), claim);
    if h.inline {
        h.count * (8 + h.stride)
    } else {
        h.count * h.sb
    }
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
    let claim = store.collection_rec(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return Vec::new();
    }
    if is_inline(store, claim) {
        return inline_entries(store, hash_ref.store_nr, claim)
            .into_iter()
            .map(|e| (e, true))
            .collect();
    }
    let count = elms(store, claim);
    let sb = slot_bytes(store, claim);
    let width = stride(store, claim);
    let mut out = Vec::new();
    for i in 0..count {
        let index = store.get_u32_raw(claim, BUCKET0 + i * sb);
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

/// `@FR-Col-Unordered` (@C151) — every live entry in SLOT order: ascending by the number its bucket
/// holds, the arena index the entry was handed out at (a record number, `SLOT_RECORD`, for an
/// entry the table borrows; those follow the arena's, in record order).
///
/// The order an unordered walk promises is a FIXED one, so it is read from the slot numbers
/// and not from the buckets: a bucket's position follows the key's hash under the table's
/// seed, which `keys::fresh_seed` draws per hash and per run, while a slot number follows
/// only the inserts and removes that made the table (growth moves no entry, and a copy
/// re-files its entries in the source's order, `copy_claims_hash_body`).  So two runs of one
/// program walk alike, on both backends.  The sort is of plain `u32`s, where the key-order
/// walk ([`records_sorted`]) compares the key fields read out of every entry.
#[must_use]
pub fn records_by_slot(hash_ref: &DbRef, stores: &[Store]) -> Vec<DbRef> {
    let store = keys::store(hash_ref, stores);
    let claim = store.collection_rec(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return Vec::new();
    }
    if is_inline(store, claim) {
        return inline_entries(store, hash_ref.store_nr, claim);
    }
    let count = elms(store, claim);
    let sb = slot_bytes(store, claim);
    let width = stride(store, claim);
    let mut slots: Vec<u32> = (0..count)
        .map(|i| store.get_u32_raw(claim, BUCKET0 + i * sb))
        .filter(|&index| index != 0)
        .collect();
    slots.sort_unstable();
    slots
        .into_iter()
        .map(|index| entry_ref(store, claim, index, hash_ref.store_nr, width))
        .collect()
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
    let claim = store.collection_rec(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return Vec::new();
    }
    if is_inline(store, claim) {
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
    if keys::fast_order_enabled() {
        let sorted = match keys {
            [_] => inline_sorted::<1>(hash_ref, stores, keys),
            [_, _] => inline_sorted::<2>(hash_ref, stores, keys),
            [_, _, _] => inline_sorted::<3>(hash_ref, stores, keys),
            [_, _, _, _] => inline_sorted::<4>(hash_ref, stores, keys),
            _ => None,
        };
        if let Some(sorted) = sorted {
            return sorted;
        }
    }
    let mut recs = records_unordered(hash_ref, stores);
    keys::sort_records(&mut recs, stores, keys);
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
    let claim = keys::store(hash_ref, stores).collection_rec(hash_ref.rec, hash_ref.pos);
    let length = live_len(keys::store(hash_ref, stores), claim);
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

// ---- `(Col-Inline)` — the operations of a table that keeps its entries in its record ----

/// The factor an inline table grows by when it is due: twice its buckets, as an arena table
/// grows (`grow_if_full`).
const GROWTH: u32 = 2;

/// Zero the `count` slots of table `claim`: an empty slot is a zero first word, and nothing
/// else in the record is read before it is written.
fn zero_slots(store: &mut Store, claim: u32, count: u32) {
    unsafe {
        store
            .base_ptr()
            .add(claim as usize * 8 + BUCKET0 as usize)
            .write_bytes(0, count as usize * 8);
    }
}

/// A new inline table of `count` buckets or more for `stride`-byte entries, owned by record
/// `owner`: its slots empty, its insertion count at 1, no tombstones.  The caller writes the
/// seed and the field.
fn inline_create(store: &mut Store, count: u32, stride: u32, owner: u32) -> u32 {
    let claim = store.claim(inline_words(count, stride));
    for fld in [SEED_FLD, SEED_FLD + 4, INLINE_LEN_FLD, arena::FREE_FLD] {
        store.set_u32_raw(claim, fld, 0);
    }
    store.set_u32_raw(claim, arena::OWNER_FLD, owner);
    store.set_u32_raw(claim, arena::NEXT_FLD, 1);
    let count = inline_count(store.record_words(claim) - RESERVED_WORDS, stride);
    store.set_u32_raw(claim, STRIDE_FLD, inline_flags(stride, count));
    zero_slots(store, claim, count);
    claim
}

/// The `STRIDE_FLD` word of an inline table of `count` buckets (a power of two).
fn inline_flags(stride: u32, count: u32) -> u32 {
    debug_assert!(count.is_power_of_two() && stride <= STRIDE_MASK);
    stride | TAGGED | INLINE | (count.trailing_zeros() << COUNT_SHIFT)
}

/// Rebuild inline table `claim` at `want` buckets or more: every live entry moved to its
/// bucket in the new record by its tag, the tombstones dropped, the seed, the insertion count
/// and the length carried; the field repointed and the old record freed.  Answers the new
/// table.  This is the one place an inline entry moves (besides a store's load), and every
/// view of an entry ends at it (`binding.md` B-Disturb, a keyed add).
fn inline_rebuild(hash: &DbRef, claim: u32, want: u32, stores: &mut [Store]) -> u32 {
    let store = keys::mut_store(hash, stores);
    // The entries move by copy, past `Store::write`: a locked store keeps its table.
    if !store.write_allowed(claim, STRIDE_FLD) {
        return claim;
    }
    let h = hdr(store, claim);
    let new_claim = store.claim(inline_words(want, h.stride));
    for fld in [
        arena::OWNER_FLD,
        SEED_FLD,
        SEED_FLD + 4,
        INLINE_LEN_FLD,
        arena::NEXT_FLD,
    ] {
        let v = store.get_u32_raw(claim, fld);
        store.set_u32_raw(new_claim, fld, v);
    }
    let new_count = inline_count(store.record_words(new_claim) - RESERVED_WORDS, h.stride);
    store.set_u32_raw(new_claim, STRIDE_FLD, inline_flags(h.stride, new_count));
    store.set_u32_raw(new_claim, arena::FREE_FLD, 0);
    let nh = hdr(store, new_claim);
    zero_slots(store, new_claim, nh.count);
    let mask = nh.count - 1;
    let (old0, new0) = (ent0(h.count), ent0(nh.count));
    let stride = h.stride as usize;
    let base = store.base_ptr();
    // Every access through it is `read_unaligned` / `write_unaligned`.
    #[allow(clippy::cast_ptr_alignment)]
    let slot = |rec: u32, at: u32| unsafe {
        base.add(rec as usize * 8 + (BUCKET0 + at * 8) as usize)
            .cast::<u32>()
    };
    for i in 0..h.count {
        let seq = unsafe { slot(claim, i).read_unaligned() };
        if seq == 0 || seq == TOMB {
            continue;
        }
        let tag = unsafe { slot(claim, i).add(1).read_unaligned() };
        let mut at = tag & mask;
        while unsafe { slot(new_claim, at).read_unaligned() } != 0 {
            at = (at + 1) & mask;
        }
        unsafe {
            slot(new_claim, at).write_unaligned(seq);
            slot(new_claim, at).add(1).write_unaligned(tag);
            std::ptr::copy_nonoverlapping(
                base.add(claim as usize * 8 + (old0 + i * h.stride) as usize),
                base.add(new_claim as usize * 8 + (new0 + at * h.stride) as usize),
                stride,
            );
        }
    }
    install_table(hash, claim, new_claim, stores);
    new_claim
}

/// The empty bucket an insert of `tag` takes in inline table `claim`, the table grown first
/// when it is due: `(table, bucket)`.  The walk for a hit came before; this one ends at the
/// first EMPTY slot (a tombstone is not reused).
fn inline_slot_for(hash: &DbRef, claim: u32, tag: u32, stores: &mut [Store]) -> (u32, u32) {
    let store = keys::store(hash, stores);
    let h = hdr(store, claim);
    let used = store.get_u32_raw(claim, INLINE_LEN_FLD) + store.get_u32_raw(claim, arena::FREE_FLD);
    let claim = if is_full(used, h.count) {
        inline_rebuild(hash, claim, h.count * GROWTH, stores)
    } else {
        claim
    };
    let store = keys::store(hash, stores);
    let mask = hdr(store, claim).count - 1;
    let mut at = tag & mask;
    while store.get_u32_raw(claim, BUCKET0 + at * 8) != 0 {
        at = (at + 1) & mask;
    }
    (claim, at)
}

/// File a new entry at empty bucket `at` of inline table `claim`: its insertion number and
/// tag, the length, and — with `zero` — its bytes zeroed, as a fresh arena slot is.  Answers
/// the entry.
fn inline_file(
    store: &mut Store,
    store_nr: u16,
    claim: u32,
    at: u32,
    tag: u32,
    zero: bool,
) -> DbRef {
    let h = hdr(store, claim);
    inline_file_at(store, store_nr, claim, h.count, h.stride, at, tag, zero)
}

/// [`inline_file`] with the table's count and stride already read.
#[allow(clippy::too_many_arguments)]
#[inline]
fn inline_file_at(
    store: &mut Store,
    store_nr: u16,
    claim: u32,
    count: u32,
    stride: u32,
    at: u32,
    tag: u32,
    zero: bool,
) -> DbRef {
    let h = Hdr {
        count,
        sb: 8,
        stride,
        inline: true,
    };
    let seq = store.get_u32_raw(claim, arena::NEXT_FLD).max(1);
    store.set_u32_raw(claim, arena::NEXT_FLD, seq + 1);
    put_slot(store, claim, 8, at, seq, tag);
    let len = store.get_u32_raw(claim, INLINE_LEN_FLD);
    store.set_u32_raw(claim, INLINE_LEN_FLD, len + 1);
    let pos = ent0(h.count) + at * h.stride;
    if zero {
        unsafe {
            store
                .base_ptr()
                .add(claim as usize * 8 + pos as usize)
                .write_bytes(0, h.stride as usize);
        }
    }
    DbRef {
        store_nr,
        rec: claim,
        pos,
    }
}

/// [`add`] on an inline table: `rec`'s bytes — a record staged by [`alloc_entry`] — copied
/// into the bucket its key takes, and the stage freed.  Answers the filed entry.
///
/// A record the table did not stage belongs to whoever built it (a sibling view's record, as
/// [`SLOT_RECORD`] describes), and an inline table cannot hold a reference: the table becomes
/// an arena table first ([`inline_demote`]) and files the record borrowed, as it always did.
fn inline_add(hash: &DbRef, claim: u32, rec: &DbRef, stores: &mut [Store], keys: &[Key]) -> DbRef {
    // `@FR-H-WriteLocked` — the entry's bytes move by copy, past `Store::write`, so the lock is
    // asked here: refused, nothing is filed and the stage stays what the caller holds.
    if !keys::store(hash, stores).write_allowed(claim, INLINE_LEN_FLD) {
        return *rec;
    }
    let own_entry = rec.store_nr == hash.store_nr && rec.rec == claim;
    let staged = rec.store_nr == hash.store_nr
        && rec.pos == 8
        && keys::store(rec, stores).get_u32_raw(rec.rec, 4) == STAGE_MARK;
    if !own_entry && !staged {
        inline_demote(hash, claim, stores, keys);
        return add(hash, rec, stores, keys);
    }
    let seed = read_seed(keys::store(hash, stores), claim);
    let tag = tag_of(keys::hash(rec, stores, keys, seed));
    let (claim, at) = inline_slot_for(hash, claim, tag, stores);
    let placed = inline_file(
        keys::mut_store(hash, stores),
        hash.store_nr,
        claim,
        at,
        tag,
        false,
    );
    let stride = hdr(keys::store(hash, stores), claim).stride;
    if rec.store_nr == hash.store_nr {
        let store = keys::mut_store(hash, stores);
        let base = store.base_ptr();
        unsafe {
            std::ptr::copy(
                base.add(rec.rec as usize * 8 + rec.pos as usize),
                base.add(placed.rec as usize * 8 + placed.pos as usize),
                stride as usize,
            );
        }
    } else {
        let src = keys::store(rec, stores).base_ptr();
        let dst = keys::store(hash, stores).base_ptr();
        unsafe {
            std::ptr::copy_nonoverlapping(
                src.add(rec.rec as usize * 8 + rec.pos as usize),
                dst.add(placed.rec as usize * 8 + placed.pos as usize),
                stride as usize,
            );
        }
    }
    if !own_entry {
        keys::mut_store(rec, stores).delete(rec.rec);
    }
    placed
}

/// Turn inline table `claim` into an arena table: the seed carried, every live entry copied
/// into an arena slot of its own and filed, the field repointed and the inline record freed.
/// Every view of an entry ends here, as at any keyed add (`binding.md` B-Disturb).
fn inline_demote(hash: &DbRef, claim: u32, stores: &mut [Store], keys: &[Key]) {
    let store = keys::mut_store(hash, stores);
    let h = hdr(store, claim);
    let (owner, seed) = (
        store.get_u32_raw(claim, arena::OWNER_FLD),
        read_seed(store, claim),
    );
    let table = arena_table(store, new_slot_bytes(), h.stride);
    write_seed(store, table, seed);
    store.set_u32_raw(hash.rec, hash.pos, table);
    let (was, now) = (
        store.get_u32_raw(claim, STRIDE_FLD),
        store.get_u32_raw(table, STRIDE_FLD),
    );
    trace_layout("demoted", hash, Some(was), now);
    let e0 = ent0(h.count);
    let mut bytes = vec![0u8; h.stride as usize];
    for at in 0..h.count {
        let store = keys::mut_store(hash, stores);
        let seq = store.get_u32_raw(claim, BUCKET0 + at * 8);
        if seq == 0 || seq == TOMB {
            continue;
        }
        let from = claim as usize * 8 + (e0 + at * h.stride) as usize;
        unsafe {
            std::ptr::copy_nonoverlapping(
                store.base_ptr().add(from),
                bytes.as_mut_ptr(),
                bytes.len(),
            )
        };
        let table = store.get_u32_raw(hash.rec, hash.pos);
        let (_, rec, pos) = arena::alloc_at(store, table, h.stride, owner);
        let to = rec as usize * 8 + pos as usize;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), store.base_ptr().add(to), bytes.len())
        };
        add(
            hash,
            &DbRef {
                store_nr: hash.store_nr,
                rec,
                pos,
            },
            stores,
            keys,
        );
    }
    keys::mut_store(hash, stores).delete(claim);
}

/// [`remove`] on an inline table: the entry at `rec` leaves a tombstone, so no other entry
/// moves.  Answers its insertion number, 0 when the table did not hold it.
fn inline_remove(hash_ref: &DbRef, claim: u32, rec: &DbRef, stores: &mut [Store]) -> u32 {
    let store = keys::mut_store(hash_ref, stores);
    if !store.write_allowed(claim, INLINE_LEN_FLD) {
        return 0;
    }
    let h = hdr(store, claim);
    let base = ent0(h.count);
    if rec.rec != claim || rec.pos < base || h.stride == 0 {
        return 0;
    }
    let at = (rec.pos - base) / h.stride;
    if at >= h.count {
        return 0;
    }
    let seq = store.get_u32_raw(claim, BUCKET0 + at * 8);
    if seq == 0 || seq == TOMB {
        return 0;
    }
    store.set_u32_raw(claim, BUCKET0 + at * 8, TOMB);
    let len = store.get_u32_raw(claim, INLINE_LEN_FLD);
    store.set_u32_raw(claim, INLINE_LEN_FLD, len.saturating_sub(1));
    let tombs = store.get_u32_raw(claim, arena::FREE_FLD);
    store.set_u32_raw(claim, arena::FREE_FLD, tombs + 1);
    seq
}

/// [`records_sorted`] for an INLINE table whose key is one to four integer parts: one sweep of
/// the slots reads each live entry's key parts (the bytes [`keys::int_key_part`] reads) into
/// `([i64; N], position)`, and the pairs are sorted by key — no list of records first, no
/// index to gather back through.  With the inlined ordered step, `hash_walk` 355 → 147 µs
/// (`bench/portal/analysis/keyed.md`).  A hash's keys are unique, so the key alone orders the
/// pairs.  `None` — the general path answers — for an arena table, a text or float part, or a
/// table whose record does not hold what its header names.
///
/// # Panics
/// Under `LOFT_KEYED_VERIFY=1`, when the order disagrees with [`keys::compare`].
fn inline_sorted<const N: usize>(
    hash_ref: &DbRef,
    stores: &[Store],
    keys: &[Key],
) -> Option<Vec<DbRef>> {
    if !keys
        .iter()
        .all(|k| matches!(k.type_nr.abs(), 1 | 2 | 8 | 9 | 10 | 11 | 12))
    {
        return None;
    }
    let store = keys::store(hash_ref, stores);
    let claim = store.collection_rec(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return None;
    }
    let (base, count, stride) = inline_span(store, claim)?;
    let e0 = ent0(count);
    let at = |pos| DbRef {
        store_nr: hash_ref.store_nr,
        rec: claim,
        pos,
    };
    // A part of 4 or 8 bytes is read raw through the checked base once its field is shown to
    // lie inside the entry — the bytes `int_key_part` reads, widened the same way; a narrow
    // part (a `start`-offset short or byte) and a descending one take that checked read.
    let raw: [Option<(u32, u8)>; N] = std::array::from_fn(|i| {
        let k = &keys[i];
        let (width, form) = match k.type_nr {
            1 | 2 => (8, 0),
            8 => (4, 1),
            12 => (4, 2),
            _ => return None,
        };
        let off = u32::from(k.position);
        (off + width <= stride).then_some((off, form))
    });
    // Branch-free: every slot's pair is written at the next free place and only a live slot
    // moves that place on — whether a slot is live follows the key's hash, so a branch on it
    // mispredicted on most slots.  A dead slot's entry bytes lie inside the checked record.
    let mut pairs: Vec<([i64; N], u32)> = Vec::with_capacity(count as usize);
    let spare = pairs.spare_capacity_mut();
    let mut n = 0;
    for slot in 0..count {
        let seq = raw_u32(base, BUCKET0 + slot * 8);
        let pos = e0 + slot * stride;
        let mut key = [0_i64; N];
        for ((part, k), r) in key.iter_mut().zip(keys).zip(&raw) {
            *part = match *r {
                Some((off, 0)) => raw_i64(base, pos + off),
                Some((off, 1)) => i64::from(raw_u32(base, pos + off).cast_signed()),
                Some((off, _)) => i64::from(raw_u32(base, pos + off)),
                None => keys::int_key_part(store, &at(pos), k)?,
            };
        }
        spare[n].write((key, pos));
        n += usize::from(seq != 0 && seq != TOMB);
    }
    // SAFETY: the first `n` places were each written above (`n` never passes `slot`).
    unsafe { pairs.set_len(n) };
    // The key alone: a hash's keys are unique, so the pairs never tie, and the comparison of
    // the key without the position was a third cheaper on a 5,000-entry sort.
    if N == 1 {
        pairs.sort_unstable_by_key(|p| p.0[0]);
    } else {
        pairs.sort_unstable_by_key(|p| p.0);
    }
    let sorted: Vec<DbRef> = pairs.into_iter().map(|(_, pos)| at(pos)).collect();
    if keys::keyed_verify() {
        for w in sorted.windows(2) {
            assert_ne!(
                keys::compare(&w[0], &w[1], stores, keys),
                std::cmp::Ordering::Greater,
                "LOFT_KEYED_VERIFY: the inline key-order walk put {:?} before {:?}, which keys::compare orders after it",
                w[0],
                w[1]
            );
        }
    }
    Some(sorted)
}

/// Every live record in no promised order: the input of a sort that imposes its own.  An inline
/// table answers its slots as they lie, where [`records`] first sorts them into insertion
/// order — a second O(n log n) sort a key-order walk throws away, since a hash's keys are
/// unique (18 % of `hash_walk`, `bench/portal/analysis/keyed.md`).
fn records_unordered(hash_ref: &DbRef, stores: &[Store]) -> Vec<DbRef> {
    let store = keys::store(hash_ref, stores);
    let claim = store.collection_rec(hash_ref.rec, hash_ref.pos);
    if claim != 0 && is_inline(store, claim) {
        return inline_slots(store, hash_ref.store_nr, claim)
            .into_iter()
            .map(|(_, at)| at)
            .collect();
    }
    records(hash_ref, stores)
}

/// The live entries of inline table `claim` in slot order, each with its insertion number.
/// The buckets are read through the base [`inline_span`] checked once — a checked read per
/// bucket was a third of a key-order walk — into a vector sized by the live count.
fn inline_slots(store: &Store, store_nr: u16, claim: u32) -> Vec<(u32, DbRef)> {
    let Some((base, count, stride)) = inline_span(store, claim) else {
        return Vec::new();
    };
    let e0 = ent0(count);
    let mut out = Vec::with_capacity(live_len(store, claim) as usize);
    for at in 0..count {
        let seq = raw_u32(base, BUCKET0 + at * 8);
        if seq != 0 && seq != TOMB {
            out.push((
                seq,
                DbRef {
                    store_nr,
                    rec: claim,
                    pos: e0 + at * stride,
                },
            ));
        }
    }
    out
}

/// The live entries of inline table `claim`, in insertion order (the order an unordered walk
/// promises, `@FR-Col-Unordered`).
fn inline_entries(store: &Store, store_nr: u16, claim: u32) -> Vec<DbRef> {
    let mut live = inline_slots(store, store_nr, claim);
    live.sort_unstable_by_key(|&(seq, _)| seq);
    live.into_iter().map(|(_, at)| at).collect()
}

// ---- `(Col-Inline)` — the one-integer-key paths over a table checked once ----

#[inline(always)]
#[allow(clippy::inline_always)]
fn raw_u32(base: *const u8, off: u32) -> u32 {
    unsafe { base.add(off as usize).cast::<u32>().read_unaligned() }
}

#[inline(always)]
#[allow(clippy::inline_always)]
fn raw_i64(base: *const u8, off: u32) -> i64 {
    unsafe { base.add(off as usize).cast::<i64>().read_unaligned() }
}

/// The base of inline table `claim`, its bucket count and stride, when the table is inline and
/// its record holds what its header says — the one bounds test every access below stands on:
/// each offset is a bucket below `count` or an entry field below `stride`, inside the record.
#[inline(always)]
#[allow(clippy::inline_always)]
fn inline_checked(store: &Store, claim: u32, kp: u32) -> Option<(*const u8, u32, u32)> {
    let (base, count, stride) = inline_span(store, claim)?;
    if kp + 8 > stride {
        return None;
    }
    Some((base, count, stride))
}

/// [`inline_checked`] without a key: the base, bucket count and stride of an inline table whose
/// record holds every bucket and entry its header names.
#[inline(always)]
#[allow(clippy::inline_always)]
fn inline_span(store: &Store, claim: u32) -> Option<(*const u8, u32, u32)> {
    let words = store.record_words(claim);
    if words <= RESERVED_WORDS
        || u64::from(claim) + u64::from(words) > u64::from(store.capacity_words())
    {
        return None;
    }
    let base = unsafe { store.base_ptr().add(claim as usize * 8) }.cast_const();
    let flags = raw_u32(base, STRIDE_FLD);
    if flags & INLINE == 0 {
        return None;
    }
    let (stride, count) = (stride_of(flags), inline_count_of(flags));
    if stride == 0 || inline_words(count, stride) > words {
        return None;
    }
    Some((base, count, stride))
}

/// [`entry_one_int`] on an inline table whose key is the 8-byte integer at `kp`: `None` where
/// the general path must answer (not inline, or a miss that grows the table first), else its
/// answer.  The walk is inlined at the caller; filing a miss is [`inline_insert_int`].
///
/// Inlined ALWAYS, not for the call it saves: a call takes `hash` by address and answers its
/// `Option<DbRef>` through memory, written a field at a time and read back eight bytes at a
/// time — a store the processor cannot forward to that load, and the stall was a sixth of
/// `field_add_cell`'s cycles.  Inlined, the reference never leaves the registers.
#[inline(always)]
#[allow(clippy::inline_always)]
pub fn inline_entry_int(
    hash: &DbRef,
    claim: u32,
    kp: u32,
    value: i64,
    stores: &mut [Store],
) -> Option<DbRef> {
    let store = keys::store(hash, stores);
    let (base, count, stride) = inline_checked(store, claim, kp)?;
    let seed = u64::from(raw_u32(base, SEED_FLD)) | (u64::from(raw_u32(base, SEED_FLD + 4)) << 32);
    // `long_hash(v)` is `ints_hash(&[v])` by construction: one seeded word, one key word.
    let tag = tag_of(keys::long_hash(value, seed));
    let (mask, e0) = (count - 1, ent0(count));
    let mut at = tag & mask;
    for _ in 0..count {
        let seq = raw_u32(base, BUCKET0 + at * 8);
        if seq == 0 {
            break;
        }
        if seq != TOMB
            && raw_u32(base, BUCKET0 + at * 8 + 4) == tag
            && raw_i64(base, e0 + at * stride + kp) == value
        {
            return Some(DbRef {
                store_nr: hash.store_nr,
                rec: claim,
                pos: e0 + at * stride,
            });
        }
        at = (at + 1) & mask;
    }
    let pos = inline_insert_int(hash, claim, count, stride, at, tag, kp, value, stores);
    (pos != 0).then_some(DbRef {
        store_nr: hash.store_nr,
        rec: claim,
        pos,
    })
}

/// File `value` at bucket `at`, the empty bucket [`inline_entry_int`]'s walk ended on, and
/// answer the entry's position — `0` when the walk ended without one or the table is due to
/// grow: the general path files it.  A position and not a reference, so the answer comes back
/// in a register (see [`inline_entry_int`] on what a reference through memory costs).
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn inline_insert_int(
    hash: &DbRef,
    claim: u32,
    count: u32,
    stride: u32,
    at: u32,
    tag: u32,
    kp: u32,
    value: i64,
    stores: &mut [Store],
) -> u32 {
    if keys::store(hash, stores).read_only {
        return 0;
    }
    let base = unsafe { keys::store(hash, stores).base_ptr().add(claim as usize * 8) }.cast_const();
    let e0 = ent0(count);
    let len = raw_u32(base, INLINE_LEN_FLD);
    if raw_u32(base, BUCKET0 + at * 8) != 0 || is_full(len + raw_u32(base, arena::FREE_FLD), count)
    {
        return 0;
    }
    let store = keys::mut_store(hash, stores);
    let base = unsafe { store.base_ptr().add(claim as usize * 8) };
    let seq = raw_u32(base, arena::NEXT_FLD).max(1);
    let pos = e0 + at * stride;
    unsafe {
        base.add(arena::NEXT_FLD as usize)
            .cast::<u32>()
            .write_unaligned(seq + 1);
        base.add((BUCKET0 + at * 8) as usize)
            .cast::<u32>()
            .write_unaligned(seq);
        base.add((BUCKET0 + at * 8 + 4) as usize)
            .cast::<u32>()
            .write_unaligned(tag);
        base.add(INLINE_LEN_FLD as usize)
            .cast::<u32>()
            .write_unaligned(len + 1);
        base.add(pos as usize).write_bytes(0, stride as usize);
        base.add((pos + kp) as usize)
            .cast::<i64>()
            .write_unaligned(value);
    }
    pos
}

/// [`find_long`] on an inline table with an 8-byte integer key: `None` where the general walk
/// must answer, else the entry (record 0 for a miss).  Inlined always, as [`inline_entry_int`].
#[inline(always)]
#[allow(clippy::inline_always)]
pub fn inline_find_int(
    store: &Store,
    store_nr: u16,
    claim: u32,
    kp: u32,
    value: i64,
) -> Option<DbRef> {
    let (base, count, stride) = inline_checked(store, claim, kp)?;
    let seed = u64::from(raw_u32(base, SEED_FLD)) | (u64::from(raw_u32(base, SEED_FLD + 4)) << 32);
    let tag = tag_of(keys::long_hash(value, seed));
    let (mask, e0) = (count - 1, ent0(count));
    let mut at = tag & mask;
    for _ in 0..count {
        let seq = raw_u32(base, BUCKET0 + at * 8);
        if seq == 0 {
            break;
        }
        if seq != TOMB
            && raw_u32(base, BUCKET0 + at * 8 + 4) == tag
            && raw_i64(base, e0 + at * stride + kp) == value
        {
            return Some(DbRef {
                store_nr,
                rec: claim,
                pos: e0 + at * stride,
            });
        }
        at = (at + 1) & mask;
    }
    Some(DbRef {
        store_nr,
        rec: 0,
        pos: 0,
    })
}

/// `(Col-Inline)` — the byte spans of inline table `claim` that hold data, as `(offset, len)`
/// into the store's buffer: the header and the slots, then each occupied bucket's entry.  The
/// entry bytes of an empty or removed bucket are not data — nothing reads them before an
/// insert writes them — so a digest of what a store HOLDS leaves them out
/// (`Stores::live_digest`).
pub fn inline_live_spans(store: &Store, claim: u32, mut span: impl FnMut(usize, usize)) {
    let h = hdr(store, claim);
    let base = claim as usize * 8;
    span(base, ent0(h.count) as usize);
    let e0 = ent0(h.count);
    for at in 0..h.count {
        let seq = store.get_u32_raw(claim, BUCKET0 + at * 8);
        if seq != 0 && seq != TOMB {
            span(base + (e0 + at * h.stride) as usize, h.stride as usize);
        }
    }
}

/// `(Col-Inline)` — the words inline table `claim` holds past what a table built fresh for its
/// live entries takes: the buckets it grew to at its peak and the tombstones of what left.  A
/// rebuild returns them, as it returns a vector's capacity past its length, so the compaction
/// gate counts them as slack (`Stores::intra_record_slack`).  0 for a table that is not inline.
#[must_use]
pub fn inline_slack(store: &Store, claim: u32) -> u32 {
    if claim == 0 || !is_inline(store, claim) {
        return 0;
    }
    let h = hdr(store, claim);
    let live = store.get_u32_raw(claim, INLINE_LEN_FLD);
    let mut need = 16;
    while is_full(live, need) {
        need *= GROWTH;
    }
    store
        .record_words(claim)
        .saturating_sub(inline_words(need, h.stride))
}
