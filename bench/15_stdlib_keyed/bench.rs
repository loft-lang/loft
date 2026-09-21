// Benchmark 15: KEYED collections — the Rust twin.
//
// `std::collections::HashMap` stands for loft's `hash`, `BTreeMap` for `sorted` and
// `index`: what a Rust author reaches for with `std` alone.  Text keys are counted with the
// entry API over borrowed `&str`.  Same keys, same order of work, same result per op as
// bench.loft — the hash column is the receipt.
use std::collections::{BTreeMap, HashMap};
use std::hint::black_box;
use std::time::Instant;

struct E {
    val: i64,
}

fn key_of(i: i64, salt: i64) -> i64 {
    (i * 7919 + salt) % 1000003
}

fn make_words(count: i64) -> Vec<String> {
    (0..count).map(|i| format!("key-{}-{}", (i * 7919) % 100003, i % 7)).collect()
}

fn fill_hash(count: i64, salt: i64) -> HashMap<i64, E> {
    let mut db = HashMap::new();
    for i in 0..count {
        db.insert(key_of(i, salt), E { val: i });
    }
    db
}

fn k_hash_fill(count: i64, salt: i64) -> i64 {
    let db = fill_hash(count, salt);
    db[&key_of(count / 2, salt)].val + db[&key_of(7, salt)].val
}

fn k_hash_find(db: &HashMap<i64, E>, count: i64, salt: i64) -> i64 {
    let mut acc = 0i64;
    for i in 0..count {
        if let Some(e) = db.get(&key_of(i, 0)) {
            acc += e.val & 1023;
        }
        if db.contains_key(&(key_of(i, 0) + 1000003 + salt)) {
            acc += 1000000;
        }
    }
    acc
}

fn k_hash_update(db: &mut HashMap<i64, E>, count: i64, salt: i64) -> i64 {
    for i in 0..count {
        if let Some(e) = db.get_mut(&key_of(i, 0)) {
            e.val = i + salt;
        }
    }
    db[&key_of(count - 1, 0)].val + db[&key_of(3, 0)].val
}

fn k_hash_remove(count: i64, salt: i64) -> i64 {
    let mut db = fill_hash(count, salt);
    for i in 0..count {
        if i % 2 == 0 {
            db.remove(&key_of(i, salt));
        }
    }
    let mut left = 0i64;
    for i in 0..count {
        if db.contains_key(&key_of(i, salt)) {
            left += 1;
        }
    }
    left
}

fn k_text_keys(words: &[String], salt: i64) -> i64 {
    let mut db: HashMap<&str, i64> = HashMap::new();
    for w in words {
        db.entry(w.as_str()).and_modify(|c| *c += 1).or_insert(1 + salt);
    }
    let mut acc = 0i64;
    for w in words {
        acc += db[w.as_str()];
    }
    acc
}

fn k_sorted(count: i64, salt: i64) -> i64 {
    let mut db: BTreeMap<i64, E> = BTreeMap::new();
    for i in 0..count {
        db.insert(key_of(i, salt), E { val: i });
    }
    let mut acc = 0i64;
    let mut prev = -1i64;
    for (id, e) in &db {
        if *id < prev {
            return -1;
        }
        prev = *id;
        acc = ((acc ^ e.val) + (id & 7)) & 16777215;
    }
    acc
}

fn k_index(count: i64, salt: i64) -> i64 {
    let mut db: BTreeMap<i64, E> = BTreeMap::new();
    for i in 0..count {
        db.insert(key_of(i, salt), E { val: i });
    }
    let mut acc = 0i64;
    for i in 0..count {
        if let Some(e) = db.get(&key_of(i, salt)) {
            acc += e.val & 255;
        }
    }
    acc
}

// the consumer's group: one record set reached by position and by key.
struct G {
    val: i64,
}

fn k_grouped(count: i64, salt: i64) -> i64 {
    let mut all: Vec<G> = Vec::new();
    let mut by_id: HashMap<i64, usize> = HashMap::new();
    for i in 0..count {
        by_id.insert(key_of(i, salt), all.len());
        all.push(G { val: i });
    }
    let mut acc = 0i64;
    for i in 0..count {
        if let Some(&at) = by_id.get(&key_of(i, salt)) {
            acc += all[at].val & 255;
        }
    }
    acc + all.len() as i64
}

fn arg_n(dflt: i64) -> i64 {
    let args: Vec<String> = std::env::args().collect();
    let mut n = dflt;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--n" && i + 1 < args.len() {
            n = args[i + 1].parse().unwrap_or(dflt);
            i += 1;
        }
        i += 1;
    }
    n.max(2)
}

fn row(name: &str, iters: i64, us: i64, items: i64, result: i64) {
    let ns = us * 1000 / iters;
    let per = (us * 1000) as f64 / (iters * items) as f64;
    println!("{name}\t{iters}\t{us}\t{ns}\t{items}\t{per:.3}\t{result:x}");
}

fn timed(n: i64, mut op: impl FnMut(i64) -> i64) -> (i64, i64) {
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(op(black_box(r)));
    }
    (t0.elapsed().as_micros() as i64, sink)
}

fn main() {
    let n = arg_n(40);
    let count: i64 = 5000;
    let mut standing = fill_hash(count, 0);
    let words = make_words(2000);
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let mut sink: i64 = 0;

    let (us, s) = timed(n, |r| k_hash_fill(black_box(count), r & 1));
    sink = sink.wrapping_add(s);
    row("hash_fill", n, us, count, k_hash_fill(count, 0));
    let (us, s) = timed(n, |r| k_hash_find(black_box(&standing), count, r & 1));
    sink = sink.wrapping_add(s);
    row("hash_find", n, us, count * 2, k_hash_find(&standing, count, 0));
    let (us, s) = timed(n, |r| k_hash_update(black_box(&mut standing), count, r & 1));
    sink = sink.wrapping_add(s);
    row("hash_update", n, us, count, k_hash_update(&mut standing, count, 0));
    let (us, s) = timed(n, |r| k_hash_remove(black_box(count), r & 1));
    sink = sink.wrapping_add(s);
    row("hash_remove", n, us, count, k_hash_remove(count, 0));
    let (us, s) = timed(n, |r| k_text_keys(black_box(&words), r & 1));
    sink = sink.wrapping_add(s);
    row("hash_text_keys", n, us, 2000, k_text_keys(&words, 0));
    let (us, s) = timed(n, |r| k_sorted(black_box(count), r & 1));
    sink = sink.wrapping_add(s);
    row("sorted_fill_walk", n, us, count, k_sorted(count, 0));
    let (us, s) = timed(n, |r| k_index(black_box(count), r & 1));
    sink = sink.wrapping_add(s);
    row("index_fill_find", n, us, count, k_index(count, 0));
    let (us, s) = timed(n, |r| k_grouped(black_box(count), r & 1));
    sink = sink.wrapping_add(s);
    row("grouped_fill_find", n, us, count, k_grouped(count, 0));

    println!("time: 0ms sink={}", black_box(sink));
}
