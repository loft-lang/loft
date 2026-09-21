// Benchmark 9: Float dot product (2 million elements)
//
// One OP is the dot product of two prebuilt vectors from element `salt` on.  The timed loop runs `--n` ops, each on an input that differs by
// the repetition number — in VALUE, never in the amount of work — so no optimiser can
// hoist or collapse the kernel across repetitions, and folds every result into `sink`.
// The `hash` column is one canonical op's result: every lane must print the same one,
// or the lanes are not computing the same thing and their times do not compare.
use std::hint::black_box;
use std::time::Instant;

fn dot(xs: &[f64], ys: &[f64], from: usize) -> i64 {
    let mut acc = 0.0f64;
    for (x, y) in xs[from..].iter().zip(&ys[from..]) {
        acc += x * y;
    }
    acc.round() as i64
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
    let size: usize = 2_000_000;
    let xs: Vec<f64> = (0..size).map(|i| i as f64 / 1000.0).collect();
    let ys: Vec<f64> = (0..size).map(|i| (size - i) as f64 / 1000.0).collect();
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(dot(black_box(&xs), black_box(&ys), black_box((r & 1) as usize)));
    }
    let us = t0.elapsed().as_micros() as i64;
    row("dot_product", n, us, size as i64, dot(&xs, &ys, 0), sink);
}
