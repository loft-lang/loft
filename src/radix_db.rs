// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! The bridge between the database's keyed collections and [`crate::radix_tree`]
//! — the counterpart of [`crate::hash`] for the `Radix` kind (`spatial<T[…]>`, and
//! later `radix<T[k]>`).  @PLN48 S2.
//!
//! The tree keys on an abstract bit-string; this module supplies the one the schema
//! implies.  Each key field becomes an **axis**, read from the element record and
//! encoded so that the tree's key order matches the order the existing keyed
//! collections use (`compare_ref`).  For one axis that is a plain ordered radix; for
//! two or three integer axes the axes interleave into a Morton (Z-order) code, so
//! spatially near records share a long key prefix.
//!
//! **Order preservation is the invariant.**  A signed integer's 2's-complement bits
//! are *not* ordered (a negative has its top bit set, so it would sort after every
//! positive).  Each axis is therefore mapped to **offset-binary** — flip the sign
//! bit — which makes an unsigned bit-compare agree with the signed value compare, so
//! the radix walk visits records in the same order `sorted`/`index` would.
//!
//! Scope of this step: **integer** key fields (the spatial games case).  Every axis
//! is read as an `i64` and given 64 bits — correct for any integer width, if wider
//! than a narrow field needs.  Text keys (the `radix<T[text]>` prefix path) and
//! per-width packing are follow-ups; until the parser gate lifts, only these tests
//! reach this code.

#![allow(dead_code)]

use crate::keys::{self, Content, DbRef, Key};
use crate::radix_tree::{self as rt, KeyOracle};
use crate::store::Store;

/// Element payload base: an element record holds its back-pointer at offset 4 and its
/// struct payload from offset 8 (the shared keyed-collection convention).
pub(crate) const PAYLOAD: u32 = 8;
/// Bits given to each axis.  Uniform so the interleave is a plain Morton code.
const AXIS_BITS: u32 = 64;
/// The most axes a spatial key interleaves (`spatial<T[x,y,z]>`).  The parser
/// rejects a `spatial<T[…]>` with more key fields than this (else the Morton
/// interleave indexes past the `[u64; MAX_AXES]` code array — a runtime panic).
pub const MAX_AXES: usize = 3;

/// A key field's signed coordinate value, read out of a resident store.
///
/// The widths are the schema's, and there is a SECOND reader of them: a paged
/// spatial query reads the same field out of an image
/// (`paged_reader::PagedSpatial::axis_value`). The two disagreeing on one width
/// makes a record's Morton code differ between the storages, which presents as a
/// box query that misses a point inside it — so
/// `paged::a_paged_box_agrees_with_the_resident_one` drives every width through both,
/// and `paged::every_axis_width_decodes_to_what_was_written` fixes each side to the
/// value that was WRITTEN rather than to the other side. Those are the contract; this
/// is one half of it.
fn axis_i64(store: &Store, rec: u32, key: &Key) -> i64 {
    let p = PAYLOAD + u32::from(key.position);
    // The narrow widths store `value - start`, so the decode adds `start` back.  It is the
    // key's own field, carried for exactly this, and the VALUE-keyed decoders next door
    // (`keys::compare_ref`, `keys::get_key`, `keys::hash_key`) already read it that way —
    // one field has one decode, whichever kind of collection asks (@FR-Col-Axis).
    let start = key.start;
    match key.type_nr.unsigned_abs() {
        2 => store.get_long(rec, p),
        8 => i64::from(store.get_i32_raw(rec, p)),
        // The unsigned 4-byte encoding (@FR-L-Narrow-Enc).  Raw like `Int` above, so it
        // takes no `start` — the bias belongs to the widths that store `value - start`.
        12 => i64::from(store.get_u32_raw(rec, p)),
        9 => i64::from(store.get_short(rec, p, start)),
        10 => i64::from(store.get_byte(rec, p, start)),
        11 => i64::from(store.get_short_full(rec, p, start)),
        // type_nr 1 (`integer`) and any other integer default.
        _ => store.get_int(rec, p),
    }
}

/// The order-preserving (offset-binary) code of a signed coordinate — the axis's
/// contribution to the Morton key.  Flipping the sign bit makes an unsigned compare
/// agree with the signed value compare.
pub(crate) fn coord_code(v: i64) -> u64 {
    (v as u64) ^ (1u64 << 63)
}

fn axis_code(store: &Store, rec: u32, key: &Key) -> u64 {
    coord_code(axis_i64(store, rec, key))
}

/// The same code from an already-extracted query value.  Must match [`axis_code`]:
/// `get_key` yields `Content::Long(i64)` for every integer width, so both sides
/// offset-binary the same `i64`.
fn probe_code(c: &Content) -> u64 {
    match c {
        Content::Long(v) => (*v as u64) ^ (1u64 << 63),
        // A non-integer probe cannot match an integer axis; encode the minimum.
        _ => 0,
    }
}

/// The `word`-th 64-bit chunk of the Z-order interleave of `n` axes.  Composite bit
/// `b` (most-significant first) is axis `b % n`'s bit `b / n` — so the axes' most
/// significant bits lead, which is what makes the interleave order-preserving.
pub(crate) fn interleave(word: u32, n: usize, code: impl Fn(usize) -> u64) -> u64 {
    let mut codes = [0u64; MAX_AXES];
    for (a, slot) in codes.iter_mut().enumerate().take(n) {
        *slot = code(a);
    }
    let total = AXIS_BITS * n as u32;
    let mut out = 0u64;
    for i in 0..64 {
        let b = word * 64 + i;
        if b >= total {
            break;
        }
        let axis = (b as usize) % n;
        let level = b / n as u32;
        let bit = (codes[axis] >> (AXIS_BITS - 1 - level)) & 1;
        out |= bit << (63 - i);
    }
    out
}

