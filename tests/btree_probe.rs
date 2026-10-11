// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN190 P0 — the probe: is a B+tree of KEY PARTS and RECORD REFS, built over loft's own
//! `Store`, faster than the red-black `index` tree (`src/tree.rs`) on the same records and
//! keys?  A measurement, not a gate (`#[ignore]`): run with
//! `cargo test --release --test btree_probe -- --ignored --nocapture`.
//!
//! Both trees index the SAME claimed `Elm { k, c }` records in one store, with the keys of
//! `bench/15_stdlib_keyed`'s `index_fill_find` (`(i·7919 + salt) mod m`), and every node
//! access of the probe goes through the `Store` reads and writes loft itself makes.  Each
//! run checks that both trees find every key and no absent one, so a fast wrong tree fails.
use loft::database::Stores;
use loft::keys::{Content, DbRef};
use loft::store::Store;
use loft::tree;
use std::time::Instant;

/// Slots a node holds.
const F: u32 = 16;
// Node layout, bytes from the record start (payload from 8): count, leaf flag, next leaf,
// then F key parts (i64) and F + 1 refs (i32: a record in a leaf, a child node above).
const COUNT: u32 = 8;
const LEAF: u32 = 12;
const NEXT: u32 = 16;
const KEY0: u32 = 24;
const REF0: u32 = KEY0 + 8 * F;
const NODE_WORDS: u32 = (REF0 + 4 * (F + 1)).div_ceil(8);

// Reads of the tree's OWN nodes go through `Store::read` — a node is the tree's record, its
// layout known, so the per-access validity test `get_int` makes is the cost a built tree
// would not pay (it resolves a node once, as `(R-RecPtr)` resolves a record).
fn key_at(s: &Store, n: u32, i: u32) -> i64 {
    s.read::<i64>(n, KEY0 + 8 * i)
}
fn ref_at(s: &Store, n: u32, i: u32) -> u32 {
    s.read::<i32>(n, REF0 + 4 * i) as u32
}
fn count(s: &Store, n: u32) -> u32 {
    s.read::<i32>(n, COUNT) as u32
}

struct Probe {
    root: u32,
}

impl Probe {
    fn new_node(stores: &mut Stores, db: &DbRef, leaf: bool) -> u32 {
        let n = stores.claim(db, NODE_WORDS).rec;
        let s = &mut stores.allocations[db.store_nr as usize];
        s.set_i32_raw(n, COUNT, 0);
        s.set_i32_raw(n, LEAF, i32::from(leaf));
        s.set_i32_raw(n, NEXT, 0);
        n
    }

    fn find(&self, s: &Store, key: i64) -> u32 {
        let mut n = self.root;
        while s.read::<i32>(n, LEAF) == 0 {
            let c = count(s, n);
            // Children: c; separators: c - 1.  Take the first separator above the key.
            let mut i = 0;
            while i + 1 < c && key >= key_at(s, n, i) {
                i += 1;
            }
            n = ref_at(s, n, i);
        }
        let c = count(s, n);
        for i in 0..c {
            if key_at(s, n, i) == key {
                return ref_at(s, n, i);
            }
        }
        0
    }

    /// Insert `key → rec` under node `n`; answers the split (separator, new right node).
    fn insert(stores: &mut Stores, db: &DbRef, n: u32, key: i64, rec: u32) -> Option<(i64, u32)> {
        let st = db.store_nr as usize;
        let leaf = stores.allocations[st].get_i32_raw(n, LEAF) != 0;
        if leaf {
            let s = &mut stores.allocations[st];
            let c = count(s, n);
            let mut at = 0;
            while at < c && key_at(s, n, at) < key {
                at += 1;
            }
            let mut j = c;
            while j > at {
                let (k, r) = (key_at(s, n, j - 1), ref_at(s, n, j - 1));
                s.set_int(n, KEY0 + 8 * j, k);
                s.set_i32_raw(n, REF0 + 4 * j, r as i32);
                j -= 1;
            }
            s.set_int(n, KEY0 + 8 * at, key);
            s.set_i32_raw(n, REF0 + 4 * at, rec as i32);
            s.set_i32_raw(n, COUNT, (c + 1) as i32);
            if c + 1 < F {
                return None;
            }
            // Split the full leaf: the upper half to a new leaf, linked after this one.
            let right = Self::new_node(stores, db, true);
            let s = &mut stores.allocations[st];
            let half = F / 2;
            for i in half..F {
                let (k, r) = (key_at(s, n, i), ref_at(s, n, i));
                s.set_int(right, KEY0 + 8 * (i - half), k);
                s.set_i32_raw(right, REF0 + 4 * (i - half), r as i32);
            }
            s.set_i32_raw(right, COUNT, (F - half) as i32);
            s.set_i32_raw(n, COUNT, half as i32);
            let next = s.get_i32_raw(n, NEXT);
            s.set_i32_raw(right, NEXT, next);
            s.set_i32_raw(n, NEXT, right as i32);
            return Some((key_at(s, right, 0), right));
        }
        let (c, child_at) = {
            let s = &stores.allocations[st];
            let c = count(s, n);
            let mut i = 0;
            while i + 1 < c && key >= key_at(s, n, i) {
                i += 1;
            }
            (c, i)
        };
        let child = ref_at(&stores.allocations[st], n, child_at);
        let (sep, new) = Self::insert(stores, db, child, key, rec)?;
        let s = &mut stores.allocations[st];
        // Separator `sep` goes at child_at, the new child at child_at + 1.
        let mut j = c;
        while j > child_at + 1 {
            let r = ref_at(s, n, j - 1);
            s.set_i32_raw(n, REF0 + 4 * j, r as i32);
            let k = key_at(s, n, j - 2);
            s.set_int(n, KEY0 + 8 * (j - 1), k);
            j -= 1;
        }
        s.set_int(n, KEY0 + 8 * child_at, sep);
        s.set_i32_raw(n, REF0 + 4 * (child_at + 1), new as i32);
        s.set_i32_raw(n, COUNT, (c + 1) as i32);
        if c < F {
            return None;
        }
        // Split the internal node: F + 1 children, the middle separator moves up.
        let right = Self::new_node(stores, db, false);
        let s = &mut stores.allocations[st];
        let children = c + 1;
        let left_children = children / 2;
        let up = key_at(s, n, left_children - 1);
        for i in left_children..children {
            let r = ref_at(s, n, i);
            s.set_i32_raw(right, REF0 + 4 * (i - left_children), r as i32);
            if i + 1 < children {
                let k = key_at(s, n, i);
                s.set_int(right, KEY0 + 8 * (i - left_children), k);
            }
        }
        s.set_i32_raw(right, COUNT, (children - left_children) as i32);
        s.set_i32_raw(n, COUNT, left_children as i32);
        Some((up, right))
    }

