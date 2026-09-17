// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// The pure-Rust reference for the FOUR rows of the drawing package's bench that were
// measured without one — `parse`, `render_lock`, `render_marks`, `resize` — written in
// the loft repo (@PLN157 § V-ah, 2026-09-14).  Appended to the package's
// `drawing/bench/bench.rs` on 2026-09-17 (loft-libs-graphics `drawing-lock`, 47947ec), which
// is now the copy to keep current; this one is the record.  It was appended with the four
// `print_row` lines added to that file's `main` (after `bench_wide_line`):
//
//     print_row(&bench_parse(n));
//     print_row(&bench_render(n, "render_lock", lock_scene()));
//     print_row(&bench_render(n, "render_marks", marks_scene()));
//     print_row(&bench_resize(n));
//
// Every hash agreed with the loft native lane on the first compile (parse 33f6d2b8,
// render_lock 432ddd47, render_marks fa8b1c64, resize 77de7581), so the four are judged
// like-for-like from here on.  It reuses the reference's own ports of `fill_poly`,
// `wide_line`, `smooth_pts`, `fronds`, `hair_brush`, `lock_layer` and `composite_layer`;
// what it adds is src/scan.loft, `parse_scene`, `render`'s four draw paths and the
// graphics package's Pillow-exact Lanczos resample.  Measured on the loft repo's quiet
// x86-64 box: see doc/claude/plans/157-native-4x-drawing/README.md § Where to resume.

// ── The four rows measured without a reference: parse, render_lock, render_marks,
// resize — a port of src/drawing.loft's `parse_scene` and `render`, src/scan.loft, and
// the graphics package's Pillow-exact Lanczos resample.  Same arithmetic, same order,
// same truncations; the hash is the receipt.  Gradient fills (`grad=` / `radial=`) are
// parsed but not rendered — no scene in the bench uses one.

fn sc_at(s: &[u8], i: i64) -> i64 {
    if i < 0 || i >= s.len() as i64 { -1 } else { s[i as usize] as i64 }
}
fn is_digit(b: i64) -> bool { (48..=57).contains(&b) }
fn is_word_byte(b: i64) -> bool {
    (48..=57).contains(&b) || (65..=90).contains(&b) || (97..=122).contains(&b) || b == 95
}
fn is_space_byte(b: i64) -> bool { matches!(b, 32 | 9 | 10 | 13 | 12 | 11) }
fn lower_byte(b: i64) -> i64 { if (65..=90).contains(&b) { b + 32 } else { b } }
fn skip_space(s: &[u8], i: i64) -> i64 {
    let mut j = i;
    while is_space_byte(sc_at(s, j)) { j += 1; }
    j
}
fn matches_at(s: &[u8], i: i64, word: &str) -> bool {
    let w = word.as_bytes();
    if i + w.len() as i64 > s.len() as i64 { return false; }
    for (k, &wb) in w.iter().enumerate() {
        if lower_byte(sc_at(s, i + k as i64)) != wb as i64 { return false; }
    }
    true
}
fn word_boundary(s: &[u8], i: i64) -> bool { i <= 0 || !is_word_byte(sc_at(s, i - 1)) }

struct Scan { ok: bool, at: i64, value: f64 }
fn scan_fail() -> Scan { Scan { ok: false, at: 0, value: 0.0 } }
fn slice_f64(s: &[u8], i: i64, j: i64) -> Option<f64> {
    std::str::from_utf8(&s[i as usize..j as usize]).ok()?.parse::<f64>().ok()
}
fn read_number(s: &[u8], i: i64) -> Scan {
    let mut j = i;
    loop {
        let b = sc_at(s, j);
        if is_digit(b) || b == 45 || b == 46 { j += 1; } else { break; }
    }
    while j > i {
        if let Some(v) = slice_f64(s, i, j) { return Scan { ok: true, at: j, value: v }; }
        j -= 1;
    }
    scan_fail()
}
fn read_unsigned(s: &[u8], i: i64) -> Scan {
    let mut j = i;
    loop {
        let b = sc_at(s, j);
        if is_digit(b) || b == 46 { j += 1; } else { break; }
    }
    while j > i {
        if let Some(v) = slice_f64(s, i, j) { return Scan { ok: true, at: j, value: v }; }
        j -= 1;
    }
    scan_fail()
}
fn read_uint(s: &[u8], i: i64) -> Scan {
    let mut j = i;
    while is_digit(sc_at(s, j)) { j += 1; }
    if j == i { return scan_fail(); }
    match std::str::from_utf8(&s[i as usize..j as usize]).ok().and_then(|t| t.parse::<i64>().ok()) {
        Some(v) => Scan { ok: true, at: j, value: v as f64 },
        None => scan_fail(),
    }
}
fn find_option(s: &[u8], key: &str) -> Scan {
    let n = s.len() as i64;
    let mut i = 0;
    while i < n {
        if word_boundary(s, i) && matches_at(s, i, key) {
            let j = skip_space(s, i + key.len() as i64);
            if sc_at(s, j) == 61 { return Scan { ok: true, at: skip_space(s, j + 1), value: 0.0 }; }
        }
        i += 1;
    }
    scan_fail()
}
fn find_word_from(s: &[u8], word: &str, start: i64) -> Scan {
    let n = s.len() as i64;
    let mut i = if start < 0 { 0 } else { start };
    while i < n {
        if matches_at(s, i, word) { return Scan { ok: true, at: i + word.len() as i64, value: 0.0 }; }
        i += 1;
    }
    scan_fail()
}

const SUPERSAMPLE: i64 = 3;
const SMOOTH_SAMPLES_N: i64 = 10;

