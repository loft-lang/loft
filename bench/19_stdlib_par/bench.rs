// Benchmark 19: the standard library's `par` as a program calls it — the Rust twin.
//
// Each row is the same kernel as bench.loft, parallelised the way a Rust author does it with
// `std` alone: `std::thread::scope` with 4 workers, worker t taking the contiguous range
// [t*n/4, (t+1)*n/4) — the partition loft's `parallel_workers` uses — and the main thread
// joining the workers in order.  A map row gathers each worker's `Vec` into one result
// vector in source order; a reduction row sums per worker and adds the partial sums, as
// idiomatic Rust does (integer sums, so the order cannot change the answer).
//
// loft runs its workers on a persistent rayon pool; `std` has no pool, so this twin spawns
// its 4 threads per region.  That is the plain idiomatic scoped-thread split, and on
// `par_small` — 256 regions per op — it charges Rust a spawn per region that loft does not
// pay.  Same result per op; the hash column is the receipt.
use std::hint::black_box;
use std::thread;
use std::time::Instant;

const THREADS: usize = 4;

struct Body { px: f64, py: f64, vx: f64, vy: f64, mass: f64, id: i64 }

/// Map `f` over `input` on `THREADS` scoped workers, gathering the results in source order.
fn par_map<T: Sync, R: Send>(input: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = input.len();
    let t = THREADS.min(n.max(1));
    let f = &f;
    thread::scope(|s| {
        let parts: Vec<_> = (0..t)
            .map(|k| {
                let part = &input[k * n / t..(k + 1) * n / t];
                s.spawn(move || part.iter().map(f).collect::<Vec<R>>())
            })
            .collect();
        let mut out = Vec::with_capacity(n);
        for p in parts {
            out.extend(p.join().unwrap());
        }
        out
    })
}

/// Map `f` over the integer range [0, n) on `THREADS` scoped workers, results in order.
fn par_range<R: Send>(n: i64, f: impl Fn(i64) -> R + Sync) -> Vec<R> {
    let t = (THREADS as i64).min(n.max(1));
    let f = &f;
    thread::scope(|s| {
        let parts: Vec<_> = (0..t)
            .map(|k| {
                let (lo, hi) = (k * n / t, (k + 1) * n / t);
                s.spawn(move || (lo..hi).map(f).collect::<Vec<R>>())
            })
            .collect();
        let mut out = Vec::with_capacity(n as usize);
        for p in parts {
            out.extend(p.join().unwrap());
        }
        out
    })
}

// ── par_map_float ────────────────────────────────────────────────────────────────────
fn fkern(x: f64, salt: i64) -> f64 {
    let mut t = x + salt as f64 * 0.5;
    let mut acc = 0.0;
    for _ in 0..24 {
        t = t * 0.5 + (t + 1.0).sqrt();
        acc += t;
    }
    acc
}

fn c_map_float(xs: &[f64], salt: i64) -> i64 {
    let out = par_map(xs, |&x| fkern(x, salt));
    out.iter().enumerate().map(|(i, v)| (v * 16.0).round() as i64 * ((i as i64 & 3) + 1)).sum()
}

// ── par_records ──────────────────────────────────────────────────────────────────────
fn body_score(b: &Body, salt: i64) -> i64 {
    let (mut x, mut y, mut e) = (b.px, b.py, 0.0f64);
    for _ in 0..16 {
        x += b.vx * 0.01;
        y += b.vy * 0.01;
        e += b.mass * (x * x + y * y + 1.0).sqrt();
    }
    e.round() as i64 + b.id * 3 + salt
}

fn c_records(bodies: &[Body], salt: i64) -> i64 {
    let scores = par_map(bodies, |b| body_score(b, salt));
    scores.iter().enumerate().map(|(i, s)| s * ((i as i64 & 3) + 1)).sum()
}