    fn add(&mut self, stores: &mut Stores, db: &DbRef, key: i64, rec: u32) {
        if let Some((sep, right)) = Self::insert(stores, db, self.root, key, rec) {
            let root = Self::new_node(stores, db, false);
            let s = &mut stores.allocations[db.store_nr as usize];
            s.set_i32_raw(root, REF0, self.root as i32);
            s.set_int(root, KEY0, sep);
            s.set_i32_raw(root, REF0 + 4, right as i32);
            s.set_i32_raw(root, COUNT, 2);
            self.root = root;
        }
    }
}

fn key_of(i: i64, salt: i64, m: i64) -> i64 {
    (i * 7919 + salt) % m
}

/// One run: fill and find `count` keys with both trees; answers ns per key (rb fill, rb find,
/// probe fill, probe find).
fn run(count: i64, m: i64) -> (f64, f64, f64, f64) {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "k", stores.name("integer"));
    stores.field(s, "c", stores.name("integer"));
    let v = stores.index(s, &[("k".to_string(), true)]);
    let mm = stores.structure("Main", 0);
    stores.field(mm, "index", v);
    stores.finish();
    let db = stores.database(2);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 8,
    };
    stores.set_default_value(mm, &into);
    let keys = stores.keys(v).to_vec();
    let fields = stores.fields(v);
    let st = db.store_nr as usize;
    // The records, claimed once and shared by both trees (the RB links live in them).
    let mut recs = Vec::with_capacity(count as usize);
    for i in 0..count {
        let rec = stores.claim(&db, 6);
        let s = &mut stores.allocations[st];
        s.set_int(rec.rec, 8, key_of(i, 0, m));
        s.set_int(rec.rec, 16, i);
        recs.push(rec);
    }
    let t = Instant::now();
    for r in &recs {
        let _ = tree::add(&into, r, fields, &mut stores.allocations, &keys);
    }
    let rb_fill = t.elapsed().as_nanos() as f64;
    let t = Instant::now();
    let mut acc = 0i64;
    for i in 0..count {
        let found = tree::find_exact(
            &into,
            fields,
            &stores.allocations,
            &keys,
            &[Content::Long(key_of(i, 0, m))],
        );
        acc += stores.allocations[st].get_int(found, 16) & 255;
    }
    let rb_find = t.elapsed().as_nanos() as f64;
    let t = Instant::now();
    let mut p = Probe {
        root: Probe::new_node(&mut stores, &db, true),
    };
    for r in &recs {
        let k = stores.allocations[st].get_int(r.rec, 8);
        p.add(&mut stores, &db, k, r.rec);
    }
    let p_fill = t.elapsed().as_nanos() as f64;
    let t = Instant::now();
    let mut pacc = 0i64;
    for i in 0..count {
        let found = p.find(&stores.allocations[st], key_of(i, 0, m));
        pacc += stores.allocations[st].get_int(found, 16) & 255;
    }
    let p_find = t.elapsed().as_nanos() as f64;
    assert_eq!(acc, pacc, "both trees answer every lookup alike");
    let expect: i64 = (0..count).map(|i| i & 255).sum();
    assert_eq!(pacc, expect, "every key found, with its own record");
    for i in 0..count.min(2000) {
        assert_eq!(
            p.find(&stores.allocations[st], key_of(i, 0, m) + m),
            0,
            "an absent key"
        );
    }
    let n = count as f64;
    (rb_fill / n, rb_find / n, p_fill / n, p_find / n)
}

#[test]
#[ignore = "a measurement: cargo test --release --test btree_probe -- --ignored --nocapture"]
fn btree_of_key_parts_against_the_red_black_index() {
    for &(count, m, reps) in &[
        (5000i64, 1_000_003i64, 200),
        (100_000, 10_000_019, 10),
        (1_000_000, 100_000_007, 3),
    ] {
        let mut best = (f64::MAX, f64::MAX, f64::MAX, f64::MAX);
        for _ in 0..reps {
            let r = run(count, m);
            best = (
                best.0.min(r.0),
                best.1.min(r.1),
                best.2.min(r.2),
                best.3.min(r.3),
            );
        }
        println!(
            "{count:>8}  red-black fill {:6.1} find {:6.1}  |  b+tree fill {:6.1} find {:6.1}  ns/key  →  fill {:.1}×  find {:.1}×",
            best.0,
            best.1,
            best.2,
            best.3,
            best.0 / best.2,
            best.1 / best.3
        );
    }
}
