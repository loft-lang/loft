// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F6 — vector<T> (append / index / slice / comprehensions / aggregates)

use crate::keys;
use crate::keys::{Content, DbRef, Key};
use crate::store::Store;
use std::cmp::Ordering;

/// Checked vector position — `8 + index * size` using u64 to detect overflow.
#[inline]
fn checked_vec_pos(index: u32, size: u32) -> u32 {
    let pos = u64::from(index) * u64::from(size) + 8;
    u32::try_from(pos)
        .unwrap_or_else(|_| panic!("Vector position overflow: index={index} size={size}"))
}

/// Checked vector capacity — `(count * size + 15) / 8` using u64.
#[inline]
fn checked_vec_cap(count: u32, size: u32) -> u32 {
    let bytes = u64::from(count) * u64::from(size) + 15;
    u32::try_from(bytes / 8)
        .unwrap_or_else(|_| panic!("Vector capacity overflow: count={count} size={size}"))
}

// TODO change slice to its own vector on updating it
pub fn insert_vector(db: &DbRef, size: u32, index: i64, stores: &mut [Store]) -> DbRef {
    if db.is_null() {
        return DbRef::NULL; // cannot insert into a null (absent) vector
    }
    let len = length_vector(db, stores);
    let real = if index < 0 {
        index + i64::from(len)
    } else {
        index
    };
    if real < 0 || real > i64::from(len) {
        return DbRef {
            store_nr: db.store_nr,
            rec: 0,
            pos: 0,
        };
    }
    let real = real as i32;
    let store = keys::mut_store(db, stores);
    let mut vec_rec = store.collection_rec(db.rec, db.pos);
    let new_length;
    if vec_rec == 0 {
        // claim a new array with minimal 11 elements
        vec_rec = store.claim(checked_vec_cap(11, size));
        store.set_u32_raw(db.rec, db.pos, vec_rec);
        new_length = 1;
    } else {
        new_length = len + 1;
        let new_vec = store.resize(vec_rec, checked_vec_cap(new_length, size));
        if new_vec != vec_rec {
            store.set_u32_raw(db.rec, db.pos, new_vec);
            vec_rec = new_vec;
        }
        store.copy_block(
            new_vec,
            checked_vec_pos(real as u32, size) as isize,
            new_vec,
            checked_vec_pos(real as u32 + 1, size) as isize,
            (len as isize - real as isize) * size as isize,
        );
    }
    store.set_u32_raw(vec_rec, 4, new_length);
    DbRef {
        store_nr: db.store_nr,
        rec: vec_rec,
        pos: checked_vec_pos(real as u32, size),
    }
}

/**
Claim more space in a vector to allow for new records. Return the next reference after the last
records though do not increase the length yet as we might want to iterate the vector before the
actual change.
*/
/// @P321(c): bulk-allocate a vector record and copy raw bytes into it
/// in one shot.  Returns the new record number; the caller stores it into the
/// owning struct field via `Store::set_u32_raw`.
///
/// `data.len()` must equal `count * elem_size`.  Mirrors
/// `loft_ffi::LoftStore::alloc_vector_from_bytes` so per-library wasm
/// bridge crates (e.g. `lib/imaging/wasm/src/lib.rs`) and the cdylib in
/// `lib/imaging/native/` share the same vector layout (length at offset
/// 4, payload at offset 8).
///
/// Pass `&[]` to allocate without filling — useful when the caller wants
/// a pre-sized buffer it (or a host bridge) will fill in place.
///
/// `#[allow(dead_code)]` because the consumers (per-library wasm
/// bridge crates, e.g. `loft-imaging-wasm` at `lib/imaging/wasm/`)
/// are wasm32-only; native builds compile this helper but never
/// call it.
#[allow(dead_code)]
pub fn alloc_vector_from_bytes(store: &mut Store, elem_size: u32, count: u32, data: &[u8]) -> u32 {
    let words = checked_vec_cap(count.max(11), elem_size);
    let rec = store.claim(words);
    store.set_u32_raw(rec, 4, count);
    if !data.is_empty() {
        store.buffer(rec)[..data.len()].copy_from_slice(data);
    }
    rec
}

/// O8.1a: Pre-allocate a vector record with capacity for `count` elements.
/// Sets the vector pointer and length=0.  Subsequent `vector_append` calls
/// will find enough space and never call `store.resize`.
pub fn pre_alloc_vector(db: &DbRef, count: u32, elem_size: u32, stores: &mut [Store]) {
    // #618: the null test comes FIRST, before any store deref — the same order
    // `clear_vector` uses, and the rule `DbRef::is_null` states ("every store
    // accessor consults it before dereferencing").  A hidden return buffer that
    // its caller never allocated arrives as the null sentinel, and reserving
    // capacity in an absent vector is a no-op; testing `rec` after the
    // `mut_store` indexed `stores[u16::MAX]` and aborted.
    if db.is_null() || db.rec == 0 {
        return;
    }
    let store = keys::mut_store(db, stores);
    let vec_rec = store.collection_rec(db.rec, db.pos);
    if vec_rec != 0 {
        return; // already allocated — don't overwrite
    }
    // Match vector_append's minimum of 11 elements to avoid OOB on remove/shift.
    let alloc_count = count.max(11);
    let words = checked_vec_cap(alloc_count, elem_size);
    let new_rec = store.claim(words);
    store.set_u32_raw(db.rec, db.pos, new_rec);
    store.set_u32_raw(new_rec, 4, 0); // length = 0
}

/// Give `db`'s vector room for `count` elements, so appending that many performs
/// no reallocation.  Backs the `reserve(v, n)` builtin (loft#710).
///
/// Changes CAPACITY only: length, contents, and the owner slot stay valid, and a
/// `count` at or below what the vector already holds room for is a no-op — never
/// a truncation.  The one visible effect is that the growth ladder does not run.
///
/// That ladder is why this exists.  `vector_append` doubles when it runs out, so
/// filling a vector of N costs ~log N reallocations, each claiming a new block
/// and orphaning the old one, and leaves the final block up to 2x the length.
/// With N vectors growing round-robin — what a streaming generator does, one
/// append per incoming item to whichever collection owns it — the block after
/// any vector is another vector, so the in-place path in `Store::resize` almost
/// never applies and every step copies.  A persisted store then carries all of
/// it: 3.28 MB claimed for 2.31 MB of data in the reported case.
///
/// Distinct from [`pre_alloc_vector`], which only ever allocates a vector that
/// has none yet (its callers rely on it leaving an existing one alone); this one
/// also grows an existing vector, which is the case a caller reserving up front
/// cares about.
pub fn reserve_vector(db: &DbRef, count: i64, elem_size: u32, stores: &mut [Store]) {
    // Null test first, before any store deref — see `pre_alloc_vector`.
    if db.is_null() || db.rec == 0 || db.pos == 0 {
        return;
    }
    let Ok(count) = u32::try_from(count) else {
        return; // negative or > u32: nothing to promise
    };
    let store = keys::mut_store(db, stores);
    // Same 11-element floor as `vector_append` / `pre_alloc_vector`, so a
    // reserved vector can never be shorter-capacity than an unreserved one.
    let words = checked_vec_cap(count.max(11), elem_size);
    let vec_rec = store.collection_rec(db.rec, db.pos);
    if vec_rec == 0 {
        let new_rec = store.claim(words);
        store.set_u32_raw(db.rec, db.pos, new_rec);
        store.set_u32_raw(new_rec, 4, 0); // length = 0
        return;
    }
    // `resize` returns early when the block is already big enough, so shrinking
    // is impossible here; when it cannot grow in place it moves the record, and
    // the owner slot has to follow — exactly as `vector_append` does.
    let new_rec = store.resize(vec_rec, words);
    if new_rec != vec_rec {
        store.set_u32_raw(db.rec, db.pos, new_rec);
    }
}

