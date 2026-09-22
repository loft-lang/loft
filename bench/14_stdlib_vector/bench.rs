// Benchmark 14: VECTORS and RECORDS — the Rust twin.
//
// Each routine is what a Rust author writes for the same job with `std` alone: `push`,
// `collect`, iterators for reads and writes, `to_vec`, `Vec::remove`, a `Vec<Vec<i64>>`,
// and a `Vec<P>` of plain structs.  Same inputs, same results per op as bench.loft.
use std::hint::black_box;
use std::time::Instant;

#[derive(Clone)]
struct P {
    x: f64,
    y: f64,
    id: i64,
}

fn make_ints(count: i64, salt: i64) -> Vec<i64> {
    (0..count).map(|i| (i * 7919 + salt) % 100003).collect()
}

fn make_points(count: i64, salt: i64) -> Vec<P> {
    (0..count)
        .map(|i| P { x: (i % 97) as f64 * 0.5, y: ((i + salt) % 13) as f64, id: i })
        .collect()
}

fn v_push(count: i64, salt: i64) -> i64 {
    let mut v: Vec<i64> = Vec::new();
    for i in 0..count {
        v.push(i * 3 + salt);
    }
    v[(count / 2) as usize] + v.len() as i64
}

fn v_comprehend(count: i64, salt: i64) -> i64 {
    let v: Vec<i64> = (0..count).map(|i| i * i + salt).collect();
    v[(count - 1) as usize] + v.len() as i64
}

fn v_read(v: &[i64]) -> i64 {
    let mut acc = 0i64;
    for &x in v {
        acc = ((acc ^ x) + (x & 7)) & 16777215;
    }
    acc
}

fn v_sum(v: &[i64]) -> i64 {
    v.iter().sum()
}

fn v_minmax(v: &[i64]) -> i64 {
    v.iter().max().copied().unwrap_or(0) - v.iter().min().copied().unwrap_or(0)
}

fn v_write(v: &mut [i64], salt: i64) -> i64 {
    for (i, x) in v.iter_mut().enumerate() {
        *x = (i as i64 * 7 + salt) & 1023;
    }
    v[v.len() / 2] + v[v.len() - 1]
}

fn v_copy(v: &[i64], salt: i64) -> i64 {
    let mut w = v.to_vec();
    w[0] = salt;
    w.len() as i64 + w[0] + w[w.len() - 1]
}

fn v_drain(count: i64, salt: i64) -> i64 {
    let mut v: Vec<i64> = (0..count).map(|i| i + salt).collect();
    let mut acc = 0i64;
    for _ in 0..count {
        acc += v[0];
        v.remove(0);
    }
    acc + v.len() as i64
}

fn v_grid(side: i64, salt: i64) -> i64 {
    let g: Vec<Vec<i64>> = (0..side).map(|y| (0..side).map(|x| x * y + salt).collect()).collect();
    let mut acc = 0i64;
    for d in 0..side as usize {
        acc += g[d][d];
    }
    acc
}

fn r_build(count: i64, salt: i64) -> i64 {
    let mut v: Vec<P> = Vec::new();
    for i in 0..count {
        v.push(P { x: i as f64 * 0.5, y: salt as f64, id: i });
    }
    let c = count as usize;
    v.len() as i64 + v[c - 1].id + (v[c / 2].x + v[1].y).round() as i64
}

fn r_walk(v: &[P]) -> i64 {
    let mut acc = 0.0f64;
    for p in v {
        acc += p.x * p.y + p.id as f64;
    }
    acc.round() as i64
}

fn r_update(v: &mut [P], salt: i64) -> i64 {
    for (i, p) in v.iter_mut().enumerate() {
        p.x = p.y + (i as i64 + salt) as f64;
    }
    (v[v.len() / 2].x + v[v.len() - 1].x).round() as i64
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
    let n = arg_n(100);
    let count: i64 = 20000;
    let (ia, ib) = (make_ints(count, 0), make_ints(count, 1));
    let mut work = make_ints(count, 0);
    let (pa, pb) = (make_points(count, 0), make_points(count, 1));
    let mut pw = make_points(count, 0);
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let mut sink: i64 = 0;

    let (us, s) = timed(n, |r| v_push(black_box(count), r & 1));
    sink = sink.wrapping_add(s);
    row("push", n, us, count, v_push(count, 0));
    let (us, s) = timed(n, |r| v_comprehend(black_box(count), r & 1));
    sink = sink.wrapping_add(s);
    row("comprehension", n, us, count, v_comprehend(count, 0));
    let (us, s) = timed(n, |r| v_read(black_box(if r & 1 == 0 { &ia } else { &ib })));
    sink = sink.wrapping_add(s);
    row("index_read", n, us, count, v_read(&ia));
    let (us, s) = timed(n, |r| v_sum(black_box(if r & 1 == 0 { &ia } else { &ib })));
    sink = sink.wrapping_add(s);
    row("sum", n, us, count, v_sum(&ia));
    let (us, s) = timed(n, |r| v_minmax(black_box(if r & 1 == 0 { &ia } else { &ib })));
    sink = sink.wrapping_add(s);
    row("min_max_of", n, us, count, v_minmax(&ia));
    let (us, s) = timed(n, |r| v_write(black_box(&mut work), r & 1));
    sink = sink.wrapping_add(s);
    row("index_write", n, us, count, v_write(&mut work, 0));
    let (us, s) = timed(n, |r| v_copy(black_box(&ia), r & 1));
    sink = sink.wrapping_add(s);
    row("copy", n, us, count, v_copy(&ia, 0));
    let (us, s) = timed(n, |r| v_drain(black_box(1024), r & 1));
    sink = sink.wrapping_add(s);
    row("remove_front", n, us, 1024, v_drain(1024, 0));
    let (us, s) = timed(n, |r| v_grid(black_box(128), r & 1));
    sink = sink.wrapping_add(s);
    row("grid", n, us, 128 * 128, v_grid(128, 0));
    let (us, s) = timed(n, |r| r_build(black_box(count), r & 1));
    sink = sink.wrapping_add(s);
    row("record_append", n, us, count, r_build(count, 0));
    let (us, s) = timed(n, |r| r_walk(black_box(if r & 1 == 0 { &pa } else { &pb })));
    sink = sink.wrapping_add(s);
    row("record_walk", n, us, count, r_walk(&pa));
    let (us, s) = timed(n, |r| r_update(black_box(&mut pw), r & 1));
    sink = sink.wrapping_add(s);
    row("record_update", n, us, count, r_update(&mut pw, 0));

    println!("time: 0ms sink={}", black_box(sink));
}
