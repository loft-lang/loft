// Benchmark 5: Mandelbrot set (200x200, at most 256 iterations)
//
// One OP is the iteration total over the grid, its origin nudged by the repetition.  The timed loop runs `--n` ops, each on an input that differs by
// the repetition number — in VALUE, never in the amount of work — so no optimiser can
// hoist or collapse the kernel across repetitions, and folds every result into `sink`.
// The `hash` column is one canonical op's result: every lane must print the same one,
// or the lanes are not computing the same thing and their times do not compare.
use std::hint::black_box;
use std::time::Instant;

fn mandelbrot(cx: f64, cy: f64) -> i64 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..256i64 {
        if zx * zx + zy * zy > 4.0 { return i; }
        let tmp = zx * zx - zy * zy + cx;
        zy = 2.0 * zx * zy + cy;
        zx = tmp;
    }
    256
}

fn grid(size: i64, nudge: f64) -> i64 {
    let mut total = 0;
    for y in 0..size {
        for x in 0..size {
            let cx = (x as f64 / size as f64) * 3.5 - 2.5 + nudge;
            let cy = (y as f64 / size as f64) * 2.0 - 1.0;
            total += mandelbrot(cx, cy);
        }
    }
    total
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
    let size: i64 = 200;
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(grid(black_box(size), black_box((r & 1) as f64 * 0.0000001)));
    }
    let us = t0.elapsed().as_micros() as i64;
    row("mandelbrot", n, us, size * size, grid(size, 0.0), sink);
}