/// Make room for one more element at the end of the vector `db` points at, and
/// answer where to write it.  Grows the backing record ~2x when it is full, and
/// follows the record if the grow had to move it.
///
/// # Panics
///
/// If the owner slot holds a non-zero handle whose target has a non-positive size
/// word — a record that has been freed, or was never claimed.  Reading a capacity
/// from that word would wrap, and the append would then copy an unbounded length
/// (loft#810); the message names the owner slot and the target record.
pub fn vector_append(db: &DbRef, size: u32, stores: &mut [Store]) -> DbRef {
    // Appending to a null (absent) vector is a no-op for now — it must never
    // index stores[u16::MAX].  Plan-25 Q4: make this a loud error once null is
    // a first-class vector value (P2/P3); P1 only guarantees no OOB.
    if db.is_null() {
        return DbRef::NULL;
    }
    let store = keys::mut_store(db, stores);
    if db.rec == 0 {
        return DbRef {
            store_nr: db.store_nr,
            rec: 0,
            pos: 0,
        };
    }
    let mut vec_rec = store.collection_rec(db.rec, db.pos);
    let pos = if vec_rec == 0 {
        // new array
        vec_rec = store.claim(checked_vec_cap(11, size)); // minimal 11 elements
        store.set_u32_raw(db.rec, db.pos, vec_rec);
        store.set_u32_raw(vec_rec, 4, 0); // initial length
        0
    } else {
        let length = store.get_u32_raw(vec_rec, 4);
        let needed = length + 1;
        // P5 — amortised (~2x) growth.  Current capacity in elements:
        // claimed words -> bytes -> minus the 8-byte header (claim:i32 +
        // length:u32) -> / element size.  When out of room grow to ~2x so
        // appends are O(1) amortised and the store doesn't fragment into
        // O(N) freed records; when room remains, request <= the current
        // claim so `store.resize` (grow-only) is a no-op.  Length lives in a
        // separate field (word 1), so the trailing slack never affects
        // `len()`, indexing, copy (length-based, shrinks to fit) or
        // serialisation.
        // The claim header (positive i32) is read via `addr`, NOT the
        // `get_u32_raw` field accessor — `valid()` forbids field offset 0
        // (header), so the accessor panics in debug builds.  `claim`/`resize`
        // read the header the same way.
        // Two ways the handle just read can be meaningless, and they need separate
        // messages because they send you to opposite halves of the runtime.
        //
        // The handle came from `db.rec`.`db.pos`, so FIRST ask whether that field is
        // inside the owner record at all.  `Store::valid` bounds a field with
        // `fld <= size * 8`, which admits a read starting exactly AT the record's end
        // — off by the width of the field being read — and it is a `debug_assert!`,
        // so it is compiled out of the test profile the loft library builds under.
        // An owner whose claimed size does not reach `pos` therefore hands back
        // whatever bytes follow it in the arena, and those bytes then read as a
        // vector handle.
        //
        // What that means is a record OF ANOTHER TYPE sitting where this one should
        // be, and the way it gets there is TWO OWNERS FOR ONE STORE SLOT: a store
        // freed while another variable still named it, recycled to somebody else, and
        // then re-`claim`ed at the original name — which is exactly loft#810 (a
        // 2-word owner under a `pos` of 16).  So the question to take upstairs is who
        // freed a store that was still named, not who computed the offset: the offset
        // is right for the type the caller thinks it is holding.  `LOFT_NO_SLOT_REUSE=1`
        // settles it in one run — if the fault vanishes, the slot had two owners.
        let owner_words = *store.addr::<i32>(db.rec, 0);
        assert!(
            owner_words >= 1 && u64::from(db.pos) + 4 <= owner_words as u64 * 8,
            "vector_append: in store {}, field {}.{} lies outside its own record, which \
             claims {owner_words} words ({} bytes) — so the vector handle read there is \
             whatever follows the record in the arena, not a vector.  The record in this \
             slot is not the one the field offset was computed for: re-run with \
             LOFT_NO_SLOT_REUSE=1, and if that clears it the store was freed while \
             another variable still named it (loft#810)",
            db.store_nr,
            db.rec,
            db.pos,
            i64::from(owner_words) * 8
        );
        // Only then: the field IS inside the record, so the handle is a real one — and
        // a non-zero handle must point at a CLAIMED record, every one of which has a
        // positive size word (`Store::claim` asserts it).  Zero or negative here means
        // the handle outlived what it pointed at.  Either way the append is about to
        // derive a capacity and a copy length from that word, and both wrap, so the
        // failure would otherwise surface far away as an unbounded `memcpy` inside
        // `resize` — naming the copy, which is innocent.
        let cur_words_signed = *store.addr::<i32>(vec_rec, 0);
        assert!(
            cur_words_signed > 0,
            "vector_append: in store {}, the vector handle in record {}.{} points at record \
             {vec_rec}, whose size word is {cur_words_signed} — the record it named has been \
             freed or was never claimed, so this vector's capacity cannot be read (loft#810)",
            db.store_nr,
            db.rec,
            db.pos
        );
        let cur_words = cur_words_signed as u32;
        let cur_cap = cur_words.saturating_mul(8).saturating_sub(8) / size;
        let target = if needed <= cur_cap {
            needed
        } else {
            needed.saturating_mul(2)
        };
        let new_vec = store.resize(vec_rec, checked_vec_cap(target, size));
        if new_vec != vec_rec {
            store.set_u32_raw(db.rec, db.pos, new_vec);
            vec_rec = new_vec;
        }
        length
    };
    DbRef {
        store_nr: db.store_nr,
        rec: vec_rec,
        pos: checked_vec_pos(pos, size),
    }
}

pub fn vector_finish(db: &DbRef, stores: &mut [Store]) {
    if db.rec == 0 {
        return;
    }
    let store = keys::mut_store(db, stores);
    let vec_rec = store.collection_rec(db.rec, db.pos);
    let length = store.get_u32_raw(vec_rec, 4);
    store.set_u32_raw(vec_rec, 4, length + 1);
}

