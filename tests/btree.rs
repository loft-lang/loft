// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN190 — the index's B+tree (`src/btree.rs`) against a MODEL: the same records in a
//! plain list kept sorted by `keys::compare`, which is the order an index is defined by and
//! shares no code with the tree's parts, nodes or descent.
//!
//! Every operation is applied to both: an insert must refuse the same duplicate, a removal
//! must find the record, every lookup must answer what a search of the list answers, and
//! the tree's walks — by position and by record, forward and back — must be the list.  A
//! walk by record also has to survive the records around it being removed and added by key
//! while it is under way (`iteration.md`, I-For), which is what its slot hints must not
//! get wrong.  Each key shape the parts treat differently has a run: an integer (exact
//! parts), a descending one, a float (signed zero is one key), a text with long shared
//! prefixes (every part ties), and a compound key with partial lookups.
//! `btree::validate` checks the tree's own invariants.
use loft::btree;
use loft::database::Stores;
use loft::keys::{self, Content, DbRef, Key, Str};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug)]
enum Shape {
    Int,
    IntDesc,
    Float,
    Text,
    Compound,
}

/// A key value; `Compound` is `(cat, name)`.
#[derive(Clone, Debug)]
enum Kv {
    I(i64),
    F(f64),
    T(String),
    C(i64, String),
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const PREFIXES: [&str; 5] = ["", "a", "ab", "common_prefix_", "common_prefix_x"];

struct World {
    stores: Stores,
    /// The holder record's slot: records are claimed in its store.
    home: DbRef,
    bt: DbRef,
    keys: Vec<Key>,
    shape: Shape,
    /// The model: every record in the tree, sorted by `keys::compare`.
    model: Vec<u32>,
}

impl World {
    fn new(shape: Shape) -> World {
        let mut stores = Stores::new();
        let s = stores.structure("Elm", 0);
        let key: Vec<(String, bool)> = match shape {
            Shape::Int => {
                stores.field(s, "k", stores.name("integer"));
                vec![("k".into(), true)]
            }
            Shape::IntDesc => {
                stores.field(s, "k", stores.name("integer"));
                vec![("k".into(), false)]
            }
            Shape::Float => {
                stores.field(s, "k", stores.name("float"));
                vec![("k".into(), true)]
            }
            Shape::Text => {
                stores.field(s, "k", stores.name("text"));
                vec![("k".into(), true)]
            }
            Shape::Compound => {
                stores.field(s, "cat", stores.name("integer"));
                stores.field(s, "name", stores.name("text"));
                vec![("cat".into(), true), ("name".into(), true)]
            }
        };
        let v = stores.index(s, &key);
        let m = stores.structure("Main", 0);
        stores.field(m, "index", v);
        stores.finish();
        let db = stores.database(2);
        let home = DbRef {
            store_nr: db.store_nr,
            rec: db.rec,
            pos: 8,
        };
        stores.set_default_value(m, &home);
        let holder = stores.claim(&db, 2);
        stores.allocations[db.store_nr as usize].set_u32_raw(holder.rec, 8, 0);
        let bt = DbRef {
            store_nr: db.store_nr,
            rec: holder.rec,
            pos: 8,
        };
        let keys = stores.keys(v).to_vec();
        World {
            stores,
            home,
            bt,
            keys,
            shape,
            model: Vec::new(),
        }
    }

    fn text(r: &mut Rng, dom: u64) -> String {
        let p = PREFIXES[r.below(PREFIXES.len() as u64) as usize];
        match r.below(8) {
            0 => p.to_string(),
            _ => format!("{p}{}", r.below(dom)),
        }
    }

    fn key(&self, r: &mut Rng, dom: u64) -> Kv {
        let half = (dom / 2) as i64;
        match self.shape {
            Shape::Int | Shape::IntDesc => Kv::I(r.below(dom) as i64 - half),
            Shape::Float => match r.below(16) {
                0 => Kv::F(-0.0),
                1 => Kv::F(0.0),
                _ => Kv::F((r.below(dom) as i64 - half) as f64 / 4.0),
            },
            Shape::Text => Kv::T(Self::text(r, dom)),
            Shape::Compound => Kv::C(r.below(5) as i64, Self::text(r, dom / 5 + 1)),
        }
    }

    fn content(kv: &Kv) -> Vec<Content> {
        match kv {
            Kv::I(v) => vec![Content::Long(*v)],
            Kv::F(v) => vec![Content::Float(*v)],
            Kv::T(t) => vec![Content::Str(Str::new(t))],
            Kv::C(c, t) => vec![Content::Long(*c), Content::Str(Str::new(t))],
        }
    }

