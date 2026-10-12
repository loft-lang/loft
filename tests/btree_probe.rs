// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN190 — the index's B+tree (`src/btree.rs`) timed against Rust's `BTreeMap`, the twin
//! `bench/15_stdlib_keyed`'s `index_fill_find` is measured against.  A measurement, not a
//! gate (`#[ignore]`): run with
//! `cargo test --release --test btree_probe -- --ignored --nocapture --test-threads=1`.
//!
//! The tree indexes claimed `Elm { k, c }` records in one store, with that routine's keys
//! (`(i·7919 + salt) mod m`); the map holds the same keys.  Each run checks that both find
//! every key with its own value, so a fast wrong tree fails.  The red-black tree this
//! replaced measured fill / find 154 · 80 ns at 5k keys and 250 · 131 at 1M (integer),
//! 235 · 123 and 418 · 236 (text), and walked 6.4 – 9.1 ns a record; the plan's issue keeps
//! the comparison.
use loft::btree;
use loft::database::Stores;
use loft::keys::{Content, DbRef, Key, Str};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

fn key_of(i: i64, salt: i64, m: i64) -> i64 {
    (i * 7919 + salt) % m
}

/// A store with an `index<Elm[k]>` over `Elm { k: <key>, c: integer }` and an empty slot
/// for it: the stores, the slot, the keys, and the byte positions of `k` and `c`.
fn world(text: bool) -> (Stores, DbRef, Vec<Key>, u32, u32) {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "k", stores.name(if text { "text" } else { "integer" }));
    stores.field(s, "c", stores.name("integer"));
    let v = stores.index(s, &[("k".to_string(), true)]);
    stores.finish();
    let db = stores.database(2);
    let st = db.store_nr as usize;
    let holder = stores.claim(&db, 2).rec;
    stores.allocations[st].set_u32_raw(holder, 8, 0);
    let slot = DbRef {
        store_nr: db.store_nr,
        rec: holder,
        pos: 8,
    };
    let keys = stores.keys(v).to_vec();
    let kpos = 8 + u32::from(keys[0].position);
    let cpos = 8 + u32::from(stores.position(s, "c"));
    (stores, slot, keys, kpos, cpos)
}

/// Fill and find `count` keys; ns per key (map fill, map find, tree fill, tree find).
fn run(count: i64, m: i64, text: bool) -> [f64; 4] {
    let (mut stores, slot, keys, kpos, cpos) = world(text);
    let st = slot.store_nr as usize;
    let texts: Vec<String> = (0..count)
        .map(|i| format!("key_{}", key_of(i, 0, m)))
        .collect();
    let mut recs = Vec::with_capacity(count as usize);
    for i in 0..count {
        let rec = stores.claim(&slot, 3).rec;
        let s = &mut stores.allocations[st];
        if text {
            let h = s.set_str(&texts[i as usize]);
            s.set_u32_raw(rec, kpos, h);
        } else {
            s.set_int(rec, kpos, key_of(i, 0, m));
        }
        s.set_int(rec, cpos, i);
        recs.push(rec);
    }
    let expect: i64 = (0..count).map(|i| i & 255).sum();
    let n = count as f64;
    // The twin: a BTreeMap from the key to the value.
    let (map_fill, map_find) = if text {
        let t = Instant::now();
        let mut map = BTreeMap::new();
        for (i, k) in texts.iter().enumerate() {
            map.insert(k.clone(), i as i64);
        }
        let fill = t.elapsed().as_nanos() as f64;
        let t = Instant::now();
        let acc: i64 = texts.iter().map(|k| map[k] & 255).sum();
        let find = t.elapsed().as_nanos() as f64;
        assert_eq!(black_box(acc), expect);
        (fill, find)
    } else {
        let t = Instant::now();
        let mut map = BTreeMap::new();
        for i in 0..count {
            map.insert(key_of(i, 0, m), i);
        }
        let fill = t.elapsed().as_nanos() as f64;
        let t = Instant::now();
        let acc: i64 = (0..count).map(|i| map[&key_of(i, 0, m)] & 255).sum();
        let find = t.elapsed().as_nanos() as f64;
        assert_eq!(black_box(acc), expect);
        (fill, find)
    };
    let t = Instant::now();
    for &r in &recs {
        btree::add(&slot, r, &mut stores.allocations, &keys);
    }
    let fill = t.elapsed().as_nanos() as f64;
    let t = Instant::now();
    let mut acc = 0i64;
    for i in 0..count {
        let key = if text {
            [Content::Str(Str::new(&texts[i as usize]))]
        } else {
            [Content::Long(key_of(i, 0, m))]
        };
        let found = btree::find_exact(&slot, &stores.allocations, &keys, &key);
        acc += stores.allocations[st].get_int(found, cpos) & 255;
    }
    let find = t.elapsed().as_nanos() as f64;
    assert_eq!(acc, expect, "the tree finds every key, with its own record");
    [map_fill / n, map_find / n, fill / n, find / n]
}