pub fn sorted_new(db: &DbRef, size: u32, stores: &mut [Store]) -> DbRef {
    // Keep an extra record between the current and the new one.
    // This is needed to allow to create a new open space to move the new record to.
    let store = keys::mut_store(db, stores);
    let mut sorted_rec = store.collection_rec(db.rec, db.pos);
    // Claim a record at the back of the current structure or create a new structure.
    if sorted_rec == 0 {
        sorted_rec = store.claim(checked_vec_cap(12, size));
        store.set_u32_raw(db.rec, db.pos, sorted_rec);
        // Set initial length to 0
        store.set_u32_raw(sorted_rec, 4, 0);
        // return the first record
        DbRef {
            store_nr: db.store_nr,
            rec: sorted_rec,
            pos: 8,
        }
    } else {
        let length = store.get_u32_raw(sorted_rec, 4);
        let new_sorted = store.resize(sorted_rec, checked_vec_cap(length + 2, size));
        if new_sorted != sorted_rec {
            store.set_u32_raw(db.rec, db.pos, new_sorted);
            sorted_rec = new_sorted;
        }
        // The scratch slot the constructor writes into is the one `sorted_finish` /
        // `ordered_finish` will read the new record back out of, and at length 0 they
        // take the "first record needs no reordering" path and read slot 0 — not
        // `length + 1`.  Both branches only ever meet at length 0 through a
        // collection that was EMPTIED and still holds its allocation, which
        // `coll[key] = null` produces and `coll = []` does not (it drops the
        // record, so the branch above claims a fresh one).  The append then landed
        // in slot 1 while `sorted_finish` published slot 0 — the bytes of the last
        // element removed, with its text already freed, so `x.a += [E{k:9,…}]` read
        // back as the removed `2:null` and the new element was simply lost.
        let slot = if length == 0 { 0 } else { length + 1 };
        // return the last record inside the allocation
        DbRef {
            store_nr: db.store_nr,
            rec: sorted_rec,
            pos: checked_vec_pos(slot, size),
        }
    }
}

pub fn sorted_finish(sorted: &DbRef, size: u32, keys: &[Key], stores: &mut [Store]) {
    let sorted_rec = keys::store(sorted, stores).get_u32_raw(sorted.rec, sorted.pos);
    let length = keys::store(sorted, stores).get_u32_raw(sorted_rec, 4);
    if length == 0 {
        // we do not have to reorder the first inserted record; set length to 1
        keys::mut_store(sorted, stores).set_u32_raw(sorted_rec, 4, 1);
        return;
    }
    let latest_pos = checked_vec_pos(length + 1, size);
    let rec = DbRef {
        store_nr: sorted.store_nr,
        rec: sorted_rec,
        pos: latest_pos,
    };
    let key = keys::get_key(&rec, stores, keys);
    let (pos, found) = sorted_find(sorted, true, size as u16, stores, keys, &key);
    let store = keys::mut_store(sorted, stores);
    // @P306 — a record with this key already exists: replace it in place
    // (latest insert wins) and do NOT grow.  The just-appended record at
    // `latest_pos` overwrites the existing slot; length stays the same so
    // the spare end slot is discarded.  (Any nested heap in the overwritten
    // record orphans within the store — consistent with the other in-place
    // keyed replaces; reclaimed when the collection is freed.)
    if found {
        store.copy_block(
            sorted_rec,
            latest_pos as isize,
            sorted_rec,
            checked_vec_pos(pos, size) as isize,
            size as isize,
        );
        return;
    }
    let end_pos = length;
    if pos < end_pos {
        // create space to write the new record to
        store.copy_block(
            sorted_rec,
            checked_vec_pos(pos, size) as isize,
            sorted_rec,
            checked_vec_pos(pos + 1, size) as isize,
            ((end_pos - pos) * size) as isize,
        );
    }
    // move last record to the found correct position
    store.copy_block(
        sorted_rec,
        latest_pos as isize,
        sorted_rec,
        checked_vec_pos(pos, size) as isize,
        size as isize,
    );
    store.set_u32_raw(sorted_rec, 4, length + 1);
}

pub fn ordered_finish(sorted: &DbRef, rec: &DbRef, keys: &[Key], stores: &mut [Store]) {
    let rec_ref = sorted_new(sorted, 4, stores);
    let sorted_rec = keys::store(sorted, stores).get_u32_raw(sorted.rec, sorted.pos);
    let length = keys::store(sorted, stores).get_u32_raw(sorted_rec, 4);
    if length == 0 {
        // we do not have to reorder the first inserted record, set length to 1
        keys::mut_store(sorted, stores).set_u32_raw(sorted_rec, 4, 1);
        keys::mut_store(sorted, stores).set_u32_raw(sorted_rec, rec_ref.pos, rec.rec);
        return;
    }
    let key = keys::get_key(rec, stores, keys);
    let pos = ordered_find(sorted, true, stores, keys, &key).0;
    // Shift the tail up one slot to open a gap at `pos` — the same three lines
    // `sorted_finish` runs for the by-value case, with the element size fixed at
    // the 4-byte rec-id an `ordered` array holds.
    //
    // Both halves used to be wrong, in units (loft#719).  The guard read
    // `8 + length * 4 > pos`, comparing a BYTE OFFSET against an ELEMENT INDEX,
    // so it was true even when appending at the end (`pos == length`) where
    // there is nothing to shift.  And the size read `8 + length * 4 - pos * 4`,
    // which is `(length - pos) * 4` PLUS EIGHT — so every insert copied eight
    // bytes too many, running past the array on the last slot.
    if pos < length {
        keys::mut_store(sorted, stores).copy_block(
            sorted_rec,
            8 + pos as isize * 4,
            sorted_rec,
            12 + pos as isize * 4,
            ((length - pos) * 4) as isize,
        );
    }
    keys::mut_store(&rec_ref, stores).set_u32_raw(sorted_rec, 8 + pos * 4, rec.rec);
    keys::mut_store(sorted, stores).set_u32_raw(sorted_rec, 4, 1 + length);
}

