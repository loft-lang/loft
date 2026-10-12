// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures
//! The `index` collection as a B+tree of KEY PARTS and RECORD REFS (@PLN190).
//!
//! The red-black tree (`tree.rs`) threads its links through the element records, so every
//! step of a descent reads a different record and compares through the general comparator.
//! This tree keeps its nodes apart, in one ARENA record of the collection's store: a node
//! holds up to [`FANOUT`] slots, each an 8-byte KEY PART beside a record ref, and a descent
//! compares parts — a node's worth of them in one cache line or two — and reads a record
//! only where two parts tie.
//!
//! **The key part** is the first key field as an `i64` whose order IS the field's order:
//! the integer itself, a float's total-order bits, the first eight bytes of a text
//! (big-endian, so byte order is integer order), negated bitwise for a descending field.
//! Equal parts say nothing more than "the first field may be equal": the tie reads the
//! record and compares through `keys::compare` / `keys::key_compare`, which are the order
//! the collection is defined by.  A part is EXACT — a tie is equality — for a one-field key
//! that is not text, which is the case where no record is read at all.
//!
//! **Layout.**  The collection's slot holds the arena record.  Node `n` starts at byte
//! `8 + n·NODE`; node 0 is the header.  A leaf slot is `(part, record)`; an internal slot
//! `i ≥ 1` is `(part, record, child)` where the record is the MINIMUM of child `i` (its
//! separator), and slot 0 carries only child 0.  Leaves are chained both ways for stepping.
//! Nodes are numbered, never addressed, so the arena may move when it grows.
//!
//! **A position** names a leaf slot, `leaf·FANOUT + slot`; 0 is "no position" (node 0 is
//! the header, so no leaf slot encodes as 0).  A position is valid until the next insert or
//! remove — a cursor that survives a mutation holds a RECORD and finds it again
//! ([`position`]).
//!
//! No record points at its node (@C145): a removal finds its slot by descending with the
//! record's own key.
use crate::keys::{self, Content, DbRef, Key, Str};
use crate::store::Store;
use std::cmp::Ordering;

/// Slots in a node.  16 measured best of 8 / 16 / 32 / 64 (`tests/btree_probe.rs`).
pub const FANOUT: u32 = 16;
/// The fewest slots a node other than the root holds after a removal.
const MIN: u32 = FANOUT / 4;
/// No sound tree is this deep: every node but the root holds at least [`MIN`] slots, so a
/// tree of `u32::MAX` records is at most 16 levels.  A guard against a corrupt arena.
const MAX_HEIGHT: usize = 24;

// Node layout, bytes from the node's start.
const N_COUNT: u32 = 0;
/// 1 for a leaf, 0 for an internal node, 2 for a freed one (a `u16`).
const N_LEAF: u32 = 4;
/// A text index's SHARED PREFIX length (a `u16`): every key in the node starts with these
/// bytes, kept at `N_PREFIX`, and the node's parts are the eight bytes after them.  0 for
/// every other key type.  In the header, so a descent learns it from a line it reads anyway.
const N_SKIP: u32 = 6;
const N_NEXT: u32 = 8; // the next leaf, or the next free node
const N_PREV: u32 = 12;
const N_PART: u32 = 16;
const N_REC: u32 = N_PART + 8 * FANOUT;
const N_CHILD: u32 = N_REC + 4 * FANOUT;
/// The prefix bytes, behind the slots: read only by a node that keeps a prefix, so a scan of
/// the parts touches the cache lines it did without them.
const N_PREFIX: u32 = N_CHILD + 4 * FANOUT;
/// The longest prefix a node keeps; keys sharing more tie on their parts past it.
const MAX_SKIP: usize = 24;
/// Bytes per node, a multiple of 8.
const NODE: u32 = (N_PREFIX + MAX_SKIP as u32).div_ceil(8) * 8;

// Header fields (node 0).
const H_ROOT: u32 = 0;
const H_HEIGHT: u32 = 4;
const H_LEN: u32 = 8;
const H_USED: u32 = 12;
const H_FREE: u32 = 16;
const H_CAP: u32 = 20;
const H_FIRST: u32 = 24;
const H_LAST: u32 = 28;

/// Nodes a fresh arena has room for: the header and the root leaf.
const FIRST_CAP: u32 = 2;

const fn node_at(n: u32) -> u32 {
    8 + n * NODE
}

const fn arena_words(cap: u32) -> u32 {
    (8 + cap * NODE) / 8
}

/// One arena: its record in its store.  Every accessor takes the store, so an arena that
/// moved while growing is the same arena under its new record.
#[derive(Clone, Copy)]
struct Arena(u32);

impl Arena {
    #[inline]
    fn hdr(self, store: &Store, f: u32) -> u32 {
        store.read::<u32>(self.0, 8 + f)
    }
    #[inline]
    fn set_hdr(self, store: &mut Store, f: u32, v: u32) {
        store.write::<u32>(self.0, 8 + f, v);
    }
    #[inline]
    fn get(self, store: &Store, n: u32, f: u32) -> u32 {
        store.read::<u32>(self.0, node_at(n) + f)
    }
    #[inline]
    fn set(self, store: &mut Store, n: u32, f: u32, v: u32) {
        store.write::<u32>(self.0, node_at(n) + f, v);
    }
    #[inline]
    fn get16(self, store: &Store, n: u32, f: u32) -> u16 {
        store.read::<u16>(self.0, node_at(n) + f)
    }
    #[inline]
    fn set16(self, store: &mut Store, n: u32, f: u32, v: u16) {
        store.write::<u16>(self.0, node_at(n) + f, v);
    }
    #[inline]
    fn count(self, store: &Store, n: u32) -> u32 {
        self.get(store, n, N_COUNT)
    }
    #[inline]
    fn is_leaf(self, store: &Store, n: u32) -> bool {
        self.get16(store, n, N_LEAF) == 1
    }
    #[inline]
    fn part(self, store: &Store, n: u32, i: u32) -> i64 {
        store.read::<i64>(self.0, node_at(n) + N_PART + 8 * i)
    }
    #[inline]
    fn rec(self, store: &Store, n: u32, i: u32) -> u32 {
        store.read::<u32>(self.0, node_at(n) + N_REC + 4 * i)
    }
    #[inline]
    fn child(self, store: &Store, n: u32, i: u32) -> u32 {
        store.read::<u32>(self.0, node_at(n) + N_CHILD + 4 * i)
    }
    fn set_slot(self, store: &mut Store, n: u32, i: u32, part: i64, rec: u32) {
        store.write::<i64>(self.0, node_at(n) + N_PART + 8 * i, part);
        store.write::<u32>(self.0, node_at(n) + N_REC + 4 * i, rec);
    }
    fn set_child(self, store: &mut Store, n: u32, i: u32, child: u32) {
        store.write::<u32>(self.0, node_at(n) + N_CHILD + 4 * i, child);
    }
    fn slot(self, store: &Store, n: u32, i: u32) -> Slot {
        Slot {
            part: self.part(store, n, i),
            rec: self.rec(store, n, i),
            child: self.child(store, n, i),
        }
    }
    fn put(self, store: &mut Store, n: u32, i: u32, v: Slot) {
        self.set_slot(store, n, i, v.part, v.rec);
        self.set_child(store, n, i, v.child);
    }

