// Benchmark 18: the crawler roguelike's hot loops — the Rust twin.
//
// Each routine is the same data layout and the same loop as bench.loft, written the way a
// Rust author writes it with `std` alone: slices borrowed, a `HashMap` keyed on the corner's
// two integers (not on a formatted text — the census row's stated twin), `String::push`,
// `format!` for the width specs, `std::fs::read` + `from_le_bytes` for the binary file.
// Same result per op; the hash column is the receipt.
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

const SQRT3: f64 = 1.7320508075688772;
const HEX_SIZE: f64 = 1.0;
const TALUS_ITERS: i64 = 120;
const DETAIL_CELL_M: f64 = 1.46484375;
const REPOSE_TAN: f64 = 0.7002;
const TREELINE: f64 = 2150.0;
const HMASK: i64 = 1099511627775;

struct Sim { qmin: i64, rmin: i64, lw: i64, lh: i64, tiles: Vec<i64>, walls: Vec<i64>, vis: Vec<i64>, seen: Vec<i64> }

#[allow(dead_code)]
struct WCorner { x: f64, y: f64, deg: i64, n1: i64, n2: i64, anc: i64 }
struct WEdge { a: i64, b: i64 }
struct WallGraph { corners: Vec<WCorner>, edges: Vec<WEdge>, lookup: HashMap<(i64, i64), u32> }

struct Region { rgn_cols: i64, rgn_rows: i64, hex_h: Vec<i64> }

fn make_sim(salt: i64) -> Sim {
    let (w, h) = (101i64, 101i64);
    let mut s = Sim { qmin: salt, rmin: 0, lw: w, lh: h, tiles: vec![], walls: vec![], vis: vec![], seen: vec![] };
    for gr in 0..h {
        for gq in 0..w {
            let wall = (gq % 12 == 0 && gr % 10 != 5) || (gr % 10 == 0 && gq % 12 != 6) || (gq * 7 + gr * 13) % 29 == 0;
            s.tiles.push(if wall { 1 } else { 0 });
            for e in 0..3 { s.walls.push(if (gq * 5 + gr * 3 + e) % 47 == 0 { 1 } else { 0 }); }
            let dq = gq - 50 - salt * 3;
            let dr = gr - 50;
            s.vis.push(if dq * dq + dr * dr < 400 { 1 } else { 0 });
            s.seen.push(if (gq + gr * 3 + salt) % 5 != 0 { 1 } else { 0 });
        }
    }
    s
}

fn tile_at(s: &Sim, q: i64, r: i64) -> i64 {
    if q < s.qmin || r < s.rmin || q >= s.qmin + s.lw || r >= s.rmin + s.lh { return 1; }
    let ti = ((r - s.rmin) * s.lw + (q - s.qmin)) as usize;
    if ti < s.tiles.len() { s.tiles[ti] } else { 1 }
}

fn is_wall(s: &Sim, q: i64, r: i64) -> bool {
    let t = tile_at(s, q, r);
    t == 1 || t == 4 || t == 5
}

fn edge_wall_raw(s: &Sim, q: i64, r: i64, edge: i64) -> i64 {
    if q < s.qmin || r < s.rmin || q >= s.qmin + s.lw || r >= s.rmin + s.lh { return 0; }
    let ei = (((r - s.rmin) * s.lw + (q - s.qmin)) * 3 + edge) as usize;
    if ei < s.walls.len() { s.walls[ei] } else { 0 }
}

fn hex_to_px(q: i64, r: i64) -> (f64, f64) {
    let par = r & 1;
    (HEX_SIZE * (SQRT3 * q as f64 + SQRT3 / 2.0 * par as f64), HEX_SIZE * (1.5 * r as f64))
}

fn hex_neighbor(q: i64, r: i64, dir: i64) -> (i64, i64) {
    if r & 1 == 0 {
        match dir { 0 => (q + 1, r), 1 => (q, r - 1), 2 => (q - 1, r - 1), 3 => (q - 1, r), 4 => (q - 1, r + 1), _ => (q, r + 1) }
    } else {
        match dir { 0 => (q + 1, r), 1 => (q + 1, r - 1), 2 => (q, r - 1), 3 => (q - 1, r), 4 => (q, r + 1), _ => (q + 1, r + 1) }
    }
}