/// Is this collection ABSENT — a declared `?` that holds no collection (loft#917)?
///
/// Enforces the runtime half of @FR-N-Index and @FR-Col-Lookup: a read that names no element
/// — an out-of-range index, an absent key — IS that type's null, so a collection reached that
/// way must answer ABSENT here.  Everything that asks *"is this collection null?"* on either
/// backend arrives through `OpVectorIsNull`, which is this function.
///
/// A collection field or local is addressed by a DbRef aimed AT its 4-byte slot, not at the
/// collection: `OpGetField` is pure pointer arithmetic (`pos + offset`) and the vector ops
/// read the record id out of `(rec, pos)` themselves. So the value-level null — `store_nr ==
/// u16::MAX`, which [`DbRef::is_null`] tests — can never appear there, and absence has to be
/// read from the SLOT instead.
///
/// `0` in that slot already means the empty collection, so absence gets its own reserved id
/// ([`DbRef::ABSENT_REC`]). This is the one place that reading is done; `OpVectorIsNull`
/// consults it so `xs == null` and `xs == []` finally answer differently.
///
/// A DbRef with no record or offset reaches no slot at all, and is answered ABSENT without
/// touching a store: a null handle is `nullref` (and so is what a missed lookup or an index
/// past the end answers — `DbRef::or_null`), and a reference whose HOLDER has no record — a
/// field read through an unallocated buffer — names no slot either.  [`length_vector`]
/// guards on the same cases and calls them all length `0`, which is the same judgement seen
/// from the other side — nothing is there to count.
///
/// `#[inline]` because a generated `--native` program links this across a crate boundary
/// with no LTO — see [`get_vector`].
#[must_use]
#[inline]
pub fn is_absent_collection(db: &DbRef, stores: &[Store]) -> bool {
    if db.is_null() {
        return true;
    }
    if db.rec == 0 || db.pos == 0 {
        // No record, hence no slot, hence no collection: a field read through a holder
        // that has no record.  Reading it as "present" is what made `vv[9] == null`
        // answer `false` while `vv[9] ?? d` beside it took the default (loft#1120).  An
        // allocated collection, empty or not, always addresses a real slot (`rec != 0`),
        // which is what keeps `xs == []` and `xs == null` different answers.
        return true;
    }
    keys::store(db, stores).get_u32_raw(db.rec, db.pos) == DbRef::ABSENT_REC
}

/// @FR-I-Empty and @FR-I-NullSrc share ONE home, and this is it: a `for` runs its body while
/// the cursor is below this length, so answering 0 for a null source is what makes a null
/// iterate zero times rather than fault.  The two rules are one line of code because they are
/// one question — how many elements does this source have — and `nullref` is a RUNTIME null of
/// a non-nullable type.  A source whose TYPE is `τ?` never reaches here: it is refused at parse
/// time, since `types.md` admits no implicit unwrap (`Parser::iterator`).
pub fn length_vector(db: &DbRef, stores: &[Store]) -> u32 {
    // A null vector (absent) and an unallocated/empty vector both have length 0;
    // the null sentinel is checked first so it never indexes stores[u16::MAX].
    if db.is_null() || db.rec == 0 || db.pos == 0 {
        return 0;
    }
    let store = keys::store(db, stores);
    let v_rec = store.collection_rec(db.rec, db.pos);
    if v_rec == 0 {
        0
    } else {
        store.get_u32_raw(v_rec, 4)
    }
}

pub fn clear_vector(db: &DbRef, stores: &mut [Store]) {
    if db.is_null() || db.rec == 0 || db.pos == 0 {
        // Null (absent) or unallocated/empty vector ref — nothing to clear.
        // The hidden return buffer arrives unallocated like this on a fn's
        // first delivery; the null sentinel is checked first so clear() never
        // indexes stores[u16::MAX].
        return;
    }
    let store = keys::mut_store(db, stores);
    let v_rec = store.collection_rec(db.rec, db.pos);
    if v_rec != 0 {
        // Only set size of the vector to 0
        // TODO when the main path to a separate allocated objects: remove these
        // TODO lower string reference counts where needed
        store.set_u32_raw(v_rec, 4, 0);
    }
}

/// Read the element at `from` out of the vector `db` points at, as a `DbRef`.
///
/// An out-of-range index and a null vector both answer the null element (`rec: 0`),
/// which is what makes `v[i]` a nullable read rather than a fault.
///
/// Keep the `#[inline]`. It is not a hint about size: a `--native` program is a
/// SEPARATE crate that links loft's runtime rlib, and it is compiled without LTO, so a
/// plain `pub fn` here is a real call per element that the caller cannot see into. The
/// caller then resolves the store a second time for the element read, because nothing
/// tells it this function already did. The whole indexed-read chain is marked inlinable
/// for that reason — this, [`length_vector`], `keys::store` and `Stores::store` — and it
/// is worth ~1.7x on an indexed-read kernel (loft#885, and PERFORMANCE.md § Native vs
/// Rust root cause 3b for the measurement).
///
/// The `length_vector` call below is not the cost it appears to be: rustc inlines it
/// within the rlib and drops the repeated resolution, so hoisting it by hand buys
/// nothing.
#[must_use]
#[inline]
pub fn get_vector(db: &DbRef, size: u32, from: i64, stores: &[Store]) -> DbRef {
    // Indexing into a null (absent) vector yields the null element, not an OOB
    // on stores[u16::MAX].  An out-of-range index on a real vector answers the SAME
    // value below: `(L-Null)` gives a reference that leaves its slot ONE spelling of
    // absence, `nullref`, and an element read is the point the value is minted
    // (`DbRef::or_null`).  Answering the container's `store_nr` with `rec == 0` instead
    // was a second spelling that only `rec`-testing consumers could read (loft#1374).
    if db.is_null() {
        return DbRef::NULL;
    }
    #[cfg(debug_assertions)]
    if db.store_nr != u16::MAX {
        debug_assert!(
            !stores[db.store_nr as usize].free,
            "get_vector: use-after-free on store {} (rec={} pos={})",
            db.store_nr, db.rec, db.pos
        );
    }
    let store = keys::store(db, stores);
    if from == i64::MIN {
        return DbRef::NULL;
    }
    let v_rec = store.collection_rec(db.rec, db.pos);
    let l = length_vector(db, stores);
    let f = if from < 0 { from + i64::from(l) } else { from };
    if f < 0 || f >= i64::from(l) {
        DbRef::NULL
    } else {
        DbRef {
            store_nr: db.store_nr,
            rec: v_rec,
            pos: checked_vec_pos(f as u32, size),
        }
    }
}

/// The part of a vector's identity that a loop cannot change while nothing writes to
/// it: which store holds it, which record its elements live in, and how many there are.
///
/// Reading one element out of a vector normally re-derives all three (resolve the store,
/// load the container slot, load the length) and every one of those loads is guarded, so
/// LLVM will not hoist them out of a loop even fully inlined — see PERFORMANCE.md
/// § Native vs Rust root cause 3c. The native emitter therefore derives this triple ONCE
/// before a loop whose body provably cannot write a store, and each element read inside
/// becomes bounds check + address arithmetic ([`get_vector_hoisted`]).
///
/// A header is only meaningful for as long as that promise holds. `loft::generation::hoist`
/// is the one place that decides it, and `LOFT_HOIST_VERIFY=1` at GENERATION time emits
/// the checking form of every hoisted read, which re-derives the triple and asserts it
/// still matches — so a gate that lets a mutation through fails loudly under one suite run
/// instead of reading a stale record. The switch is at generation time rather than run time
/// because the check costs exactly the loads the hoist removed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VecHeader {
    pub store_nr: u16,
    pub rec: u32,
    pub len: u32,
}

