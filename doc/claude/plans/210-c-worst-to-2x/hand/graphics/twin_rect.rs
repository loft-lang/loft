// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// graphics-reference — the pure-Rust twin of the `graphics` package's performance pass
// (bench/bench.loft), one file built with `rustc -O`.  It computes the SAME workloads with
// the SAME arithmetic, in the same order, and prints the same rows, hash included: a row
// whose hash matches the loft build's is a like-for-like comparison, and only then is a
// routine's loft time judged against it (@FR-Perf-Weight).  No dependencies and no
// cleverness — plain idiomatic Rust, the speed an industry implementation reaches without
// effort, which is exactly what the bar should be.
//
//     rustc -O --edition=2021 bench/bench.rs -o bench/.build/stats_rs && bench/.build/stats_rs --n 2
//
// Each routine is a port of its loft original in src/graphics.loft: a canvas is a row-major
// `Vec<u32>` of packed ARGB, coordinates are i64 like loft's, and every clip and bounds test
// the library makes is made here.  Where the census names a Rust idiom (a slice fill, a
// reused crossings vector with `sort_unstable`, an explicit `Vec<[i64; 8]>` stack) the twin
// uses it: the answer is the same, the idiom is what the bar is.  `black_box` guards each
// op's INPUT (the repetition number) and the sink — never anything inside a kernel.
use std::hint::black_box;
use std::time::Instant;

const FNV_OFFSET: i64 = 2166136261;
const FNV_PRIME: i64 = 16777619;
const PI: f64 = 3.14159265358979323846;

const RECTS: i64 = 4000;
const TRIS: i64 = 20000;
const LINES: i64 = 5000;
const CURVES: i64 = 2000;
const SHAPES: i64 = 20;

fn fnv(h0: i64, v: &[u32]) -> i64 {
    let mut h = h0;
    for &x in v {
        let w = x as i64;
        for sh in [24, 16, 8, 0] {
            h = ((h ^ ((w >> sh) & 255)) * FNV_PRIME) & 0xFFFF_FFFF;
        }
    }
    h
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
}

fn timed<F: FnMut(i64) -> i64>(n: i64, mut f: F) -> (i64, i64) {
    let t0 = Instant::now();
    let mut sink = 0i64;
    for r in 0..n {
        sink = sink.wrapping_add(f(black_box(r)));
    }
    (t0.elapsed().as_micros() as i64, black_box(sink))
}

// ── graphics.loft ───────────────────────────────────────────────────

fn rgba(r: i64, g: i64, b: i64, a: i64) -> u32 {
    (((a & 255) << 24) | ((r & 255) << 16) | ((g & 255) << 8) | (b & 255)) as u32
}

fn blend(dst: u32, src: u32) -> u32 {
    let sa = src >> 24;
    if sa == 255 {
        return src;
    }
    if sa == 0 {
        return dst;
    }
    let inv = 255 - sa;
    let ch = |c: u32, sh: u32| (c >> sh) & 255;
    let r = (ch(src, 16) * sa + ch(dst, 16) * inv) / 255;
    let g = (ch(src, 8) * sa + ch(dst, 8) * inv) / 255;
    let b = (ch(src, 0) * sa + ch(dst, 0) * inv) / 255;
    let a = sa + (ch(dst, 24) * inv) / 255;
    ((a & 255) << 24) | ((r & 255) << 16) | ((g & 255) << 8) | (b & 255)
}

struct Canvas {
    width: i64,
    height: i64,
    data: Vec<u32>,
}

fn canvas(w: i64, h: i64, fill: u32) -> Canvas {
    Canvas { width: w, height: h, data: vec![fill; (w * h).max(0) as usize] }
}

impl Canvas {
    fn get_pixel(&self, x: i64, y: i64) -> i64 {
        if x < 0 || x >= self.width || y < 0 || y >= self.height {
            return 0;
        }
        self.data.get((y * self.width + x) as usize).map_or(0, |&c| c as i64)
    }