    /// Move slots `from..from+len` of node `n` to start at `to` (overlap allowed): parts and
    /// records, and the children of an internal node (`children`).
    #[allow(clippy::fn_params_excessive_bools)]
    fn shift(self, store: &mut Store, n: u32, from: u32, to: u32, len: u32, children: bool) {
        if len == 0 || from == to {
            return;
        }
        let b = node_at(n) as isize;
        let (from, to, len) = (from as isize, to as isize, len as isize);
        store.copy_block(
            self.0,
            b + N_PART as isize + 8 * from,
            self.0,
            b + N_PART as isize + 8 * to,
            8 * len,
        );
        store.copy_block(
            self.0,
            b + N_REC as isize + 4 * from,
            self.0,
            b + N_REC as isize + 4 * to,
            4 * len,
        );
        if !children {
            return;
        }
        store.copy_block(
            self.0,
            b + N_CHILD as isize + 4 * from,
            self.0,
            b + N_CHILD as isize + 4 * to,
            4 * len,
        );
    }

    /// The position after (`back`: before) leaf position `pos`, or 0 off either end.
    #[inline]
    fn step(self, store: &Store, pos: u32, back: bool) -> u32 {
        let (n, i) = (pos / FANOUT, pos % FANOUT);
        if back {
            if i > 0 {
                return pos - 1;
            }
            match self.get(store, n, N_PREV) {
                0 => 0,
                p => p * FANOUT + self.count(store, p) - 1,
            }
        } else if i + 1 < self.count(store, n) {
            pos + 1
        } else {
            self.get(store, n, N_NEXT) * FANOUT
        }
    }

    fn init_node(self, store: &mut Store, n: u32, leaf: bool) {
        self.set(store, n, N_COUNT, 0);
        self.set16(store, n, N_LEAF, u16::from(leaf));
        self.set(store, n, N_NEXT, 0);
        self.set(store, n, N_PREV, 0);
        self.set16(store, n, N_SKIP, 0);
    }

    /// The shared prefix of node `n`: its length and its bytes (only the first `skip` mean
    /// anything).
    #[inline]
    fn prefix(self, store: &Store, n: u32) -> (usize, [u8; MAX_SKIP]) {
        let skip = usize::from(self.get16(store, n, N_SKIP));
        if skip == 0 {
            return (0, [0; MAX_SKIP]);
        }
        (
            skip,
            store.read::<[u8; MAX_SKIP]>(self.0, node_at(n) + N_PREFIX),
        )
    }

    fn set_prefix(self, store: &mut Store, n: u32, skip: usize, bytes: &[u8]) {
        let mut buf = [0u8; MAX_SKIP];
        buf[..skip].copy_from_slice(&bytes[..skip]);
        self.set16(store, n, N_SKIP, skip as u16);
        store.write::<[u8; MAX_SKIP]>(self.0, node_at(n) + N_PREFIX, buf);
    }
}

/// One slot's contents, for the rare rebalancing moves that gather a node's slots.
#[derive(Clone, Copy, Default)]
struct Slot {
    part: i64,
    rec: u32,
    child: u32,
}

/// The order-preserving bits of a float, as `f64::total_cmp` orders them after the
/// `-0.0 → 0.0` normalisation `keys::float_cmp` applies (@FR-E-Eq-Key).
fn float_part(v: f64) -> i64 {
    let v = if v == 0.0 { 0.0 } else { v };
    let b = v.to_bits() as i64;
    b ^ ((((b >> 63) as u64) >> 1) as i64)
}

fn single_part(v: f32) -> i64 {
    let v = if v == 0.0 { 0.0 } else { v };
    let b = v.to_bits() as i32;
    i64::from(b ^ ((((b >> 31) as u32) >> 1) as i32))
}

/// The first eight bytes of a text, zero-padded, big-endian with the sign bit flipped: a
/// text that orders below another never has the greater part.  A tie says nothing — a
/// shorter text and one with a NUL there pad alike — so text parts are never exact.
fn text_part(t: &str) -> i64 {
    text_part_at(t.as_bytes(), 0)
}

/// [`text_part`] of the bytes from `skip` on: the part a node with that shared prefix keeps.
/// Between two texts that share their first `skip` bytes it orders as the texts do.
fn text_part_at(b: &[u8], skip: usize) -> i64 {
    let b = b.get(skip..).unwrap_or(&[]);
    let mut w = [0u8; 8];
    let n = b.len().min(8);
    w[..n].copy_from_slice(&b[..n]);
    (u64::from_be_bytes(w) ^ (1 << 63)) as i64
}

/// The text of record `rec`'s first key field (a text index).
fn key_text<'s>(store: &'s Store, rec: u32, k: &Key) -> &'s [u8] {
    let s: &'s str = store.get_str(store.get_u32_raw(rec, 8 + u32::from(k.position)));
    s.as_bytes()
}

/// Is the first key field of this index a text — the one kind whose nodes keep a prefix?
fn is_text(keys: &[Key]) -> bool {
    keys[0].type_nr.abs() == 6
}

/// The part record `rec` takes in a node whose shared prefix is `skip` bytes long.
fn part_in(store: &Store, rec: u32, keys: &[Key], skip: usize) -> i64 {
    if skip == 0 {
        return record_part(store, rec, &keys[0]);
    }
    let v = text_part_at(key_text(store, rec, &keys[0]), skip);
    if keys[0].type_nr < 0 { !v } else { v }
}

/// The part of a record's first key field — the read `keys::compare_ref` makes for it,
/// arm for arm, mapped to an order-preserving `i64`.
fn record_part(store: &Store, rec: u32, k: &Key) -> i64 {
    let at = 8 + u32::from(k.position);
    let v = match k.type_nr.abs() {
        1 => store.get_int(rec, at),
        2 => store.get_long(rec, at),
        3 => single_part(store.get_single(rec, at)),
        4 => float_part(store.get_float(rec, at)),
        6 => text_part(store.get_str(store.get_u32_raw(rec, at))),
        8 => i64::from(store.get_i32_raw(rec, at)),
        12 => i64::from(store.get_u32_raw(rec, at)),
        9 => i64::from(store.get_short(rec, at, k.start)),
        11 => i64::from(store.get_short_full(rec, at, k.start)),
        _ => i64::from(store.get_byte(rec, at, k.start)),
    };
    if k.type_nr < 0 { !v } else { v }
}

/// The part of a lookup key's first value, or `None` where its `Content` is not the kind
/// [`record_part`] reads for that width — the probe then compares every slot in full, so
/// it answers exactly what `keys::key_compare` answers.
fn content_part(c: &Content, k: &Key) -> Option<i64> {
    let v = match (c, k.type_nr.abs()) {
        (Content::Single(v), 3) => single_part(*v),
        (Content::Float(v), 4) => float_part(*v),
        (Content::Str(v), 6) => text_part(v.str()),
        (Content::Long(v), t) if t != 3 && t != 4 && t != 6 => *v,
        _ => return None,
    };
    Some(if k.type_nr < 0 { !v } else { v })
}

/// What a search orders slots against: a record's own key (an insert, a removal) or a
/// lookup key, with its part resolved once.
struct Probe<'a> {
    part: Option<i64>,
    /// The first key value as bytes, for a text index: a node with a shared prefix compares
    /// the probe's own prefix first and takes its part past it.
    text: Option<Str>,
    /// The text's first `MAX_SKIP + 8` bytes, zero-padded, and its length: what that
    /// comparison and that part read, as whole words.
    head: [u8; MAX_SKIP + 8],
    len: usize,
    /// A tie of parts is equality: a one-value key on a field that is not text.
    exact: bool,
    by: By<'a>,
    store_nr: u16,
}

enum By<'a> {
    Rec(u32),
    Key(&'a [Content]),
}