// ── par_reduce ───────────────────────────────────────────────────────────────────────
fn mix(i: i64, salt: i64) -> i64 {
    let mut h = (i * 2654435761 + salt) & 2147483647;
    for _ in 0..48 {
        h = (h * 1103515245 + 12345) & 2147483647;
        h ^= h >> 13;
    }
    h & 1023
}

fn c_reduce(count: i64, salt: i64) -> i64 {
    let t = (THREADS as i64).min(count.max(1));
    thread::scope(|s| {
        let parts: Vec<_> = (0..t)
            .map(|k| {
                let (lo, hi) = (k * count / t, (k + 1) * count / t);
                s.spawn(move || (lo..hi).map(|i| mix(i, salt)).sum::<i64>())
            })
            .collect();
        parts.into_iter().map(|p| p.join().unwrap()).sum()
    })
}

// ── par_text ─────────────────────────────────────────────────────────────────────────
fn fmt_num(i: i64, salt: i64) -> String {
    let v = i * 37 + salt;
    let f = v as f64 / 7.0;
    format!("n{v}:{f:.2}/{v:x}")
}

fn c_text(count: i64, salt: i64) -> i64 {
    let out = par_range(count, |i| fmt_num(i, salt));
    out.iter()
        .enumerate()
        .map(|(i, s)| s.len() as i64 * ((i as i64 & 3) + 1) + s.as_bytes()[s.len() - 2] as i64)
        .sum()
}

// ── par_small ────────────────────────────────────────────────────────────────────────
fn tiny(x: i64, salt: i64) -> i64 { x * 3 + salt }

fn c_small(vals: &[i64], salt: i64) -> i64 {
    let n = vals.len();
    let t = THREADS.min(n.max(1));
    let mut acc = 0i64;
    for call in 0..256i64 {
        let sc = salt + call;
        let part: i64 = thread::scope(|s| {
            let parts: Vec<_> = (0..t)
                .map(|k| {
                    let chunk = &vals[k * n / t..(k + 1) * n / t];
                    s.spawn(move || chunk.iter().map(|&x| tiny(x, sc)).sum::<i64>())
                })
                .collect();
            parts.into_iter().map(|p| p.join().unwrap()).sum()
        });
        acc += part * (call + 1);
    }
    acc
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
    let n = arg_n(20);
    let xs: Vec<f64> = (0..200000i64).map(|i| (i % 1000) as f64 * 0.125).collect();
    let bodies: Vec<Body> = (0..100000i64)
        .map(|i| Body { px: (i % 97) as f64 * 1.5, py: (i % 89) as f64 * 0.75,
                        vx: ((i * 7) % 13 - 6) as f64, vy: ((i * 11) % 17 - 8) as f64,
                        mass: 1.0 + (i % 5) as f64, id: i })
        .collect();
    let vals: Vec<i64> = (0..64i64).map(|i| i * 5 + 1).collect();
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let mut sink: i64 = c_small(&vals, 0);

    let (us, s) = timed(n, |r| c_map_float(black_box(&xs), r & 1));
    sink = sink.wrapping_add(s);
    row("par_map_float", n, us, 200000, c_map_float(&xs, 0));
    let (us, s) = timed(n, |r| c_records(black_box(&bodies), r & 1));
    sink = sink.wrapping_add(s);
    row("par_records", n, us, 100000, c_records(&bodies, 0));
    let (us, s) = timed(n, |r| c_reduce(black_box(200000), r & 1));
    sink = sink.wrapping_add(s);
    row("par_reduce", n, us, 200000, c_reduce(200000, 0));
    let (us, s) = timed(n, |r| c_text(black_box(100000), r & 1));
    sink = sink.wrapping_add(s);
    row("par_text", n, us, 100000, c_text(100000, 0));
    let (us, s) = timed(n, |r| c_small(black_box(&vals), r & 1));
    sink = sink.wrapping_add(s);
    row("par_small", n, us, 256, c_small(&vals, 0));

    println!("time: 0ms sink={}", black_box(sink));
}