    fn set_pixel(&mut self, x: i64, y: i64, color: u32) {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            if let Some(p) = self.data.get_mut((y * self.width + x) as usize) {
                *p = color;
            }
        }
    }

    fn clear(&mut self, color: u32) {
        self.data.fill(color);
    }

    fn blend_pixel(&mut self, x: i64, y: i64, color: u32) {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            if let Some(p) = self.data.get_mut((y * self.width + x) as usize) {
                *p = blend(*p, color);
            }
        }
    }

    fn fill_rect(&mut self, rx: i64, ry: i64, rw: i64, rh: i64, color: u32) {
        let x0 = rx.max(0);
        let y0 = ry.max(0);
        let x1 = (rx + rw).min(self.width);
        let y1 = (ry + rh).min(self.height);
        if x1 <= x0 {
            return;
        }
        for y in y0..y1 {
            let row = (y * self.width) as usize;
            for p in &mut self.data[row + x0 as usize..row + x1 as usize] {
                *p = color;
            }
        }
    }

    fn hline(&mut self, hx0: i64, hx1: i64, hy: i64, color: u32) {
        if hy < 0 || hy >= self.height {
            return;
        }
        let left = hx0.max(0);
        let right = hx1.min(self.width);
        if right <= left {
            return;
        }
        let row = (hy * self.width) as usize;
        self.data[row + left as usize..row + right as usize].fill(color);
    }

    /// Bresenham with the pixel write inline.
    fn draw_line(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, color: u32) {
        let dx = (x1 - x0).abs();
        let dy = (y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx - dy;
        let (mut cx, mut cy) = (x0, y0);
        for _ in 0..dx + dy + 1 {
            if cx >= 0 && cx < self.width && cy >= 0 && cy < self.height {
                self.data[(cy * self.width + cx) as usize] = color;
            }
            if cx == x1 && cy == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                cx += sx;
            }
            if e2 < dx {
                err += dx;
                cy += sy;
            }
        }
    }

    /// Adaptive de Casteljau subdivision on an explicit stack of segments, each
    /// `[x0, x1, x2, x3, y0, y1, y2, y3]`; at most 1000 pops, like the library.
    #[allow(clippy::too_many_arguments)]
    fn draw_bezier(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, x2: i64, y2: i64, x3: i64,
                   y3: i64, color: u32) {
        let mut stack: Vec<[i64; 8]> = vec![[x0, x1, x2, x3, y0, y1, y2, y3]];
        for _ in 0..1000 {
            let Some([p0x, p1x, p2x, p3x, p0y, p1y, p2y, p3y]) = stack.pop() else { break };
            let dx = p3x - p0x;
            let dy = p3y - p0y;
            let d1 = ((p1x - p3x) * dy - (p1y - p3y) * dx).abs();
            let d2 = ((p2x - p3x) * dy - (p2y - p3y) * dx).abs();
            let flat = d1.max(d2);
            let chord2 = dx * dx + dy * dy;
            if flat * flat <= chord2 || chord2 == 0 {
                self.draw_line(p0x, p0y, p3x, p3y, color);
            } else {
                let (m01x, m01y) = ((p0x + p1x) / 2, (p0y + p1y) / 2);
                let (m12x, m12y) = ((p1x + p2x) / 2, (p1y + p2y) / 2);
                let (m23x, m23y) = ((p2x + p3x) / 2, (p2y + p3y) / 2);
                let (m012x, m012y) = ((m01x + m12x) / 2, (m01y + m12y) / 2);
                let (m123x, m123y) = ((m12x + m23x) / 2, (m12y + m23y) / 2);
                let (mx, my) = ((m012x + m123x) / 2, (m012y + m123y) / 2);
                stack.push([mx, m123x, m23x, p3x, my, m123y, m23y, p3y]);
                stack.push([p0x, m01x, m012x, mx, p0y, m01y, m012y, my]);
            }
        }
    }

    /// Flat-top / flat-bottom scanline fill, integer edges multiplied before dividing.
    #[allow(clippy::too_many_arguments)]
    fn fill_triangle(&mut self, tx0: i64, ty0: i64, tx1: i64, ty1: i64, tx2: i64, ty2: i64,
                     color: u32) {
        let mut v = [(tx0, ty0), (tx1, ty1), (tx2, ty2)];
        if v[0].1 > v[1].1 {
            v.swap(0, 1);
        }
        if v[1].1 > v[2].1 {
            v.swap(1, 2);
        }
        if v[0].1 > v[1].1 {
            v.swap(0, 1);
        }
        let [(ax, ay), (bx, by), (cx, cy)] = v;
        if ay == cy {
            return;
        }
        for y in ay..cy + 1 {
            let xac = ax + (cx - ax) * (y - ay) / (cy - ay);
            let mut xother = xac;
            if y < by && by != ay {
                xother = ax + (bx - ax) * (y - ay) / (by - ay);
            }
            if y >= by && cy != by {
                xother = bx + (cx - bx) * (y - by) / (cy - by);
            }
            if y >= by && cy == by {
                xother = bx;
            }
            self.hline(xac.min(xother), xac.max(xother) + 1, y, color);
        }
    }

    /// Even-odd scanline fill; the crossings go into one reused vector.
    fn fill_polygon(&mut self, pts: &[(i64, i64)], xs: &mut Vec<i64>, color: u32) {
        let n = pts.len();
        if n < 3 {
            return;
        }
        let top = pts.iter().map(|p| p.1).min().unwrap_or(0).max(0);
        let bot = pts.iter().map(|p| p.1).max().unwrap_or(0).min(self.height - 1);
        for y in top..=bot {
            let closing = y == bot;
            xs.clear();
            for i in 0..n {
                let a = pts[i];
                let b = pts[(i + 1) % n];
                let (t, u) = if a.1 < b.1 { (a, b) } else { (b, a) };
                if t.1 == u.1 || y < t.1 || y > u.1 || (y == u.1 && !closing) {
                    continue;
                }
                xs.push(t.0 + (u.0 - t.0) * (y - t.1) / (u.1 - t.1));
            }
            xs.sort_unstable();
            for pair in xs.chunks_exact(2) {
                self.hline(pair[0], pair[1] + 1, y, color);
            }
        }
    }
}

