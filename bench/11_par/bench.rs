// Benchmark 11: par equivalent — 100,000 elements of 50-step Newton's sqrt, 4 threads
//
// One OP is one parallel pass: a 4-way range partition, each worker summing its part, the
// main thread adding the parts in order — the shape of loft's `par(r = work(s), 4)`.  No
// external crates: the harness compiles each bench.rs with bare `rustc`.
use std::hint::black_box;
use std::thread;
use std::time::Instant;

fn newton_sqrt(x: f64) -> f64 {
    let mut g = x / 2.0;
    for _ in 0..50 {
        g = (g + x / g) / 2.0;
    }
    g
}

fn chunk(lo: i64, hi: i64) -> Vec<f64> {
    (lo..hi).map(|i| newton_sqrt((i + 1) as f64)).collect()
}

/// One pass.  The workers return their elements and the main thread sums them in item
/// order, because that is the order loft's `par` delivers results in: a float sum taken
/// per worker and then combined would round differently and the hash would not agree.
fn pass(count: i64) -> i64 {
    let workers: i64 = 4;
    let part = count / workers;
    let handles: Vec<_> = (0..workers)
        .map(|t| {
            let lo = t * part;
            let hi = if t == workers - 1 { count } else { lo + part };
            thread::spawn(move || chunk(lo, hi))
        })
        .collect();
    let mut sum = 0.0f64;
    for h in handles {
        for v in h.join().unwrap() {
            sum += v;
        }
    }
    sum.round() as i64
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

fn main() {
    let n = arg_n(20);
    let count: i64 = 100_000;
    let warm = pass(count);
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for _ in 0..n {
        sink = sink.wrapping_add(pass(black_box(count)));
    }
    let us = t0.elapsed().as_micros() as i64;
    let ns = us * 1000 / n;
    let per = (us * 1000) as f64 / (n * count) as f64;
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    println!("par\t{n}\t{us}\t{ns}\t{count}\t{per:.3}\t{warm:x}");
    println!("time: {}ms sink={}", us / 1000, black_box(sink));
}
