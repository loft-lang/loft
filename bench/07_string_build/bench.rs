// Benchmark 7: String building (200,000 formatted appends)
//
// One OP is one text built from 200,000 `item-<i + salt>;` pieces; the result is its length.  The timed loop runs `--n` ops, each on an input that differs by
// the repetition number — in VALUE, never in the amount of work — so no optimiser can
// hoist or collapse the kernel across repetitions, and folds every result into `sink`.
// The `hash` column is one canonical op's result: every lane must print the same one,
// or the lanes are not computing the same thing and their times do not compare.
use std::fmt::Write;
use std::hint::black_box;
use std::time::Instant;

fn build(count: i64, salt: i64) -> i64 {
    let mut s = String::new();
    for i in 0..count {
        let _ = write!(s, "item-{};", i + salt);
    }
    s.len() as i64
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
    let count: i64 = 200_000;
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(build(black_box(count), black_box(r & 1)));
    }
    let us = t0.elapsed().as_micros() as i64;
    row("string_build", n, us, count, build(count, 0), sink);
}