// ── The rows ────────────────────────────────────────────────────────

fn ink(r: i64, k: i64) -> u32 {
    rgba((k * 7 + r) & 255, (k >> 3) & 255, (r * 5 + 11) & 255, 255)
}

fn rect_op(cv: &mut Canvas, r: i64) {
    for k in 0..RECTS {
        cv.fill_rect((k * 97) % 961, (k * 61) % 977, 64, 48, ink(r, k));
    }
}

fn bench_fill_rect(n: i64) -> Row {
    let mut cv = canvas(1024, 1024, 0);
    let (us, sink) = timed(n, |r| {
        rect_op(&mut cv, r);
        cv.get_pixel(500, 500) & 255
    });
    let mut one = canvas(1024, 1024, 0);
    rect_op(&mut one, black_box(0));
    Row { name: "fill_rect", iters: n, us, px: RECTS * 64 * 48, hash: fnv(FNV_OFFSET, &one.data), sink }
}

fn bench_canvas(n: i64) -> Row {
    let (us, sink) = timed(n, |r| {
        let cv = black_box(canvas(1920, 1080, ink(r, 0)));
        cv.data.len() as i64 + (cv.get_pixel(r & 1023, 700) & 255)
    });
    let one = canvas(1920, 1080, ink(black_box(0), 0));
    Row { name: "canvas", iters: n, us, px: 1920 * 1080, hash: fnv(FNV_OFFSET, &one.data), sink }
}

fn bench_clear(n: i64) -> Row {
    let mut cv = canvas(1280, 720, 0);
    let (us, sink) = timed(n, |r| {
        cv.clear(ink(r, 0));
        cv.get_pixel(r & 1023, 400) & 255
    });
    let mut one = canvas(1280, 720, 0);
    one.clear(ink(black_box(0), 0));
    Row { name: "clear", iters: n, us, px: 1280 * 720, hash: fnv(FNV_OFFSET, &one.data), sink }
}

fn blend_canvas() -> Canvas {
    let mut cv = canvas(512, 512, 0);
    for y in 0..512 {
        for x in 0..512 {
            cv.set_pixel(x, y, rgba(y & 255, x & 255, 128, (x * y + 64) & 255));
        }
    }
    cv
}

fn blend_op(cv: &mut Canvas, r: i64) {
    for y in 0..512 {
        for x in 0..512 {
            cv.blend_pixel(x, y, rgba(x & 255, y & 255, (x ^ y) & 255, (x + y + r) & 255));
        }
    }
}

fn bench_blend(n: i64) -> Row {
    let mut cv = blend_canvas();
    let (us, sink) = timed(n, |r| {
        blend_op(&mut cv, r);
        cv.get_pixel(r & 511, 300) & 255
    });
    let mut one = blend_canvas();
    blend_op(&mut one, black_box(0));
    Row { name: "blend_pixel", iters: n, us, px: 512 * 512, hash: fnv(FNV_OFFSET, &one.data), sink }
}

fn tri_op(cv: &mut Canvas, r: i64) {
    for k in 0..TRIS {
        let cx = (k * 97) & 511;
        let cy = (k * 61) & 511;
        cv.fill_triangle(cx + ((k * 13) & 63) - 32, cy - ((k * 7) & 31) - 8,
                         cx - ((k * 29) & 63) + 16, cy + ((k * 11) & 31) + 4,
                         cx + ((k * 5) & 47) - 20, cy + ((k * 17) & 63) - 10, ink(r, k));
    }
}

fn bench_triangle(n: i64) -> Row {
    let mut cv = canvas(512, 512, 0);
    let (us, sink) = timed(n, |r| {
        tri_op(&mut cv, r);
        cv.get_pixel(256, 256) & 255
    });
    let mut one = canvas(512, 512, 0);
    tri_op(&mut one, black_box(0));
    Row { name: "fill_triangle", iters: n, us, px: TRIS, hash: fnv(FNV_OFFSET, &one.data), sink }
}

fn line_ends(k: i64) -> [i64; 4] {
    let (mut x0, mut y0) = (16 + ((k * 37) & 255), 16 + ((k * 53) & 255));
    let (mut x1, mut y1) = (x0 + 400 + ((k * 11) & 255), y0 + ((k * 19) & 511));
    if k & 2 == 2 {
        std::mem::swap(&mut x0, &mut y0);
        std::mem::swap(&mut x1, &mut y1);
    }
    if k & 1 == 1 { [x1, y1, x0, y0] } else { [x0, y0, x1, y1] }
}