/// Derive [`VecHeader`] for the vector `db` points at.
///
/// A null, unallocated or empty vector answers `len: 0`, which makes every fast-path
/// bounds test in [`get_vector_hoisted`] fail — so those cases route to `get_vector` and
/// keep its exact answer without needing a marker of their own.
#[must_use]
#[inline]
pub fn vec_header(db: &DbRef, stores: &[Store]) -> VecHeader {
    if db.is_null() || db.rec == 0 || db.pos == 0 {
        return VecHeader {
            store_nr: db.store_nr,
            rec: 0,
            len: 0,
        };
    }
    let store = keys::store(db, stores);
    let v_rec = store.collection_rec(db.rec, db.pos);
    let len = if v_rec == 0 {
        0
    } else {
        store.get_u32_raw(v_rec, 4)
    };
    VecHeader {
        store_nr: db.store_nr,
        rec: v_rec,
        len,
    }
}

/// [`get_vector`] with the store resolution, the container-slot load and the length load
/// supplied by an already-derived [`VecHeader`].
///
/// Only an index that is in range for the hoisted length takes the fast path. Everything
/// else — a negative index, an out-of-range one, `i64::MIN`, a null or empty vector —
/// falls through to `get_vector` itself, so there is no second definition of what those
/// answer.
///
/// `VERIFY` is the `LOFT_HOIST_VERIFY=1` form: it re-derives the header and panics if the
/// loop moved the vector under it. It is a const parameter rather than a `debug_assert!`
/// because loft's own library is built with debug assertions off, so a guard written that
/// way could never fire.
///
/// # Panics
///
/// Under `VERIFY`, when the header no longer describes `db` — that is the whole point of
/// the switch. Never in the emitted default (`VERIFY == false`).
#[must_use]
#[inline]
pub fn get_vector_hoisted<const VERIFY: bool>(
    h: &VecHeader,
    db: &DbRef,
    size: u32,
    from: i64,
    stores: &[Store],
) -> DbRef {
    if from >= 0 && from < i64::from(h.len) {
        if VERIFY {
            assert_eq!(
                *h,
                vec_header(db, stores),
                "hoisted vector header is stale — the loop wrote the vector it was hoisted for"
            );
        }
        DbRef {
            store_nr: h.store_nr,
            rec: h.rec,
            pos: checked_vec_pos(from as u32, size),
        }
    } else {
        get_vector(db, size, from, stores)
    }
}

/// One indexed element read against an already-derived [`VecHeader`]: the bounds test and
/// the typed load, with no `DbRef` built between them (loft#885 stage 2).
///
/// [`get_vector_hoisted`] removes the store resolution and the length load; this removes what
/// was left — building the element `DbRef`, testing its `rec` for the null element, resolving
/// the store a second time from it, and the `rec != 0 && valid(..)` guard inside the typed
/// getter, which had already been decided by the bounds test. What remains is one comparison
/// and one load.
///
/// `absent` is the getter's own null sentinel (`f32::NAN`, `i64::MIN`, …), so an index the
/// fast path refuses answers exactly what the unfused pair answered. Off the fast path this
/// routes back through [`get_vector`], which is what keeps negative indices addressing from
/// the end rather than reading `absent`.
///
/// # Panics
///
/// Under `VERIFY`, when the header no longer describes `db`. Never in the emitted default.
#[must_use]
#[inline]
pub fn get_elem_hoisted<T: Copy, const VERIFY: bool>(
    h: &VecHeader,
    db: &DbRef,
    size: u32,
    from: i64,
    fld: u32,
    absent: T,
    stores: &[Store],
) -> T {
    if from >= 0 && from < i64::from(h.len) {
        if VERIFY {
            assert_eq!(
                *h,
                vec_header(db, stores),
                "hoisted vector header is stale — the loop wrote the vector it was hoisted for"
            );
        }
        return *stores[h.store_nr as usize]
            .addr::<T>(h.rec, checked_vec_pos(from as u32, size) + fld);
    }
    let elem = get_vector(db, size, from, stores);
    if elem.rec == 0 {
        absent
    } else {
        *keys::store(&elem, stores).addr::<T>(elem.rec, elem.pos + fld)
    }
}

/// @FR-Col-RemoveDense — a vector stays DENSE: removing index `i` shifts every later
/// element down one, so there are no holes and no tombstones and index `j > i` now names what
/// was at `j+1`.  That renumbering is what ends the place a view names (@FR-B-Disturb), so it
/// is a contract rather than an implementation choice (@PLN130 F3 declined hole-punching).
pub fn remove_vector(db: &DbRef, size: u32, index: i64, stores: &mut [Store]) -> bool {
    if db.is_null() {
        return false; // nothing to remove from a null (absent) vector
    }
    let len = i64::from(length_vector(db, stores));
    let store = keys::mut_store(db, stores);
    let vec_rec = store.collection_rec(db.rec, db.pos);
    let i = if index < 0 { index + len } else { index };
    if i >= len || i < 0 || vec_rec == 0 {
        return false;
    }
    if len - i > 1 {
        // Shift elements [i+1 .. len) down to [i .. len-1): that is
        // `len - i - 1` elements.  Using `len - i` reads element `len` — one
        // past the last valid element — an out-of-bounds read into adjacent
        // memory (the stray element lands in the now-dead tail slot, so the
        // observable result was correct, masking the UB).
        store.copy_block(
            vec_rec,
            checked_vec_pos(i as u32 + 1, size) as isize,
            vec_rec,
            checked_vec_pos(i as u32, size) as isize,
            (len as isize - i as isize - 1) * size as isize,
        );
    }
    store.set_u32_raw(vec_rec, 4, len as u32 - 1);
    true
}