/// How the tree reads a record's key: the Morton interleave of its coordinate axes.
struct RadixOracle<'a> {
    keys: &'a [Key],
}

impl KeyOracle for RadixOracle<'_> {
    fn bits(&self, _store: &Store, _rec: u32) -> u32 {
        AXIS_BITS * self.keys.len() as u32
    }
    fn word(&self, store: &Store, rec: u32, word: u32) -> u64 {
        interleave(word, self.keys.len(), |a| {
            axis_code(store, rec, &self.keys[a])
        })
    }
}

pub(crate) fn key_bits(keys: &[Key]) -> u32 {
    AXIS_BITS * keys.len() as u32
}

/// Insert element record `rec` into the `Radix` collection at field `coll`.
///
/// The tree lives in `coll`'s store, its record id held in `coll`'s 4-byte field
/// (the `hash`-bucket convention).  A growing insert can relocate the tree, so the
/// (possibly new) id is written back.  Two records with the same coordinates both
/// stay — they differ in the id suffix and land adjacent — which is what lets many
/// entities share a bucket; no dedup here.
pub fn add(coll: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    let store = keys::mut_store(coll, stores);
    let mut tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        tree = rt::rtree_init(store, 0);
        store.set_u32_raw(coll.rec, coll.pos, tree);
    }
    let tree = rt::rtree_insert(store, tree, rec.rec, &RadixOracle { keys });
    store.set_u32_raw(coll.rec, coll.pos, tree);
}

/// The record whose key equals `key`, or a null `DbRef` (`rec == 0`) when absent.
/// When several records share the key, the first in order.
#[must_use]
pub fn find(coll: &DbRef, stores: &[Store], keys: &[Key], key: &[Content]) -> DbRef {
    let store = keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    let rec = if tree == 0 {
        0
    } else {
        let probe = |word: u32| interleave(word, key.len(), |a| probe_code(&key[a]));
        rt::rtree_get(store, tree, &probe, key_bits(keys), &RadixOracle { keys })
    };
    DbRef {
        store_nr: coll.store_nr,
        rec,
        pos: PAYLOAD,
    }
}

/// Unlink element record `rec` from the collection; the caller frees `rec` itself.
/// `false` if it was not present (the tree removes only the record whose key AND id
/// match, so a same-bucket sibling is never removed by mistake).
pub fn remove(coll: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) -> bool {
    let store = keys::mut_store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        return false;
    }
    rt::rtree_remove(store, tree, rec.rec, &RadixOracle { keys })
}

/// Number of element records in the collection.  Reads the tree's cached length
/// word (O(1)); key-free, mirroring `hash::count`.
#[must_use]
pub fn count(coll: &DbRef, stores: &[Store]) -> u32 {
    let store = keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        0
    } else {
        rt::rtree_len(store, tree)
    }
}

