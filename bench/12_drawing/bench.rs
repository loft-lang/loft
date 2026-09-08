// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Benchmark 12 reference: the same two rows (`hash`, `lock`) in plain Rust,
// ported routine-for-routine from the drawing library's bench/bench.rs — same
// arithmetic in the same order, i64 pixels, truncating casts.  Built by
// bench/run_bench.sh with a bare `rustc -O`; prints the same TSV rows plus the
// trailing "time: Xms" the runner parses, and asserts the same output hashes.
#![allow(dead_code)]
use std::time::Instant;

// Pinned on 2026-09-07 — see bench.loft's HASH_EXPECT/LOCK_EXPECT.
const HASH_EXPECT: i64 = 0x7698ffff;
const LOCK_EXPECT: i64 = 0x33e56005;

const FNV_OFFSET: i64 = 2166136261;
const FNV_PRIME: i64 = 16777619;
const PI: f64 = 3.141592653589793;

fn fnv(h0: i64, v: &[i64]) -> i64 {
    let mut h = h0;
    for &x in v {
        let w = x & 0xFFFF_FFFF;
        for sh in [24, 16, 8, 0] {
            h = ((h ^ ((w >> sh) & 255)) * FNV_PRIME) & 0xFFFF_FFFF;
        }
    }
    h
}

fn milli(v: f64) -> i64 {
    (v * 1000.0) as i64
}

struct Row {
    name: &'static str,
    iters: i64,
    us: i64,
    px: i64,
    hash: i64,
    sink: i64,
}

fn print_row(r: &Row) {
    let ns_op = r.us * 1000 / r.iters;
    let ns_px = if r.px > 0 { (r.us * 1000) as f64 / (r.iters * r.px) as f64 } else { 0.0 };
    println!("{}\t{}\t{}\t{}\t{}\t{:.3}\t{:x}", r.name, r.iters, r.us, ns_op, r.px, ns_px, r.hash);
    if r.sink == i64::MIN {
        println!("(unreachable — keeps the sink alive)");
    }
}

fn timed<F: FnMut() -> i64>(n: i64, mut f: F) -> (i64, i64) {
    let t0 = Instant::now();
    let mut sink = 0i64;
    for _ in 0..n {
        sink = sink.wrapping_add(f());
    }
    (t0.elapsed().as_micros() as i64, sink)
}

// ── noise.loft ──────────────────────────────────────────────────────

fn seed_hash(seed: i64, idx: i64, salt: i64) -> f64 {
    let mut hx = ((seed * 73856093) ^ (idx * 19349663) ^ (salt * 83492791)) & 0xFFFF_FFFF;
    hx = (hx ^ (hx >> 13)) & 0xFFFF_FFFF;
    hx = (hx * 1274126177) & 0xFFFF_FFFF;
    (hx as f64 / 4294967295.0) * 2.0 - 1.0
}

fn seed_wave(seed: i64, u: f64) -> f64 {
    let p1 = seed_hash(seed, 0, 11) * PI;
    let p2 = seed_hash(seed, 0, 22) * PI;
    0.6 * (2.0 * PI * u + p1).sin() + 0.4 * (4.0 * PI * u + p2).sin()
}

// ── brush.loft: the footprint ───────────────────────────────────────

fn clampf(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo { lo } else if v > hi { hi } else { v }
}

fn clampi(v: i64, lo: i64, hi: i64) -> i64 {
    if v < lo { lo } else if v > hi { hi } else { v }
}

fn floor_i(v: f64) -> i64 {
    let i = v as i64;
    if (i as f64) > v { i - 1 } else { i }
}

fn hair_brush(bw: i64, bh: i64, seed: i64, gap: f64) -> Vec<i64> {
    let mut img = vec![0i64; (bw * bh) as usize];
    let mut x = 0i64;
    let mut ci = 0i64;
    while x < bw {
        let cw = 1 + ((2.99 * (seed_hash(seed, ci, 7) + 1.0) * 0.5) as i64);
        let val = 0.72 + 0.28 * (seed_hash(seed, ci, 8) + 1.0) * 0.5;
        let split = (seed_hash(seed, ci, 9) + 1.0) * 0.5 < gap;
        for k in 0..cw {
            let xx = x + k;
            if xx >= bw {
                break;
            }
            let edge = if xx == 0 || xx == bw - 1 { 0.6 } else { 1.0 };
            for y in 0..bh {
                let u = y as f64 / bh as f64;
                let v = val * (1.0 - 0.12 * (1.0 + seed_wave(seed * 3 + ci, u)) * 0.5);
                let mut a = edge;
                if split {
                    a = 0.35 * edge * clampf((seed_wave(seed * 7 + ci, u) + 0.6) / 0.6, 0.0, 1.0);
                }
                let g = (255.0 * v + 0.5) as i64;
                let ai = (255.0 * a + 0.5) as i64;
                img[(y * bw + xx) as usize] = (ai << 24) | (g << 16) | (g << 8) | g;
            }
        }
        x += cw;
        ci += 1;
    }
    img
}