/**
With before this returns index+1 before any matching element.
Otherwise, return the index of the element after.
*/
#[must_use]
pub fn sorted_find(
    sorted: &DbRef,
    before: bool,
    size: u16,
    stores: &[Store],
    keys: &[Key],
    key: &[Content],
) -> (u32, bool) {
    if sorted.rec == 0 {
        return (0, false);
    }
    let store = keys::store(sorted, stores);
    let sorted_rec = store.get_u32_raw(sorted.rec, sorted.pos);
    if sorted_rec == 0 {
        return (0, false);
    }
    let length = store.get_u32_raw(sorted_rec, 4);
    if length == 0 {
        return (0, false);
    }
    let mut result = DbRef {
        store_nr: sorted.store_nr,
        rec: sorted_rec,
        pos: 0,
    };
    let mut left = 0;
    let mut right = length - 1;
    let mut found = false;
    loop {
        let mid = left + (right - left) / 2;
        result.pos = 8 + mid * u32::from(size);
        let cmp = keys::key_compare(key, &result, stores, keys);
        let action = if cmp == Ordering::Equal {
            found = true;
            if before {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else {
            cmp
        };
        if action == Ordering::Less {
            if mid > 0 {
                right = mid - 1;
            } else {
                right = 0;
                left += 1;
            }
        } else {
            left = mid + 1;
        }
        if left > right {
            return (
                if action == Ordering::Greater {
                    mid + 1
                } else {
                    mid
                },
                found,
            );
        }
    }
}

#[must_use]
pub fn ordered_find(
    sorted: &DbRef,
    before: bool,
    stores: &[Store],
    keys: &[Key],
    key: &[Content],
) -> (u32, bool) {
    let store = keys::store(sorted, stores);
    let sorted_rec = store.get_u32_raw(sorted.rec, sorted.pos);
    let length = store.get_u32_raw(sorted_rec, 4);
    let mut result = DbRef {
        store_nr: sorted.store_nr,
        rec: 0,
        pos: 0,
    };
    if sorted_rec == 0 || length == 0 {
        // An EMPTY collection has no slot to compare against, and `length - 1` below
        // would wrap to u32::MAX and walk the binary search off the vector.  The
        // `sorted_find` twin has always guarded this; here it was unreachable only
        // while every caller was dead code (loft#904).
        return (0, false);
    }
    let mut found = false;
    let mut left = 0;
    let mut right = length - 1;
    loop {
        let mid = (left + right + 1) >> 1;
        result.rec = store.get_u32_raw(sorted_rec, 8 + mid * 4);
        result.pos = 8;
        let cmp = keys::key_compare(key, &result, stores, keys);
        let action = if cmp == Ordering::Equal {
            found = true;
            if before {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else {
            cmp
        };
        if action == Ordering::Less {
            if mid > 0 {
                right = mid - 1;
            } else {
                right = 0;
                left += 1;
            }
        } else {
            left = mid + 1;
        }
        if left > right {
            return (
                if action == Ordering::Greater {
                    mid + 1
                } else {
                    mid
                },
                found,
            );
        }
    }
}

pub fn vector_next(data: &DbRef, pos: &mut i32, size: u16, stores: &[Store]) {
    let rec = keys::store(data, stores).collection_rec(data.rec, data.pos);
    if rec == 0 {
        *pos = i32::MAX;
        return;
    }
    let length = keys::store(data, stores).get_u32_raw(rec, 4) as i32;
    if *pos == i32::MAX && length != 0 {
        *pos = 8;
    } else if length != 0 && *pos < 8 + (length - 1) * i32::from(size) {
        *pos += i32::from(size);
    } else {
        *pos = i32::MAX;
    }
}

/// Step one element BACKWARDS through a slot vector, in byte offsets.
///
/// The byte-offset mirror of [`vector_step_rev`], which does the same walk in
/// element indices for an inline `sorted`.  `pos == i32::MAX` or any position past
/// the last slot is the not-yet-started sentinel and begins at the last element;
/// `i32::MAX` is answered once the walk has passed the first.
///
/// Its absence is why `rev()` over an `ordered` collection walked FORWARDS: the
/// stepper had only [`vector_next`], so the reverse bit had nothing to select
/// (loft#904).
pub fn vector_prev(data: &DbRef, pos: &mut i32, size: u16, stores: &[Store]) {
    let rec = keys::store(data, stores).collection_rec(data.rec, data.pos);
    if rec == 0 {
        *pos = i32::MAX;
        return;
    }
    let length = keys::store(data, stores).get_u32_raw(rec, 4) as i32;
    if length == 0 {
        *pos = i32::MAX;
        return;
    }
    let last = 8 + (length - 1) * i32::from(size);
    if *pos == i32::MAX || *pos > last {
        // Not started, or an upper bound past the end — begin at the last element.
        *pos = last;
    } else if *pos > 8 {
        *pos -= i32::from(size);
    } else {
        *pos = i32::MAX; // passed the first element
    }
}

/// The slot stride of an `ordered` collection: it holds 4-byte record ids.
///
/// A hash's iteration scratch can stride 8 (`(record, offset)` pairs for arena
/// entries, @PLN135 arc H) and reads its width out of its own header — but that
/// scratch is only ever walked UNBOUNDED, so it never reaches the cursor
/// arithmetic below.
const ORDERED_STRIDE: u32 = 4;

/// The `(start, finish)` cursor pair for iterating an `ordered` collection over a
/// key range — the shared derivation both backends call, the way `tree::range_cursors`
/// is shared for an `index`.
///
/// Both cursors are BYTE OFFSETS into the slot vector, which is the unit
/// [`vector_next`] / [`vector_prev`] walk in and [`step_ordered`] returns. Deriving
/// them as slot INDICES is what loft#904 was: `State::iterate` produced indices,
/// the stepper read them as byte offsets, and a bounded range therefore walked from
/// somewhere inside the vector's header — every element reading zero going forwards,
/// a SIGSEGV going backwards. Each backend had its own copy of the derivation and
/// they had already drifted apart, which nothing noticed because neither was ever
/// consumed.
///
/// The index arithmetic mirrors the `sorted` arm (`State::iterate`, `on = 2`), which
/// is correct and whose bound semantics loft#691 settled — a collection's kind must
/// not change what a range means, and which of the two a program gets depends only on
/// whether some other struct mentions a keyed collection over the element type:
///
/// * forward — `start` is the cursor the first step advances FROM, so one before the
///   first element to visit (`i32::MAX`, the not-started sentinel, when that is the
///   very first slot); `finish` is the first position NOT to visit.
/// * reverse — `start` is one PAST the highest element to visit (anything beyond the
///   last slot reads as not-started); `finish` is the LOWEST position to visit.
/// * `ex` describes the UPPER bound only; the lower bound is always inclusive, which
///   is why `from` is always searched with `true`.
///
/// A `finish` of 0 means NO bound in either direction — it is below every valid
/// position, so it can never trip, and it is what the unbounded form returns. That
/// keeps `on = 4` (a hash / radix / trie scratch, always unbounded) walking to the
/// end unchanged.
#[must_use]
pub fn ordered_range_cursors(
    data: &DbRef,
    stores: &[Store],
    keys: &[Key],
    from: &[Content],
    till: &[Content],
    ex: bool,
    reverse: bool,
) -> (u32, u32) {
    if from.is_empty() && till.is_empty() {
        return (i32::MAX as u32, 0);
    }
    let store = keys::store(data, stores);
    let vec_rec = store.collection_rec(data.rec, data.pos);
    let length = if vec_rec == 0 {
        0
    } else {
        store.get_u32_raw(vec_rec, 4)
    };
    let at = |slot: u32| 8 + slot * ORDERED_STRIDE;
    if reverse {
        let start = if till.is_empty() {
            length
        } else {
            ordered_find(data, ex, stores, keys, till).0
        };
        let finish = if from.is_empty() {
            0
        } else {
            ordered_find(data, true, stores, keys, from).0
        };
        (at(start), at(finish))
    } else {
        let first = if from.is_empty() {
            0
        } else {
            ordered_find(data, true, stores, keys, from).0
        };
        let last = if till.is_empty() {
            length
        } else {
            let (t, cmp) = ordered_find(data, ex, stores, keys, till);
            if ex || cmp { t } else { t + 1 }
        };
        let start = if first == 0 {
            i32::MAX as u32
        } else {
            at(first - 1)
        };
        (start, at(last))
    }
}

/// Byte offset in an iteration-scratch header holding [`scratch_tag`].
///
/// The header is two words: offset 4 the element vector, offset 8 the source store,
/// and this. It exists so a scratch can be RECOGNISED — a released scratch leaves its
/// block on the free list, the next claim takes it, and every field read after that is
/// somebody else's. `Store::is_claimed_record` cannot tell those apart, and acting on
/// the difference cost a whole store (see `Stores::free_iteration_scratch`).
pub const SCRATCH_TAG_FLD: u32 = 12;

/// High half: the marker. Low half: bytes per element — 4 for a rec-number scratch
/// (radix, index), 8 for the `(record, offset)` pairs a hash's arena entries need.
const SCRATCH_MARK: u32 = 0x5C4A_0000;
const SCRATCH_MARK_MASK: u32 = 0xFFFF_0000;

/// The tag an iteration-scratch header carries, for an element width of `width` bytes.
#[must_use]
pub fn scratch_tag(width: u32) -> u32 {
    SCRATCH_MARK | (width & 0xFFFF)
}

/// Is `scratch` still the header its builder wrote?
#[must_use]
pub fn is_scratch_header(store: &Store, scratch: &DbRef) -> bool {
    store.record_words(scratch.rec) >= 2
        && store.get_u32_raw(scratch.rec, scratch.pos + SCRATCH_TAG_FLD - 4) & SCRATCH_MARK_MASK
            == SCRATCH_MARK
}

/// One step of the Ordered iteration path over a `hash`/`index`/`radix`/`spatial`
/// collection: advance the u32-stride rec-nr cursor and yield the record it points at.
/// Shared by the interpreter (`State::step`) and the native runtime
/// (`codegen_runtime::step`) so the two cannot drift.  `cur` is the current cursor;
/// returns `(yielded element, new cursor)`.
///
/// The mode picks where the yielded RECORD lives:
/// * `on=3` (`sourced == false`) — scratch and records are co-located, so the element
///   is in `data.store_nr`.  `data.pos` locates the rec-nr vector pointer, which is a
///   struct-field offset when a keyed field is iterated in place — NOT a fixed slot.
/// * `on=4` (`sourced == true`) — the scratch is a fresh record this iteration built
///   (`build_rec_scratch`), so `data.pos` is always 4 and the SOURCE (records) store_nr
///   sits at the header's offset 8 (`data.pos + 4`).  Yielding in that store lets the
///   scratch live in a different (writable) store than a read-only/exposed source — see
///   expose-iteration-scratch.md.
pub fn step_ordered(
    data: &DbRef,
    cur: u32,
    stores: &[Store],
    sourced: bool,
    reverse: bool,
) -> (DbRef, u32) {
    let store = keys::store(data, stores);
    // @PLN135 arc H — a hash's entries are SLOTS in a chunked arena, so an element is
    // `(record, offset)` and no longer a record number whose body starts at 8.  Its
    // scratch therefore strides 8 bytes and says so in the header word the builder
    // writes; every other producer (radix, index, the co-located on=3 path) leaves
    // that word 0 and keeps the 4-byte rec-number form.  Reading the width from the
    // scratch is what lets one stepper serve both without the caller having to know
    // which kind built it.
    let tag = if sourced {
        store.get_u32_raw(data.rec, data.pos + SCRATCH_TAG_FLD - 4)
    } else {
        0
    };
    let wide = tag & SCRATCH_MARK_MASK == SCRATCH_MARK && tag & 0xFFFF == 8;
    let stride: u16 = if wide { 8 } else { 4 };
    let mut pos = cur as i32;
    if reverse {
        vector_prev(data, &mut pos, stride, stores);
    } else {
        vector_next(data, &mut pos, stride, stores);
    }
    let vector = store.collection_rec(data.rec, data.pos);
    let (rec, elem_pos) = if pos == i32::MAX {
        (0, 8)
    } else if wide {
        (
            store.get_u32_raw(vector, pos as u32),
            store.get_u32_raw(vector, pos as u32 + 4),
        )
    } else {
        (store.get_u32_raw(vector, pos as u32), 8)
    };
    let elem_store = if sourced {
        store.get_u32_raw(data.rec, data.pos + 4) as u16
    } else {
        data.store_nr
    };
    (
        DbRef {
            store_nr: elem_store,
            rec,
            pos: elem_pos,
        },
        pos as u32,
    )
}

pub fn vector_step(data: &DbRef, pos: &mut i32, stores: &[Store]) {
    let rec = keys::store(data, stores).collection_rec(data.rec, data.pos);
    if rec == 0 {
        *pos = i32::MAX;
        return;
    }
    let length = keys::store(data, stores).get_u32_raw(rec, 4) as i32;
    if *pos == i32::MAX && length != 0 {
        *pos = 0;
    } else if length != 0 && *pos < length - 1 {
        *pos += 1;
    } else {
        *pos = i32::MAX;
    }
}

/// Advance the sorted-vector position one step backwards (reverse iteration).
/// `pos == i32::MAX` or `pos >= length` is the not-yet-started sentinel;
/// the first call sets `pos` to `length - 1` (last element).
/// Returns `i32::MAX` when the iterator has moved past the first element.
pub fn vector_step_rev(data: &DbRef, pos: &mut i32, stores: &[Store]) {
    let rec = keys::store(data, stores).collection_rec(data.rec, data.pos);
    if rec == 0 {
        *pos = i32::MAX;
        return;
    }
    let length = keys::store(data, stores).get_u32_raw(rec, 4) as i32;
    if length == 0 || *pos == i32::MAX || *pos >= length {
        // Not started yet (sentinel) or past the end — begin at the last element.
        *pos = if length == 0 { i32::MAX } else { length - 1 };
    } else if *pos > 0 {
        *pos -= 1;
    } else {
        *pos = i32::MAX; // Passed the beginning.
    }
}

/// Sort a vector of `text` elements in-place by lexicographic
/// string comparison.  Storage layout: each element is a u32
/// string-offset (size=4) pointing at the text payload elsewhere
/// in the same store.  We sort the OFFSETS by what they point
/// at — the string data stays in place; only the offset order
/// changes.
///
/// Added in @PLAN37 phase 10.8 to extend `sort()` from
/// numeric-only to also cover `vector<text>`.
pub fn sort_text_vector(db: &DbRef, stores: &mut [Store]) {
    let len = length_vector(db, stores) as usize;
    if len < 2 {
        return;
    }
    let store = keys::mut_store(db, stores);
    let v_rec = store.collection_rec(db.rec, db.pos);
    if v_rec == 0 {
        return;
    }
    // Collect (offset, owned String) pairs.  The owned String
    // copy lets us drop the immutable store borrow before
    // writing the sorted offsets back.  Memory cost is one
    // String per element, but vectors that hit sort_text are
    // usually small (UI lists, validator outputs); the alloc is
    // negligible vs the sort itself.
    //
    // `store.get_str(rec)` is the canonical accessor for a
    // string record (interprets `rec` as a string-rec offset
    // into the store's payload area, returning a `&str`).
    let mut entries: Vec<(u32, String)> = (0..len)
        .map(|i| {
            let off = store.get_u32_raw(v_rec, 8 + (i as u32) * 4);
            let s = store.get_str(off);
            (off, s.to_string())
        })
        .collect();
    entries.sort_unstable_by(|a, b| a.1.cmp(&b.1));
    for (i, (off, _)) in entries.iter().enumerate() {
        store.set_u32_raw(v_rec, 8 + (i as u32) * 4, *off);
    }
}

/// Sort a vector of primitive elements in-place (ascending).
/// `elem_size` is the byte size of each element (1, 2, 4, or 8).
/// `is_float` must be true for floating-point types (f32 at size=4, f64 at size=8).
pub fn sort_vector(db: &DbRef, elem_size: u16, is_float: bool, stores: &mut [Store]) {
    let len = length_vector(db, stores) as usize;
    if len < 2 {
        return;
    }
    let store = keys::mut_store(db, stores);
    let v_rec = store.collection_rec(db.rec, db.pos);
    if v_rec == 0 {
        return;
    }
    match elem_size {
        1 => {
            let mut vals: Vec<i32> = (0..len)
                .map(|i| store.get_byte(v_rec, 8 + (i as u32), 0))
                .collect();
            vals.sort_unstable();
            for (i, &v) in vals.iter().enumerate() {
                store.set_byte(v_rec, 8 + (i as u32), 0, v);
            }
        }
        2 => {
            let mut vals: Vec<i32> = (0..len)
                .map(|i| store.get_short(v_rec, 8 + (i as u32) * 2, 0))
                .collect();
            vals.sort_unstable();
            for (i, &v) in vals.iter().enumerate() {
                store.set_short(v_rec, 8 + (i as u32) * 2, 0, v);
            }
        }
        4 => {
            if is_float {
                let mut vals: Vec<f32> = (0..len)
                    .map(|i| f32::from_bits(store.get_u32_raw(v_rec, 8 + (i as u32) * 4)))
                    .collect();
                vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Greater));
                for (i, &v) in vals.iter().enumerate() {
                    store.set_u32_raw(v_rec, 8 + (i as u32) * 4, v.to_bits());
                }
            } else {
                let mut vals: Vec<i32> = (0..len)
                    .map(|i| store.get_i32_raw(v_rec, 8 + (i as u32) * 4))
                    .collect();
                vals.sort_unstable();
                for (i, &v) in vals.iter().enumerate() {
                    store.set_i32_raw(v_rec, 8 + (i as u32) * 4, v);
                }
            }
        }
        8 => {
            if is_float {
                let mut vals: Vec<f64> = (0..len)
                    .map(|i| store.get_float(v_rec, 8 + (i as u32) * 8))
                    .collect();
                vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Greater));
                for (i, &v) in vals.iter().enumerate() {
                    store.set_float(v_rec, 8 + (i as u32) * 8, v);
                }
            } else {
                let mut vals: Vec<i64> = (0..len)
                    .map(|i| store.get_long(v_rec, 8 + (i as u32) * 8))
                    .collect();
                vals.sort_unstable();
                for (i, &v) in vals.iter().enumerate() {
                    store.set_long(v_rec, 8 + (i as u32) * 8, v);
                }
            }
        }
        _ => {} // unsupported element size — no-op
    }
}