/// Every element record in the collection, in key order.  Key-free (a plain tree
/// walk), so teardown and iteration reach it without building an oracle.
#[must_use]
pub fn records(coll: &DbRef, stores: &[Store]) -> Vec<u32> {
    let store = keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    let mut out = Vec::new();
    if tree != 0 {
        let mut it = rt::rtree_first(store, tree);
        let mut r = it.rec();
        while r != 0 {
            out.push(r);
            r = it.next(store, tree).unwrap_or(0);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Proximity queries — @PLN48 S3, the Rust ports of `src/spatial.rs`, reading
// coordinates through the schema's key fields instead of fixed offsets.
// ---------------------------------------------------------------------------

/// The full multi-word Morton code of a point given each axis's order-preserving
/// code.  `n` axes × 64 bits interleave into `n` words (word 0 most significant).
fn morton_words(n: usize, code: impl Fn(usize) -> u64) -> [u64; MAX_AXES] {
    let mut words = [0u64; MAX_AXES];
    for (w, slot) in words.iter_mut().enumerate().take(n) {
        *slot = interleave(w as u32, n, &code);
    }
    words
}

/// Lexicographic compare of two `n`-word Morton codes (word 0 most significant).
fn code_gt(a: &[u64; MAX_AXES], b: &[u64; MAX_AXES], n: usize) -> bool {
    for i in 0..n {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    false
}

/// Is `rec` inside the closed box `[from, till]` on EVERY axis?
///
/// The geometric test [`range`] does not do: its result is the Morton code interval,
/// which for any non-degenerate box is a strict superset — Z-order threads out of
/// the box and back.
///
/// The BOX surface no longer composes the two ([`box_walk`] walks the box itself,
/// @PLN136), so what this pair is for now is the ORACLE that says the walk did not
/// change the answer: `d5_box_walk_is_exactly_the_box` runs the old composition
/// beside the new walk on every probe. Removing the composition would remove the
/// proof of its own replacement.
///
/// The axis decoding lives here rather than in a caller because `axis_i64` is what
/// knows how each integer width is stored, and a second reader of that layout is how
/// the two drift apart.
#[must_use]
pub(crate) fn in_box(store: &Store, rec: u32, keys: &[Key], from: &[i64], till: &[i64]) -> bool {
    keys.iter().enumerate().all(|(a, key)| {
        let v = axis_i64(store, rec, key);
        let (lo, hi) = if from[a] <= till[a] {
            (from[a], till[a])
        } else {
            // A box given corner-swapped on an axis still names that interval.
            (till[a], from[a])
        };
        v >= lo && v <= hi
    })
}

/// The records whose Morton code lies in `[from, till]` (or `[from, ∞)` when `till`
/// is `None`), in natural Morton order, capped at `limit` records (`None` = all).
///
/// The primitive behind the OPEN `spatial` slices, `xs[(x,y)..]` and `xs[(x,y)..:n]`
/// — the Z-order walk onward from a corner, which is what makes `..:n` a proximity
/// query and what filtering it would destroy. The bounding box is [`box_range`]'s;
/// it used to be this call plus [`in_box`], and the `till` form is what that pairing
/// left behind — kept because `d5_box_walk_is_exactly_the_box` measures the new walk
/// against it.
#[must_use]
pub fn range(
    coll: &DbRef,
    stores: &[Store],
    keys: &[Key],
    from: &[i64],
    till: Option<&[i64]>,
    limit: Option<usize>,
) -> Vec<u32> {
    let store = keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        return Vec::new();
    }
    let n = keys.len();
    let from_lo = |a: usize| coord_code(from[a]);
    let till_code = till.map(|t| morton_words(n, |a| coord_code(t[a])));
    let oracle = RadixOracle { keys };
    let probe = |word: u32| interleave(word, n, from_lo);
    let mut it = rt::rtree_seek(store, tree, &probe, key_bits(keys), &oracle);
    let cap = limit.unwrap_or(usize::MAX);
    let mut out = Vec::new();
    let mut rec = it.rec();
    while rec != 0 && out.len() < cap {
        if let Some(hi) = &till_code {
            let rec_code = morton_words(n, |a| axis_code(store, rec, &keys[a]));
            if code_gt(&rec_code, hi, n) {
                break;
            }
        }
        out.push(rec);
        rec = it.next(store, tree).unwrap_or(0);
    }
    out
}

/// `|a - b|` over two `n`-word Morton codes, word 0 most significant.
///
/// A single axis is 64 bits, so two axes are already 128 and three are 192 — wider than
/// any integer this crate can subtract in one step, which is why the distance is computed
/// word by word with a borrow rather than by casting down to `u64`. Truncating instead
/// would make every pair of points sharing their high word look equidistant, which is most
/// of a map.
fn code_abs_diff(a: &[u64; MAX_AXES], b: &[u64; MAX_AXES], n: usize) -> [u64; MAX_AXES] {
    let (hi, lo) = if code_gt(a, b, n) { (a, b) } else { (b, a) };
    let mut out = [0u64; MAX_AXES];
    let mut borrow = 0u64;
    for i in (0..n).rev() {
        let (d, b1) = hi[i].overflowing_sub(lo[i]);
        let (d, b2) = d.overflowing_sub(borrow);
        out[i] = d;
        borrow = u64::from(b1 || b2);
    }
    out
}

/// The records nearest `from`, in roughly-increasing distance, capped at `limit`.
///
/// The n-axis form of [`crate::spatial::near`], and what the OPEN `spatial` slices
/// (`xs[(x,y)..]`, `xs[(x,y)..:n]`) are documented to do. [`range`] answers the Z-order
/// TAIL — every record whose code is `>= from` — so half the neighbourhood is structurally
/// unreachable: a query at the far end of the map answered nothing at all, and `..:n`
/// silently under-delivered by however close the query sat to the end of the curve
/// (loft#1002).
///
/// Two cursors, seeded either side of the query, each step yielding whichever is closer.
/// That is what makes the answer independent of where the query lands, and it costs no
/// allocation and no distance sort.
///
/// **Approximate**, exactly as `spatial::near` is: the order is by MORTON distance, which
/// tracks spatial distance closely but jumps at quadrant boundaries, so a truly-near point
/// can arrive a little late. `xs[lo..hi]` is the exact form and stays the answer when the
/// caller needs containment. Every record is yielded eventually, each once.
#[must_use]
pub fn near_range(
    coll: &DbRef,
    stores: &[Store],
    keys: &[Key],
    from: &[i64],
    limit: Option<usize>,
) -> Vec<u32> {
    let store = keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        return Vec::new();
    }
    let n = keys.len();
    if n == 0 || n > MAX_AXES {
        return Vec::new();
    }
    let query = morton_words(n, |a| coord_code(from[a]));
    let oracle = RadixOracle { keys };
    let probe = |word: u32| interleave(word, n, |a| coord_code(from[a]));
    // `fore` is the first record with code >= query.  When the query sits past every
    // record `fore` is empty, and then the LAST record is the nearest — which is the
    // case that used to answer nothing.
    let mut fore = rt::rtree_seek(store, tree, &probe, key_bits(keys), &oracle);
    let mut back = if fore.rec() == 0 {
        rt::rtree_last(store, tree)
    } else {
        let mut b = fore;
        b.prev(store, tree);
        b
    };
    let cap = limit.unwrap_or(usize::MAX);
    let mut out = Vec::new();
    while out.len() < cap {
        let gap = |rec: u32| {
            let code = morton_words(n, |a| axis_code(store, rec, &keys[a]));
            code_abs_diff(&code, &query, n)
        };
        match (back.rec(), fore.rec()) {
            (0, 0) => break,
            (b, 0) => {
                back.prev(store, tree);
                out.push(b);
            }
            (0, f) => {
                fore.next(store, tree);
                out.push(f);
            }
            (b, f) => {
                // Ties go to `back`, so a record sitting exactly on the query is yielded
                // before its forward neighbour rather than after it.
                if code_gt(&gap(b), &gap(f), n) {
                    fore.next(store, tree);
                    out.push(f);
                } else {
                    back.prev(store, tree);
                    out.push(b);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The bounding-box WALK — @PLN136
// ---------------------------------------------------------------------------

/// A [`TreeKeys`](rt::TreeKeys) source that can also answer a record's per-axis
/// codes — what a bounding-box walk needs and a seek does not.
///
/// The two answer different questions about the same record. `TreeKeys` yields the
/// COMPOSITE key a descent compares bit by bit; a box test wants each axis on its
/// own, and de-interleaving a composite to get there would decode 128 bits to
/// recover two coordinates the record stores side by side. The codes are
/// order-preserving ([`coord_code`]), so a box test is an unsigned compare per axis.
pub(crate) trait BoxNodes: rt::TreeKeys {
    /// Record `rec`'s `n` axis codes, offset-binary, axis 0 first.
    fn rec_axes(&mut self, rec: u32, n: usize) -> [u64; MAX_AXES];
}

/// [`BoxNodes`] over a resident tree — the counterpart of `rt::StoreKeys` for the
/// box walk.
pub(crate) struct StoreBox<'a> {
    nodes: rt::StoreNodes<'a>,
    store: &'a Store,
    keys: &'a [Key],
}

impl<'a> StoreBox<'a> {
    pub(crate) fn new(store: &'a Store, tree: u32, keys: &'a [Key]) -> Self {
        StoreBox {
            nodes: rt::StoreNodes::new(store, tree),
            store,
            keys,
        }
    }
}

impl rt::TreeNodes for StoreBox<'_> {
    fn top(&mut self) -> rt::Child {
        self.nodes.top()
    }
    fn len(&mut self) -> u32 {
        self.nodes.len()
    }
    fn node_bit(&mut self, n: u32) -> u32 {
        self.nodes.node_bit(n)
    }
    fn node_parent(&mut self, n: u32) -> u32 {
        self.nodes.node_parent(n)
    }
    fn child(&mut self, n: u32, dir: bool) -> rt::Child {
        self.nodes.child(n, dir)
    }
}

impl rt::TreeKeys for StoreBox<'_> {
    fn rec_bits(&mut self, _rec: u32) -> u32 {
        key_bits(self.keys)
    }
    fn rec_word(&mut self, rec: u32, word: u32) -> u64 {
        interleave(word, self.keys.len(), |a| {
            axis_code(self.store, rec, &self.keys[a])
        })
    }
}

impl BoxNodes for StoreBox<'_> {
    fn rec_axes(&mut self, rec: u32, n: usize) -> [u64; MAX_AXES] {
        let mut out = [0u64; MAX_AXES];
        for (a, slot) in out.iter_mut().enumerate().take(n) {
            *slot = axis_code(self.store, rec, &self.keys[a]);
        }
        out
    }
}

/// `v` with bit `p` set and every lower bit cleared — the smallest value agreeing
/// with `v` above `p` and having a `1` at `p`.
fn set_one_below_zero(v: u64, p: u32) -> u64 {
    let mask = if p == 63 {
        u64::MAX
    } else {
        (1 << (p + 1)) - 1
    };
    (v & !mask) | (1 << p)
}

/// `v` with bit `p` cleared and every lower bit set — the largest value agreeing
/// with `v` above `p` and having a `0` at `p`.
fn set_zero_below_one(v: u64, p: u32) -> u64 {
    let mask = if p == 63 {
        u64::MAX
    } else {
        (1 << (p + 1)) - 1
    };
    (v & !mask) | (mask >> 1)
}

/// The smallest code `>= cur` that is INSIDE the box, per axis — `None` when the
/// box holds nothing above `cur`.
///
/// The reason a box query is not a range query. Z-order visits a box's cells in
/// several disjoint runs, so a walk that reaches a record outside the box must know
/// where the next run STARTS; stepping to it costs the whole gap, and this computes
/// it instead. Tropf & Herzog's BIGMIN, 1981: read the composite bits from the most
/// significant, keeping each axis's live search bounds, and the last point at which
/// the box could still be re-entered from above is the answer.
///
/// Composite bit `b` is axis `b % n`'s bit at level `b / n`, which is the
/// interleave [`interleave`] writes; the terminator and id bits the tree appends
/// lie beyond `n * AXIS_BITS` and name no coordinate, so they end the scan.
///
/// **Why not prune on the path instead.** A subtree's records agree on every bit
/// its path SKIPPED as well as every bit the path tested, but the path does not say
/// what the skipped ones are — and near the root a PATRICIA tree skips exactly the
/// high-order bits every record shares. Bounds built from the tested bits alone
/// therefore stay the whole plane, reject nothing, and the walk degrades to a full
/// traversal that answers correctly. This reads its bounds off a RECORD, which
/// carries the skipped bits with it.
fn bigmin(
    cur: &[u64; MAX_AXES],
    n: usize,
    qlo: &[u64; MAX_AXES],
    qhi: &[u64; MAX_AXES],
) -> Option<[u64; MAX_AXES]> {
    let (mut lo, mut hi) = (*qlo, *qhi);
    let mut best: Option<[u64; MAX_AXES]> = None;
    for b in 0..AXIS_BITS * n as u32 {
        let a = (b as usize) % n;
        let p = AXIS_BITS - 1 - b / n as u32;
        let bit = |v: u64| (v >> p) & 1;
        match (bit(cur[a]), bit(lo[a]), bit(hi[a])) {
            // The box's bounds straddle this bit and the point is below it: the box
            // is re-enterable here, at the bottom of its upper half.
            (0, 0, 1) => {
                let mut cand = lo;
                cand[a] = set_one_below_zero(lo[a], p);
                best = Some(cand);
                hi[a] = set_zero_below_one(hi[a], p);
            }
            // The point is below the box on this axis and cannot climb back into it
            // any lower down: the box's own minimum is the answer.
            (0, 1, 1) => return Some(lo),
            // The point is above the box on this axis; nothing deeper re-enters, so
            // the last re-entry seen from above is all there is.
            (1, 0, 0) => return best,
            // Still inside the bounds — keep reading.
            (1, 0, 1) => lo[a] = set_one_below_zero(lo[a], p),
            _ => {}
        }
    }
    best
}

/// Every record inside the closed box `[qlo, qhi]` (per-axis codes), in Morton
/// order, capped at `cap`.
///
/// **The box decides the walk, not a filter after it.** The Morton code interval
/// between two corners is a strict superset of the box — Z-order threads out and
/// back — so seeking to one corner and walking to the other reads every record in
/// between and throws most of them away. Here a record outside the box does not
/// advance the walk by one step: [`bigmin`] says where the box resumes and the walk
/// SEEKS there, so the records in the gap are never read and neither are their
/// pages. That is the difference between a query that reads what it returns and one
/// that reads the whole index (@PLN136).
///
/// **The cap bounds the WALK.** The records come out in Morton order and the
/// `cap`-th is the last one anything is read for — the lesson @PLN134 pinned for the
/// paged prefix query, and the one place a paged spatial query could quietly become
/// a whole-image read.
///
/// The descent is `radix_tree`'s own ([`seek_gen`](rt::seek_gen)), so a query over
/// a paged image and one over resident memory cannot drift apart, and the source's
/// per-walk fuel is what bounds a hostile image. `steps` bounds the outer loop for
/// the same reason: every iteration either yields a record or seeks strictly
/// higher, so a healthy query never reaches it.
pub(crate) fn box_walk<S: BoxNodes>(
    src: &mut S,
    n: usize,
    qlo: &[u64; MAX_AXES],
    qhi: &[u64; MAX_AXES],
    cap: usize,
) -> Vec<u32> {
    let mut out = Vec::new();
    if cap == 0 || n == 0 || n > MAX_AXES {
        return out;
    }
    let bits = AXIS_BITS * n as u32;
    // A record's code compares against the box's far corner as a whole number, so
    // both sides are the interleave of the per-axis bounds.
    let hi_code = morton_words(n, |a| qhi[a]);
    // Where the current run starts: the box's own minimum first, then wherever
    // `bigmin` says the box resumes.
    let mut at = *qlo;
    let mut steps = src.len().saturating_add(4);
    'runs: loop {
        let probe = |w: u32| interleave(w, n, |a| at[a]);
        let (mut it, _) = rt::seek_gen(src, &probe, bits);
        let mut rec = it.rec();
        while rec != 0 {
            if out.len() >= cap || steps == 0 {
                return out;
            }
            steps -= 1;
            let axes = src.rec_axes(rec, n);
            if (0..n).all(|a| axes[a] >= qlo[a] && axes[a] <= qhi[a]) {
                out.push(rec);
                rec = it.step_gen(src, true).unwrap_or(0);
                continue;
            }
            // Outside the box. Past its far corner there is nothing left; otherwise
            // skip the gap rather than step through it.
            if code_gt(&morton_words(n, |a| axes[a]), &hi_code, n) {
                return out;
            }
            let Some(next) = bigmin(&axes, n, qlo, qhi) else {
                return out;
            };
            at = next;
            continue 'runs;
        }
        // The walk ran off the end of the tree.
        return out;
    }
}

/// A [`BoxNodes`] that remembers which RECORDS a walk read.
///
/// The node reads are already recorded, by `radix_tree`'s own touch window; a
/// record read is not, because nothing in the tree performs it. Counting them here
/// keeps both halves of a walk's cost coming out of ONE run, which is what makes
/// the node/record split a comparison rather than two measurements — and what lets
/// `d5_box_walk_skips_the_gaps` assert that the walk SKIPS, not merely that it
/// answers.
///
/// `keep` is what makes an UNCAPPED walk affordable: a province-wide box touches a
/// large slice of the index, and holding the ids of every record for every probe is
/// how a measurement of paging ends up as the memory event CLAUDE.md warns about.
#[cfg(test)]
pub(crate) struct Counting<'a> {
    inner: StoreBox<'a>,
    reads: Vec<u32>,
    read_count: usize,
    /// The record the last read landed on. A seek compares one candidate's key word
    /// after word and the box test then reads the same record's axes; a reader pays
    /// ONE page for all of it, so consecutive reads of one record are one read.
    last: u32,
    keep: bool,
}

#[cfg(test)]
impl<'a> Counting<'a> {
    pub(crate) fn new(inner: StoreBox<'a>, keep: bool) -> Self {
        Counting {
            inner,
            reads: Vec::new(),
            read_count: 0,
            last: 0,
            keep,
        }
    }

    /// How many distinct records the walk read.
    pub(crate) fn read_count(&self) -> usize {
        self.read_count
    }

    /// Which ones, when the walk was opened with `keep`.
    pub(crate) fn into_reads(self) -> Vec<u32> {
        self.reads
    }

    fn note(&mut self, rec: u32) {
        if rec == self.last {
            return;
        }
        self.last = rec;
        self.read_count += 1;
        if self.keep {
            self.reads.push(rec);
        }
    }
}

#[cfg(test)]
impl rt::TreeNodes for Counting<'_> {
    fn walk_begin(&mut self) {
        self.inner.walk_begin();
    }
    fn top(&mut self) -> rt::Child {
        self.inner.top()
    }
    fn len(&mut self) -> u32 {
        self.inner.len()
    }
    fn node_bit(&mut self, n: u32) -> u32 {
        self.inner.node_bit(n)
    }
    fn node_parent(&mut self, n: u32) -> u32 {
        self.inner.node_parent(n)
    }
    fn child(&mut self, n: u32, dir: bool) -> rt::Child {
        self.inner.child(n, dir)
    }
}