// ── drawing.loft: smoothing ─────────────────────────────────────────

struct Brush {
    bw: i64,
    bh: i64,
    img: Vec<i64>,
}

#[derive(Clone)]
struct LockStyle {
    w0: f64,
    w: f64,
    swell: f64,
    body: f64,
    tips: i64,
    tipvar: f64,
    spread: f64,
    seed: i64,
    period: f64,
    dark: i64,
    base: i64,
    lit: i64,
    lx: f64,
    ly: f64,
    lz: f64,
    alpha: f64,
    flip: bool,
}

struct Layer {
    x0: i64,
    y0: i64,
    lw: i64,
    lh: i64,
    px: Vec<i64>,
}

fn rgba(r: i64, g: i64, b: i64, a: i64) -> i64 {
    (a << 24) | (r << 16) | (g << 8) | b
}
fn color_r(c: i64) -> i64 { (c >> 16) & 255 }
fn color_g(c: i64) -> i64 { (c >> 8) & 255 }
fn color_b(c: i64) -> i64 { c & 255 }
fn color_a(c: i64) -> i64 { (c >> 24) & 255 }

struct PathPt {
    x: f64,
    y: f64,
    tx: f64,
    ty: f64,
}

fn path_at(px: &[f64], py: &[f64], cum: &[f64], dist: f64) -> PathPt {
    let n = px.len();
    let mut k = n - 2;
    for i in 0..(n - 1) {
        if dist <= cum[i + 1] {
            k = i;
            break;
        }
    }
    let seg = cum[k + 1] - cum[k];
    let u = (dist - cum[k]) / seg;
    let dx = px[k + 1] - px[k];
    let dy = py[k + 1] - py[k];
    PathPt { x: px[k] + u * dx, y: py[k] + u * dy, tx: dx / seg, ty: dy / seg }
}

fn lock_width(t: f64, w0: f64, w: f64, swell: f64) -> f64 {
    if swell <= 0.000000001 || t >= swell {
        return w;
    }
    w0 + (w - w0) * (0.5 * PI * t / swell).sin()
}

struct Ribbon {
    rx: Vec<f64>,
    ry: Vec<f64>,
    rhw: Vec<f64>,
    ral: Vec<f64>,
    rmx: Vec<f64>,
    slo: f64,
    shi: f64,
}