/// Reverse a vector in-place by swapping elements from the ends toward the middle.
pub fn reverse_vector(db: &DbRef, elem_size: u32, stores: &mut [Store]) {
    let len = length_vector(db, stores);
    if len < 2 {
        return;
    }
    let store = keys::mut_store(db, stores);
    let v_rec = store.collection_rec(db.rec, db.pos);
    if v_rec == 0 {
        return;
    }
    let mut buf = vec![0u8; elem_size as usize];
    let mut lo = 0u32;
    let mut hi = len - 1;
    while lo < hi {
        let lo_pos = 8 + lo * elem_size;
        let hi_pos = 8 + hi * elem_size;
        // Copy lo → buf
        for i in 0..elem_size {
            buf[i as usize] = store.get_byte(v_rec, lo_pos + i, 0) as u8;
        }
        // Copy hi → lo
        for i in 0..elem_size {
            let v = store.get_byte(v_rec, hi_pos + i, 0);
            store.set_byte(v_rec, lo_pos + i, 0, v);
        }
        // Copy buf → hi
        for i in 0..elem_size {
            store.set_byte(v_rec, hi_pos + i, 0, i32::from(buf[i as usize]));
        }
        lo += 1;
        hi -= 1;
    }
}

#[cfg(test)]
mod plan25_null_vector_tests {
    //! @PLN25 Phase 1: a null vector (`DbRef::NULL`, `store_nr == u16::MAX`) must
    //! flow through every store accessor WITHOUT indexing `stores[u16::MAX]`.
    //! Passing an empty `stores` slice proves the `is_null()` guard returns
    //! before any dereference — a missed guard would panic on the empty slice.
    use super::{
        clear_vector, get_vector, insert_vector, length_vector, remove_vector, reverse_vector,
        sort_vector, vector_append,
    };
    use crate::keys::DbRef;
    use crate::store::Store;