    fn record(&mut self, kv: &Kv) -> u32 {
        let rec = self.stores.claim(&self.home, 8).rec;
        let s = &mut self.stores.allocations[self.home.store_nr as usize];
        let at = |k: &Key| 8 + u32::from(k.position);
        match kv {
            Kv::I(v) => {
                s.set_int(rec, at(&self.keys[0]), *v);
            }
            Kv::F(v) => {
                s.set_float(rec, at(&self.keys[0]), *v);
            }
            Kv::T(t) => {
                let h = s.set_str(t);
                s.set_u32_raw(rec, at(&self.keys[0]), h);
            }
            Kv::C(c, t) => {
                s.set_int(rec, at(&self.keys[0]), *c);
                let h = s.set_str(t);
                s.set_u32_raw(rec, at(&self.keys[1]), h);
            }
        }
        rec
    }

    fn at(&self, rec: u32) -> DbRef {
        DbRef {
            store_nr: self.home.store_nr,
            rec,
            pos: 8,
        }
    }

    /// How `a` orders against `b` — the collection's own order.
    fn order(&self, a: u32, b: u32) -> Ordering {
        keys::compare(
            &self.at(a),
            &self.at(b),
            &self.stores.allocations,
            &self.keys,
        )
    }

    /// The first index of the model whose record is NOT below `rec`.
    fn model_at(&self, rec: u32) -> usize {
        self.model
            .partition_point(|&m| self.order(m, rec) == Ordering::Less)
    }

    /// The model's records below `key` (a prefix of the list) and at-or-below it.
    fn model_bounds(&self, key: &[Content]) -> (usize, usize) {
        let a = &self.stores.allocations;
        let below = self.model.partition_point(|&m| {
            keys::key_compare(key, &self.at(m), a, &self.keys) == Ordering::Greater
        });
        let upto = self.model.partition_point(|&m| {
            keys::key_compare(key, &self.at(m), a, &self.keys) != Ordering::Less
        });
        (below, upto)
    }

    /// Insert into both; answers the new record and the duplicate both name, or 0.
    fn insert(&mut self, kv: &Kv) -> (u32, u32) {
        let rec = self.record(kv);
        let i = self.model_at(rec);
        let expect = match self.model.get(i) {
            Some(&m) if self.order(m, rec) == Ordering::Equal => m,
            _ => 0,
        };
        let got = btree::add(&self.bt, rec, &mut self.stores.allocations, &self.keys);
        assert_eq!(
            got, expect,
            "{:?}: insert {kv:?} — the duplicate",
            self.shape
        );
        if expect == 0 {
            self.model.insert(i, rec);
        }
        (rec, expect)
    }

    fn remove(&mut self, rec: u32) {
        let i = self.model_at(rec);
        assert_eq!(self.model[i], rec, "the model holds {rec}");
        self.model.remove(i);
        assert!(
            btree::remove(&self.bt, rec, &mut self.stores.allocations, &self.keys),
            "{:?}: record {rec} was not found to remove",
            self.shape
        );
    }

    fn bt_walk(&self) -> Vec<u32> {
        let a = &self.stores.allocations;
        let mut out = Vec::new();
        let mut p = btree::first(&self.bt, a);
        while p != 0 {
            out.push(btree::record(&self.bt, a, p));
            p = btree::next(&self.bt, a, p);
        }
        let mut back = Vec::new();
        let mut p = btree::last(&self.bt, a);
        while p != 0 {
            back.push(btree::record(&self.bt, a, p));
            p = btree::previous(&self.bt, a, p);
        }
        back.reverse();
        assert_eq!(
            out, back,
            "{:?}: the forward and the backward walk differ",
            self.shape
        );
        let mut by_rec = Vec::new();
        let mut r = btree::first_record(&self.bt, a);
        while r != 0 {
            by_rec.push(r);
            r = btree::next_record(&self.bt, a, &self.keys, r);
        }
        assert_eq!(out, by_rec, "{:?}: the walk by record differs", self.shape);
        let mut back = Vec::new();
        let mut r = btree::last_record(&self.bt, a);
        while r != 0 {
            back.push(r);
            r = btree::previous_record(&self.bt, a, &self.keys, r);
        }
        back.reverse();
        assert_eq!(
            out, back,
            "{:?}: the backward walk by record differs",
            self.shape
        );
        out
    }