fn lock_ribbons(xs: &[f64], ys: &[f64], st: &LockStyle) -> Vec<Ribbon> {
    let mut px = vec![xs[0]];
    let mut py = vec![ys[0]];
    let mut cum = vec![0.0];
    for i in 1..xs.len() {
        let ddx = xs[i] - px[px.len() - 1];
        let ddy = ys[i] - py[py.len() - 1];
        let d = (ddx * ddx + ddy * ddy).sqrt();
        if d > 0.000001 {
            px.push(xs[i]);
            py.push(ys[i]);
            cum.push(cum[cum.len() - 1] + d);
        }
    }
    let mut out: Vec<Ribbon> = Vec::new();
    if px.len() < 2 {
        return out;
    }
    let len = cum[cum.len() - 1];
    let body = clampf(st.body, 0.05, 1.0);
    let swell = clampf(st.swell, 0.0, body);
    let ds = clampf(0.5 * st.w, 2.0, 6.0);
    let mut nb = -floor_i(-(body * len / ds));
    if nb < 2 {
        nb = 2;
    }
    nb += 1;
    let mut bx = Vec::new();
    let mut by = Vec::new();
    let mut bhw = Vec::new();
    let mut bal = Vec::new();
    let mut bmx = Vec::new();
    for j in 0..nb {
        let t = body * j as f64 / (nb - 1) as f64;
        let dist = t * len;
        let q = path_at(&px, &py, &cum, dist);
        bx.push(q.x);
        by.push(q.y);
        bhw.push(0.5 * lock_width(t, st.w0, st.w, swell));
        bal.push(dist);
        bmx.push(0.0);
    }
    out.push(Ribbon { rx: bx, ry: by, rhw: bhw, ral: bal, rmx: bmx, slo: -1.0, shi: 1.0 });
    let hwb = 0.5 * lock_width(body, st.w0, st.w, swell);
    let tail = (1.0 - body) * len;
    if st.tips <= 0 || tail < 1.0 || hwb < 0.3 {
        return out;
    }
    let mut raw = Vec::new();
    let mut tot = 0.0;
    for i in 0..st.tips {
        let r = 1.0 + 0.5 * seed_hash(st.seed, i, 41);
        raw.push(r);
        tot += r;
    }
    let mut a1 = -1.0;
    for i in 0..st.tips {
        let a0 = a1;
        a1 = a0 + (2.0 * raw[i as usize] / tot);
        let c = 0.5 * (a0 + a1);
        let hs = 0.5 * (a1 - a0);
        let mut ell = tail * (1.0 + st.tipvar * seed_hash(st.seed, i, 42));
        if ell < 0.15 * tail {
            ell = 0.15 * tail;
        }
        let th = st.spread * seed_hash(st.seed, i, 43) * (PI / 180.0);
        let mut m = -floor_i(-(ell / ds));
        if m < 2 {
            m = 2;
        }
        m += 1;
        let mut sx = Vec::new();
        let mut sy = Vec::new();
        let mut shw = Vec::new();
        let mut sal = Vec::new();
        let mut smx = Vec::new();
        for j in 0..m {
            let u = j as f64 / (m - 1) as f64;
            let sd = body * len + u * ell * th.cos();
            let sq = path_at(&px, &py, &cum, sd);
            let off = c * hwb + u * ell * th.sin();
            sx.push(sq.x + sq.ty * off);
            sy.push(sq.y - sq.tx * off);
            shw.push(hs * hwb * (1.0 - u * u));
            sal.push(sd);
            smx.push(u);
        }
        out.push(Ribbon { rx: sx, ry: sy, rhw: shw, ral: sal, rmx: smx, slo: a0, shi: a1 });
    }
    out
}

struct Lay {
    x0: i64,
    y0: i64,
    lw: i64,
    lh: i64,
    best: Vec<f64>,
    sb: Vec<f64>,
    sl: Vec<f64>,
    al: Vec<f64>,
    mx: Vec<f64>,
    nx: Vec<f64>,
    ny: Vec<f64>,
}

#[allow(clippy::too_many_arguments)]
fn raster_segment(lay: &mut Lay, ax: f64, ay: f64, bx: f64, by: f64, hwa: f64, hwb: f64,
                  ala: f64, alb: f64, mxa: f64, mxb: f64, slo: f64, shi: f64) {
    let dx = bx - ax;
    let dy = by - ay;
    let l2 = dx * dx + dy * dy;
    if l2 < 0.000000001 {
        return;
    }
    let ln = l2.sqrt();
    let nx = dy / ln;
    let ny = -dx / ln;
    let hm = if hwa > hwb { hwa } else { hwb };
    let mut x0 = floor_i((if ax < bx { ax } else { bx }) - hm);
    if x0 < lay.x0 {
        x0 = lay.x0;
    }
    let mut x1 = -floor_i(-((if ax > bx { ax } else { bx }) + hm));
    if x1 > lay.x0 + lay.lw - 1 {
        x1 = lay.x0 + lay.lw - 1;
    }
    let mut y0 = floor_i((if ay < by { ay } else { by }) - hm);
    if y0 < lay.y0 {
        y0 = lay.y0;
    }
    let mut y1 = -floor_i(-((if ay > by { ay } else { by }) + hm));
    if y1 > lay.y0 + lay.lh - 1 {
        y1 = lay.y0 + lay.lh - 1;
    }
    if x1 < x0 || y1 < y0 {
        return;
    }
    for yy in y0..=y1 {
        let cy = yy as f64 + 0.5;
        for xx in x0..=x1 {
            let cx = xx as f64 + 0.5;
            let u = clampf(((cx - ax) * dx + (cy - ay) * dy) / l2, 0.0, 1.0);
            let hw = hwa + (hwb - hwa) * u;
            if hw <= 0.01 {
                continue;
            }
            let dist = (cx - ax - u * dx) * nx + (cy - ay - u * dy) * ny;
            let s = dist / hw;
            let a = if s >= 0.0 { s } else { -s };
            if a > 1.0 {
                continue;
            }
            let idx = ((yy - lay.y0) * lay.lw + (xx - lay.x0)) as usize;
            if a < lay.best[idx] {
                lay.best[idx] = a;
                lay.sl[idx] = s;
                lay.sb[idx] = slo + (s + 1.0) * 0.5 * (shi - slo);
                lay.al[idx] = ala + (alb - ala) * u;
                lay.mx[idx] = mxa + (mxb - mxa) * u;
                lay.nx[idx] = nx;
                lay.ny[idx] = ny;
            }
        }
    }
}

