// Benchmark 16: the shapes CONSUMER programs run — the Rust twin.
//
// Each routine is the same data layout and the same loop as bench.loft, written the way a
// Rust author writes it with `std` alone: tuples returned by value, plain structs in `Vec`s,
// an `enum` matched, a `HashMap` on a tuple key, a `Vec<f32>` extended six at a time, a
// catalogue of `String`-carrying records rebuilt per call.  Same result per op; the hash
// column is the receipt.
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

struct Seg { x0: f64, y0: f64, x1: f64, y1: f64 }

#[derive(Clone)]
struct Ent { id: i64, q: i64, r: i64, tq: i64, tr: i64, #[allow(dead_code)] hp: i64,
             energy: i64, speed: i64, cool: i64, age: i64, state: i64, flags: i64 }

struct Hex { h_height: i64, h_material: i64, #[allow(dead_code)] h_wall: i64, #[allow(dead_code)] h_flags: i64 }
struct Chunk { cx: i64, cy: i64, cz: i64, hexes: Vec<Hex> }
struct Map { chunks: Vec<Chunk> }

struct V3 { x: f64, y: f64, z: f64 }
#[allow(dead_code)]
struct V2 { u: f64, v: f64 }
#[allow(dead_code)]
struct Vertex { pos: V3, nrm: V3, uv: V2 }
#[allow(dead_code)]
struct Tri { a: i64, b: i64, c: i64 }
struct Mesh { verts: Vec<Vertex>, tris: Vec<Tri> }

enum Edit {
    SetHeight { q: i64, r: i64, h: i64 },
    Paint { q: i64, r: i64, mat: i64 },
    Wall { q: i64, r: i64, edge: i64 },
}

#[allow(dead_code)]
struct ItemDef { id: i64, name: String, short: String, desc: String, weight: i64, value: i64, kind: i64, tier: i64 }

fn make_segs(count: i64) -> Vec<Seg> {
    (0..count)
        .map(|i| {
            let a = i as f64 * 1.25;
            Seg { x0: a, y0: (i % 7) as f64 * 3.0, x1: a + 2.5, y1: ((i + 3) % 7) as f64 * 3.0 }
        })
        .collect()
}

fn nearest(segs: &[Seg], px: f64, py: f64) -> (f64, i64) {
    let mut best = 1000000.0f64;
    let mut which = -1i64;
    for (i, s) in segs.iter().enumerate() {
        let dx = s.x1 - s.x0;
        let dy = s.y1 - s.y0;
        let mut t = ((px - s.x0) * dx + (py - s.y0) * dy) / (dx * dx + dy * dy);
        if t < 0.0 { t = 0.0; }
        if t > 1.0 { t = 1.0; }
        let ex = px - (s.x0 + dx * t);
        let ey = py - (s.y0 + dy * t);
        let d = (ex * ex + ey * ey).sqrt();
        if d < best { best = d; which = i as i64; }
    }
    (best, which)
}

fn c_tuple_kernel(segs: &[Seg], salt: i64) -> i64 {
    let mut acc = 0.0f64;
    let mut hits = 0i64;
    for c in 0..1024i64 {
        let px = (c % 32) as f64 * 1.5 + salt as f64 * 0.25;
        let py = (c / 32) as f64 * 0.75;
        let n = nearest(segs, px, py);
        acc += n.0;
        hits += n.1;
    }
    (acc * 16.0).round() as i64 + hits
}

fn make_tiles(w: i64, h: i64) -> Vec<i64> {
    (0..w * h).map(|i| if ((i % w) * 7 + (i / w) * 13) % 11 == 0 { 1 } else { 0 }).collect()
}

fn c_fov(tiles: &[i64], vis: &mut [i64], w: i64, salt: i64) -> i64 {
    let oq = 50 + salt;
    let orr = 50i64;
    let mut seen = 0i64;
    for r in 40..61i64 {
        for q in (40 + salt)..(61 + salt) {
            let dq = q - oq;
            let dr = r - orr;
            if dq * dq + dr * dr > 100 { continue; }
            let n = dq.abs().max(dr.abs());
            let mut clear = 1i64;
            for k in 1..n {
                let rq = (oq * n + dq * k + n / 2) / n;
                let rr = (orr * n + dr * k + n / 2) / n;
                if tiles[(rr * w + rq) as usize] == 1 { clear = 0; break; }
            }
            vis[(r * w + q) as usize] = clear;
            seen += clear;
        }
    }
    seen
}

fn nbr(q: i64, r: i64, d: i64) -> (i64, i64) {
    let odd = r & 1;
    match d {
        0 => (q + 1, r),
        1 => (q - 1, r),
        2 => (q + odd, r - 1),
        3 => (q + odd - 1, r - 1),
        4 => (q + odd, r + 1),
        _ => (q + odd - 1, r + 1),
    }
}

fn c_flow(tiles: &[i64], w: i64, h: i64, salt: i64) -> i64 {
    let mut flow: Vec<i64> = Vec::new();
    for _ in 0..w * h { flow.push(-1); }
    let mut frontier: Vec<i64> = Vec::new();
    let start = 50 * w + 50 + salt;
    flow[start as usize] = 0;
    frontier.push(start);
    let mut head = 0usize;
    let mut total = 0i64;
    while head < frontier.len() {
        let cur = frontier[head];
        head += 1;
        let cd = flow[cur as usize];
        let (cq, cr) = (cur % w, cur / w);
        for d in 0..6 {
            let p = nbr(cq, cr, d);
            if p.0 < 0 || p.0 >= w || p.1 < 0 || p.1 >= h { continue; }
            let ni = (p.1 * w + p.0) as usize;
            if tiles[ni] == 1 || flow[ni] >= 0 { continue; }
            flow[ni] = cd + 1;
            total += cd + 1;
            frontier.push(ni as i64);
        }
    }
    total + frontier.len() as i64
}

fn sign(v: i64) -> i64 { if v > 0 { 1 } else if v < 0 { -1 } else { 0 } }

fn c_entity_tick(salt: i64) -> i64 {
    let mut ents: Vec<Ent> = (0..60i64)
        .map(|i| Ent { id: i, q: (i * 7) % 40, r: (i * 11) % 40, tq: (i * 3 + salt) % 40, tr: (i * 5) % 40,
                       hp: 20, energy: 0, speed: 30 + i % 50, cool: i % 4, age: 0, state: 0, flags: 0 })
        .collect();
    for _tick in 0..50 {
        for i in 0..ents.len() {
            let mut e = ents[i].clone();
            e.energy += e.speed;
            if e.cool > 0 { e.cool -= 1; }
            if e.energy >= 100 {
                e.energy -= 100;
                let nq = e.q + sign(e.tq - e.q);
                let nr = e.r + sign(e.tr - e.r);
                let taken = ents.iter().any(|o| o.id != e.id && o.q == nq && o.r == nr);
                if !taken { e.q = nq; e.r = nr; e.state = 1; } else { e.state = 2; e.flags += 1; }
            }
            e.age += 1;
            ents[i] = e;
        }
    }
    ents.iter().map(|e| e.q * 31 + e.r + e.flags).sum()
}

fn make_map() -> Map {
    let mut m = Map { chunks: Vec::new() };
    for cy in 0..4i64 {
        for cx in 0..4i64 {
            let hexes = (0..1024i64)
                .map(|i| Hex { h_height: (i * 3 + cx * 5 + cy * 7) % 17, h_material: 0, h_wall: 0, h_flags: 0 })
                .collect();
            m.chunks.push(Chunk { cx, cy, cz: 0, hexes });
        }
    }
    m
}

fn map_get(m: &Map, q: i64, r: i64) -> i64 {
    let (cx, cy) = (q / 32, r / 32);
    for c in &m.chunks {
        if c.cx == cx && c.cy == cy && c.cz == 0 {
            return c.hexes[((r % 32) * 32 + q % 32) as usize].h_height;
        }
    }
    -1
}

fn map_set(m: &mut Map, q: i64, r: i64, mat: i64) {
    let (cx, cy) = (q / 32, r / 32);
    for c in &mut m.chunks {
        if c.cx == cx && c.cy == cy && c.cz == 0 {
            c.hexes[((r % 32) * 32 + q % 32) as usize].h_material = mat;
            return;
        }
    }
}

fn c_chunk_lookup(m: &mut Map, salt: i64) -> i64 {
    let mut acc = 0i64;
    let mut state = 12345 + salt;
    for i in 0..20000i64 {
        state = (state * 1103515245 + 12345) & 2147483647;
        let q = (state >> 8) % 128;
        let r = (state >> 16) % 128;
        acc += map_get(m, q, r);
        if i % 4 == 0 { map_set(m, q, r, (i + salt) & 7); }
    }
    acc
}

fn emit_mesh(hexes: i64, salt: i64) -> Mesh {
    let ox = [1.0, 0.5, -0.5, -1.0, -0.5, 0.5];
    let oz = [0.0, 0.866, 0.866, 0.0, -0.866, -0.866];
    let mut m = Mesh { verts: Vec::new(), tris: Vec::new() };
    for i in 0..hexes {
        let bx = (i % 32) as f64 * 1.5;
        let bz = (i / 32) as f64 * 1.732 + salt as f64 * 0.5;
        let by = ((i * 7) % 5) as f64 * 0.25;
        let base = m.verts.len() as i64;
        m.verts.push(Vertex { pos: V3 { x: bx, y: by, z: bz }, nrm: V3 { x: 0.0, y: 1.0, z: 0.0 }, uv: V2 { u: 0.5, v: 0.5 } });
        for k in 0..6usize {
            m.verts.push(Vertex { pos: V3 { x: bx + ox[k], y: by, z: bz + oz[k] },
                                  nrm: V3 { x: 0.0, y: 1.0, z: 0.0 }, uv: V2 { u: ox[k] * 0.5 + 0.5, v: oz[k] * 0.5 + 0.5 } });
            m.tris.push(Tri { a: base, b: base + 1 + k as i64, c: base + 1 + (k as i64 + 1) % 6 });
        }
    }
    m
}

fn c_mesh_emit(salt: i64) -> i64 {
    let m = emit_mesh(1024, salt);
    m.verts.len() as i64 + m.tris.len() as i64 + (m.verts[m.verts.len() - 1].pos.z * 8.0).round() as i64
        + m.tris[m.tris.len() - 1].c
}

fn c_mesh_aabb(m: &Mesh, salt: i64) -> i64 {
    let (mut lox, mut hix) = (1000000.0f64, -1000000.0f64);
    let (mut loy, mut hiy) = (1000000.0f64, -1000000.0f64);
    let (mut loz, mut hiz) = (1000000.0f64, -1000000.0f64);
    for v in &m.verts {
        if v.pos.x < lox { lox = v.pos.x; }
        if v.pos.x > hix { hix = v.pos.x; }
        if v.pos.y < loy { loy = v.pos.y; }
        if v.pos.y > hiy { hiy = v.pos.y; }
        if v.pos.z < loz { loz = v.pos.z; }
        if v.pos.z > hiz { hiz = v.pos.z; }
    }
    (((hix - lox) + (hiy - loy) + (hiz - loz)) * 16.0).round() as i64 + salt
}

fn c_enum_match(salt: i64) -> i64 {
    let mut edits: Vec<Edit> = Vec::new();
    for i in 0..3000i64 {
        let q = (i * 7 + salt) % 64;
        let r = (i * 13) % 64;
        edits.push(match i % 3 {
            0 => Edit::SetHeight { q, r, h: i % 19 },
            1 => Edit::Paint { q, r, mat: i % 5 },
            _ => Edit::Wall { q, r, edge: i % 6 },
        });
    }
    let mut heights = vec![0i64; 4096];
    let mut mats = vec![0i64; 4096];
    let mut walls = 0i64;
    for e in &edits {
        match e {
            Edit::SetHeight { q, r, h } => heights[(r * 64 + q) as usize] = *h,
            Edit::Paint { q, r, mat } => mats[(r * 64 + q) as usize] = *mat,
            Edit::Wall { q, r, edge } => walls += edge + q - r,
        }
    }
    let mut acc = walls;
    for i in 0..4096 { acc += heights[i] * 3 + mats[i]; }
    acc
}

fn c_composite_hash(salt: i64) -> i64 {
    let mut painted: HashMap<(i64, i64), i64> = HashMap::new();
    for line in 0..40i64 {
        for s in 0..50i64 {
            let key = (line * 3 + s + salt, (line * 5 + s * 2) % 97);
            painted.entry(key).and_modify(|k| *k = (*k + 1) % 7).or_insert((line + s) % 7);
        }
    }
    for line in 0..40i64 {
        for s in 0..50i64 {
            if (line + s) % 3 == 0 { painted.remove(&(line * 3 + s + salt, (line * 5 + s * 2) % 97)); }
        }
    }
    let mut acc = 0i64;
    for line in 0..40i64 {
        for s in 0..50i64 {
            if let Some(k) = painted.get(&(line * 3 + s + salt, (line * 5 + s * 2) % 97)) { acc += k + 10; }
        }
    }
    acc
}

fn c_f32_build(salt: i64) -> i64 {
    let mut buf: Vec<f32> = Vec::new();
    for h in 0..1024i64 {
        let bx = (h % 32) as f32;
        let bz = (h / 32 + salt) as f32;
        for t in 0..18 {
            let a = t as f32 * 0.125;
            buf.extend_from_slice(&[bx, a, bz, 0.0, 1.0, 0.0]);
            buf.extend_from_slice(&[bx + 1.0, a, bz, 0.0, 1.0, 0.0]);
            buf.extend_from_slice(&[bx, a, bz + 1.0, 0.0, 1.0, 0.0]);
        }
    }
    buf.len() as i64 + (buf[buf.len() - 4] as f64 * 4.0).round() as i64
}

fn game_items(salt: i64) -> Vec<ItemDef> {
    (0..64i64)
        .map(|i| ItemDef { id: i, name: format!("item-{}", i + salt), short: format!("i{i}"),
                           desc: format!("A thing numbered {} of tier {}", i, i % 5),
                           weight: i % 9, value: i * 3 + salt, kind: i % 4, tier: i % 5 })
        .collect()
}

fn c_catalog(salt: i64) -> i64 {
    let mut acc = 0i64;
    for d in 0..64usize {
        let items = game_items(salt);
        acc += items[d].short.len() as i64 + items[d].value;
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
    let n = arg_n(40);
    let segs = make_segs(40);
    let tiles = make_tiles(101, 101);
    let mut vis = vec![0i64; 101 * 101];
    let mut map = make_map();
    let mesh = emit_mesh(1024, 0);
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let mut sink: i64 = 0;

    let (us, s) = timed(n, |r| c_tuple_kernel(black_box(&segs), r & 1));
    sink = sink.wrapping_add(s);
    row("tuple_kernel", n, us, 1024, c_tuple_kernel(&segs, 0));
    let (us, s) = timed(n, |r| c_fov(black_box(&tiles), &mut vis, 101, r & 1));
    sink = sink.wrapping_add(s);
    row("fov_rays", n, us, 441, c_fov(&tiles, &mut vis, 101, 0));
    let (us, s) = timed(n, |r| c_flow(black_box(&tiles), 101, 101, r & 1));
    sink = sink.wrapping_add(s);
    row("bfs_flow", n, us, 10201, c_flow(&tiles, 101, 101, 0));
    let (us, s) = timed(n, |r| c_entity_tick(r & 1));
    sink = sink.wrapping_add(s);
    row("entity_tick", n, us, 3000, c_entity_tick(0));
    let (us, s) = timed(n, |r| c_chunk_lookup(black_box(&mut map), r & 1));
    sink = sink.wrapping_add(s);
    row("chunk_lookup", n, us, 20000, c_chunk_lookup(&mut map, 0));
    let (us, s) = timed(n, |r| c_mesh_emit(r & 1));
    sink = sink.wrapping_add(s);
    row("mesh_emit", n, us, 1024, c_mesh_emit(0));
    let (us, s) = timed(n, |r| c_mesh_aabb(black_box(&mesh), r & 1));
    sink = sink.wrapping_add(s);
    row("mesh_aabb", n, us, 7168, c_mesh_aabb(&mesh, 0));
    let (us, s) = timed(n, |r| c_enum_match(r & 1));
    sink = sink.wrapping_add(s);
    row("enum_match", n, us, 3000, c_enum_match(0));
    let (us, s) = timed(n, |r| c_composite_hash(r & 1));
    sink = sink.wrapping_add(s);
    row("composite_hash", n, us, 2000, c_composite_hash(0));
    let (us, s) = timed(n, |r| c_f32_build(r & 1));
    sink = sink.wrapping_add(s);
    row("f32_build", n, us, 1024 * 18 * 18, c_f32_build(0));
    let (us, s) = timed(n, |r| c_catalog(r & 1));
    sink = sink.wrapping_add(s);
    row("catalog_churn", n, us, 64, c_catalog(0));

    println!("time: 0ms sink={}", black_box(sink));
}