fn hex_corner_offset(i: i64) -> (f64, f64) {
    let hw = SQRT3 / 2.0;
    match i { 0 => (0.0, 1.0), 1 => (-hw, 0.5), 2 => (-hw, -0.5), 3 => (0.0, -1.0), 4 => (hw, -0.5), _ => (hw, 0.5) }
}

fn hex_corner_px(q: i64, r: i64, i: i64) -> (f64, f64) {
    let (ccx, ccy) = hex_to_px(q, r);
    let (cox, coy) = hex_corner_offset(i);
    (ccx + cox * HEX_SIZE, ccy + coy * HEX_SIZE)
}

fn hex_edge_corners(dir: i64) -> (i64, i64) {
    match dir { 0 => (4, 5), 1 => (3, 4), 2 => (2, 3), 3 => (1, 2), 4 => (0, 1), _ => (5, 0) }
}

fn get_or_add(g: &mut WallGraph, x: f64, y: f64) -> i64 {
    let key = ((x * 1000.0).round() as i64, (y * 1000.0).round() as i64);
    if let Some(&idx) = g.lookup.get(&key) { return idx as i64; }
    let idx = g.corners.len();
    g.corners.push(WCorner { x, y, deg: 0, n1: -1, n2: -1, anc: 0 });
    g.lookup.insert(key, idx as u32);
    idx as i64
}

fn add_edge(g: &mut WallGraph, q: i64, r: i64, dir: i64) {
    let (ca, cb) = hex_edge_corners(dir);
    let (ax, ay) = hex_corner_px(q, r, ca);
    let (bx, by) = hex_corner_px(q, r, cb);
    let ia = get_or_add(g, ax, ay);
    let ib = get_or_add(g, bx, by);
    g.edges.push(WEdge { a: ia, b: ib });
}

fn c_build_walls(s: &Sim) -> i64 {
    let mut g = WallGraph { corners: vec![], edges: vec![], lookup: HashMap::new() };
    for gr in 0..s.lh {
        for gq in 0..s.lw {
            let (q, r) = (s.qmin + gq, s.rmin + gr);
            if !is_wall(s, q, r) {
                for d in 0..6 {
                    let (nq, nr) = hex_neighbor(q, r, d);
                    if is_wall(s, nq, nr) { add_edge(&mut g, q, r, d); }
                }
            }
            for e in 0..3 {
                if edge_wall_raw(s, q, r, e) != 0 { add_edge(&mut g, q, r, e); }
            }
        }
    }
    let mut acc = g.corners.len() as i64 * 100003 + g.edges.len() as i64;
    for ed in &g.edges { acc += ed.a * 7 + ed.b * 13; }
    for c in &g.corners { acc += (c.x * 8.0).round() as i64 + (c.y * 8.0).round() as i64 * 3; }
    acc
}

fn rgba(cr: i64, cg: i64, cb: i64, ca: i64) -> i64 {
    ((ca & 255) << 24) | ((cr & 255) << 16) | ((cg & 255) << 8) | (cb & 255)
}

fn sim_hex_state(s: &Sim, q: i64, r: i64) -> i64 {
    let (gq, gr) = (q - s.qmin, r - s.rmin);
    if gq < 0 || gq >= s.lw || gr < 0 || gr >= s.lh { return 0; }
    let idx = (gr * s.lw + gq) as usize;
    if s.vis.get(idx).copied().unwrap_or(0) != 0 { return 2; }
    if s.seen.get(idx).copied().unwrap_or(0) != 0 { return 1; }
    0
}

fn build_vis_texture(s: &Sim) -> Vec<i64> {
    let mut pix = Vec::new();
    for gr in 0..s.lh {
        for gq in 0..s.lw {
            let st = sim_hex_state(s, s.qmin + gq, s.rmin + gr);
            let b = if st == 2 { 255 } else if st == 1 { 115 } else { 0 };
            pix.push(rgba(b, 0, 0, 255));
        }
    }
    pix
}

fn c_vis(s: &Sim) -> i64 {
    let mut acc = 0i64;
    for f in 0..16usize {
        let pix = build_vis_texture(s);
        acc += pix.len() as i64 + (pix[f * 631] >> 16) + (pix[pix.len() - 1 - f * 97] >> 16);
    }
    acc
}