#[cfg(test)]
impl rt::TreeKeys for Counting<'_> {
    fn rec_bits(&mut self, rec: u32) -> u32 {
        self.inner.rec_bits(rec)
    }
    fn rec_word(&mut self, rec: u32, word: u32) -> u64 {
        // A seek's own key reads land on the same records a box test does, and a
        // reader pays a page for either — so they count the same.
        self.note(rec);
        self.inner.rec_word(rec, word)
    }
}

#[cfg(test)]
impl BoxNodes for Counting<'_> {
    fn rec_axes(&mut self, rec: u32, n: usize) -> [u64; MAX_AXES] {
        self.note(rec);
        self.inner.rec_axes(rec, n)
    }
}

/// The records inside the closed box with corners `from` and `till`, in Morton
/// order, capped at `limit` — the collection-level [`box_walk`].
///
/// A corner-swapped axis names the same interval, exactly as [`in_box`] reads it:
/// the box is what the caller asked for, and which corner they wrote first is not
/// part of it.
#[must_use]
pub fn box_range(
    coll: &DbRef,
    stores: &[Store],
    keys: &[Key],
    from: &[i64],
    till: &[i64],
    limit: Option<usize>,
) -> Vec<u32> {
    let store = keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        return Vec::new();
    }
    let n = keys.len();
    let (mut qlo, mut qhi) = ([0u64; MAX_AXES], [0u64; MAX_AXES]);
    for a in 0..n.min(MAX_AXES) {
        let (lo, hi) = if from[a] <= till[a] {
            (from[a], till[a])
        } else {
            (till[a], from[a])
        };
        qlo[a] = coord_code(lo);
        qhi[a] = coord_code(hi);
    }
    let mut src = StoreBox::new(store, tree, keys);
    box_walk(&mut src, n, &qlo, &qhi, limit.unwrap_or(usize::MAX))
}