struct Smp {
    a: f64,
    r: f64,
    g: f64,
    b: f64,
}

fn chan(c: i64, sh: i64) -> f64 {
    ((c >> sh) & 255) as f64
}

fn brush_sample(img: &[i64], bw: i64, bh: i64, fu: f64, fv: f64) -> Smp {
    let iu = floor_i(fu);
    let tu = fu - iu as f64;
    let iv = fv as i64;
    let tv = fv - iv as f64;
    let u0 = clampi(iu, 0, bw - 1);
    let u1 = clampi(iu + 1, 0, bw - 1);
    let v0 = iv % bh;
    let v1 = (iv + 1) % bh;
    let c00 = img[(v0 * bw + u0) as usize];
    let c10 = img[(v0 * bw + u1) as usize];
    let c01 = img[(v1 * bw + u0) as usize];
    let c11 = img[(v1 * bw + u1) as usize];
    let w00 = (1.0 - tu) * (1.0 - tv);
    let w10 = tu * (1.0 - tv);
    let w01 = (1.0 - tu) * tv;
    let w11 = tu * tv;
    let mix = |sh: i64| chan(c00, sh) * w00 + chan(c10, sh) * w10 + chan(c01, sh) * w01 + chan(c11, sh) * w11;
    Smp { a: mix(24), r: mix(16), g: mix(8), b: mix(0) }
}

fn ramp(c0: i64, c1: i64, f: f64, v: f64) -> i64 {
    ((c0 as f64 + (c1 - c0) as f64 * f) * v / 255.0 + 0.5) as i64
}

fn lock_layer(xs: &[f64], ys: &[f64], cw: i64, ch: i64, br: &Brush, st: &LockStyle) -> Layer {
    let none = Layer { x0: 0, y0: 0, lw: 0, lh: 0, px: Vec::new() };
    let rib = lock_ribbons(xs, ys, st);
    if rib.is_empty() {
        return none;
    }
    let mut minx = 1000000000000000000.0f64;
    let mut miny = 1000000000000000000.0f64;
    let mut maxx = -1000000000000000000.0f64;
    let mut maxy = -1000000000000000000.0f64;
    for r in &rib {
        for i in 0..r.rx.len() {
            let x = r.rx[i];
            let y = r.ry[i];
            let h = r.rhw[i];
            if x - h < minx { minx = x - h; }
            if x + h > maxx { maxx = x + h; }
            if y - h < miny { miny = y - h; }
            if y + h > maxy { maxy = y + h; }
        }
    }
    let mut x0 = floor_i(minx) - 1;
    if x0 < 0 { x0 = 0; }
    let mut y0 = floor_i(miny) - 1;
    if y0 < 0 { y0 = 0; }
    let mut x1 = floor_i(maxx) + 1;
    if x1 > cw - 1 { x1 = cw - 1; }
    let mut y1 = floor_i(maxy) + 1;
    if y1 > ch - 1 { y1 = ch - 1; }
    if x1 < x0 || y1 < y0 {
        return none;
    }
    let lw = x1 - x0 + 1;
    let lh = y1 - y0 + 1;
    let n = (lw * lh) as usize;
    let mut lay = Lay {
        x0, y0, lw, lh,
        best: vec![2.0; n], sb: vec![0.0; n], sl: vec![0.0; n], al: vec![0.0; n],
        mx: vec![0.0; n], nx: vec![0.0; n], ny: vec![0.0; n],
    };
    for r in &rib {
        for i in 0..(r.rx.len() - 1) {
            raster_segment(&mut lay, r.rx[i], r.ry[i], r.rx[i + 1], r.ry[i + 1],
                           r.rhw[i], r.rhw[i + 1], r.ral[i], r.ral[i + 1],
                           r.rmx[i], r.rmx[i + 1], r.slo, r.shi);
        }
    }
    let mut lx = st.lx;
    let mut ly = st.ly;
    let mut lz = st.lz;
    let mut ll = (lx * lx + ly * ly + lz * lz).sqrt();
    if ll < 0.000000001 {
        lx = 0.0;
        ly = 0.0;
        lz = 1.0;
        ll = 1.0;
    }
    lx /= ll;
    ly /= ll;
    lz /= ll;
    let crest = clampf(lz, 0.05, 0.95);
    let period = if st.period > 0.000001 { st.period } else { 1.0 };
    let phase = (seed_hash(st.seed, 0, 44) + 1.0) * 0.5 * period;
    let bwf = br.bw as f64;
    let bhf = br.bh as f64;
    let mut out = vec![0i64; n];
    for idx in 0..n {
        if lay.best[idx] > 1.5 {
            continue;
        }
        let mut s = lay.sb[idx];
        let mut uu = (s + 1.0) * 0.5;
        if st.flip {
            uu = 1.0 - uu;
        }
        let smp = brush_sample(&br.img, br.bw, br.bh, uu * bwf - 0.5,
                               ((lay.al[idx] + phase) / period) * bhf - 0.5 + bhf * 4096.0);
        let ia = smp.a * st.alpha;
        if ia < 0.5 {
            continue;
        }
        s = s + (lay.sl[idx] - s) * lay.mx[idx];
        let nz2 = 1.0 - s * s;
        let nz = (if nz2 > 0.0 { nz2 } else { 0.0 }).sqrt();
        let lit = clampf(s * lay.nx[idx] * lx + s * lay.ny[idx] * ly + nz * lz, 0.0, 1.0);
        let (c0, c1, f) = if lit < crest {
            (st.dark, st.base, lit / crest)
        } else {
            (st.base, st.lit, (lit - crest) / (1.0 - crest))
        };
        let r = ramp(color_r(c0), color_r(c1), f, smp.r);
        let g = ramp(color_g(c0), color_g(c1), f, smp.g);
        let b = ramp(color_b(c0), color_b(c1), f, smp.b);
        out[idx] = (((ia + 0.5) as i64) << 24) | (clampi(r, 0, 255) << 16) | (clampi(g, 0, 255) << 8) | clampi(b, 0, 255);
    }
    Layer { x0, y0, lw, lh, px: out }
}