impl<'a> Probe<'a> {
    fn of_record(store: &Store, store_nr: u16, rec: u32, keys: &[Key]) -> Probe<'a> {
        let text = is_text(keys).then(|| {
            Str::new(store.get_str(store.get_u32_raw(rec, 8 + u32::from(keys[0].position))))
        });
        let (head, len) = head_of(text.as_ref());
        Probe {
            part: Some(record_part(store, rec, &keys[0])),
            text,
            head,
            len,
            exact: keys.len() == 1 && keys[0].type_nr.abs() != 6,
            by: By::Rec(rec),
            store_nr,
        }
    }

    fn of_key(store_nr: u16, key: &'a [Content], keys: &[Key]) -> Probe<'a> {
        let part = key.first().and_then(|c| content_part(c, &keys[0]));
        let text = match key.first() {
            Some(Content::Str(s)) if is_text(keys) => Some(s.clone()),
            _ => None,
        };
        let (head, len) = head_of(text.as_ref());
        Probe {
            part,
            text,
            head,
            len,
            exact: part.is_some() && key.len() == 1 && keys[0].type_nr.abs() != 6,
            by: By::Key(key),
            store_nr,
        }
    }

    /// The general comparison against record `rec`, for a tie of parts.
    #[inline(never)]
    fn full(&self, rec: u32, stores: &[Store], keys: &[Key]) -> Ordering {
        let other = DbRef {
            store_nr: self.store_nr,
            rec,
            pos: 8,
        };
        match self.by {
            By::Rec(r) => keys::compare(
                &DbRef {
                    store_nr: self.store_nr,
                    rec: r,
                    pos: 8,
                },
                &other,
                stores,
                keys,
            ),
            By::Key(k) => keys::key_compare(k, &other, stores, keys),
        }
    }
}

/// The path a descent took: per level the node and the child it went into.
struct Path {
    steps: [(u32, u32); MAX_HEIGHT],
    leaf: u32,
}

/// Descend from the root to the leaf the probe belongs in.  At a separator equal to the
/// probe it goes RIGHT when `ties_right` (the records equal to it start there) and left
/// otherwise (records equal to it may end before it — a partial key).
fn descend(
    arena: Arena,
    stores: &[Store],
    st: usize,
    keys: &[Key],
    probe: &Probe,
    ties_right: bool,
) -> Path {
    let store = &stores[st];
    let height = arena.hdr(store, H_HEIGHT) as usize;
    assert!(height < MAX_HEIGHT, "index tree: corrupt height {height}");
    let mut path = Path {
        steps: [(0, 0); MAX_HEIGHT],
        leaf: 0,
    };
    let mut n = arena.hdr(store, H_ROOT);
    for step in path.steps.iter_mut().take(height) {
        let i = child_index(arena, stores, st, keys, n, probe, ties_right);
        *step = (n, i);
        n = arena.child(store, n, i);
    }
    path.leaf = n;
    path
}

/// [`descend`] for a lookup, which needs no path: the leaf alone.
fn leaf_of(
    arena: Arena,
    stores: &[Store],
    st: usize,
    keys: &[Key],
    probe: &Probe,
    ties_right: bool,
) -> u32 {
    let store = &stores[st];
    let mut n = arena.hdr(store, H_ROOT);
    for _ in 0..arena.hdr(store, H_HEIGHT).min(MAX_HEIGHT as u32) {
        n = arena.child(
            store,
            n,
            child_index(arena, stores, st, keys, n, probe, ties_right),
        );
    }
    n
}

/// Which child of internal node `n` the probe descends into: past every separator below
/// it, and past an equal one when `ties_right`.
// Measured: as a hint the descent kept a call per node (`tests/btree_probe.rs`).
#[allow(clippy::inline_always)]
#[inline(always)]
fn child_index(
    arena: Arena,
    stores: &[Store],
    st: usize,
    keys: &[Key],
    n: u32,
    probe: &Probe,
    ties_right: bool,
) -> u32 {
    stop_at(arena, stores, st, keys, n, 1, probe, ties_right).0 - 1
}

/// The first slot from `from` of node `n` where the probe stops — the probe is below it,
/// or equal to it and `!ties_right` — and whether that slot equals the probe (answered
/// for `!ties_right` only).
///
/// Parts ascend, so the slots split in three runs: parts below the probe's (passed), parts
/// equal to it, parts above (stopped at).  The first two are integer scans; only the equal
/// run needs the records, and it is BINARY-searched — a text index whose keys share their
/// first eight bytes ties across whole nodes, and compared slot by slot it read a record
/// at every one.  A probe without a part has one run: the whole node.
#[allow(clippy::too_many_arguments, clippy::inline_always)]
#[inline(always)]
fn stop_at(
    arena: Arena,
    stores: &[Store],
    st: usize,
    keys: &[Key],
    n: u32,
    from: u32,
    probe: &Probe,
    ties_right: bool,
) -> (u32, bool) {
    let store = &stores[st];
    let c = arena.count(store, n);
    let (mut lo, mut hi) = (from, c);
    let mut part = probe.part;
    if probe.text.is_some() && part.is_some() && arena.get16(store, n, N_SKIP) != 0 {
        match text_order(arena, store, n, probe, keys) {
            TextOrder::Whole => {}
            TextOrder::Part(v) => part = Some(v),
            TextOrder::Below => return (from, false),
            TextOrder::Above => return (c, false),
        }
    }
    if let (Some(p), true) = (part, probe.exact) {
        // A tie of exact parts is equality, so one integer scan settles it.
        while lo < c {
            let q = arena.part(store, n, lo);
            if p < q || (p == q && !ties_right) {
                return (lo, p == q);
            }
            lo += 1;
        }
        return (c, false);
    }
    if let Some(p) = part {
        while lo < c && arena.part(store, n, lo) < p {
            lo += 1;
        }
        hi = lo;
        while hi < c && arena.part(store, n, hi) == p {
            hi += 1;
        }
    }
    let mut equal = false;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let o = probe.full(arena.rec(store, n, mid), stores, keys);
        if o == Ordering::Less || (o == Ordering::Equal && !ties_right) {
            hi = mid;
            equal = o == Ordering::Equal;
        } else {
            lo = mid + 1;
        }
    }
    // `lo` is the last `mid` that stopped, whose `equal` was recorded, or the run's end.
    (lo, equal)
}

/// A text's first `MAX_SKIP + 8` bytes, zero-padded, and its length.
fn head_of(text: Option<&Str>) -> ([u8; MAX_SKIP + 8], usize) {
    let mut head = [0u8; MAX_SKIP + 8];
    let Some(t) = text else {
        return (head, 0);
    };
    let b = t.str().as_bytes();
    let n = b.len().min(head.len());
    head[..n].copy_from_slice(&b[..n]);
    (head, b.len())
}

/// Word `k` (eight bytes, big-endian) of `bytes`, keeping only those below `skip`.
#[inline]
fn word_below(bytes: &[u8], k: usize, skip: usize) -> u64 {
    let keep = skip.saturating_sub(8 * k).min(8);
    let w = u64::from_be_bytes(bytes[8 * k..8 * k + 8].try_into().unwrap_or([0; 8]));
    if keep == 8 {
        w
    } else {
        w & !(u64::MAX >> (8 * keep))
    }
}

/// Where a text probe stands against node `n`'s shared prefix.
enum TextOrder {
    /// The node keeps no prefix: the probe's whole part orders it.
    Whole,
    /// The probe shares the prefix; this is its part past it.
    Part(i64),
    /// The probe is below every key in the node, in tree order.
    Below,
    /// The probe is above every key in the node.
    Above,
}