fn make_region(salt: i64) -> Region {
    let (cols, rows) = (80i64, 80i64);
    let mut hh = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let bowl = (c - 40) * (c - 40) + (r - 40) * (r - 40);
            hh.push(1200 + bowl / 4 + (((c * 7919 + r * 104729 + salt * 31) * 2654435761) & 1023) / 8);
        }
    }
    Region { rgn_cols: cols, rgn_rows: rows, hex_h: hh }
}

fn heap_sift_up(hk: &mut [i64], hv: &mut [i64], start: usize) {
    let mut i = start;
    while i > 0 {
        let p = (i - 1) / 2;
        if hk[p] <= hk[i] { break; }
        hk.swap(p, i);
        hv.swap(p, i);
        i = p;
    }
}

fn heap_sift_down(hk: &mut [i64], hv: &mut [i64], size: usize) {
    let mut i = 0;
    loop {
        let (l, r) = (2 * i + 1, 2 * i + 2);
        let mut smallest = i;
        if l < size && hk[l] < hk[smallest] { smallest = l; }
        if r < size && hk[r] < hk[smallest] { smallest = r; }
        if smallest == i { break; }
        hk.swap(i, smallest);
        hv.swap(i, smallest);
        i = smallest;
    }
}

fn hydro_pitfill(rgn: &Region) -> Vec<i64> {
    let (cols, rows) = (rgn.rgn_cols, rgn.rgn_rows);
    let n = (cols * rows) as usize;
    let mut filled = rgn.hex_h.clone();
    let mut seen = vec![0u8; n];
    let mut hk = vec![0i64; n];
    let mut hv = vec![0i64; n];
    let mut size = 0usize;
    for c in 0..cols {
        for r in 0..rows {
            if c == 0 || c == cols - 1 || r == 0 || r == rows - 1 {
                let i = (r * cols + c) as usize;
                hk[size] = filled[i];
                hv[size] = i as i64;
                seen[i] = 1;
                heap_sift_up(&mut hk, &mut hv, size);
                size += 1;
            }
        }
    }
    while size > 0 {
        let lvl = hk[0];
        let idx = hv[0];
        size -= 1;
        if size > 0 {
            hk[0] = hk[size];
            hv[0] = hv[size];
            heap_sift_down(&mut hk, &mut hv, size);
        }
        let cr = idx / cols;
        let cc = idx - cr * cols;
        for d in 0..6 {
            let (nc, nr) = hex_neighbor(cc, cr, d);
            if nc >= 0 && nc < cols && nr >= 0 && nr < rows {
                let ni = (nr * cols + nc) as usize;
                if seen[ni] == 0 {
                    let new_f = rgn.hex_h[ni].max(lvl + 1);
                    filled[ni] = new_f;
                    seen[ni] = 1;
                    hk[size] = new_f;
                    hv[size] = ni as i64;
                    heap_sift_up(&mut hk, &mut hv, size);
                    size += 1;
                }
            }
        }
    }
    filled
}

fn c_pitfill(rgn: &Region) -> i64 {
    hydro_pitfill(rgn).iter().enumerate().map(|(i, f)| f * ((i as i64 & 7) + 1)).sum()
}

fn make_bed(salt: i64) -> Vec<f64> {
    let g = 40i64;
    let mut bed = Vec::new();
    for gz in 0..g {
        for gx in 0..g {
            let dx = (gx - 20) as f64;
            let dz = (gz - 18) as f64;
            bed.push(2600.0 - (dx * dx + dz * dz).sqrt() * 14.0 + ((gx * 7 + gz * 13 + salt * 5) % 11) as f64 * 0.75);
        }
    }
    bed
}

fn weather_rub(h: f64) -> f64 {
    let t = ((h - TREELINE) / 300.0).clamp(0.0, 1.0);
    0.5 + 4.0 * t
}

fn shed(bed: &[f64], rub: &mut [f64], ri: usize, nj: usize, sr: f64) -> f64 {
    let d = (bed[ri] + rub[ri]) - (bed[nj] + rub[nj]);
    if d <= sr { return 0.0; }
    let cur = rub[ri];
    let m = ((d - sr) / 2.0).min(cur);
    rub[ri] = cur - m;
    rub[nj] += m;
    m
}

