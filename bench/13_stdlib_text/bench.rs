// Benchmark 13: the standard library's TEXT routines — the Rust twin.
//
// Each routine is what a Rust author writes for the same job with `std` alone: `split`
// yields borrowed slices, `find`/`contains`/`replace`/`trim`/`to_lowercase` are the
// standard ones, numbers are formatted with `write!` and parsed with `str::parse`.  Same
// inputs, same order of work, same result per op as bench.loft — the hash is the receipt.
use std::fmt::Write;
use std::hint::black_box;
use std::time::Instant;

fn make_text(lines: i64, salt: i64) -> String {
    let mut out = String::new();
    for i in 0..lines {
        let _ = write!(out, "  Item-{}: alpha,beta;GAMMA delta={}.{}  \n", i + salt, (i * 7 + salt) % 1000, i % 100);
    }
    out
}

fn make_numbers(count: i64, salt: i64) -> String {
    let mut out = String::new();
    for i in 0..count {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{}.{}", (i * 37 + salt) % 5000, i % 4 * 25);
    }
    out
}

fn make_words(count: i64, salt: i64) -> Vec<String> {
    (0..count).map(|i| format!("w{}", (i * 13 + salt) % 977)).collect()
}

fn t_split(src: &str) -> i64 {
    let parts: Vec<&str> = src.split('\n').collect();
    let mut n = parts.len() as i64;
    for p in &parts {
        n += p.len() as i64;
    }
    n
}

fn t_lines(src: &str) -> i64 {
    let mut n = 0;
    for line in src.split('\n') {
        n += line.len() as i64 + 1;
    }
    n
}

fn t_search(src: &str) -> i64 {
    let mut n = 0;
    for line in src.split('\n') {
        if line.contains("GAMMA") {
            n += 1;
        }
        n += line.find("delta=").map_or(-1, |p| p as i64);
    }
    n
}

fn t_replace(src: &str) -> i64 {
    src.replace("alpha", "omega-omega").len() as i64
}

fn t_lower(src: &str) -> i64 {
    let mut n = 0;
    for line in src.split('\n') {
        let low = line.trim().to_lowercase();
        n += low.len() as i64;
        if low.starts_with("item") {
            n += 1;
        }
    }
    n
}

fn t_chars(src: &str) -> i64 {
    let mut n = 0;
    for c in src.chars() {
        if c == 'a' || c == 'e' || c == 'i' {
            n += 1;
        }
        if c > '9' {
            n += 2;
        }
    }
    n
}

fn t_bytes(src: &str) -> i64 {
    let mut n = 0;
    for &b in src.as_bytes() {
        if b == 44 {
            n += 1;
        }
    }
    n
}

fn t_format(count: i64, salt: i64) -> i64 {
    let mut out = String::new();
    for i in 0..count {
        let _ = write!(out, "{}:{:.2};", i + salt, i as f64 * 0.25);
    }
    out.len() as i64
}

fn t_parse(nums: &str) -> i64 {
    let mut acc = 0.0f64;
    for tok in nums.split(' ') {
        acc += tok.parse::<f64>().unwrap_or(0.0);
    }
    (acc * 4.0).round() as i64
}

fn t_join(words: &[String]) -> i64 {
    words.join(", ").len() as i64
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

/// Time `n` ops of `op`, which is handed the repetition number, and return (µs, sink).
fn timed(n: i64, mut op: impl FnMut(i64) -> i64) -> (i64, i64) {
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(op(black_box(r)));
    }
    (t0.elapsed().as_micros() as i64, sink)
}

fn main() {
    let n = arg_n(200);
    let (a, b) = (make_text(200, 0), make_text(200, 1));
    let (na, nb) = (make_numbers(2000, 0), make_numbers(2000, 1));
    let (wa, wb) = (make_words(2000, 0), make_words(2000, 1));
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let mut sink: i64 = 0;
    let pick = |r: i64| -> &str { if r & 1 == 0 { &a } else { &b } };

    let (us, s) = timed(n, |r| t_split(black_box(pick(r))));
    sink = sink.wrapping_add(s);
    row("split", n, us, 200, t_split(&a));
    let (us, s) = timed(n, |r| t_lines(black_box(pick(r))));
    sink = sink.wrapping_add(s);
    row("split_walk", n, us, 200, t_lines(&a));
    let (us, s) = timed(n, |r| t_search(black_box(pick(r))));
    sink = sink.wrapping_add(s);
    row("find_contains", n, us, 200, t_search(&a));
    let (us, s) = timed(n, |r| t_replace(black_box(pick(r))));
    sink = sink.wrapping_add(s);
    row("replace", n, us, 200, t_replace(&a));
    let (us, s) = timed(n, |r| t_lower(black_box(pick(r))));
    sink = sink.wrapping_add(s);
    row("trim_lower", n, us, 200, t_lower(&a));
    let (us, s) = timed(n, |r| t_chars(black_box(pick(r))));
    sink = sink.wrapping_add(s);
    row("char_walk", n, us, a.len() as i64, t_chars(&a));
    let (us, s) = timed(n, |r| t_bytes(black_box(pick(r))));
    sink = sink.wrapping_add(s);
    row("byte_walk", n, us, a.len() as i64, t_bytes(&a));
    let (us, s) = timed(n, |r| t_format(black_box(2000), r & 1));
    sink = sink.wrapping_add(s);
    row("format_num", n, us, 2000, t_format(2000, 0));
    let (us, s) = timed(n, |r| t_parse(black_box(if r & 1 == 0 { &na } else { &nb })));
    sink = sink.wrapping_add(s);
    row("parse_num", n, us, 2000, t_parse(&na));
    let (us, s) = timed(n, |r| t_join(black_box(if r & 1 == 0 { &wa } else { &wb })));
    sink = sink.wrapping_add(s);
    row("join", n, us, 2000, t_join(&wa));

    println!("time: 0ms sink={}", black_box(sink));
}