    /// The tree valid, the model's sequence, every record found at its own position.
    fn agree(&self, live: usize) {
        let a = &self.stores.allocations;
        btree::validate(&self.bt, a, &self.keys);
        assert_eq!(
            self.model,
            self.bt_walk(),
            "{:?}: the in-order sequences differ",
            self.shape
        );
        assert_eq!(self.model.len(), live, "{:?}: length", self.shape);
        assert_eq!(
            btree::count(&self.bt, a) as usize,
            live,
            "{:?}: count",
            self.shape
        );
        for &r in &self.model {
            let p = btree::position(&self.bt, a, &self.keys, r);
            assert_eq!(
                btree::record(&self.bt, a, p),
                r,
                "{:?}: position of {r}",
                self.shape
            );
        }
    }

    fn lookups(&self, kv: &Kv, r: &mut Rng) {
        let a = &self.stores.allocations;
        let full = Self::content(kv);
        let (below, upto) = self.model_bounds(&full);
        let exact = if upto > below { self.model[below] } else { 0 };
        assert_eq!(
            exact,
            btree::find_exact(&self.bt, a, &self.keys, &full),
            "{:?}: exact lookup of {kv:?}",
            self.shape
        );
        // A partial key on the compound index: the first value only.
        let key = if matches!(self.shape, Shape::Compound) && r.below(2) == 0 {
            full[..1].to_vec()
        } else {
            full
        };
        let (below, upto) = self.model_bounds(&key);
        let expect_before = if below == 0 { 0 } else { self.model[below - 1] };
        let expect_after = self.model.get(upto).copied().unwrap_or(0);
        for (before, expect) in [(true, expect_before), (false, expect_after)] {
            let bt = btree::find_record(&self.bt, before, a, &self.keys, &key);
            assert_eq!(
                expect,
                bt,
                "{:?}: find {kv:?} before={before} (len {})",
                self.shape,
                key.len()
            );
        }
    }