fn talus_relax(bed: &[f64], rub: &mut [f64], g: usize) -> i64 {
    let sr = REPOSE_TAN * DETAIL_CELL_M;
    let mut it = 0;
    let mut sweeps = 0;
    while it < TALUS_ITERS {
        let mut moved = 0.0;
        for r in 0..g {
            for c in 0..g {
                let ri = r * g + c;
                if rub[ri] > 0.0 {
                    if c + 1 < g { moved += shed(bed, rub, ri, ri + 1, sr); }
                    if c > 0 { moved += shed(bed, rub, ri, ri - 1, sr); }
                    if r + 1 < g { moved += shed(bed, rub, ri, ri + g, sr); }
                    if r > 0 { moved += shed(bed, rub, ri, ri - g, sr); }
                }
            }
        }
        sweeps += 1;
        if moved < 0.001 { it = TALUS_ITERS; } else { it += 1; }
    }
    sweeps
}

fn c_talus(bed: &[f64]) -> i64 {
    let mut rub: Vec<f64> = bed.iter().map(|&h| weather_rub(h)).collect();
    let sweeps = talus_relax(bed, &mut rub, 40);
    let mut acc = sweeps * 1000003;
    for (i, v) in rub.iter().enumerate() { acc += (v * 1000.0).round() as i64 * ((i as i64 % 13) + 1); }
    acc
}

fn glyph_w(ch: char) -> f64 {
    match ch {
        'i' | 'l' | '.' | ' ' => 3.0,
        'm' | 'w' | 'M' | 'W' => 11.0,
        'A'..='Z' => 9.0,
        _ => 7.0,
    }
}

fn measure_text(s: &str) -> f64 { s.chars().map(glyph_w).sum() }

fn fit_text(s: &str, max_w: f64) -> String {
    if measure_text(s) <= max_w { return s.to_string(); }
    let mut out = s;
    while out.len() > 1 && measure_text(&format!("{out}…")) > max_w {
        out = &out[..out.len() - 1];
    }
    format!("{out}…")
}

fn make_names() -> Vec<String> {
    let words = ["Potion", "Cure", "Light", "Wounds", "Scroll", "Magic", "Mapping", "Ring", "Protection",
                 "Wand", "Stinking", "Cloud", "Amulet", "Slow", "Digestion", "Resist"];
    (0..3000usize).map(|i| format!("{} of {} {} #{}", words[i % 16], words[(i * 7 + 3) % 16], words[(i * 11 + 5) % 16], i)).collect()
}

fn make_source_lines() -> Vec<String> {
    (0..20000i64)
        .map(|i| match i % 4 {
            0 => format!("  pub fn fx_bolt_{i}(s: Sim, caster: integer, tgt: integer) {{"),
            1 => format!("    dmg = roll(s, {}, 6);", i % 9 + 1),
            2 => "pub fn fx_(s: Sim)".to_string(),
            _ => format!("fn helper_{i}(s: Sim) -> integer {{ {i} }}"),
        })
        .collect()
}

fn c_slice_shrink(names: &[String], lines: &[String], salt: usize) -> i64 {
    let mut acc = 0i64;
    let nn = names.len();
    for k in 0..nn {
        let fitted = fit_text(&names[(k + salt) % nn], 120.0);
        let b = fitted.as_bytes();
        acc = (acc * 31 + fitted.chars().count() as i64 + b[b.len() - 4] as i64) & HMASK;
    }
    let mut fx_id: Vec<String> = Vec::new();
    let nl = lines.len();
    for k in 0..nl {
        let ln = lines[(k + salt) % nl].trim();
        if ln.starts_with("pub fn fx_") {
            if let Some(par) = ln.find('(') {
                if par > 10 { fx_id.push(ln[10..par].to_string()); }
            }
        }
    }
    for id in &fx_id { acc = (acc * 31 + id.len() as i64 + id.as_bytes()[id.len() - 1] as i64) & HMASK; }
    acc
}