/// @PLN136 step 1 — what a bounding-box query READS, counted in PAGES, before
/// anything is built on the answer.
///
/// @PLN134 measured the same thing for a prefix query and the numbers decided the
/// plan: 27 pages of node reads as built, 2.8 renumbered van Emde Boas. Its
/// motivation named `spatial` as the next consumer of the same geometry — and
/// @PLN136 exists because that is only half true. A prefix is a seek followed by
/// one contiguous run; a box over a Morton code is not one run, so the walk is a
/// different walk and its page count is a different number. This module takes it.
///
/// Four questions, one fixture:
///
/// * [`box_query_page_span`] — the node array, under five numberings, per box shape.
/// * [`interval_versus_box`] — what the code-interval walk reads that the box does
///   not, which is the question of whether pruning is needed at all.
/// * [`record_placement`] — the records a query reads, which live elsewhere.
/// * [`pan_session`] — what the second viewport costs, cache still warm.
///
/// The corpus is REAL: 3.2 M tagged OpenStreetMap nodes over the Benelux, since
/// clustering is the whole subject and a generated point set has whatever
/// clustering its generator produced. Point it at your own with
/// `LOFT_SPATIAL_POINTS`; the file is `x<TAB>y<TAB>name` with coordinates in units
/// of 1e-7 degrees, and the default path is `~/.cache/loft-spatial/points.tsv`.
/// To rebuild it from an OSM extract:
///
/// ```text
/// osmium tags-filter benelux-latest.osm.pbf \
///     n/amenity n/shop n/tourism n/leisure n/historic n/office n/craft \
///     n/natural n/man_made n/highway -o poi.osm.pbf
/// osmium cat -f opl poi.osm.pbf | scripts/opl_points.py > ~/.cache/loft-spatial/points.tsv
/// ```
///
/// `#[ignore]` because they read a host corpus and PRINT rather than assert — they
/// answer a design question, they do not defend an invariant. What defends the walk
/// is `d5_box_walk_is_exactly_the_box`.
///
/// ```text
/// cargo test --release --lib radix_db::pages -- --ignored --nocapture
/// ```
#[cfg(test)]
mod pages;