    fn range(&self, from: &[Content], till: &[Content], ex: bool) {
        let a = &self.stores.allocations;
        let expect: Vec<u32> = self
            .model
            .iter()
            .copied()
            .filter(|&r| {
                let at = self.at(r);
                let lo = from.is_empty()
                    || keys::key_compare(from, &at, a, &self.keys) != Ordering::Greater;
                let hi = till.is_empty() || {
                    let c = keys::key_compare(till, &at, a, &self.keys);
                    c == Ordering::Greater || (!ex && c == Ordering::Equal)
                };
                lo && hi
            })
            .collect();
        let mut got = Vec::new();
        if let Some((lo, hi)) = btree::range(&self.bt, a, &self.keys, from, till, ex) {
            let mut p = lo;
            loop {
                got.push(btree::record(&self.bt, a, p));
                if p == hi {
                    break;
                }
                p = btree::next(&self.bt, a, p);
                assert_ne!(p, 0, "{:?}: the range walked off the end", self.shape);
            }
        }
        assert_eq!(got, expect, "{:?}: range ex={ex}", self.shape);
    }
}

/// One differential run: `ops` random operations over a key domain of `dom` values, the
/// trees compared in full every `every` operations and once more after draining.
fn run(shape: Shape, seed: u64, dom: u64, ops: u32, every: u32, insert_pct: u64) {
    let mut w = World::new(shape);
    let mut r = Rng(seed | 1);
    let mut live: Vec<u32> = Vec::new();
    let mut refused: Vec<u32> = Vec::new();
    for step in 0..ops {
        let roll = r.below(100);
        if roll < insert_pct {
            let kv = w.key(&mut r, dom);
            let (rec, dup) = w.insert(&kv);
            if dup == 0 {
                live.push(rec);
            } else {
                refused.push(rec);
            }
        } else if roll < insert_pct + (100 - insert_pct) / 2 && !live.is_empty() {
            let i = r.below(live.len() as u64) as usize;
            let rec = live.swap_remove(i);
            w.remove(rec);
            if let Some(&no) = refused.last() {
                // A record whose key was refused is not in the tree, though its key is.
                assert!(!btree::remove(
                    &w.bt,
                    no,
                    &mut w.stores.allocations,
                    &w.keys
                ));
            }
        } else if r.below(2) == 0 {
            let kv = w.key(&mut r, dom);
            w.lookups(&kv, &mut r);
        } else {
            // The contents borrow their texts: keep the key values alive beside them.
            let (kf, kt) = (w.key(&mut r, dom), w.key(&mut r, dom));
            let (mut from, mut till) = (World::content(&kf), World::content(&kt));
            if r.below(5) == 0 {
                from.clear();
            }
            if r.below(5) == 0 {
                till.clear();
            }
            if from.len() > 1 && r.below(2) == 0 {
                from.truncate(1);
            }
            if till.len() > 1 && r.below(2) == 0 {
                till.truncate(1);
            }
            w.range(&from, &till, r.below(2) == 0);
        }
        if step % every == 0 {
            w.agree(live.len());
        }
    }
    w.agree(live.len());
    // Drain: every removal shrinks the tree towards a single leaf and then nothing.
    let mut k = 0;
    while let Some(rec) = live.pop() {
        w.remove(rec);
        k += 1;
        if k % every == 0 || live.len() < 64 {
            w.agree(live.len());
        }
    }
    w.agree(0);
    assert_eq!(btree::first(&w.bt, &w.stores.allocations), 0);
    assert_eq!(btree::last(&w.bt, &w.stores.allocations), 0);
}

const SHAPES: [Shape; 5] = [
    Shape::Int,
    Shape::IntDesc,
    Shape::Float,
    Shape::Text,
    Shape::Compound,
];

#[test]
fn small_domains_validated_after_every_operation() {
    for (n, shape) in SHAPES.into_iter().enumerate() {
        for seed in 1..=4u64 {
            run(shape, seed * 7919 + n as u64, 60, 3000, 1, 55);
        }
    }
}

#[test]
fn growing_to_thousands_with_splits_and_merges() {
    for (n, shape) in SHAPES.into_iter().enumerate() {
        run(shape, 104_729 + n as u64, 50_000, 20_000, 500, 75);
    }
}

#[test]
fn a_hundred_thousand_records() {
    for (n, shape) in [Shape::Int, Shape::Text].into_iter().enumerate() {
        run(shape, 1_299_709 + n as u64, 10_000_000, 140_000, 20_000, 90);
    }
}

/// Ascending and descending insertion — the order a program most often fills an index in,
/// and the one that splits the same edge of the tree every time.
#[test]
fn sequential_inserts_both_ways() {
    for shape in [Shape::Int, Shape::IntDesc] {
        for up in [true, false] {
            let mut w = World::new(shape);
            let mut live = Vec::new();
            for i in 0..5000i64 {
                let v = if up { i } else { 5000 - i };
                let (rec, dup) = w.insert(&Kv::I(v));
                assert_eq!(dup, 0);
                live.push(rec);
            }
            w.agree(live.len());
            // Remove every other record, then the rest from the front.
            let odd: Vec<u32> = live.iter().copied().skip(1).step_by(2).collect();
            for rec in odd {
                w.remove(rec);
            }
            w.agree(2500);
            for rec in live.into_iter().step_by(2) {
                w.remove(rec);
            }
            w.agree(0);
        }
    }
}

/// Hand-computed answers, so the two trees agreeing is not the only evidence.
#[test]
fn hand_computed_integer_index() {
    let mut w = World::new(Shape::Int);
    let mut by_key = std::collections::HashMap::new();
    for k in [5, 3, 8, 1] {
        let (rec, dup) = w.insert(&Kv::I(k));
        assert_eq!(dup, 0);
        by_key.insert(k, rec);
    }
    let (again, dup) = w.insert(&Kv::I(3));
    assert_eq!(dup, by_key[&3], "a second 3 names the first");
    assert_ne!(again, dup);
    let a = &w.stores.allocations;
    let key_of =
        |rec: u32| a[w.bt.store_nr as usize].get_int(rec, 8 + u32::from(w.keys[0].position));
    let keys_of = |seq: Vec<u32>| seq.into_iter().map(key_of).collect::<Vec<_>>();
    assert_eq!(keys_of(w.bt_walk()), vec![1, 3, 5, 8]);
    assert_eq!(btree::count(&w.bt, a), 4);
    let find = |before, k| {
        key_of(btree::record(
            &w.bt,
            a,
            btree::find(&w.bt, before, a, &w.keys, &[Content::Long(k)]),
        ))
    };
    assert_eq!(find(true, 4), 3);
    assert_eq!(find(false, 4), 5);
    assert_eq!(find(true, 5), 3);
    assert_eq!(find(false, 5), 8);
    assert_eq!(
        btree::find(&w.bt, true, a, &w.keys, &[Content::Long(1)]),
        0,
        "nothing below 1"
    );
    assert_eq!(
        btree::find(&w.bt, false, a, &w.keys, &[Content::Long(8)]),
        0,
        "nothing above 8"
    );
    assert_eq!(btree::find_exact(&w.bt, a, &w.keys, &[Content::Long(4)]), 0);
    assert_eq!(
        btree::find_exact(&w.bt, a, &w.keys, &[Content::Long(8)]),
        by_key[&8]
    );
    let span = |from: i64, till: i64, ex: bool| {
        btree::range(
            &w.bt,
            a,
            &w.keys,
            &[Content::Long(from)],
            &[Content::Long(till)],
            ex,
        )
        .map(|(lo, hi)| {
            (
                key_of(btree::record(&w.bt, a, lo)),
                key_of(btree::record(&w.bt, a, hi)),
            )
        })
    };
    assert_eq!(span(3, 5, false), Some((3, 5)));
    assert_eq!(span(3, 5, true), Some((3, 3)));
    assert_eq!(span(2, 7, false), Some((3, 5)));
    assert_eq!(
        span(4, 5, true),
        None,
        "3..5 exclusive of 5 from 4 holds nothing"
    );
    assert_eq!(span(6, 2, false), None, "an inverted range");
    w.remove(by_key[&3]);
    let a = &w.stores.allocations;
    let key_of =
        |rec: u32| a[w.bt.store_nr as usize].get_int(rec, 8 + u32::from(w.keys[0].position));
    assert_eq!(
        w.bt_walk().into_iter().map(key_of).collect::<Vec<_>>(),
        vec![1, 5, 8]
    );
    assert_eq!(btree::count(&w.bt, a), 3);
}

/// A walk holds a RECORD and finds its slot again each step; the slot it found last is a
/// HINT.  The records around it are removed and added by key while it walks — splits,
/// merges and shifts under the hint — and each step must still yield the record that now
/// follows the one it holds.  Two walks interleave over the same index (one hint evicts the
/// other's) and over a second index (hints of other arenas), forward and back.
#[test]
fn a_walk_by_record_survives_changes_around_it() {
    for (n, shape) in [Shape::Int, Shape::Text].into_iter().enumerate() {
        for seed in 1..=3u64 {
            let mut w = World::new(shape);
            let mut other = World::new(shape);
            let mut r = Rng(seed * 104_729 + n as u64);
            for _ in 0..3000 {
                let kv = w.key(&mut r, 20_000);
                w.insert(&kv);
                let kv = other.key(&mut r, 20_000);
                other.insert(&kv);
            }
            for back in [false, true] {
                let first = |w: &World| {
                    let a = &w.stores.allocations;
                    if back {
                        btree::last_record(&w.bt, a)
                    } else {
                        btree::first_record(&w.bt, a)
                    }
                };
                let step = |w: &World, rec: u32| {
                    let a = &w.stores.allocations;
                    if back {
                        btree::previous_record(&w.bt, a, &w.keys, rec)
                    } else {
                        btree::next_record(&w.bt, a, &w.keys, rec)
                    }
                };
                // The model's neighbour of `rec` in the walk's direction.
                let follows = |w: &World, rec: u32| {
                    let i = w.model_at(rec);
                    if back {
                        if i == 0 { 0 } else { w.model[i - 1] }
                    } else {
                        w.model.get(i + 1).copied().unwrap_or(0)
                    }
                };
                let (mut cur, mut twin) = (first(&w), first(&w));
                let mut o = first(&other);
                let mut steps = 0;
                while cur != 0 {
                    // Change the tree around the walk: remove records other than the one
                    // held (and the twin's), add new ones.
                    for _ in 0..r.below(4) {
                        if w.model.len() > 2 {
                            let victim = w.model[r.below(w.model.len() as u64) as usize];
                            if victim != cur && victim != twin {
                                w.remove(victim);
                            }
                        }
                    }
                    for _ in 0..r.below(3) {
                        let kv = w.key(&mut r, 20_000);
                        w.insert(&kv);
                    }
                    let expect = follows(&w, cur);
                    let got = step(&w, cur);
                    assert_eq!(got, expect, "{shape:?} back={back}: the step after {cur}");
                    cur = got;
                    // The twin walks the same index every other step; the other index's
                    // walk takes a hint slot in between.
                    if steps % 2 == 0 && twin != 0 {
                        let expect = follows(&w, twin);
                        twin = step(&w, twin);
                        assert_eq!(twin, expect, "{shape:?} back={back}: the twin's step");
                    }
                    if o != 0 {
                        let expect = follows(&other, o);
                        o = step(&other, o);
                        assert_eq!(o, expect, "{shape:?} back={back}: the other index's step");
                    }
                    steps += 1;
                }
                assert!(steps > 1000, "the walk ran ({steps} steps)");
                w.agree(w.model.len());
            }
        }
    }
}
