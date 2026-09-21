// Benchmark 2: Integer loop (5 million steps of a loop-carried mix)
//
// One OP is 5,000,000 steps of `acc = ((acc ^ i) + (i & 7)) & 0xFFFFFF` — loop-carried, so it has no
// closed form and cannot be vectorised: the row measures the scalar loop itself.  The timed loop runs `--n` ops, each on an input that differs by
// the repetition number — in VALUE, never in the amount of work — so no optimiser can
// hoist or collapse the kernel across repetitions, and folds every result into `sink`.
// The `hash` column is one canonical op's result: every lane must print the same one,
// or the lanes are not computing the same thing and their times do not compare.
use std::hint::black_box;
use std::time::Instant;

fn mix(count: i64, salt: i64) -> i64 {
    let mut acc = salt;
    for i in 0..count {
        acc = ((acc ^ i) + (i & 7)) & 16777215;
    }
    acc
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
    let count: i64 = 5_000_000;
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(mix(black_box(count), black_box(r)));
    }
    let us = t0.elapsed().as_micros() as i64;
    row("sum_loop", n, us, count, mix(count, 0), sink);
}