/// @PLN136 — the paged box query answers what the resident one answers, and reads a
/// small part of the image doing it.
///
/// The whole point of sharing `radix_tree`'s geometry is that there is ONE walk with
/// two byte sources. What the sources do not share is how a record's COORDINATES are
/// read: a resident query goes through `Store`'s checked accessors, a paged one
/// decodes the same bytes out of an image. Six integer widths, each with its own null
/// sentinel, and a disagreement on any one of them makes a record's Morton code
/// differ between the storages — which presents not as an error but as a box query
/// missing a point that is inside it. So every width is driven through both.
#[cfg(all(test, paged_store))]
mod paged;

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u64) -> i64 {
        *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (*seed >> 32) as i64
    }

    /// Two `integer` axes at payload offsets 0 and 8.
    fn xy_keys() -> Vec<Key> {
        vec![
            Key {
                type_nr: 1,
                position: 0,
                start: 0,
            },
            Key {
                type_nr: 1,
                position: 8,
                start: 0,
            },
        ]
    }

    /// Claim an element record and write its coordinates (payload at offset 8).
    fn add_point(store: &mut Store, coll: &DbRef, keys: &[Key], x: i64, y: i64) -> u32 {
        let rec = store.claim(3); // 24 bytes: header + 16-byte payload from offset 8
        store.set_int(rec, PAYLOAD, x);
        store.set_int(rec, PAYLOAD + 8, y);
        add(
            coll,
            &DbRef {
                store_nr: 0,
                rec,
                pos: PAYLOAD,
            },
            std::slice::from_mut(store),
            keys,
        );
        rec
    }

    /// D1 — the bridge round-trips: every inserted point is found by its key, the
    /// walk enumerates exactly the inserted records, and the order-preserving code
    /// is what makes `find` land on the right one.
    #[test]
    fn d1_insert_and_find_round_trip() {
        let mut store = Store::new_in_use(1 << 15);
        let keys = xy_keys();
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };

        let mut seed = 0x2024_1111_2222_3333;
        let mut pts = Vec::new();
        for _ in 0..400 {
            // Signed coordinates, including negatives, to exercise offset-binary.
            let (x, y) = (lcg(&mut seed) % 2000 - 1000, lcg(&mut seed) % 2000 - 1000);
            // Keep coordinates distinct so `find` has one answer.
            if pts.iter().any(|&(px, py, _)| px == x && py == y) {
                continue;
            }
            let rec = add_point(&mut store, &coll, &keys, x, y);
            pts.push((x, y, rec));
        }

        let stores = std::slice::from_ref(&store);
        for &(x, y, rec) in &pts {
            let found = find(&coll, stores, &keys, &[Content::Long(x), Content::Long(y)]);
            assert_eq!(found.rec, rec, "find({x},{y}) must reach its record");
            assert_eq!(found.pos, PAYLOAD);
        }

        // A coordinate never inserted is absent.
        let absent = find(
            &coll,
            stores,
            &keys,
            &[Content::Long(999_999), Content::Long(1)],
        );
        assert_eq!(absent.rec, 0, "an absent key returns null");

        let walked = records(&coll, stores);
        assert_eq!(
            walked.len(),
            pts.len(),
            "the walk enumerates every record once"
        );
        let unique: std::collections::HashSet<u32> = walked.iter().copied().collect();
        assert_eq!(unique.len(), pts.len(), "no record appears twice");
    }

    /// D2 — several records at the *same* bucket all stay (no dedup), adjacent in the
    /// walk: the per-bucket bucket @PLN48 needs.
    #[test]
    fn d2_same_cell_keeps_every_record() {
        let mut store = Store::new_in_use(1 << 13);
        let keys = xy_keys();
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };

        // Three entities at (5, 7), plus neighbours either side.
        add_point(&mut store, &coll, &keys, 1, 1);
        let bucket: Vec<u32> = (0..3)
            .map(|_| add_point(&mut store, &coll, &keys, 5, 7))
            .collect();
        add_point(&mut store, &coll, &keys, 9, 9);

        let stores = std::slice::from_ref(&store);
        assert_eq!(records(&coll, stores).len(), 5, "no record was dropped");

        // The bucket's records are contiguous in the walk.
        let walk = records(&coll, stores);
        let first = walk.iter().position(|r| bucket.contains(r)).unwrap();
        let run: Vec<u32> = walk[first..first + 3].to_vec();
        let mut got = run.clone();
        got.sort_unstable();
        let mut want = bucket.clone();
        want.sort_unstable();
        assert_eq!(
            got, want,
            "the three same-bucket records are one contiguous run"
        );
    }

    /// D4 — `range` returns exactly the records whose Morton code is in the interval,
    /// in Morton order, respecting the limit.
    #[test]
    fn d4_range_matches_the_code_interval() {
        let mut store = Store::new_in_use(1 << 15);
        let keys = xy_keys();
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };
        // The full 128-bit Morton code of a point, for the brute-force oracle.
        let code_of = |store: &Store, rec: u32| -> [u64; MAX_AXES] {
            morton_words(2, |a| axis_code(store, rec, &keys[a]))
        };
        let mut seed = 0x77aa_33cc_11ee_9988;
        let mut pts = Vec::new();
        for _ in 0..400 {
            let (x, y) = (lcg(&mut seed) % 400 - 200, lcg(&mut seed) % 400 - 200);
            let rec = add_point(&mut store, &coll, &keys, x, y);
            pts.push(rec);
        }
        let stores = std::slice::from_ref(&store);
        for _ in 0..100 {
            let (fx, fy) = (lcg(&mut seed) % 400 - 200, lcg(&mut seed) % 400 - 200);
            let from = morton_words(2, |a| coord_code([fx, fy][a]));
            let got = range(&coll, stores, &keys, &[fx, fy], None, None);
            // brute: records with code >= from, sorted by (code, rec).
            let mut want: Vec<([u64; MAX_AXES], u32)> = pts
                .iter()
                .map(|&rec| (code_of(&store, rec), rec))
                .filter(|(c, _)| !code_gt(&from, c, 2))
                .collect();
            want.sort_unstable();
            let want_recs: Vec<u32> = want.into_iter().map(|(_, r)| r).collect();
            assert_eq!(
                got, want_recs,
                "range from ({fx},{fy}) must be the code tail in order"
            );
        }
        let all = range(&coll, stores, &keys, &[-1000, -1000], None, None);
        let capped = range(&coll, stores, &keys, &[-1000, -1000], None, Some(5));
        assert_eq!(capped.len(), 5.min(all.len()));
        assert_eq!(
            capped,
            all[..capped.len()].to_vec(),
            "limit is a prefix of the full walk"
        );
    }

    /// D5 — @PLN136: the PRUNING box walk answers exactly the box, in exactly the
    /// order the code-interval walk plus a filter answers it.
    ///
    /// Two oracles, because they fail differently. Brute force over every record
    /// says the ANSWER is right and cannot be fooled by a shared bug in the tree;
    /// the `range` + [`in_box`] composition is the surface's behaviour TODAY, so
    /// agreement is what makes the pruning walk a replacement rather than a second
    /// answer. Boxes are drawn to straddle, contain and miss the point cloud, since
    /// a walk that pruned nothing would also pass a test where nothing is pruned.
    #[test]
    fn d5_box_walk_is_exactly_the_box() {
        let mut store = Store::new_in_use(1 << 15);
        let keys = xy_keys();
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };
        let code_of = |store: &Store, rec: u32| -> [u64; MAX_AXES] {
            morton_words(2, |a| axis_code(store, rec, &keys[a]))
        };
        let mut seed = 0x51ee_2c40_9a17_0031;
        let mut pts = Vec::new();
        for _ in 0..500 {
            let (x, y) = (lcg(&mut seed) % 400 - 200, lcg(&mut seed) % 400 - 200);
            pts.push((x, y, add_point(&mut store, &coll, &keys, x, y)));
        }
        let stores = std::slice::from_ref(&store);
        let mut nonempty = 0;
        for _ in 0..300 {
            let (cx, cy) = (lcg(&mut seed) % 500 - 250, lcg(&mut seed) % 500 - 250);
            let (rx, ry) = (lcg(&mut seed) % 90, lcg(&mut seed) % 90);
            let (from, till) = ([cx - rx, cy - ry], [cx + rx, cy + ry]);
            let got = box_range(&coll, stores, &keys, &from, &till, None);

            // Brute force: every record in the box, in (code, rec) order — which is
            // the order the tree walks, so the comparison is on the set AND the order.
            let mut want: Vec<([u64; MAX_AXES], u32)> = pts
                .iter()
                .filter(|&&(x, y, _)| x >= from[0] && x <= till[0] && y >= from[1] && y <= till[1])
                .map(|&(_, _, rec)| (code_of(&store, rec), rec))
                .collect();
            want.sort_unstable();
            let want: Vec<u32> = want.into_iter().map(|(_, r)| r).collect();
            assert_eq!(got, want, "box {from:?}..{till:?} must be exactly the box");
            nonempty += usize::from(!want.is_empty());

            // And the composition the surface uses today.
            let today: Vec<u32> = range(&coll, stores, &keys, &from, Some(&till), None)
                .into_iter()
                .filter(|&r| in_box(&store, r, &keys, &from, &till))
                .collect();
            assert_eq!(got, today, "pruning must not change the answer");

            // The cap is a prefix of the full answer, so it bounds the walk without
            // choosing different records.
            let cap = box_range(&coll, stores, &keys, &from, &till, Some(3));
            assert_eq!(cap, want[..want.len().min(3)].to_vec());
        }
        assert!(
            nonempty > 200,
            "only {nonempty} of 300 boxes held a record — the fixture is not exercising the walk"
        );

        // A corner-swapped axis names the same interval, as `in_box` reads it.
        let a = box_range(&coll, stores, &keys, &[-50, -50], &[50, 50], None);
        let b = box_range(&coll, stores, &keys, &[50, -50], &[-50, 50], None);
        assert_eq!(a, b, "which corner is written first is not part of the box");
        assert!(!a.is_empty(), "the fixture must have points in the middle");
    }

    /// D6 — @PLN136: the box walk SKIPS the gaps, rather than reading them and
    /// discarding what it read.
    ///
    /// `d5_box_walk_is_exactly_the_box` says the answer is right, and a walk that
    /// read every record in the index and filtered would pass it — which is the
    /// version this plan exists to replace. What has to be true is that the records
    /// READ are close to the records RETURNED, and far below the code interval the
    /// two corners span. A small box over a wide, clustered point set is where
    /// those three numbers separate.
    #[test]
    fn d6_box_walk_skips_the_gaps() {
        let mut store = Store::new_in_use(1 << 18);
        let keys = xy_keys();
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };
        // Clustered, like a real point set: 24 towns on a grid, 400 points each,
        // over a plane fifty times wider than one town.
        let mut seed = 0x0bad_c0de_1234_5678;
        for i in 0..3i64 {
            for j in 0..8i64 {
                let (cx, cy) = (i * 10_000 + 500, j * 10_000 + 500);
                for _ in 0..400 {
                    let (x, y) = (cx + lcg(&mut seed) % 200, cy + lcg(&mut seed) % 200);
                    add_point(&mut store, &coll, &keys, x, y);
                }
            }
        }
        let stores = std::slice::from_ref(&store);
        let tree = store.get_u32_raw(coll.rec, coll.pos);

        // A strip across the bottom row of towns — the degenerate shape, where the
        // interval between the corners threads through nearly the whole plane and
        // the box holds three towns' worth.
        let (from, till) = ([0i64, 400], [30_000i64, 750]);
        let hits = box_range(&coll, stores, &keys, &from, &till, None);
        let interval = range(&coll, stores, &keys, &from, Some(&till), None).len();

        let (mut qlo, mut qhi) = ([0u64; MAX_AXES], [0u64; MAX_AXES]);
        for a in 0..2 {
            qlo[a] = coord_code(from[a]);
            qhi[a] = coord_code(till[a]);
        }
        let mut src = Counting::new(StoreBox::new(&store, tree, &keys), false);
        let walked = box_walk(&mut src, 2, &qlo, &qhi, usize::MAX);
        assert_eq!(walked, hits, "the counted walk is the same walk");
        let read = src.read_count();

        let found = hits.len();
        assert!(
            interval > 3 * found.max(1),
            "the fixture must have a code interval much wider than the box \
             (interval {interval}, box {found}) — it is not exercising the skip"
        );
        // The floor is the answer itself; what is under test is the GAP, and more
        // than half of it must go unread.
        assert!(
            read < found + (interval - found) / 2,
            "the walk read {read} records for a box of {found} inside an interval \
             of {interval} — it is stepping through the gaps, not skipping them"
        );
    }

    /// D3 — the code is order-preserving    /// D3 — the code is order-preserving    /// D3 — the code is order-preserving even across zero: a negative axis must sort
    /// before a positive one, which raw 2's-complement bits would get backwards.
    #[test]
    fn d3_offset_binary_orders_negatives_below_positives() {
        assert!(axis_code_of(-1) < axis_code_of(0), "-1 must encode below 0");
        assert!(axis_code_of(0) < axis_code_of(1), "0 below 1");
        assert!(
            axis_code_of(i64::MIN) < axis_code_of(i64::MAX),
            "min below max"
        );
        // And a raw cast would NOT: (-1 as u64) is all-ones, the largest.
        assert!(
            (-1i64 as u64) > (1i64 as u64),
            "sanity: raw bits are misordered"
        );
    }

    fn axis_code_of(v: i64) -> u64 {
        (v as u64) ^ (1u64 << 63)
    }
}