fn line_op(cv: &mut Canvas, ends: &[[i64; 4]], r: i64) {
    for (k, e) in ends.iter().enumerate() {
        cv.draw_line(e[0], e[1], e[2], e[3], ink(r, k as i64));
    }
}

fn bench_line(n: i64) -> Row {
    let ends: Vec<[i64; 4]> = (0..LINES).map(line_ends).collect();
    let px: i64 = ends.iter().map(|e| (e[2] - e[0]).abs().max((e[3] - e[1]).abs()) + 1).sum();
    let mut cv = canvas(1024, 1024, 0);
    let (us, sink) = timed(n, |r| {
        line_op(&mut cv, &ends, r);
        cv.get_pixel(300, 300) & 255
    });
    let mut one = canvas(1024, 1024, 0);
    line_op(&mut one, &ends, black_box(0));
    Row { name: "draw_line", iters: n, us, px, hash: fnv(FNV_OFFSET, &one.data), sink }
}

fn bezier_op(cv: &mut Canvas, r: i64) {
    for k in 0..CURVES {
        let x0 = 40 + ((k * 37) & 511);
        let y0 = 200 + ((k * 53) & 511);
        let x3 = x0 + 200 + ((k * 11) & 127);
        let y3 = y0 + ((k * 19) & 255);
        cv.draw_bezier(x0, y0, x0 + ((k * 7) & 127), y0 + 100 + ((k * 23) & 127),
                       x3 - ((k * 13) & 127), y3 - 100 - ((k * 29) & 63), x3, y3, ink(r, k));
    }
}

fn bench_bezier(n: i64) -> Row {
    let mut cv = canvas(1024, 1024, 0);
    let (us, sink) = timed(n, |r| {
        bezier_op(&mut cv, r);
        cv.get_pixel(400, 500) & 255
    });
    let mut one = canvas(1024, 1024, 0);
    bezier_op(&mut one, black_box(0));
    Row { name: "draw_bezier", iters: n, us, px: CURVES, hash: fnv(FNV_OFFSET, &one.data), sink }
}

fn regular(cx: f64, cy: f64, r: f64, n: i64) -> Vec<(i64, i64)> {
    (0..n)
        .map(|i| {
            let a = 2.0 * PI * (i as f64) / (n as f64);
            ((cx + r * a.cos()) as i64, (cy + r * a.sin()) as i64)
        })
        .collect()
}

fn pentagram(cx: f64, cy: f64) -> Vec<(i64, i64)> {
    (0..5)
        .map(|k| {
            let a = (-90.0 + (((k * 2) % 5) as f64) * 72.0) * PI / 180.0;
            ((cx + 110.0 * a.cos()) as i64, (cy + 110.0 * a.sin()) as i64)
        })
        .collect()
}

fn shapes() -> Vec<Vec<(i64, i64)>> {
    let mut out = Vec::new();
    for j in 0..SHAPES {
        out.push(regular(118.0 + j as f64, 123.0 + ((j * 7) % 11) as f64, 100.0, 28));
        out.push(pentagram(123.0 + (j % 10) as f64, 120.0 + (j / 2) as f64));
    }
    out
}

fn poly_op(cv: &mut Canvas, shapes: &[Vec<(i64, i64)>], xs: &mut Vec<i64>, r: i64) {
    for (j, s) in shapes.iter().enumerate() {
        cv.fill_polygon(s, xs, ink(r, j as i64));
    }
}

fn bench_polygon(n: i64) -> Row {
    let shapes = shapes();
    let mut xs: Vec<i64> = Vec::new();
    let mut cv = canvas(256, 256, 0);
    let (us, sink) = timed(n, |r| {
        poly_op(&mut cv, &shapes, &mut xs, r);
        cv.get_pixel(128, 60) & 255
    });
    let mut one = canvas(256, 256, 0);
    poly_op(&mut one, &shapes, &mut xs, black_box(0));
    Row { name: "fill_polygon", iters: n, us, px: shapes.len() as i64,
          hash: fnv(FNV_OFFSET, &one.data), sink }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut n: i64 = 20;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--n" && i + 1 < args.len() {
            n = args[i + 1].parse().unwrap_or(20);
            i += 1;
        }
        i += 1;
    }
    if n < 1 {
        n = 1;
    }
    let t0 = Instant::now();
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let rows = [bench_fill_rect(n)];
    let mut sink = 0i64;
    for row in &rows {
        print_row(row);
        sink = sink.wrapping_add(row.sink);
    }
    println!("time: {}ms sink={}", t0.elapsed().as_millis(), sink);
}