/// Every key in node `n` starts with its prefix: a probe that does not lies below or above
/// them all, and one that does orders by the bytes past it.  Out of line, so the descent of
/// every other key type does not carry it.
#[inline(never)]
fn text_order(arena: Arena, store: &Store, n: u32, probe: &Probe, keys: &[Key]) -> TextOrder {
    let (skip, prefix) = arena.prefix(store, n);
    if skip == 0 || probe.text.is_none() {
        return TextOrder::Whole;
    }
    let desc = keys[0].type_nr < 0;
    let order = if probe.len >= skip {
        // Whole words, masked to the prefix: the probe's head is zero-padded past its end,
        // and it is at least `skip` long here, so the padding never meets the mask.
        (0..skip.div_ceil(8))
            .map(|k| word_below(&probe.head, k, skip).cmp(&word_below(&prefix, k, skip)))
            .find(|o| *o != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    } else {
        // Shorter than the prefix: below every key if it is a prefix of theirs.
        let b = &probe.head[..probe.len];
        b.cmp(&prefix[..probe.len]).then(Ordering::Less)
    };
    match order {
        Ordering::Equal => {
            let v = u64::from_be_bytes(probe.head[skip..skip + 8].try_into().unwrap_or([0; 8]));
            let v = (v ^ (1 << 63)) as i64;
            TextOrder::Part(if desc { !v } else { v })
        }
        o if (o == Ordering::Less) != desc => TextOrder::Below,
        _ => TextOrder::Above,
    }
}

/// The first slot of leaf `n` above the probe (`ties_right`) or at-or-above it, and whether
/// the slot there equals the probe.
fn leaf_slot(
    arena: Arena,
    stores: &[Store],
    st: usize,
    keys: &[Key],
    n: u32,
    probe: &Probe,
    ties_right: bool,
) -> (u32, bool) {
    stop_at(arena, stores, st, keys, n, 0, probe, ties_right)
}

fn arena_of(data: &DbRef, stores: &[Store]) -> Option<Arena> {
    if data.is_null() {
        return None;
    }
    match keys::store(data, stores).collection_rec(data.rec, data.pos) {
        0 => None,
        arena => Some(Arena(arena)),
    }
}

fn create(store: &mut Store, data: &DbRef) -> Arena {
    let arena = Arena(store.claim(arena_words(FIRST_CAP)));
    for f in [H_HEIGHT, H_LEN, H_FREE] {
        arena.set_hdr(store, f, 0);
    }
    arena.set_hdr(store, H_CAP, FIRST_CAP);
    arena.set_hdr(store, H_USED, 2);
    arena.init_node(store, 1, true);
    for f in [H_ROOT, H_FIRST, H_LAST] {
        arena.set_hdr(store, f, 1);
    }
    store.set_u32_raw(data.rec, data.pos, arena.0);
    arena
}

/// A fresh node, from the free list or the arena's unused tail; the arena doubles when it
/// is full, which may MOVE its record — the collection's slot is rewritten and `a` updated.
fn alloc(store: &mut Store, data: &DbRef, arena: &mut Arena, leaf: bool) -> u32 {
    let free = arena.hdr(store, H_FREE);
    let n = if free != 0 {
        let next = arena.get(store, free, N_NEXT);
        arena.set_hdr(store, H_FREE, next);
        free
    } else {
        let used = arena.hdr(store, H_USED);
        let cap = arena.hdr(store, H_CAP);
        if used == cap {
            let grown = cap * 2;
            *arena = Arena(store.resize(arena.0, arena_words(grown)));
            store.set_u32_raw(data.rec, data.pos, arena.0);
            arena.set_hdr(store, H_CAP, grown);
        }
        arena.set_hdr(store, H_USED, used + 1);
        used
    };
    arena.init_node(store, n, leaf);
    n
}

fn release(store: &mut Store, arena: Arena, n: u32) {
    arena.set16(store, n, N_LEAF, 2);
    let free = arena.hdr(store, H_FREE);
    arena.set(store, n, N_NEXT, free);
    arena.set_hdr(store, H_FREE, n);
}

/// The slots of node `n` that carry a key: all of a leaf's, an internal node's from 1.
fn keyed_slots(store: &Store, arena: Arena, n: u32) -> std::ops::Range<u32> {
    let c = arena.count(store, n);
    if arena.is_leaf(store, n) {
        0..c
    } else {
        1.min(c)..c
    }
}

/// Make node `n`'s shared prefix the one its keys have now — the common prefix of its first
/// and last key, which is every key's, since they are sorted — and restamp every part when
/// that changed, or always with `force` (slots moved in from a node with another prefix).
/// A text index only: every other index keeps whole parts.
#[inline]
fn fit(store: &mut Store, arena: Arena, n: u32, keys: &[Key], force: bool) {
    if is_text(keys) {
        fit_text(store, arena, n, keys, force);
    }
}

#[inline(never)]
fn fit_text(store: &mut Store, arena: Arena, n: u32, keys: &[Key], force: bool) {
    let slots = keyed_slots(store, arena, n);
    let (old, prefix) = arena.prefix(store, n);
    let (cp, bytes) = if slots.len() < 2 {
        (0, Vec::new())
    } else {
        let a = key_text(store, arena.rec(store, n, slots.start), &keys[0]);
        let b = key_text(store, arena.rec(store, n, slots.end - 1), &keys[0]);
        let cp = a
            .iter()
            .zip(b)
            .take_while(|(x, y)| x == y)
            .count()
            .min(MAX_SKIP);
        (cp, a[..cp].to_vec())
    };
    // A prefix pays only where parts TIE — a tie reads records — and costs a check in every
    // descent through the node, and changing it restamps every part.  So a node takes or
    // lengthens its prefix only once its parts tie at the one it keeps; slots moved in from
    // elsewhere take the common prefix outright.  (Without a move, the keys still share the
    // prefix kept, so it never has to shrink here; `place` shrinks it for a key that does not.)
    // "Tie" means common: a quarter of the slots equal to their neighbour.  One tied pair
    // costs a descent that lands in it a record read or two; a prefix costs every descent.
    let ties = || {
        let tied = slots
            .clone()
            .skip(1)
            .filter(|&i| arena.part(store, n, i) == arena.part(store, n, i - 1))
            .count();
        tied >= 2 && tied * 4 >= slots.len()
    };
    let skip = if force || (old > 0 && cp < old) || ties() {
        cp
    } else {
        old
    };
    if !force && old == skip && prefix[..skip] == bytes[..skip] {
        return;
    }
    arena.set_prefix(store, n, skip, &bytes);
    for i in slots {
        let rec = arena.rec(store, n, i);
        let part = part_in(store, rec, keys, skip);
        store.write::<i64>(arena.0, node_at(n) + N_PART + 8 * i, part);
    }
}

/// Write `rec` into slot `i` of node `n` (the slot already shifted open), with the part this
/// node keeps for it.  A key outside the node's prefix widens the prefix ([`fit`]).
#[inline]
fn place(store: &mut Store, arena: Arena, n: u32, i: u32, rec: u32, part: i64, keys: &[Key]) {
    if is_text(keys) {
        place_text(store, arena, n, i, rec, keys);
    } else {
        arena.set_slot(store, n, i, part, rec);
    }
}

#[inline(never)]
fn place_text(store: &mut Store, arena: Arena, n: u32, i: u32, rec: u32, keys: &[Key]) {
    let (skip, prefix) = arena.prefix(store, n);
    let t = key_text(store, rec, &keys[0]);
    if t.len() >= skip && t[..skip] == prefix[..skip] {
        let v = text_part_at(t, skip);
        let part = if keys[0].type_nr < 0 { !v } else { v };
        arena.set_slot(store, n, i, part, rec);
    } else {
        arena.set_slot(store, n, i, 0, rec);
        fit(store, arena, n, keys, true);
    }
}

/// Add record `rec`, which carries its key, to the index at `data`.
///
/// Answers 0 when the record went in (or was already in), and otherwise the record that
/// already carries its key — the tree is then UNCHANGED, so the caller can displace that record and add again
/// (@FR-Col-Insert), exactly as `tree::add` answers.
pub fn add(data: &DbRef, rec: u32, stores: &mut [Store], keys: &[Key]) -> u32 {
    let st = data.store_nr as usize;
    let mut arena = match arena_of(data, stores) {
        Some(arena) => arena,
        None => create(&mut stores[st], data),
    };
    let probe = Probe::of_record(&stores[st], data.store_nr, rec, keys);
    let path = descend(arena, stores, st, keys, &probe, true);
    let (slot, equal) = leaf_slot(arena, stores, st, keys, path.leaf, &probe, false);
    if equal {
        // The record itself is already filed: nothing to add and nothing displaced, as
        // `tree::add` answered.
        let held = arena.rec(&stores[st], path.leaf, slot);
        return if held == rec { 0 } else { held };
    }
    let part = probe.part.unwrap_or_default();
    let store = &mut stores[st];
    let mut up = insert_leaf(store, data, &mut arena, path.leaf, slot, part, rec, keys);
    let height = arena.hdr(store, H_HEIGHT) as usize;
    let mut level = height;
    while let Some(sep) = up {
        if level == 0 {
            let root = alloc(store, data, &mut arena, false);
            let old = arena.hdr(store, H_ROOT);
            arena.put(
                store,
                root,
                0,
                Slot {
                    part: 0,
                    rec: 0,
                    child: old,
                },
            );
            arena.set(store, root, N_COUNT, 2);
            arena.set_child(store, root, 1, sep.child);
            place(store, arena, root, 1, sep.rec, sep.part, keys);
            arena.set_hdr(store, H_ROOT, root);
            arena.set_hdr(store, H_HEIGHT, height as u32 + 1);
            break;
        }
        level -= 1;
        let (p, i) = path.steps[level];
        up = insert_internal(store, data, &mut arena, p, i + 1, sep, keys);
    }
    let len = arena.hdr(store, H_LEN);
    arena.set_hdr(store, H_LEN, len + 1);
    0
}

/// Put `(part, rec)` at `slot` of leaf `n`; answers the separator slot for the parent when
/// the leaf split.  `part` is the record's whole part; a text node restamps it for its prefix.
#[allow(clippy::too_many_arguments)]
fn insert_leaf(
    store: &mut Store,
    data: &DbRef,
    arena: &mut Arena,
    n: u32,
    slot: u32,
    part: i64,
    rec: u32,
    keys: &[Key],
) -> Option<Slot> {
    let c = arena.count(store, n);
    if c < FANOUT {
        arena.shift(store, n, slot, slot + 1, c - slot, false);
        arena.set(store, n, N_COUNT, c + 1);
        place(store, *arena, n, slot, rec, part, keys);
        return None;
    }
    let r = alloc(store, data, arena, true);
    // The new leaf takes the old one's prefix with the slots it copies, then each half fits
    // its own (a half shares at least what the whole did).
    if is_text(keys) {
        let (skip, prefix) = arena.prefix(store, n);
        arena.set_prefix(store, r, skip, &prefix);
    }
    let half = FANOUT / 2;
    for i in half..FANOUT {
        let v = arena.slot(store, n, i);
        arena.put(store, r, i - half, v);
    }
    arena.set(store, n, N_COUNT, half);
    arena.set(store, r, N_COUNT, FANOUT - half);
    let next = arena.get(store, n, N_NEXT);
    arena.set(store, r, N_NEXT, next);
    arena.set(store, r, N_PREV, n);
    arena.set(store, n, N_NEXT, r);
    if next == 0 {
        arena.set_hdr(store, H_LAST, r);
    } else {
        arena.set(store, next, N_PREV, r);
    }
    let (into, at) = if slot <= half {
        (n, slot)
    } else {
        (r, slot - half)
    };
    let c = arena.count(store, into);
    arena.shift(store, into, at, at + 1, c - at, false);
    arena.set(store, into, N_COUNT, c + 1);
    place(store, *arena, into, at, rec, part, keys);
    fit(store, *arena, n, keys, false);
    fit(store, *arena, r, keys, false);
    // The separator travels with its WHOLE part: the parent restamps it for its own prefix.
    let first = arena.rec(store, r, 0);
    Some(Slot {
        part: if is_text(keys) {
            0
        } else {
            arena.part(store, r, 0)
        },
        rec: first,
        child: r,
    })
}

/// Put separator slot `sep` at `at` of internal node `n`; answers the separator for the
/// parent when the node split.
fn insert_internal(
    store: &mut Store,
    data: &DbRef,
    arena: &mut Arena,
    n: u32,
    at: u32,
    sep: Slot,
    keys: &[Key],
) -> Option<Slot> {
    let c = arena.count(store, n);
    if c < FANOUT {
        arena.shift(store, n, at, at + 1, c - at, true);
        arena.set(store, n, N_COUNT, c + 1);
        arena.set_child(store, n, at, sep.child);
        place(store, *arena, n, at, sep.rec, sep.part, keys);
        return None;
    }
    let mut all = [Slot::default(); FANOUT as usize + 1];
    for i in 0..c {
        let v = arena.slot(store, n, i);
        all[(if i < at { i } else { i + 1 }) as usize] = v;
    }
    all[at as usize] = sep;
    let r = alloc(store, data, arena, false);
    let keep = FANOUT.div_ceil(2);
    for (i, v) in all.iter().enumerate() {
        let i = i as u32;
        if i < keep {
            arena.put(store, n, i, *v);
        } else {
            arena.put(store, r, i - keep, *v);
        }
    }
    arena.set(store, n, N_COUNT, keep);
    arena.set(store, r, N_COUNT, FANOUT + 1 - keep);
    // The incoming separator's part is whole, not this node's: every slot is restamped.
    fit(store, *arena, n, keys, true);
    fit(store, *arena, r, keys, true);
    let up = all[keep as usize];
    Some(Slot {
        part: if is_text(keys) { 0 } else { up.part },
        rec: up.rec,
        child: r,
    })
}

/// Remove record `rec` from the index at `data`; answers whether it was there.
///
/// The record is found by descending with its OWN key, so its key fields must still hold
/// what they held when it was added.
pub fn remove(data: &DbRef, rec: u32, stores: &mut [Store], keys: &[Key]) -> bool {
    let st = data.store_nr as usize;
    let Some(arena) = arena_of(data, stores) else {
        return false;
    };
    let probe = Probe::of_record(&stores[st], data.store_nr, rec, keys);
    let path = descend(arena, stores, st, keys, &probe, true);
    let (slot, equal) = leaf_slot(arena, stores, st, keys, path.leaf, &probe, false);
    let store = &mut stores[st];
    if !equal || arena.rec(store, path.leaf, slot) != rec {
        return false;
    }
    let height = arena.hdr(store, H_HEIGHT) as usize;
    // The one separator that can name `rec`: where the descent went right of an equal one.
    let named = path.steps[..height]
        .iter()
        .copied()
        .find(|&(n, i)| i > 0 && arena.rec(store, n, i) == rec);
    let n = path.leaf;
    let c = arena.count(store, n);
    // The successor is the subtree's new minimum whenever the subtree keeps a record.
    let successor = if slot + 1 < c {
        Some((
            arena.part(store, n, slot + 1),
            arena.rec(store, n, slot + 1),
        ))
    } else {
        match arena.get(store, n, N_NEXT) {
            0 => None,
            nx => Some((arena.part(store, nx, 0), arena.rec(store, nx, 0))),
        }
    };
    arena.shift(store, n, slot + 1, slot, c - slot - 1, false);
    arena.set(store, n, N_COUNT, c - 1);
    if let (Some((p, i)), Some((part, r))) = (named, successor) {
        // When the subtree held only `rec` there is no successor INSIDE it; it is a single
        // leaf then, and the rebalance below drops it together with this separator.  A text
        // successor's part is its leaf's; the parent restamps it for its own prefix.
        place(store, arena, p, i, r, part, keys);
    }
    let len = arena.hdr(store, H_LEN);
    arena.set_hdr(store, H_LEN, len - 1);
    rebalance(store, arena, &path, height, keys);
    true
}

/// Restore the fill of the nodes on `path` after a removal, bottom-up.
fn rebalance(store: &mut Store, arena: Arena, path: &Path, height: usize, keys: &[Key]) {
    let mut n = path.leaf;
    let mut level = height;
    loop {
        if level == 0 {
            if !arena.is_leaf(store, n) && arena.count(store, n) == 1 {
                let only = arena.child(store, n, 0);
                arena.set_hdr(store, H_ROOT, only);
                arena.set_hdr(store, H_HEIGHT, arena.hdr(store, H_HEIGHT) - 1);
                release(store, arena, n);
            }
            return;
        }
        if arena.count(store, n) >= MIN {
            return;
        }
        let (parent, i) = path.steps[level - 1];
        let ri = if i > 0 { i } else { 1 };
        let (left, right) = (
            arena.child(store, parent, ri - 1),
            arena.child(store, parent, ri),
        );
        let leaf = arena.is_leaf(store, n);
        let (cl, cr) = (arena.count(store, left), arena.count(store, right));
        if cl + cr <= FANOUT {
            // Merge `right` into `left`: an internal node takes the parent's separator down as
            // the bound of `right`'s first child.
            for k in 0..cr {
                let mut v = arena.slot(store, right, k);
                if k == 0 && !leaf {
                    v.part = arena.part(store, parent, ri);
                    v.rec = arena.rec(store, parent, ri);
                }
                arena.put(store, left, cl + k, v);
            }
            arena.set(store, left, N_COUNT, cl + cr);
            if leaf {
                let next = arena.get(store, right, N_NEXT);
                arena.set(store, left, N_NEXT, next);
                if next == 0 {
                    arena.set_hdr(store, H_LAST, left);
                } else {
                    arena.set(store, next, N_PREV, left);
                }
            }
            release(store, arena, right);
            let cp = arena.count(store, parent);
            arena.shift(store, parent, ri + 1, ri, cp - ri - 1, true);
            arena.set(store, parent, N_COUNT, cp - 1);
            // The slots came from two nodes, and the parent's separator from a third.
            fit(store, arena, left, keys, true);
        } else {
            // Even the two out; the parent's separator becomes the right node's new minimum.
            let mut all = [Slot::default(); 2 * FANOUT as usize];
            for k in 0..cl {
                all[k as usize] = arena.slot(store, left, k);
            }
            for k in 0..cr {
                let mut v = arena.slot(store, right, k);
                if k == 0 && !leaf {
                    v.part = arena.part(store, parent, ri);
                    v.rec = arena.rec(store, parent, ri);
                }
                all[(cl + k) as usize] = v;
            }
            let total = cl + cr;
            let keep = total / 2;
            for k in 0..total {
                let v = all[k as usize];
                if k < keep {
                    arena.put(store, left, k, v);
                } else {
                    arena.put(store, right, k - keep, v);
                }
            }
            arena.set(store, left, N_COUNT, keep);
            arena.set(store, right, N_COUNT, total - keep);
            fit(store, arena, left, keys, true);
            fit(store, arena, right, keys, true);
            let up = all[keep as usize];
            let whole = if is_text(keys) { 0 } else { up.part };
            place(store, arena, parent, ri, up.rec, whole, keys);
        }
        n = parent;
        level -= 1;
    }
}

/// The record that carries exactly `key`, or 0 — the point lookup (@FR-Col-Lookup); the
/// caller passes a key as long as `keys`.
#[must_use]
pub fn find_exact(data: &DbRef, stores: &[Store], keys: &[Key], key: &[Content]) -> u32 {
    let Some(arena) = arena_of(data, stores) else {
        return 0;
    };
    let st = data.store_nr as usize;
    let probe = Probe::of_key(data.store_nr, key, keys);
    let n = leaf_of(arena, stores, st, keys, &probe, true);
    match leaf_slot(arena, stores, st, keys, n, &probe, false) {
        (slot, true) => arena.rec(&stores[st], n, slot),
        _ => 0,
    }
}

/// The first position at or above `key` (`above` false) or strictly above it, or 0.
fn bound(
    arena: Arena,
    data: &DbRef,
    stores: &[Store],
    keys: &[Key],
    key: &[Content],
    above: bool,
) -> u32 {
    let st = data.store_nr as usize;
    let probe = Probe::of_key(data.store_nr, key, keys);
    let path = descend(arena, stores, st, keys, &probe, above);
    let (slot, _) = leaf_slot(arena, stores, st, keys, path.leaf, &probe, above);
    let store = &stores[st];
    if slot < arena.count(store, path.leaf) {
        return path.leaf * FANOUT + slot;
    }
    match arena.get(store, path.leaf, N_NEXT) {
        0 => 0,
        nx => nx * FANOUT,
    }
}

/// The boundary lookup `tree::find` answers, as a position: with `before` the last record
/// strictly below `key`, otherwise the first strictly above it; 0 when there is none.  A
/// PARTIAL key (shorter than `keys`) compares on the values it has.
#[must_use]
pub fn find(data: &DbRef, before: bool, stores: &[Store], keys: &[Key], key: &[Content]) -> u32 {
    let Some(arena) = arena_of(data, stores) else {
        return 0;
    };
    if before {
        match bound(arena, data, stores, keys, key, false) {
            0 => last(data, stores),
            p => previous(data, stores, p),
        }
    } else {
        bound(arena, data, stores, keys, key, true)
    }
}

/// The positions of the lowest and the highest record of the range `from ..(=) till`, in
/// tree order, or `None` when it holds no record.  `from` is inclusive and `ex` describes
/// `till`; an empty bound is open (@FR-Col-Order-Sign: a descending field reverses in the
/// comparator, never here).
#[must_use]
pub fn range(
    data: &DbRef,
    stores: &[Store],
    keys: &[Key],
    from: &[Content],
    till: &[Content],
    ex: bool,
) -> Option<(u32, u32)> {
    let arena = arena_of(data, stores)?;
    let low = if from.is_empty() {
        first(data, stores)
    } else {
        bound(arena, data, stores, keys, from, false)
    };
    let high = if till.is_empty() {
        last(data, stores)
    } else {
        match bound(arena, data, stores, keys, till, !ex) {
            0 => last(data, stores),
            p => previous(data, stores, p),
        }
    };
    if low == 0 || high == 0 {
        return None;
    }
    if !till.is_empty() {
        let low_rec = DbRef {
            store_nr: data.store_nr,
            rec: record(data, stores, low),
            pos: 8,
        };
        let c = keys::key_compare(till, &low_rec, stores, keys);
        if c == Ordering::Less || (ex && c == Ordering::Equal) {
            return None;
        }
    }
    Some((low, high))
}

/// The position of the first record, or 0 for an empty index.
#[must_use]
pub fn first(data: &DbRef, stores: &[Store]) -> u32 {
    let Some(arena) = arena_of(data, stores) else {
        return 0;
    };
    let store = keys::store(data, stores);
    let n = arena.hdr(store, H_FIRST);
    if arena.count(store, n) == 0 {
        0
    } else {
        n * FANOUT
    }
}

/// The position of the last record, or 0 for an empty index.
#[must_use]
pub fn last(data: &DbRef, stores: &[Store]) -> u32 {
    let Some(arena) = arena_of(data, stores) else {
        return 0;
    };
    let store = keys::store(data, stores);
    let n = arena.hdr(store, H_LAST);
    match arena.count(store, n) {
        0 => 0,
        c => n * FANOUT + c - 1,
    }
}

/// The position after `pos`, or 0 past the last record.
#[must_use]
pub fn next(data: &DbRef, stores: &[Store], pos: u32) -> u32 {
    let Some(arena) = arena_of(data, stores) else {
        return 0;
    };
    if pos == 0 {
        return 0;
    }
    let store = keys::store(data, stores);
    let (n, i) = (pos / FANOUT, pos % FANOUT);
    if i + 1 < arena.count(store, n) {
        return pos + 1;
    }
    arena.get(store, n, N_NEXT) * FANOUT
}

/// The position before `pos`, or 0 before the first record.
#[must_use]
pub fn previous(data: &DbRef, stores: &[Store], pos: u32) -> u32 {
    let Some(arena) = arena_of(data, stores) else {
        return 0;
    };
    if pos == 0 {
        return 0;
    }
    let store = keys::store(data, stores);
    let (n, i) = (pos / FANOUT, pos % FANOUT);
    if i > 0 {
        return pos - 1;
    }
    match arena.get(store, n, N_PREV) {
        0 => 0,
        p => p * FANOUT + arena.count(store, p) - 1,
    }
}

/// The record at position `pos`, or 0 for no position.
#[must_use]
pub fn record(data: &DbRef, stores: &[Store], pos: u32) -> u32 {
    let Some(arena) = arena_of(data, stores) else {
        return 0;
    };
    if pos == 0 {
        return 0;
    }
    arena.rec(keys::store(data, stores), pos / FANOUT, pos % FANOUT)
}

/// The position of record `rec`, found by its own key, or 0 when it is not in the index.
#[must_use]
pub fn position(data: &DbRef, stores: &[Store], keys: &[Key], rec: u32) -> u32 {
    let Some(arena) = arena_of(data, stores) else {
        return 0;
    };
    let st = data.store_nr as usize;
    let probe = Probe::of_record(&stores[st], data.store_nr, rec, keys);
    let path = descend(arena, stores, st, keys, &probe, true);
    match leaf_slot(arena, stores, st, keys, path.leaf, &probe, false) {
        (slot, true) if arena.rec(&stores[st], path.leaf, slot) == rec => path.leaf * FANOUT + slot,
        _ => 0,
    }
}

// The RECORD-level surface the runtime walks with.  A walk holds a record, never a
// position: a loop body may remove records by key while the walk is under way (it cannot
// append — `iteration.md`, I-For), and a removal shifts slots and merges leaves under any
// held position.  A record stays itself; its slot is found again by its key.

/// The first record, or 0 for an empty index.
#[must_use]
pub fn first_record(data: &DbRef, stores: &[Store]) -> u32 {
    end_record(data, stores, first(data, stores))
}

/// The last record, or 0 for an empty index.
#[must_use]
pub fn last_record(data: &DbRef, stores: &[Store]) -> u32 {
    end_record(data, stores, last(data, stores))
}

/// The record at an end position, remembered as a walk's first hint.
fn end_record(data: &DbRef, stores: &[Store], pos: u32) -> u32 {
    let found = record(data, stores, pos);
    if found != 0 {
        remember(data.store_nr, arena(data, stores), found, pos);
    }
    found
}

/// The record after `rec` in key order, or 0 past the last (or when `rec` is not in the
/// index).
#[must_use]
pub fn next_record(data: &DbRef, stores: &[Store], keys: &[Key], rec: u32) -> u32 {
    step_record(data, stores, keys, rec, false)
}

/// The record before `rec` in key order, or 0 before the first.
#[must_use]
pub fn previous_record(data: &DbRef, stores: &[Store], keys: &[Key], rec: u32) -> u32 {
    step_record(data, stores, keys, rec, true)
}

/// One step of a walk that holds a RECORD.  Finding the record's slot by key is a full
/// descent; a walk asks for the slot of the record it was handed one step ago, so the slot
/// that step found is kept as a HINT ([`WALK_HINTS`]) and tried first.
fn step_record(data: &DbRef, stores: &[Store], keys: &[Key], rec: u32, back: bool) -> u32 {
    let Some(arena) = arena_of(data, stores) else {
        return 0;
    };
    let store = keys::store(data, stores);
    let at = match hinted(arena, store, data.store_nr, rec) {
        Some(p) => p,
        None => position(data, stores, keys, rec),
    };
    if at == 0 {
        return 0;
    }
    let to = arena.step(store, at, back);
    if to == 0 {
        return 0;
    }
    let found = arena.rec(store, to / FANOUT, to % FANOUT);
    remember(data.store_nr, arena.0, found, to);
    found
}

/// How many walks keep a hint at once, per thread: nested walks over different indexes
/// each keep theirs.
const HINTS: usize = 4;

/// `(store, arena, record, position)` per walk.  A hint is never trusted: [`hinted`] reads
/// the slot it names in the CURRENT arena and takes it only when that slot holds exactly the
/// record asked for — and a record is in an index once — so a hint made stale by any change
/// to the tree costs a descent and never a wrong slot.  Kept per thread and outside the
/// store, so a walk writes nothing: a locked store and `par` workers reading one store
/// concurrently are unaffected.
type Hint = (u16, u32, u32, u32);

thread_local! {
    static WALK_HINTS: [std::cell::Cell<Hint>; HINTS] =
        const { [const { std::cell::Cell::new((0, 0, 0, 0)) }; HINTS] };
}

/// The position the hints name for `rec` in this arena, if its slot still holds `rec`.
fn hinted(arena: Arena, store: &Store, store_nr: u16, rec: u32) -> Option<u32> {
    let pos = WALK_HINTS.with(|hints| {
        hints
            .iter()
            .map(std::cell::Cell::get)
            .find(|&(s, a, r, _)| r == rec && a == arena.0 && s == store_nr)
            .map(|h| h.3)
    })?;
    let (n, i) = (pos / FANOUT, pos % FANOUT);
    // Bounds before reads: the node must be one the arena handed out, a live leaf, and the
    // slot inside its count.
    let live = n != 0
        && n < arena.hdr(store, H_USED)
        && arena.get16(store, n, N_LEAF) == 1
        && i < arena.count(store, n)
        && arena.rec(store, n, i) == rec;
    live.then_some(pos)
}

/// Keep `pos` as the hint for `rec`, replacing this arena's hint or the oldest.
fn remember(store_nr: u16, arena: u32, rec: u32, pos: u32) {
    WALK_HINTS.with(|hints| {
        let mine = |h: &std::cell::Cell<Hint>| {
            let (s, a, _, _) = h.get();
            s == store_nr && a == arena
        };
        if !mine(&hints[0]) {
            // Most recent first, so the slot a new walk evicts is the least recently used.
            let at = hints.iter().position(mine).unwrap_or(HINTS - 1);
            for k in (1..=at).rev() {
                hints[k].set(hints[k - 1].get());
            }
        }
        hints[0].set((store_nr, arena, rec, pos));
    });
}

/// [`find`] as a record: with `before` the last record strictly below `key`, otherwise
/// the first strictly above it; 0 when there is none.
#[must_use]
pub fn find_record(
    data: &DbRef,
    before: bool,
    stores: &[Store],
    keys: &[Key],
    key: &[Content],
) -> u32 {
    record(data, stores, find(data, before, stores, keys, key))
}

/// Every record in key order.
#[must_use]
pub fn records(data: &DbRef, stores: &[Store]) -> Vec<u32> {
    let mut out = Vec::with_capacity(count(data, stores) as usize);
    let mut p = first(data, stores);
    while p != 0 {
        out.push(record(data, stores, p));
        p = next(data, stores, p);
    }
    out
}

/// The arena record the index's nodes live in — its whole spine, which a teardown frees
/// beside the element records — or 0 when the index has none yet.
#[must_use]
pub fn arena(data: &DbRef, stores: &[Store]) -> u32 {
    arena_of(data, stores).map_or(0, |a| a.0)
}

/// The `(start, finish)` cursor pair a walk of `from ..(=) till` starts from — the contract
/// `State::iterate` and `codegen_runtime::OpIterate` share, in RECORDS (@FR-Col-Order-Sign:
/// a descending key reverses in the comparator, never here).
///
/// * forward — `start` the record before the lowest in range (0 = from the first: the
///   step yields what follows `start`), `finish` the highest, which the step yields and
///   then stops on;
/// * reverse — `start` the record after the highest, `finish` the lowest.
///
/// An ABSENT bound keeps `start`/`finish` at 0 rather than a concrete record, because a
/// `#remove` during the walk can remove that record.  An EMPTY range is `(0, u32::MAX)`,
/// which the step refuses outright.
#[must_use]
pub fn range_cursors(
    data: &DbRef,
    stores: &[Store],
    keys: &[Key],
    from: &[Content],
    till: &[Content],
    ex: bool,
    reverse: bool,
) -> (u32, u32) {
    let Some((low, high)) = range(data, stores, keys, from, till, ex) else {
        return (0, u32::MAX);
    };
    let rec = |p| record(data, stores, p);
    if reverse {
        let start = if till.is_empty() {
            0
        } else {
            rec(next(data, stores, high))
        };
        (start, if from.is_empty() { 0 } else { rec(low) })
    } else {
        let start = if from.is_empty() {
            0
        } else {
            rec(previous(data, stores, low))
        };
        (start, if till.is_empty() { 0 } else { rec(high) })
    }
}

/// The number of records — kept in the header, so O(1).
#[must_use]
pub fn count(data: &DbRef, stores: &[Store]) -> u32 {
    arena_of(data, stores).map_or(0, |arena| arena.hdr(keys::store(data, stores), H_LEN))
}

/// Free the arena and empty the index.  The records themselves are the caller's.
pub fn clear(data: &DbRef, stores: &mut [Store]) {
    let Some(arena) = arena_of(data, stores) else {
        return;
    };
    let store = keys::mut_store(data, stores);
    store.delete(arena.0);
    store.set_u32_raw(data.rec, data.pos, 0);
}

/// Check every invariant of the tree: uniform depth, node fill, slots ascending, each part
/// the part of its record, each separator the minimum of its child, the leaf chain both
/// ways, the length.
/// # Panics
/// When the tree is not correctly defined.
pub fn validate(data: &DbRef, stores: &[Store], keys: &[Key]) {
    let Some(arena) = arena_of(data, stores) else {
        return;
    };
    let store = keys::store(data, stores);
    let root = arena.hdr(store, H_ROOT);
    let height = arena.hdr(store, H_HEIGHT);
    let mut leaves = Vec::new();
    let (_, _, total) = check(arena, data, stores, keys, root, height, true, &mut leaves);
    assert_eq!(total, arena.hdr(store, H_LEN), "index tree: length");
    assert_eq!(
        arena.hdr(store, H_FIRST),
        leaves[0],
        "index tree: first leaf"
    );
    assert_eq!(
        arena.hdr(store, H_LAST),
        *leaves.last().unwrap(),
        "index tree: last leaf"
    );
    for (k, &n) in leaves.iter().enumerate() {
        let prev = if k == 0 { 0 } else { leaves[k - 1] };
        let next = leaves.get(k + 1).copied().unwrap_or(0);
        assert_eq!(
            arena.get(store, n, N_PREV),
            prev,
            "index tree: leaf {n} prev link"
        );
        assert_eq!(
            arena.get(store, n, N_NEXT),
            next,
            "index tree: leaf {n} next link"
        );
    }
}

/// Validate the subtree at `n`, `depth` levels above the leaves; answers its minimum and
/// maximum record and its record count, and collects its leaves in order.
#[allow(clippy::too_many_arguments)]
fn check(
    arena: Arena,
    data: &DbRef,
    stores: &[Store],
    keys: &[Key],
    n: u32,
    depth: u32,
    root: bool,
    leaves: &mut Vec<u32>,
) -> (u32, u32, u32) {
    let store = keys::store(data, stores);
    let c = arena.count(store, n);
    let at = |rec| DbRef {
        store_nr: data.store_nr,
        rec,
        pos: 8,
    };
    assert!(c <= FANOUT, "index tree: node {n} holds {c} slots");
    // The node's prefix: every key it holds starts with it, and its parts are taken past it.
    let (skip, prefix) = arena.prefix(store, n);
    assert!(
        skip == 0 || (is_text(keys) && skip <= MAX_SKIP),
        "index tree: node {n} keeps a {skip}-byte prefix"
    );
    let part_ok = |i: u32| {
        let r = arena.rec(store, n, i);
        if skip > 0 {
            let t = key_text(store, r, &keys[0]);
            assert!(
                t.len() >= skip && t[..skip] == prefix[..skip],
                "index tree: node {n} slot {i} does not share the node's prefix"
            );
        }
        assert_eq!(
            arena.part(store, n, i),
            part_in(store, r, keys, skip),
            "index tree: part of node {n} slot {i}"
        );
    };
    if depth == 0 {
        assert!(
            arena.is_leaf(store, n),
            "index tree: node {n} at leaf depth is no leaf"
        );
        assert!(root || c >= MIN, "index tree: leaf {n} holds {c} slots");
        leaves.push(n);
        for i in 0..c {
            let r = arena.rec(store, n, i);
            part_ok(i);
            if i > 0 {
                let o = keys::compare(&at(arena.rec(store, n, i - 1)), &at(r), stores, keys);
                assert_eq!(
                    o,
                    Ordering::Less,
                    "index tree: leaf {n} not ascending at slot {i}"
                );
            }
        }
        return if c == 0 {
            (0, 0, 0)
        } else {
            (arena.rec(store, n, 0), arena.rec(store, n, c - 1), c)
        };
    }
    assert!(
        !arena.is_leaf(store, n),
        "index tree: node {n} above leaf depth is a leaf"
    );
    assert!(
        if root { c >= 2 } else { c >= MIN },
        "index tree: node {n} holds {c} children"
    );
    let (min, mut max, mut total) = check(
        arena,
        data,
        stores,
        keys,
        arena.child(store, n, 0),
        depth - 1,
        false,
        leaves,
    );
    for i in 1..c {
        let sep = arena.rec(store, n, i);
        part_ok(i);
        let (lo, hi, t) = check(
            arena,
            data,
            stores,
            keys,
            arena.child(store, n, i),
            depth - 1,
            false,
            leaves,
        );
        assert_eq!(
            lo, sep,
            "index tree: node {n} separator {i} is not its child's minimum"
        );
        let o = keys::compare(&at(max), &at(sep), stores, keys);
        assert_eq!(
            o,
            Ordering::Less,
            "index tree: node {n} children overlap at {i}"
        );
        max = hi;
        total += t;
    }
    (min, max, total)
}
