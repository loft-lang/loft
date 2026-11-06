// Benchmark 22: A recursive enum — arithmetic expressions as a user-built tree (the Rust twin:
// `Box<Expr>` children).
//
// Four routines, each ONE OP per repetition on a complete expression of depth 10 (1,024
// numbers, 1,023 operators): its SHAPE is fixed, the salt changes which operator each node is
// and which number each leaf holds, so the work never varies with it.  Folded into `sink`:
//   expr_build     an expression built by recursion, then its size and its value
//   expr_eval      the value of a prebuilt expression (modulo 1,000,000,007), its numbers shifted
//                  by the salt
//   expr_simplify  a prebuilt expression rewritten without `x * 0`, `x * 1` and `x + 0`, then the
//                  size and the value of the result (the value must not change)
//   expr_render    a prebuilt expression rendered as fully parenthesised text, then its length
//                  and a hash of its characters
// The `hash` column is one canonical op's result: every lane must print the same one.
use std::hint::black_box;
use std::time::Instant;

enum Expr {
    Num(i64),
    Add(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
}

fn key(i: i64, salt: i64) -> i64 {
    ((i * 1103515245 + 12345 + salt * 7919) % 2147483648) % 100000
}

/// The complete expression of depth `d` whose root is node `i` in heap order.
fn gen(d: i64, i: i64, salt: i64) -> Expr {
    let h = key(i, salt);
    if d == 0 {
        return Expr::Num(h % 10);
    }
    let l = Box::new(gen(d - 1, 2 * i + 1, salt));
    let r = Box::new(gen(d - 1, 2 * i + 2, salt));
    if (h / 7) % 2 == 0 { Expr::Add(l, r) } else { Expr::Mul(l, r) }
}

fn size(e: &Expr) -> i64 {
    match e {
        Expr::Num(_) => 1,
        Expr::Add(l, r) | Expr::Mul(l, r) => 1 + size(l) + size(r),
    }
}

fn eval(e: &Expr, shift: i64) -> i64 {
    match e {
        Expr::Num(v) => v + shift,
        Expr::Add(l, r) => (eval(l, shift) + eval(r, shift)) % 1000000007,
        Expr::Mul(l, r) => (eval(l, shift) * eval(r, shift)) % 1000000007,
    }
}

fn is_num(e: &Expr, n: i64) -> bool {
    matches!(e, Expr::Num(v) if *v == n)
}

/// `x * 0` is 0, `x * 1` and `x + 0` are x; applied bottom-up.
fn simplify(e: &Expr) -> Expr {
    match e {
        Expr::Num(v) => Expr::Num(*v),
        Expr::Add(l, r) => {
            let a = simplify(l);
            let b = simplify(r);
            if is_num(&a, 0) {
                return b;
            }
            if is_num(&b, 0) {
                return a;
            }
            Expr::Add(Box::new(a), Box::new(b))
        }
        Expr::Mul(l, r) => {
            let a = simplify(l);
            let b = simplify(r);
            if is_num(&a, 0) || is_num(&b, 0) {
                return Expr::Num(0);
            }
            if is_num(&a, 1) {
                return b;
            }
            if is_num(&b, 1) {
                return a;
            }
            Expr::Mul(Box::new(a), Box::new(b))
        }
    }
}

fn render(e: &Expr) -> String {
    match e {
        Expr::Num(v) => format!("{v}"),
        Expr::Add(l, r) => format!("({}+{})", render(l), render(r)),
        Expr::Mul(l, r) => format!("({}*{})", render(l), render(r)),
    }
}

fn expr_build(salt: i64) -> i64 {
    let e = gen(10, 0, salt);
    size(&e) * 1000003 + eval(&e, 0)
}

fn expr_eval(e: &Expr, salt: i64) -> i64 {
    eval(e, salt)
}

fn expr_simplify(e: &Expr) -> i64 {
    let s = simplify(e);
    size(&s) * 1000003 + eval(&s, 0)
}

fn expr_render(e: &Expr) -> i64 {
    let s = render(e);
    let mut h = 0;
    for c in s.chars() {
        h = (h * 31 + c as i64) % 1000000007;
    }
    s.len() as i64 * 1000003 + h
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
        sink = sink.wrapping_add(expr_build(black_box(r & 1)));
    }
    row("expr_build", n, t0.elapsed().as_micros() as i64, 2047, expr_build(0), sink);
    let e = [gen(10, 0, 0), gen(10, 0, 1)];
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(expr_eval(&e[black_box(r & 1) as usize], r & 1));
    }
    row("expr_eval", n, t0.elapsed().as_micros() as i64, 2047, expr_eval(&e[0], 0), sink);
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(expr_simplify(&e[black_box(r & 1) as usize]));
    }
    row("expr_simplify", n, t0.elapsed().as_micros() as i64, 2047, expr_simplify(&e[0]), sink);
    let t0 = Instant::now();
    let mut sink: i64 = 0;
    for r in 0..n {
        sink = sink.wrapping_add(expr_render(&e[black_box(r & 1) as usize]));
    }
    row("expr_render", n, t0.elapsed().as_micros() as i64, 2047, expr_render(&e[0]), sink);
}