fn make_region_names() -> Vec<String> {
    let parts = ["ortler", "wales", "cairngorm", "dolomiti", "vercors", "tatra", "snowdon", "jotunheimen"];
    (0..20000usize).map(|i| format!("{}_{}_{}", parts[i % 8], parts[(i * 5 + 1) % 8], i)).collect()
}

fn c_char_roundtrip(names: &[String], salt: usize) -> i64 {
    let mut acc = 0i64;
    let nn = names.len();
    for k in 0..nn {
        let nm = &names[(k + salt) % nn];
        let mut big = String::new();
        for ch in nm.chars() {
            let mut code = ch as u32;
            if (97..=122).contains(&code) { code -= 32; }
            big.push(char::from_u32(code).unwrap());
        }
        let b = big.as_bytes();
        acc = (acc * 31 + b.len() as i64 + b[0] as i64 + b[b.len() - 1] as i64 * 3) & HMASK;
    }
    acc
}

fn make_floats(salt: i64) -> Vec<f64> {
    (0..160000i64)
        .map(|i| 100.0 + ((i * 7919) % 1000) as f64 * 0.1 + salt as f64 * 0.25 + (i % 97) as f64 * 0.0078125)
        .collect()
}

fn lit_float_body(vs: &[f64]) -> String {
    let mut out = String::new();
    for (i, v) in vs.iter().enumerate() {
        if i > 0 { out = out + ","; }
        out = out + &format!("{v:.9}");
    }
    out
}

fn c_quadratic_out(vs: &[f64]) -> i64 {
    let out = lit_float_body(vs);
    let b = out.as_bytes();
    let mut acc = b.len() as i64;
    let mut p = 0;
    while p < b.len() { acc = (acc * 31 + b[p] as i64) & HMASK; p += 997; }
    acc
}

