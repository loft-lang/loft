// Benchmark 3: Count primes below 300,000 (trial division)
//
// One OP is the count over `lo..300000`, `lo` 0 or 1 by repetition.  The timed loop runs `--n` ops, each on an input that differs by
// the repetition number — in VALUE, never in the amount of work — so no optimiser can
// hoist or collapse the kernel across repetitions, and folds every result into `sink`.
// The `hash` column is one canonical op's result: every lane must print the same one,
// or the lanes are not computing the same thing and their times do not compare.
use std::hint::black_box;
use std::time::Instant;

fn is_prime(n: i64) -> bool {
    if n < 2 { return false; }
    if n == 2 { return true; }
    if n % 2 == 0 { return false; }
    let mut i = 3i64;
    while i * i <= n {
        if n % i == 0 { return false; }
        i += 2;
    }
    true
}

fn count_primes(lo: i64, limit: i64) -> i64 {
    let mut count = 0;
    for n in lo..limit {
        if is_prime(n) { count += 1; }
    }
    count
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
    let n = arg_n(20);
    let limit: i64 = 300_000;
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(count_primes(black_box(r & 1), black_box(limit)));
    }
    let us = t0.elapsed().as_micros() as i64;
    row("sieve", n, us, limit, count_primes(0, limit), sink);
}
