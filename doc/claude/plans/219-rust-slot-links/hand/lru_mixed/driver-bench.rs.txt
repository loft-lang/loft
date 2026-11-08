// Benchmark 23: An LRU cache — the Rust twin: a `HashMap` from key to a slot in an arena of
// entries, and the recency list threaded through the arena by index, the shape Rust programs use
// where a doubly linked list of owned nodes would fight the borrow checker.
//
// One routine, ONE OP per repetition, on keys that differ by the salt in VALUE, never in the
// amount of work, folded into `sink`:
//   lru_mixed  a fresh cache of 500 entries given 4,000 accesses over 1,000 keys: a hit moves the
//              entry to the front, a miss inserts it there and evicts the least recently used
//              one when full; then the hits, the sum of the values read and a walk of the list
// The `hash` column is one canonical op's result: every lane must print the same one.
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

const NIL: usize = usize::MAX;

struct Node {
    key: i64,
    val: i64,
    prev: usize,
    next: usize,
}

struct Lru {
    map: HashMap<i64, usize>,
    nodes: Vec<Node>,
    free: Vec<usize>,
    head: usize,
    tail: usize,
    cap: usize,
}

fn key(i: i64, salt: i64) -> i64 {
    ((i * 1103515245 + 12345 + salt * 7919) % 2147483648) % 100000
}

impl Lru {
    fn new(cap: usize) -> Lru {
        Lru { map: HashMap::new(), nodes: Vec::new(), free: Vec::new(), head: NIL, tail: NIL, cap }
    }

    fn unlink(&mut self, n: usize) {
        let (p, x) = (self.nodes[n].prev, self.nodes[n].next);
        if p != NIL { self.nodes[p].next = x; } else { self.head = x; }
        if x != NIL { self.nodes[x].prev = p; } else { self.tail = p; }
        self.nodes[n].prev = NIL;
        self.nodes[n].next = NIL;
    }

    fn push_front(&mut self, n: usize) {
        self.nodes[n].next = self.head;
        if self.head != NIL { self.nodes[self.head].prev = n; } else { self.tail = n; }
        self.head = n;
    }

    /// The value under `k`, -1 when absent; a hit becomes the most recently used entry.
    fn get(&mut self, k: i64) -> i64 {
        let Some(&n) = self.map.get(&k) else { return -1 };
        if self.head != n {
            self.unlink(n);
            self.push_front(n);
        }
        self.nodes[n].val
    }

    /// Store `v` under `k` as the most recently used entry, evicting the least recently used
    /// one when the cache is full.
    fn put(&mut self, k: i64, v: i64) {
        if let Some(&n) = self.map.get(&k) {
            self.nodes[n].val = v;
            self.unlink(n);
            self.push_front(n);
            return;
        }
        if self.map.len() >= self.cap && self.tail != NIL {
            let t = self.tail;
            self.unlink(t);
            self.map.remove(&self.nodes[t].key);
            self.free.push(t);
        }
        let node = Node { key: k, val: v, prev: NIL, next: NIL };
        let n = if let Some(n) = self.free.pop() {
            self.nodes[n] = node;
            n
        } else {
            self.nodes.push(node);
            self.nodes.len() - 1
        };
        self.map.insert(k, n);
        self.push_front(n);
    }
}

fn lru_mixed(total: i64, salt: i64) -> i64 {
    let mut c = Lru::new(500);
    let mut hits = 0;
    let mut sum = 0;
    for i in 0..total {
        let k = key(i, salt) % 1000;
        let v = c.get(k);
        if v < 0 {
            c.put(k, i);
        } else {
            hits += 1;
            sum = (sum + v) % 1000000007;
        }
    }
    let mut walk = 0;
    let mut p = c.head;
    while p != NIL {
        walk = (walk * 31 + c.nodes[p].key) % 1000000007;
        p = c.nodes[p].next;
    }
    hits * 1000003 + (sum + walk) % 1000000007
}

/// `--n N`: how many ops the timed loop runs (the harness calibrates it per lane).
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

/// One measured routine, in the row format every lane prints (bench/README.md).
fn row(name: &str, iters: i64, us: i64, items: i64, result: i64, sink: i64) {
    let ns = us * 1000 / iters;
    let per = (us * 1000) as f64 / (iters * items) as f64;
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    println!("{name}\t{iters}\t{us}\t{ns}\t{items}\t{per:.3}\t{result:x}");
    println!("time: {}ms sink={}", us / 1000, black_box(sink));
}

fn main() {
    let n = arg_n(10);
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(lru_mixed(black_box(4000), black_box(r & 1)));
    }
    row("lru_mixed", n, t0.elapsed().as_micros() as i64, 4000, lru_mixed(4000, 0), sink);
}