fn write_region_bin(path: &str, salt: i64, n: i64) {
    let mut bytes = Vec::with_capacity(n as usize * 2);
    for i in 0..n {
        let v = ((i * 7919 + salt * 131) & 65535) - 32768;
        bytes.extend_from_slice(&(v as i16).to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

fn c_binary_read(path: &str, n: usize) -> i64 {
    let bytes = std::fs::read(path).unwrap();
    let vals: Vec<i64> = bytes.chunks_exact(2).take(n).map(|c| i16::from_le_bytes([c[0], c[1]]) as i64).collect();
    let mut acc = vals.len() as i64;
    for v in &vals { acc = (acc * 31 + v + 32768) & HMASK; }
    acc
}

fn sort_floats(v: &[f64]) -> Vec<f64> {
    let n = v.len();
    let mut out = Vec::new();
    let mut used = vec![false; n];
    for _ in 0..n {
        let mut best: i64 = -1;
        let mut bestv = 0.0;
        for i in 0..n {
            if !used[i] {
                let vi = v[i];
                if best < 0 || vi < bestv { best = i as i64; bestv = vi; }
            }
        }
        if best >= 0 {
            out.push(bestv);
            used[best as usize] = true;
        }
    }
    out
}

fn row_crossings(xs: &[f64], ys: &[f64], yc: f64) -> Vec<f64> {
    let n = xs.len();
    let mut xcs = Vec::new();
    let mut j = n - 1;
    for i in 0..n {
        let (ax, ay, bx, by) = (xs[i], ys[i], xs[j], ys[j]);
        if (ay <= yc && by > yc) || (by <= yc && ay > yc) {
            let t = (yc - ay) / (by - ay);
            xcs.push(ax + t * (bx - ax));
        }
        j = i;
    }
    sort_floats(&xcs)
}

fn star_xs(salt: i64) -> Vec<f64> {
    (0..128i64).map(|k| 220.0 + salt as f64 + (if k % 2 == 0 { 200.0 } else { 60.0 }) * (k as f64 * std::f64::consts::PI / 64.0).cos()).collect()
}

fn star_ys() -> Vec<f64> {
    (0..128i64).map(|k| 220.0 + (if k % 2 == 0 { 200.0 } else { 60.0 }) * (k as f64 * std::f64::consts::PI / 64.0).sin()).collect()
}

fn c_sort_floats(xs: &[f64], ys: &[f64]) -> i64 {
    let mut acc = 0i64;
    for py in 20..420 {
        let sorted = row_crossings(xs, ys, py as f64 + 0.5);
        for (i, x) in sorted.iter().enumerate() { acc = (acc * 31 + (x * 16.0).round() as i64 * (i as i64 + 1)) & HMASK; }
    }
    acc
}

fn c_format_specs(salt: i64) -> i64 {
    let mut acc = 0i64;
    for f in 0..20000i64 {
        let altitude = 100.0 + (f % 400) as f64 * 0.37 + salt as f64 * 0.25;
        let fps = 30 + f % 90;
        let magic = 0x314E4752 + f * 17 + salt;
        let s = format!("{altitude:4.1}  FPS{fps:3}");
        let m = format!("0x{magic:X}");
        let (sb, mb) = (s.as_bytes(), m.as_bytes());
        acc = (acc * 31 + sb.len() as i64 + sb[2] as i64 + sb[4] as i64 * 3 + sb[sb.len() - 1] as i64 * 5
               + mb.len() as i64 + mb[mb.len() - 1] as i64 * 7) & HMASK;
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
    let sims = [make_sim(0), make_sim(1)];
    let rgns = [make_region(0), make_region(1)];
    let beds = [make_bed(0), make_bed(1)];
    let names = make_names();
    let lines = make_source_lines();
    let rnames = make_region_names();
    let fls = [make_floats(0), make_floats(1)];
    let nbin = 150000usize;
    let tmp = std::env::temp_dir();
    let bins = [tmp.join("loft_bench18_region0_rs.bin").to_string_lossy().into_owned(),
                tmp.join("loft_bench18_region1_rs.bin").to_string_lossy().into_owned()];
    write_region_bin(&bins[0], 0, nbin as i64);
    write_region_bin(&bins[1], 1, nbin as i64);
    let xss = [star_xs(0), star_xs(1)];
    let ys = star_ys();
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let mut sink: i64 = 0;

    let (us, s) = timed(n, |r| c_build_walls(black_box(&sims[(r & 1) as usize])));
    sink = sink.wrapping_add(s);
    row("build_walls", n, us, 10201, c_build_walls(&sims[0]));
    let (us, s) = timed(n, |r| c_vis(black_box(&sims[(r & 1) as usize])));
    sink = sink.wrapping_add(s);
    row("build_vis", n, us, 16 * 10201, c_vis(&sims[0]));
    let (us, s) = timed(n, |r| c_pitfill(black_box(&rgns[(r & 1) as usize])));
    sink = sink.wrapping_add(s);
    row("hydro_pitfill", n, us, 6400, c_pitfill(&rgns[0]));
    let (us, s) = timed(n, |r| c_talus(black_box(&beds[(r & 1) as usize])));
    sink = sink.wrapping_add(s);
    row("talus_relax", n, us, 1600, c_talus(&beds[0]));
    let (us, s) = timed(n, |r| c_slice_shrink(black_box(&names), black_box(&lines), (r & 1) as usize));
    sink = sink.wrapping_add(s);
    row("slice_shrink", n, us, 3000, c_slice_shrink(&names, &lines, 0));
    let (us, s) = timed(n, |r| c_char_roundtrip(black_box(&rnames), (r & 1) as usize));
    sink = sink.wrapping_add(s);
    row("char_roundtrip", n, us, 20000, c_char_roundtrip(&rnames, 0));
    let (us, s) = timed(n, |r| c_quadratic_out(black_box(&fls[(r & 1) as usize])));
    sink = sink.wrapping_add(s);
    row("quadratic_out", n, us, 160000, c_quadratic_out(&fls[0]));
    let (us, s) = timed(n, |r| c_binary_read(black_box(&bins[(r & 1) as usize]), nbin));
    sink = sink.wrapping_add(s);
    row("binary_read", n, us, nbin as i64, c_binary_read(&bins[0], nbin));
    let (us, s) = timed(n, |r| c_sort_floats(black_box(&xss[(r & 1) as usize]), &ys));
    sink = sink.wrapping_add(s);
    row("sort_floats", n, us, 400, c_sort_floats(&xss[0], &ys));
    let (us, s) = timed(n, |r| c_format_specs(black_box(r & 1)));
    sink = sink.wrapping_add(s);
    row("format_specs", n, us, 20000, c_format_specs(0));

    println!("time: 0ms sink={}", black_box(sink));
}