fn bench_hash(n: i64) -> Row {
    let (us, sink) = timed(n, || {
        let mut acc = 0.0;
        for i in 0..100000 {
            acc += seed_hash(1, i, 7);
        }
        (acc * 1000.0) as i64
    });
    let mut one = 0.0;
    for i in 0..100000 {
        one += seed_hash(1, i, 7);
    }
    Row { name: "hash", iters: n, us, px: 100000, hash: fnv(FNV_OFFSET, &[(one * 1000000.0) as i64]), sink }
}

fn hair() -> Brush {
    Brush { bw: 12, bh: 48, img: hair_brush(12, 48, 1, 0.35) }
}

fn lock_style(w: f64, tips: i64, seed: i64) -> LockStyle {
    LockStyle {
        w0: 6.0, w, swell: 0.3, body: 0.8, tips, tipvar: 0.35, spread: 8.0, seed, period: 144.0,
        dark: rgba(60, 40, 20, 255), base: rgba(120, 80, 40, 255), lit: rgba(167, 141, 115, 255),
        lx: -0.5, ly: -0.8, lz: 0.6, alpha: 1.0, flip: false,
    }
}

fn bench_lock(n: i64) -> Row {
    let br = hair();
    let st = lock_style(60.0, 3, 1);
    let xs = [18.0, 342.0];
    let ys = [90.0, 90.0];
    let (us, sink) = timed(n, || lock_layer(&xs, &ys, 360, 180, &br, &st).lw);
    let one = lock_layer(&xs, &ys, 360, 180, &br, &st);
    Row { name: "lock", iters: n, us, px: one.lw * one.lh, hash: fnv(FNV_OFFSET, &one.px), sink }
}


fn main() {
    let t0 = Instant::now();
    let args: Vec<String> = std::env::args().collect();
    let mut n: i64 = 5;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--n" && i + 1 < args.len() {
            n = args[i + 1].parse().unwrap_or(5);
            i += 1;
        }
        i += 1;
    }
    if n < 1 {
        n = 1;
    }
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let h = bench_hash(n);
    print_row(&h);
    let l = bench_lock(n);
    print_row(&l);
    assert!(h.hash == HASH_EXPECT, "hash row: got {:x}, want {:x}", h.hash, HASH_EXPECT);
    assert!(l.hash == LOCK_EXPECT, "lock row: got {:x}, want {:x}", l.hash, LOCK_EXPECT);
    println!("time: {}ms", t0.elapsed().as_millis());
}