fn grey(gl: f64) -> i64 {
    let gy = round_half_even(gl * 255.0) as i64;
    if gy < 0 { return rgba(0, 0, 0, 255); }
    if gy > 255 { return rgba(255, 255, 255, 255); }
    rgba(gy, gy, gy, 255)
}
fn default_ink() -> i64 { rgba(38, 32, 36, 255) }
fn read_width(s: &[u8], fallback: i64) -> i64 {
    let o = find_option(s, "w");
    if !o.ok { return fallback; }
    let n = read_uint(s, o.at);
    if !n.ok { return fallback; }
    n.value as i64
}
struct Scan3 { ok: bool, at: i64, color: i64 }
fn scan3_fail() -> Scan3 { Scan3 { ok: false, at: 0, color: 0 } }
fn read_rgb_at(s: &[u8], i: i64) -> Scan3 {
    let mut ri = i;
    let paren = sc_at(s, ri) == 40;
    if paren { ri = skip_space(s, ri + 1); }
    let r = read_uint(s, ri);
    if !r.ok { return scan3_fail(); }
    ri = skip_space(s, r.at);
    if sc_at(s, ri) != 44 { return scan3_fail(); }
    let g = read_uint(s, skip_space(s, ri + 1));
    if !g.ok { return scan3_fail(); }
    ri = skip_space(s, g.at);
    if sc_at(s, ri) != 44 { return scan3_fail(); }
    let b = read_uint(s, skip_space(s, ri + 1));
    if !b.ok { return scan3_fail(); }
    ri = b.at;
    if sc_at(s, ri) == 41 { ri += 1; }
    Scan3 { ok: true, at: ri, color: rgba(r.value as i64, g.value as i64, b.value as i64, 255) }
}
fn read_colour_opt(s: &[u8], key: &str, fallback: i64) -> i64 {
    let o = find_option(s, key);
    if !o.ok { return fallback; }
    let c = read_rgb_at(s, o.at);
    if !c.ok { return fallback; }
    c.color
}
fn read_stroke_colour(s: &[u8]) -> i64 { read_colour_opt(s, "stroke", default_ink()) }
fn half_colour(c: i64) -> i64 { rgba(color_r(c) / 2, color_g(c) / 2, color_b(c) / 2, 255) }
fn lifted_colour(c: i64) -> i64 {
    let (r, g, b) = (color_r(c), color_g(c), color_b(c));
    rgba(r + (255 - r) * 35 / 100, g + (255 - g) * 35 / 100, b + (255 - b) * 35 / 100, 255)
}

#[derive(Clone, Copy, PartialEq)]
enum PaintKind { Stroked, Solid, Linear, Radial }
#[derive(Clone)]
struct Paint { pk: PaintKind, c1: i64, c2: i64, spec: Vec<f64> }
fn stroked_paint() -> Paint { Paint { pk: PaintKind::Stroked, c1: 0, c2: 0, spec: Vec::new() } }
struct ColourPair { ok: bool, c1: i64, c2: i64 }
fn read_colour_pair(s: &[u8], i: i64) -> ColourPair {
    let a = read_rgb_at(s, i);
    if !a.ok { return ColourPair { ok: false, c1: 0, c2: 0 }; }
    let j = skip_space(s, a.at);
    if sc_at(s, j) != 62 { return ColourPair { ok: false, c1: 0, c2: 0 }; }
    let b = read_rgb_at(s, skip_space(s, j + 1));
    if !b.ok { return ColourPair { ok: false, c1: 0, c2: 0 }; }
    ColourPair { ok: true, c1: a.color, c2: b.color }
}
fn read_spec(s: &[u8], key: &str, count: i64) -> Vec<f64> {
    let mut out = Vec::new();
    let o = find_option(s, key);
    if !o.ok { return out; }
    let mut i = o.at;
    for k in 0..count {
        if k > 0 {
            if sc_at(s, i) != 44 { return Vec::new(); }
            i += 1;
        }
        let n = read_number(s, i);
        if !n.ok { return Vec::new(); }
        out.push(n.value);
        i = n.at;
    }
    out
}
fn read_paint(s: &[u8]) -> Paint {
    let rad = find_option(s, "radial");
    if rad.ok {
        let pair = read_colour_pair(s, rad.at);
        if pair.ok { return Paint { pk: PaintKind::Radial, c1: pair.c1, c2: pair.c2, spec: read_spec(s, "at", 3) }; }
    }
    let grad = find_option(s, "grad");
    if grad.ok {
        let pair = read_colour_pair(s, grad.at);
        if pair.ok { return Paint { pk: PaintKind::Linear, c1: pair.c1, c2: pair.c2, spec: read_spec(s, "dir", 4) }; }
    }
    let rgb = find_option(s, "rgb");
    if rgb.ok {
        let c = read_rgb_at(s, rgb.at);
        if c.ok { return Paint { pk: PaintKind::Solid, c1: c.color, c2: 0, spec: Vec::new() }; }
    }
    let fill = find_option(s, "fill");
    if fill.ok {
        let l = read_unsigned(s, fill.at);
        if l.ok { return Paint { pk: PaintKind::Solid, c1: grey(l.value), c2: 0, spec: Vec::new() }; }
    }
    stroked_paint()
}