    const NULL: DbRef = DbRef::NULL;

    #[test]
    fn null_is_distinct_from_empty() {
        assert!(NULL.is_null());
        // A valid-but-EMPTY vector lives on a real store (store_nr 0, rec 0) —
        // length 0 but NOT null.  This is the distinction the feature rests on.
        assert!(
            !DbRef {
                store_nr: 0,
                rec: 0,
                pos: 0
            }
            .is_null()
        );
    }

    #[test]
    fn length_of_null_is_zero() {
        assert_eq!(length_vector(&NULL, &[]), 0);
    }

    #[test]
    fn index_into_null_yields_null_element() {
        assert!(get_vector(&NULL, 8, 0, &[]).is_null());
        assert!(get_vector(&NULL, 8, 5, &[]).is_null());
        assert!(get_vector(&NULL, 8, -1, &[]).is_null());
    }

    #[test]
    fn mutators_on_null_are_safe_noops() {
        let mut stores: Vec<Store> = vec![];
        assert!(vector_append(&NULL, 8, &mut stores).is_null());
        assert!(insert_vector(&NULL, 8, 0, &mut stores).is_null());
        assert!(!remove_vector(&NULL, 8, 0, &mut stores));
        clear_vector(&NULL, &mut stores);
        sort_vector(&NULL, 8, false, &mut stores);
        reverse_vector(&NULL, 8, &mut stores);
        // Reaching here = no accessor indexed stores[u16::MAX].
    }
}