/// A whole walk in key order, ns per record: the map's iterator, the tree stepping by
/// RECORD (what a loop does) and by POSITION.
fn run_walk(count: i64, m: i64) -> [f64; 3] {
    let (mut stores, slot, keys, kpos, _) = world(false);
    let st = slot.store_nr as usize;
    let mut map = BTreeMap::new();
    for i in 0..count {
        let rec = stores.claim(&slot, 3).rec;
        stores.allocations[st].set_int(rec, kpos, key_of(i, 0, m));
        btree::add(&slot, rec, &mut stores.allocations, &keys);
        map.insert(key_of(i, 0, m), rec);
    }
    let all = &stores.allocations;
    let c = count as f64;
    let t = Instant::now();
    let n = black_box(map.values().count());
    let in_map = t.elapsed().as_nanos() as f64;
    assert_eq!(n as i64, count);
    let t = Instant::now();
    let (mut n, mut cur) = (0, btree::first_record(&slot, all));
    while cur != 0 {
        n += 1;
        cur = btree::next_record(&slot, all, &keys, cur);
    }
    let by_rec = t.elapsed().as_nanos() as f64;
    assert_eq!(n, count);
    let t = Instant::now();
    let (mut n, mut p) = (0, btree::first(&slot, all));
    while p != 0 {
        n += 1;
        p = btree::next(&slot, all, p);
    }
    let by_pos = t.elapsed().as_nanos() as f64;
    assert_eq!(n, count);
    [in_map / c, by_rec / c, by_pos / c]
}

const SIZES: [(i64, i64, u32); 3] = [
    (5000, 1_000_003, 100),
    (100_000, 10_000_019, 5),
    (1_000_000, 100_000_007, 2),
];

fn best<const N: usize>(reps: u32, f: impl Fn() -> [f64; N]) -> [f64; N] {
    let mut best = [f64::MAX; N];
    for _ in 0..reps {
        for (b, v) in best.iter_mut().zip(f()) {
            *b = b.min(v);
        }
    }
    best
}

#[test]
#[ignore = "a measurement: cargo test --release --test btree_probe -- --ignored --nocapture"]
fn fill_and_find_against_btreemap() {
    for text in [false, true] {
        for (count, m, reps) in SIZES {
            let b = best(reps, || run(count, m, text));
            println!(
                "{count:>8} {}  BTreeMap fill {:6.1} find {:6.1}  |  btree.rs fill {:6.1} find {:6.1}  ns/key  →  {:.1}× · {:.1}× the map",
                if text { "text" } else { "int " },
                b[0],
                b[1],
                b[2],
                b[3],
                b[2] / b[0],
                b[3] / b[1]
            );
        }
    }
}

#[test]
#[ignore = "a measurement: cargo test --release --test btree_probe -- --ignored --nocapture"]
fn a_walk_against_btreemap() {
    for (count, m, reps) in SIZES {
        let b = best(reps, || run_walk(count, m));
        println!(
            "{count:>8} walk  BTreeMap {:5.1}  |  btree.rs by record {:5.1}  by position {:5.1}  ns/record",
            b[0], b[1], b[2]
        );
    }
}