struct PointList { pts: Vec<Pt>, smooth: Vec<bool>, widths: Vec<f64>, has_width: Vec<bool>, any_width: bool }
fn read_points(s: &[u8]) -> PointList {
    let mut out = PointList { pts: Vec::new(), smooth: Vec::new(), widths: Vec::new(), has_width: Vec::new(), any_width: false };
    let n = s.len() as i64;
    let mut i = 0;
    while i < n {
        if sc_at(s, i) != 40 { i += 1; continue; }
        let mut j = skip_space(s, i + 1);
        let x = read_number(s, j);
        if !x.ok { i += 1; continue; }
        j = skip_space(s, x.at);
        if sc_at(s, j) != 44 { i += 1; continue; }
        let y = read_number(s, skip_space(s, j + 1));
        if !y.ok { i += 1; continue; }
        j = skip_space(s, y.at);
        if sc_at(s, j) != 41 { i += 1; continue; }
        j += 1;
        let mut k = skip_space(s, j);
        let sm = sc_at(s, k) == 126;
        if sm { k = skip_space(s, k + 1); }
        let mut wv = 0.0;
        let mut has = false;
        if sc_at(s, k) == 64 {
            let w = read_number(s, skip_space(s, k + 1));
            if w.ok { wv = w.value; has = true; k = w.at; }
        }
        out.pts.push(pt(x.value, y.value));
        out.smooth.push(sm);
        out.widths.push(wv);
        out.has_width.push(has);
        if has { out.any_width = true; }
        i = k;
    }
    out
}
fn circle_pts(cx: f64, cy: f64, r: f64, n: i64, flat: f64, pw: i64, ph: i64) -> Vec<Pt> {
    let ratio = if ph == 0 { 1.0 } else { pw as f64 / ph as f64 };
    let ary = r * ratio * (1.0 - flat);
    let mut out = Vec::new();
    for i in 0..=n {
        let a = if n == 0 { 0.0 } else { (2.0 * PI * i as f64) / n as f64 };
        out.push(pt(cx + r * a.cos(), cy + ary * a.sin()));
    }
    out
}
fn smooth_applies(pts: &[Pt], flags: &[bool]) -> bool { pts.len() >= 3 && flags.iter().any(|&f| f) }
fn smooth_vals(vals: &[f64], closed: bool) -> Vec<f64> {
    let n = vals.len() as i64;
    let mut out = vec![vals[0]];
    let segs = if closed { n } else { n - 1 };
    for i in 0..segs {
        let a = vals[(i % n) as usize];
        let b = vals[((i + 1) % n) as usize];
        for k in 1..=SMOOTH_SAMPLES_N {
            out.push(a + (b - a) * (k as f64 / SMOOTH_SAMPLES_N as f64));
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum OpKind { Sky, Fill, Stroke, Lock }
struct Op { kind: OpKind, pts: Vec<Pt>, paint: Paint, widths: Vec<f64>, w: i64, color: i64, color2: i64, style: LockStyle, brush: i64 }
fn default_style() -> LockStyle {
    LockStyle { w0: 2.0, w: 10.0, swell: 0.3, body: 0.8, tips: 3, tipvar: 0.35, spread: 8.0, seed: 1, period: 48.0,
                dark: 0, base: 0, lit: 0, lx: -0.5, ly: -0.8, lz: 0.6, alpha: 1.0, flip: false }
}
fn op_of(kind: OpKind, pts: Vec<Pt>, paint: Paint, widths: Vec<f64>, w: i64, color: i64, color2: i64) -> Op {
    Op { kind, pts, paint, widths, w, color, color2, style: default_style(), brush: -1 }
}
struct Elem { ename: String, seen: bool, bx0: f64, by0: f64, bx1: f64, by1: f64, first_at: i64 }
struct NamedBrush { bname: String, period: f64, br: Brush }
struct Sketch { sw: i64, sh: i64, transparent: bool, ops: Vec<Op>, elems: Vec<Elem>, landmarks: Vec<(String, f64)>,
                checks: Vec<String>, unparsed: Vec<String>, deferred: Vec<String>, brushes: Vec<NamedBrush> }
struct Mark { matched: bool, bad: bool, pts: Vec<Pt> }
fn no_mark() -> Mark { Mark { matched: false, bad: false, pts: Vec::new() } }

fn parse_circle(sc: &mut Sketch, s: &[u8]) -> Mark {
    let mut from = 0;
    loop {
        let w = find_word_from(s, "circle", from);
        if !w.ok { return no_mark(); }
        from = w.at;
        let mut i = skip_space(s, w.at);
        if sc_at(s, i) != 40 { continue; }
        let x = read_number(s, skip_space(s, i + 1));
        if !x.ok { continue; }
        i = skip_space(s, x.at);
        if sc_at(s, i) != 44 { continue; }
        let y = read_number(s, skip_space(s, i + 1));
        if !y.ok { continue; }
        i = skip_space(s, y.at);
        if sc_at(s, i) != 41 { continue; }
        i = skip_space(s, i + 1);
        if lower_byte(sc_at(s, i)) != 114 { continue; }
        if sc_at(s, i + 1) != 61 { continue; }
        let r = read_number(s, i + 2);
        if !r.ok { continue; }
        i = r.at;
        let mut n = 28;
        let mut j = skip_space(s, i);
        if j > i && lower_byte(sc_at(s, j)) == 110 && sc_at(s, j + 1) == 61 {
            let cnt = read_uint(s, j + 2);
            if cnt.ok { n = cnt.value as i64; i = cnt.at; }
        }
        let mut flat = 0.0;
        j = skip_space(s, i);
        if j > i && matches_at(s, j, "flat") && sc_at(s, j + 4) == 61 {
            let f = read_number(s, j + 5);
            if f.ok { flat = f.value; }
        }
        let pts = circle_pts(x.value, y.value, r.value, n, flat, sc.sw, sc.sh);
        let paint = read_paint(s);
        if paint.pk == PaintKind::Stroked {
            let (wd, col) = (read_width(s, 3), read_stroke_colour(s));
            sc.ops.push(op_of(OpKind::Stroke, pts.clone(), paint, Vec::new(), wd, col, 0));
        } else {
            sc.ops.push(op_of(OpKind::Fill, pts.clone(), paint, Vec::new(), 3, 0, 0));
        }
        return Mark { matched: true, bad: false, pts };
    }
}
fn read_at_width(s: &[u8], i: i64) -> Scan {
    let j = skip_space(s, i);
    if sc_at(s, j) != 64 { return Scan { ok: false, at: i, value: 0.0 }; }
    let n = read_number(s, skip_space(s, j + 1));
    if !n.ok { return Scan { ok: false, at: i, value: 0.0 }; }
    n
}
fn parse_line_cmd(sc: &mut Sketch, s: &[u8]) -> Mark {
    let mut from = 0;
    loop {
        let w = find_word_from(s, "line", from);
        if !w.ok { return no_mark(); }
        from = w.at;
        let mut i = skip_space(s, w.at);
        if sc_at(s, i) != 40 { continue; }
        let x0 = read_number(s, skip_space(s, i + 1));
        if !x0.ok { continue; }
        i = skip_space(s, x0.at);
        if sc_at(s, i) != 44 { continue; }
        let y0 = read_number(s, skip_space(s, i + 1));
        if !y0.ok { continue; }
        i = skip_space(s, y0.at);
        if sc_at(s, i) != 41 { continue; }
        i += 1;
        let wa = read_at_width(s, i);
        i = wa.at;
        i = skip_space(s, i);
        if sc_at(s, i) != 45 && wa.ok && sc_at(s, i - 1) == 45 { i -= 1; }
        if sc_at(s, i) != 45 { continue; }
        i = skip_space(s, i + 1);
        if sc_at(s, i) != 40 { continue; }
        let x1 = read_number(s, skip_space(s, i + 1));
        if !x1.ok { continue; }
        i = skip_space(s, x1.at);
        if sc_at(s, i) != 44 { continue; }
        let y1 = read_number(s, skip_space(s, i + 1));
        if !y1.ok { continue; }
        i = skip_space(s, y1.at);
        if sc_at(s, i) != 41 { continue; }
        let wb = read_at_width(s, i + 1);
        let base = read_width(s, 3);
        let pts = vec![pt(x0.value, y0.value), pt(x1.value, y1.value)];
        let mut widths = Vec::new();
        if wa.ok || wb.ok {
            widths.push(if wa.ok { wa.value } else { base as f64 });
            widths.push(if wb.ok { wb.value } else { base as f64 });
        }
        let col = read_stroke_colour(s);
        sc.ops.push(op_of(OpKind::Stroke, pts.clone(), stroked_paint(), widths, base, col, 0));
        return Mark { matched: true, bad: false, pts };
    }
}
fn opt_num(s: &[u8], key: &str, dflt: f64) -> f64 {
    let o = find_option(s, key);
    if !o.ok { return dflt; }
    let n = read_number(s, o.at);
    if !n.ok { return dflt; }
    n.value
}
fn parse_fronds(sc: &mut Sketch, s: &[u8]) -> Mark {
    let mut from = 0;
    loop {
        let w = find_word_from(s, "fronds", from);
        if !w.ok { return no_mark(); }
        from = w.at;
        let mut i = skip_space(s, w.at);
        if sc_at(s, i) != 40 { continue; }
        let x0 = read_number(s, skip_space(s, i + 1));
        if !x0.ok { continue; }
        i = skip_space(s, x0.at);
        if sc_at(s, i) != 44 { continue; }
        let y0 = read_number(s, skip_space(s, i + 1));
        if !y0.ok { continue; }
        i = skip_space(s, y0.at);
        if sc_at(s, i) != 41 { continue; }
        i = skip_space(s, i + 1);
        if sc_at(s, i) != 45 { continue; }
        i = skip_space(s, i + 1);
        if sc_at(s, i) != 40 { continue; }
        let x1 = read_number(s, skip_space(s, i + 1));
        if !x1.ok { continue; }
        i = skip_space(s, x1.at);
        if sc_at(s, i) != 44 { continue; }
        let y1 = read_number(s, skip_space(s, i + 1));
        if !y1.ok { continue; }
        i = skip_space(s, y1.at);
        if sc_at(s, i) != 41 { continue; }
        let mut j = skip_space(s, i + 1);
        if j == i + 1 { continue; }
        if lower_byte(sc_at(s, j)) != 110 { continue; }
        j = skip_space(s, j + 1);
        if sc_at(s, j) != 61 { continue; }
        let cnt = read_uint(s, skip_space(s, j + 1));
        if !cnt.ok { continue; }
        let mut k = skip_space(s, cnt.at);
        if k == cnt.at { continue; }
        if !matches_at(s, k, "len") { continue; }
        k = skip_space(s, k + 3);
        if sc_at(s, k) != 61 { continue; }
        let ln = read_number(s, skip_space(s, k + 1));
        if !ln.ok { continue; }
        let wv = opt_num(s, "w", 3.0);
        let av = opt_num(s, "ang", 30.0);
        let spec = FrondSpec {
            count: cnt.value as i64, len1: ln.value, len2: opt_num(s, "len2", ln.value),
            wid1: wv, wid2: opt_num(s, "w2", wv), ang1: av, ang2: opt_num(s, "ang2", av),
            mirror: (opt_num(s, "mirror", 0.0) as i64) != 0, jitter: opt_num(s, "jitter", 0.5),
            clump: opt_num(s, "field", 0.6), fray: opt_num(s, "fray", 0.4), bow: opt_num(s, "bow", 0.0),
            seed: opt_num(s, "seed", 1.0) as i64, depth: opt_num(s, "depth", 1.0) as i64, sub: opt_num(s, "sub", 0.32),
        };
        let ink = read_stroke_colour(s);
        let mut all = Vec::new();
        for f in fronds(x0.value, y0.value, x1.value, y1.value, &spec, sc.sw, sc.sh) {
            let (mut line, mut wids) = (f.pts.clone(), f.wid.clone());
            if f.pts.len() > 2 {
                line = smooth_pts(&f.pts, &[false, true, false], false);
                wids = smooth_vals(&f.wid, false);
            }
            all.extend_from_slice(&line);
            sc.ops.push(op_of(OpKind::Stroke, line, stroked_paint(), wids, 3, ink, 0));
        }
        return Mark { matched: true, bad: false, pts: all };
    }
}
fn read_word_at(s: &[u8], i: i64) -> String {
    let mut j = i;
    while is_word_byte(sc_at(s, j)) { j += 1; }
    String::from_utf8_lossy(&s[i as usize..j as usize]).into_owned()
}
fn brush_index(sc: &Sketch, nm: &str) -> i64 {
    for (i, b) in sc.brushes.iter().enumerate() {
        if b.bname == nm { return i as i64; }
    }
    -1
}
fn parse_brush(sc: &mut Sketch, s: &[u8]) -> bool {
    let i = skip_space(s, 5);
    let name = read_word_at(s, i);
    if name.is_empty() { return false; }
    let kind = read_word_at(s, skip_space(s, i + name.len() as i64)).to_lowercase();
    if kind == "hair" {
        let w = opt_num(s, "w", 12.0) as i64;
        let h = opt_num(s, "period", 48.0) as i64;
        let img = hair_brush(w, h, opt_num(s, "seed", 1.0) as i64, opt_num(s, "gap", 0.35));
        sc.brushes.push(NamedBrush { bname: name, period: h as f64, br: Brush { bw: w, bh: h, img } });
        return true;
    }
    false
}
fn parse_lock(sc: &mut Sketch, s: &[u8]) -> Mark {
    let raw = read_points(s);
    if raw.pts.len() < 2 { return Mark { matched: true, bad: true, pts: Vec::new() }; }
    let bi;
    let bo = find_option(s, "brush");
    if bo.ok {
        bi = brush_index(sc, &read_word_at(s, bo.at));
        if bi < 0 { return Mark { matched: true, bad: true, pts: Vec::new() }; }
    } else {
        let found = brush_index(sc, "hair");
        if found < 0 {
            sc.brushes.push(NamedBrush { bname: "hair".to_string(), period: 48.0, br: Brush { bw: 12, bh: 48, img: hair_brush(12, 48, 1, 0.35) } });
            bi = sc.brushes.len() as i64 - 1;
        } else {
            bi = found;
        }
    }
    let line = smooth_pts(&raw.pts, &raw.smooth, false);
    let base = read_colour_opt(s, "rgb", rgba(120, 80, 40, 255));
    let light = read_spec(s, "light", 3);
    let (mut lx, mut ly, mut lz) = (-0.5, -0.8, 0.6);
    if light.len() == 3 { lx = light[0]; ly = light[1]; lz = light[2]; }
    let period = sc.brushes[bi as usize].period;
    let style = LockStyle {
        w0: opt_num(s, "w0", 2.0), w: opt_num(s, "w", 10.0), swell: opt_num(s, "swell", 0.3), body: opt_num(s, "body", 0.8),
        tips: opt_num(s, "tips", 3.0) as i64, tipvar: opt_num(s, "tipvar", 0.35), spread: opt_num(s, "spread", 8.0),
        seed: opt_num(s, "seed", 1.0) as i64, period: opt_num(s, "period", period),
        dark: read_colour_opt(s, "dark", half_colour(base)), base, lit: read_colour_opt(s, "lit", lifted_colour(base)),
        lx, ly, lz, alpha: opt_num(s, "alpha", 1.0), flip: (opt_num(s, "flip", 0.0) as i64) != 0,
    };
    sc.ops.push(Op { kind: OpKind::Lock, pts: line.clone(), paint: stroked_paint(), widths: Vec::new(), w: 3, color: 0, color2: 0, style, brush: bi });
    Mark { matched: true, bad: false, pts: line }
}
fn parse_poly(sc: &mut Sketch, s: &[u8]) -> Mark {
    let raw = read_points(s);
    let paint = read_paint(s);
    let need = if paint.pk == PaintKind::Stroked { 2 } else { 3 };
    if raw.pts.len() < need { return Mark { matched: true, bad: true, pts: Vec::new() }; }
    if paint.pk != PaintKind::Stroked {
        let pts = smooth_pts(&raw.pts, &raw.smooth, true);
        sc.ops.push(op_of(OpKind::Fill, pts.clone(), paint, Vec::new(), 3, 0, 0));
        return Mark { matched: true, bad: false, pts };
    }
    let base = read_width(s, 3);
    let ink = read_stroke_colour(s);
    if raw.any_width {
        let vals: Vec<f64> = (0..raw.pts.len()).map(|i| if raw.has_width[i] { raw.widths[i] } else { base as f64 }).collect();
        let open = smooth_pts(&raw.pts, &raw.smooth, false);
        let ow = if smooth_applies(&raw.pts, &raw.smooth) { smooth_vals(&vals, false) } else { vals };
        sc.ops.push(op_of(OpKind::Stroke, open.clone(), paint, ow, base, ink, 0));
        return Mark { matched: true, bad: false, pts: open };
    }
    let line = smooth_pts(&raw.pts, &raw.smooth, false);
    sc.ops.push(op_of(OpKind::Stroke, line.clone(), paint, Vec::new(), base, ink, 0));
    Mark { matched: true, bad: false, pts: line }
}
fn elem_index(sc: &mut Sketch, nm: &str) -> i64 {
    for (i, e) in sc.elems.iter().enumerate() {
        if e.ename == nm { return i as i64; }
    }
    sc.elems.push(Elem { ename: nm.to_string(), seen: false, bx0: 0.0, by0: 0.0, bx1: 0.0, by1: 0.0, first_at: -1 });
    sc.elems.len() as i64 - 1
}
fn acc_pts(sc: &mut Sketch, idx: i64, pts: &[Pt]) {
    if idx < 0 || pts.is_empty() { return; }
    let n_ops = sc.ops.len() as i64;
    let e = &mut sc.elems[idx as usize];
    for p in pts {
        if !e.seen {
            e.bx0 = p.x; e.by0 = p.y; e.bx1 = p.x; e.by1 = p.y;
            e.seen = true;
        } else {
            if p.x < e.bx0 { e.bx0 = p.x; }
            if p.y < e.by0 { e.by0 = p.y; }
            if p.x > e.bx1 { e.bx1 = p.x; }
            if p.y > e.by1 { e.by1 = p.y; }
        }
    }
    if e.first_at < 0 { e.first_at = n_ops; }
}
fn parse_background(sc: &mut Sketch, s: &[u8], low: &str) -> bool {
    if low.contains("transparent") || low.contains("none") { sc.transparent = true; return true; }
    let (tc, bc) = (find_option(s, "topc"), find_option(s, "botc"));
    if tc.ok && bc.ok {
        let (t, b) = (read_rgb_at(s, tc.at), read_rgb_at(s, bc.at));
        if t.ok && b.ok {
            sc.ops.push(op_of(OpKind::Sky, Vec::new(), stroked_paint(), Vec::new(), 3, t.color, b.color));
            return true;
        }
    }
    let gt = find_option(s, "top");
    if !gt.ok { return false; }
    let mut gb = find_option(s, "bottom");
    if !gb.ok { gb = find_option(s, "bot"); }
    if !gb.ok { return false; }
    let (tv, bv) = (read_unsigned(s, gt.at), read_unsigned(s, gb.at));
    if !tv.ok || !bv.ok { return false; }
    sc.ops.push(op_of(OpKind::Sky, Vec::new(), stroked_paint(), Vec::new(), 3, grey(tv.value), grey(bv.value)));
    true
}
fn parse_size(sc: &mut Sketch, s: &[u8]) -> bool {
    if !matches_at(s, 0, "size") { return false; }
    let mut i = skip_space(s, 4);
    if i == 4 { return false; }
    let w = read_uint(s, i);
    if !w.ok { return false; }
    i = skip_space(s, w.at);
    if lower_byte(sc_at(s, i)) != 120 { return false; }
    let h = read_uint(s, skip_space(s, i + 1));
    if !h.ok || h.at != s.len() as i64 { return false; }
    sc.sw = w.value as i64;
    sc.sh = h.value as i64;
    true
}
fn parse_landmark(sc: &mut Sketch, s: &[u8]) -> bool {
    let i = skip_space(s, 8);
    if i == 8 { return false; }
    let mut j = i;
    while is_word_byte(sc_at(s, j)) { j += 1; }
    if j == i { return false; }
    let k = skip_space(s, j);
    if sc_at(s, k) != 61 { return false; }
    let v = read_number(s, skip_space(s, k + 1));
    if !v.ok { return false; }
    sc.landmarks.push((String::from_utf8_lossy(&s[i as usize..j as usize]).into_owned(), v.value));
    true
}
fn parse_scene(src: &str) -> Sketch {
    let mut sc = Sketch { sw: 800, sh: 800, transparent: false, ops: Vec::new(), elems: Vec::new(), landmarks: Vec::new(),
                          checks: Vec::new(), unparsed: Vec::new(), deferred: Vec::new(), brushes: Vec::new() };
    let mut cur: i64 = -1;
    let mut no = 0;
    for raw in src.split('\n') {
        no += 1;
        let st = raw.trim();
        if st.is_empty() || st.starts_with('#') { continue; }
        let s = st.as_bytes();
        let low = st.to_lowercase();
        if low.starts_with("name ") {
            let nm = st[5..].trim().to_string();
            cur = elem_index(&mut sc, &nm);
            continue;
        }
        if low.starts_with("landmark") {
            if !parse_landmark(&mut sc, s) { sc.unparsed.push(format!("line {no}: {st}")); }
            continue;
        }
        if low.starts_with("check") { sc.checks.push(st[5..].trim().to_string()); continue; }
        if low.starts_with("background") {
            if !parse_background(&mut sc, s, &low) { sc.unparsed.push(format!("line {no}: {st}")); }
            continue;
        }
        if parse_size(&mut sc, s) { continue; }
        let m = parse_circle(&mut sc, s);
        if m.matched { acc_pts(&mut sc, cur, &m.pts); continue; }
        if low.starts_with("petals") { sc.deferred.push(format!("line {no}: Petals")); continue; }
        if low.starts_with("fronds") {
            let f = parse_fronds(&mut sc, s);
            if f.matched { acc_pts(&mut sc, cur, &f.pts); }
            continue;
        }
        if low.starts_with("brush ") {
            if !parse_brush(&mut sc, s) { sc.unparsed.push(format!("line {no}: {st}")); }
            continue;
        }
        if low.starts_with("lock") {
            let k = parse_lock(&mut sc, s);
            if k.bad { sc.unparsed.push(format!("line {no}: {st}")); } else { acc_pts(&mut sc, cur, &k.pts); }
            continue;
        }
        if low.starts_with("poly") {
            let p = parse_poly(&mut sc, s);
            if p.bad { sc.unparsed.push(format!("line {no}: {st}")); } else { acc_pts(&mut sc, cur, &p.pts); }
            continue;
        }
        let l = parse_line_cmd(&mut sc, s);
        if l.matched { acc_pts(&mut sc, cur, &l.pts); continue; }
        sc.unparsed.push(format!("line {no}: {st}"));
    }
    sc
}

// ── Rendering ──────────────────────────────────────────────────────────────────────
fn cv_set(cv: &mut Canvas, x: i64, y: i64, c: i64) {
    if x >= 0 && x < cv.w && y >= 0 && y < cv.h { cv.data[(y * cv.w + x) as usize] = c; }
}
fn to_pixels(pts: &[Pt], bw: i64, bh: i64) -> Vec<Pt> {
    pts.iter().map(|p| pt(p.x * bw as f64, p.y * bh as f64)).collect()
}
fn bresenham(cv: &mut Canvas, bx0: i64, by0: i64, bx1: i64, by1: i64, ink: i64) {
    let (mut x, mut y) = (bx0, by0);
    let mut dx = bx1 - bx0;
    let mut xs = 1;
    if dx < 0 { dx = -dx; xs = -1; }
    let mut dy = by1 - by0;
    let mut ys = 1;
    if dy < 0 { dy = -dy; ys = -1; }
    if dx == 0 {
        for _ in 0..dy { cv_set(cv, x, y, ink); y += ys; }
        return;
    }
    if dy == 0 {
        for _ in 0..dx { cv_set(cv, x, y, ink); x += xs; }
        return;
    }
    if dx > dy {
        let n = dx;
        dy += dy;
        let mut e = dy - dx;
        dx += dx;
        for _ in 0..n {
            cv_set(cv, x, y, ink);
            if e >= 0 { y += ys; e -= dx; }
            e += dy;
            x += xs;
        }
        return;
    }
    let n2 = dy;
    dx += dx;
    let mut e2 = dx - dy;
    dy += dy;
    for _ in 0..n2 {
        cv_set(cv, x, y, ink);
        if e2 >= 0 { x += xs; e2 -= dy; }
        e2 += dx;
        y += ys;
    }
}
fn thin_line(cv: &mut Canvas, pts: &[Pt], ink: i64) {
    let n = pts.len();
    if n < 2 { return; }
    let (mut lx, mut ly) = (0, 0);
    for i in 0..n - 1 {
        let (a, b) = (pts[i], pts[i + 1]);
        lx = b.x as i64;
        ly = b.y as i64;
        bresenham(cv, a.x as i64, a.y as i64, lx, ly, ink);
    }
    cv_set(cv, lx, ly, ink);
}
fn draw_sky(cv: &mut Canvas, top: i64, bot: i64, bw: i64, bh: i64) {
    let den = if bh - 1 < 1 { 1 } else { bh - 1 };
    let (tr, tg, tb) = (color_r(top), color_g(top), color_b(top));
    let (dr, dg, db) = (color_r(bot) - tr, color_g(bot) - tg, color_b(bot) - tb);
    for y in 0..bh {
        let t = y as f64 / den as f64;
        let c = rgba((tr as f64 + dr as f64 * t) as i64, (tg as f64 + dg as f64 * t) as i64, (tb as f64 + db as f64 * t) as i64, 255);
        let row = [pt(0.0, y as f64), pt(bw as f64, y as f64)];
        thin_line(cv, &row, c);
    }
}
fn draw_fill(cv: &mut Canvas, op: &Op, bw: i64, bh: i64) {
    if op.paint.pk == PaintKind::Stroked { return; }
    let px = to_pixels(&op.pts, bw, bh);
    if op.paint.pk == PaintKind::Solid { fill_poly(cv, &px, op.paint.c1); return; }
    unimplemented!("gradient fills are not in the bench's scenes");
}
fn ribbon(cv: &mut Canvas, pxs: &[Pt], widths: &[f64], color: i64) {
    let n = pxs.len();
    if n < 2 { return; }
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for i in 0..n {
        let mut hw = widths[i] * SUPERSAMPLE as f64 / 2.0;
        if hw < 0.5 { hw = 0.5; }
        let p = pxs[i];
        let (tx, ty) = if i == 0 {
            let q = pxs[1];
            (q.x - p.x, q.y - p.y)
        } else if i == n - 1 {
            let q = pxs[i - 1];
            (p.x - q.x, p.y - q.y)
        } else {
            let (a, b) = (pxs[i + 1], pxs[i - 1]);
            (a.x - b.x, a.y - b.y)
        };
        let mut l = (tx * tx + ty * ty).sqrt();
        if l == 0.0 { l = 1.0; }
        let (nx, ny) = (-ty / l, tx / l);
        left.push(pt(p.x + nx * hw, p.y + ny * hw));
        right.push(pt(p.x - nx * hw, p.y - ny * hw));
    }
    let mut poly: Vec<Pt> = left;
    for i in 0..n { poly.push(right[n - 1 - i]); }
    fill_poly(cv, &poly, color);
}
fn draw_stroke(cv: &mut Canvas, op: &Op, bw: i64, bh: i64) {
    let px = to_pixels(&op.pts, bw, bh);
    if px.len() < 2 { return; }
    if !op.widths.is_empty() { ribbon(cv, &px, &op.widths, op.color); return; }
    let mut w = op.w * SUPERSAMPLE;
    if w < 1 { w = 1; }
    for i in 0..px.len() - 1 {
        let (a, b) = (px[i], px[i + 1]);
        if w <= 1 {
            thin_line(cv, &[a, b], op.color);
        } else {
            wide_line(cv, a.x, a.y, b.x, b.y, op.color, w);
        }
    }
}
fn scaled_style(st: &LockStyle, k: f64) -> LockStyle {
    let mut s = st.clone();
    s.w0 = st.w0 * k;
    s.w = st.w * k;
    s.period = st.period * k;
    s
}
fn draw_lock(cv: &mut Canvas, sc: &Sketch, op: &Op, bw: i64, bh: i64) {
    if op.brush < 0 || op.brush >= sc.brushes.len() as i64 { return; }
    let px = to_pixels(&op.pts, bw, bh);
    let xs: Vec<f64> = px.iter().map(|p| p.x).collect();
    let ys: Vec<f64> = px.iter().map(|p| p.y).collect();
    let lay = lock_layer(&xs, &ys, bw, bh, &sc.brushes[op.brush as usize].br, &scaled_style(&op.style, SUPERSAMPLE as f64));
    if lay.lw > 0 { composite_layer(cv, &lay); }
}
fn render(sc: &Sketch) -> Canvas {
    let (bw, bh) = (sc.sw * SUPERSAMPLE, sc.sh * SUPERSAMPLE);
    let mut cv = canvas(bw, bh, if sc.transparent { 0 } else { rgba(255, 255, 255, 255) });
    for op in &sc.ops {
        match op.kind {
            OpKind::Sky => draw_sky(&mut cv, op.color, op.color2, bw, bh),
            OpKind::Fill => draw_fill(&mut cv, op, bw, bh),
            OpKind::Stroke => draw_stroke(&mut cv, op, bw, bh),
            OpKind::Lock => draw_lock(&mut cv, sc, op, bw, bh),
        }
    }
    resize_lanczos(&cv, sc.sw, sc.sh)
}

// ── Pillow's Lanczos resample (graphics package) ───────────────────────────────────
const RESAMPLE_PREC: i64 = 22;
const RESAMPLE_PI: f64 = 3.14159265358979323846;
fn sinc_at(x: f64) -> f64 {
    if x == 0.0 { return 1.0; }
    let xp = x * RESAMPLE_PI;
    xp.sin() / xp
}
fn lanczos_at(x0: f64) -> f64 {
    let x = if x0 < 0.0 { -x0 } else { x0 };
    if x < 3.0 { return sinc_at(x) * sinc_at(x / 3.0); }
    0.0
}
fn resample_coeffs(in_size: i64, out_size: i64, support0: f64, filt: fn(f64) -> f64,
                   starts: &mut Vec<i64>, counts: &mut Vec<i64>, kk: &mut Vec<i64>) {
    let scale = in_size as f64 / out_size as f64;
    let fscale = if scale < 1.0 { 1.0 } else { scale };
    let support = support0 * fscale;
    let ss = 1.0 / fscale;
    for xx in 0..out_size {
        let centre = (xx as f64 + 0.5) * scale;
        let mut xmin = (centre - support + 0.5) as i64;
        if xmin < 0 { xmin = 0; }
        let mut xmax = (centre + support + 0.5) as i64;
        if xmax > in_size { xmax = in_size; }
        xmax -= xmin;
        let mut row = Vec::new();
        let mut ww = 0.0;
        for x in 0..xmax {
            let w = filt(((x + xmin) as f64 - centre + 0.5) * ss);
            row.push(w);
            ww += w;
        }
        for x in 0..xmax {
            let mut k = row[x as usize];
            if ww != 0.0 { k /= ww; }
            let scaled = k * (1i64 << RESAMPLE_PREC) as f64;
            kk.push(if k < 0.0 { (scaled - 0.5) as i64 } else { (scaled + 0.5) as i64 });
        }
        starts.push(xmin);
        counts.push(xmax);
    }
}
fn resample_clip(v: i64) -> i64 {
    let s = v >> RESAMPLE_PREC;
    if s < 0 { return 0; }
    if s > 255 { return 255; }
    s
}
fn resample(cv: &Canvas, ow: i64, oh: i64, support0: f64, filt: fn(f64) -> f64) -> Canvas {
    if ow < 1 || oh < 1 || cv.w < 1 || cv.h < 1 {
        return canvas(if ow < 1 { 1 } else { ow }, if oh < 1 { 1 } else { oh }, 0);
    }
    let (iw, ih) = (cv.w, cv.h);
    let mut pre: Vec<i64> = Vec::with_capacity((iw * ih * 4) as usize);
    for i in 0..(iw * ih) as usize {
        let c = cv.data[i];
        let a = (c >> 24) & 255;
        pre.push((((c >> 16) & 255) * a + 127) / 255);
        pre.push((((c >> 8) & 255) * a + 127) / 255);
        pre.push(((c & 255) * a + 127) / 255);
        pre.push(a);
    }
    let (mut hs, mut hc, mut hk) = (Vec::new(), Vec::new(), Vec::new());
    resample_coeffs(iw, ow, support0, filt, &mut hs, &mut hc, &mut hk);
    let mut mid = vec![0i64; (ow * ih * 4) as usize];
    let mut base = 0usize;
    for xx in 0..ow {
        let xmin = hs[xx as usize];
        let n = hc[xx as usize];
        for yy in 0..ih {
            for ch in 0..4 {
                let mut acc = 1i64 << (RESAMPLE_PREC - 1);
                for x in 0..n {
                    acc += pre[(((yy * iw + xmin + x) * 4) + ch) as usize] * hk[base + x as usize];
                }
                mid[(((yy * ow + xx) * 4) + ch) as usize] = resample_clip(acc);
            }
        }
        base += n as usize;
    }
    let (mut vs, mut vc, mut vk) = (Vec::new(), Vec::new(), Vec::new());
    resample_coeffs(ih, oh, support0, filt, &mut vs, &mut vc, &mut vk);
    let mut out = canvas(ow, oh, 0);
    let mut vbase = 0usize;
    for yy in 0..oh {
        let ymin = vs[yy as usize];
        let n = vc[yy as usize];
        for xx in 0..ow {
            let mut chs = [0i64; 4];
            for ch in 0..4 {
                let mut acc = 1i64 << (RESAMPLE_PREC - 1);
                for y in 0..n {
                    acc += mid[((((ymin + y) * ow + xx) * 4) + ch) as usize] * vk[vbase + y as usize];
                }
                chs[ch as usize] = resample_clip(acc);
            }
            let a = chs[3];
            let mut rgb = [0i64; 3];
            for ch in 0..3 {
                let v = chs[ch];
                rgb[ch] = if a == 0 { v } else { let u = (v * 255) / a; if u > 255 { 255 } else { u } };
            }
            out.data[(yy * ow + xx) as usize] = (a << 24) | (rgb[0] << 16) | (rgb[1] << 8) | rgb[2];
        }
        vbase += n as usize;
    }
    out
}
fn resize_lanczos(cv: &Canvas, ow: i64, oh: i64) -> Canvas { resample(cv, ow, oh, 3.0, lanczos_at) }

// ── The rows ───────────────────────────────────────────────────────────────────────
fn lock_scene() -> &'static str {
    "\n  size 120x60\n  Background top=0.5 bottom=0.5\n  Lock (0.05,0.5) (0.95,0.5) w0=2 w=20 tips=3 tipvar=0 spread=0 rgb=120,80,40\n  "
}
fn marks_scene() -> &'static str {
    "\n  size 64x64\n  Background transparent\n  name body\n  Poly (0.15,0.20) (0.80,0.25) (0.70,0.85) (0.20,0.80) rgb=190,60,50\n  Circle (0.50,0.45) r=0.22 rgb=40,120,180\n  name trim\n  Poly (0.10,0.90)~ (0.50,0.55)~ (0.90,0.92)~ stroke=20,220,90 w=2\n  Poly (0.20,0.30)@6 (0.85,0.40)@1 stroke=250,240,60\n  Line (0.05,0.05) - (0.95,0.15) w=1\n  Fronds (0.2,0.7)-(0.8,0.7) n=8 len=0.08 w=2 stroke=60,120,40\n  "
}
fn fnv_sketch(sk: &Sketch) -> i64 {
    let mut ints: Vec<i64> = vec![sk.ops.len() as i64, sk.elems.len() as i64, sk.unparsed.len() as i64];
    for op in &sk.ops {
        for p in &op.pts { ints.push(milli(p.x)); ints.push(milli(p.y)); }
    }
    fnv(FNV_OFFSET, &ints)
}
fn bench_parse(n: i64) -> Row {
    let src = marks_scene();
    let (us, sink) = timed(n, || parse_scene(src).ops.len() as i64);
    let one = parse_scene(src);
    Row { name: "parse", iters: n, us, px: one.ops.len() as i64, hash: fnv_sketch(&one), sink }
}
fn bench_render(n: i64, name: &'static str, src: &str) -> Row {
    let sk = parse_scene(src);
    let (us, sink) = timed(n, || render(&sk).w);
    let one = render(&sk);
    Row { name, iters: n, us, px: one.w * one.h, hash: fnv(FNV_OFFSET, &one.data), sink }
}
fn bench_resize(n: i64) -> Row {
    let mut cv = canvas(768, 768, 0);
    for y in 0..768 {
        for x in 0..768 {
            cv_set(&mut cv, x, y, rgba(x & 255, y & 255, (x ^ y) & 255, 255));
        }
    }
    let (us, sink) = timed(n, || resize_lanczos(&cv, 256, 256).w);
    let one = resize_lanczos(&cv, 256, 256);
    Row { name: "resize", iters: n, us, px: 256 * 256, hash: fnv(FNV_OFFSET, &one.data), sink }
}
