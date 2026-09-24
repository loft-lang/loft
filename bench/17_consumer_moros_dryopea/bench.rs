// Benchmark 17: the hot loops of the two loft GAMES, moros and dryopea — the Rust twin.
//
// Each routine is the same data layout and the same loop as bench.loft, written the way a
// Rust author writes it with `std` alone: small `Copy` structs passed by value, `&Map` /
// `&mut Map`, `Vec<Button>` with `String` labels and `format!`, `to_string()` + a linear
// `find`, a `HashMap` on a `(q, r)` key, `Vec::truncate` / `Vec::drain` for the timeline, and
// a hand-written JSON writer and parser that produce and read the byte-identical text loft's
// `"{m:j}"` writes.  Same result per op; the hash column is the receipt.
use std::collections::HashMap;
use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Instant;

// ── moros_map types ───────────────────────────────────────────────────────────────────
#[derive(Clone, Copy, Default)]
struct HexAddress { ha_q: i64, ha_r: i64, ha_cy: i64 }

#[derive(Clone, Copy, Default)]
struct Hex { h_height: i64, h_material: i64, h_item: i64, h_item_rotation: i64, h_wall_n: i64, h_wall_ne: i64, h_wall_se: i64 }

#[derive(Clone, Default)]
struct Chunk { ck_cx: i64, ck_cy: i64, ck_cz: i64, ck_hexes: Vec<Hex> }

#[derive(Clone, Default)]
struct MaterialDef {
    md_name: String, md_category: String, md_stair_kind: String, md_texture: i64, md_tint_r: i64, md_tint_g: i64,
    md_tint_b: i64, md_walkable: i64, md_swimmable: i64, md_climbable: i64, md_slippery: i64, md_loud: i64,
}

#[derive(Clone, Default)]
struct WallDef {
    wd_name: String, wd_body: String, wd_base: String, wd_thickness: f64, wd_texture: i64,
    wd_tint_r: i64, wd_tint_g: i64, wd_tint_b: i64,
}

#[derive(Clone, Default)]
struct ItemDef { id_name: String, id_kind: String, id_model: i64, id_symmetric: i64, id_arc_radius: f64 }

#[derive(Clone, Default)]
struct SpawnPoint {
    sp_hex: HexAddress, sp_kind: String, sp_creature: i64, sp_npc_id: i64, sp_count: i64, sp_facing: i64,
    sp_condition: String,
}

#[derive(Clone, Default)]
struct NpcWaypoint {
    wp_hex: HexAddress, wp_activity: String, wp_time_start: i64, wp_time_end: i64, wp_facing: i64, wp_note: String,
}

#[derive(Clone, Default)]
struct NpcRoutine { nr_npc_id: i64, nr_name: String, nr_creature: i64, nr_waypoints: Vec<NpcWaypoint> }

#[derive(Clone, Default)]
struct Map {
    m_name: String, m_chunks: Vec<Chunk>, m_material_palette: Vec<MaterialDef>, m_wall_palette: Vec<WallDef>,
    m_item_palette: Vec<ItemDef>, m_spawn_points: Vec<SpawnPoint>, m_npc_routines: Vec<NpcRoutine>,
}

// ── graphics math / mesh types ────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct Vec2 { x: f64, y: f64 }
#[derive(Clone, Copy)]
struct Vec3 { x: f64, y: f64, z: f64 }
#[allow(dead_code)]
struct Vertex { pos: Vec3, normal: Vec3, uv: Vec2 }
#[allow(dead_code)]
struct Triangle { a: i64, b: i64, c: i64 }
struct Mesh { name: String, vertices: Vec<Vertex>, triangles: Vec<Triangle> }

fn vec2(x: f64, y: f64) -> Vec2 { Vec2 { x, y } }
fn vec3(x: f64, y: f64, z: f64) -> Vec3 { Vec3 { x, y, z } }
fn vertex(pos: Vec3, normal: Vec3, uv: Vec2) -> Vertex { Vertex { pos, normal, uv } }
fn mesh(name: String) -> Mesh { Mesh { name, vertices: Vec::new(), triangles: Vec::new() } }

const HEX_WIDTH: f64 = 1.7320508;
const HEX_ROW_HEIGHT: f64 = 1.5;
const HEIGHT_SCALE: f64 = 0.25;
const STEP_UP_LIMIT: f64 = 0.5;

// ── moros_map: address helpers and hex access ─────────────────────────────────────────
fn chunk_idx_32(v: i64) -> i64 {
    let vr = v % 32;
    if vr < 0 { (v - vr - 32) / 32 } else { (v - vr) / 32 }
}

fn hex_idx_32(v: i64) -> i64 {
    let vr = v % 32;
    if vr < 0 { vr + 32 } else { vr }
}

fn build_chunk(cx: i64, cy: i64, cz: i64) -> Chunk {
    Chunk { ck_cx: cx, ck_cy: cy, ck_cz: cz, ck_hexes: vec![Hex::default(); 1024] }
}

fn map_ensure_chunk(m: &mut Map, q: i64, r: i64, cy: i64) {
    let (cx, cz) = (chunk_idx_32(q), chunk_idx_32(r));
    if m.m_chunks.iter().any(|c| c.ck_cx == cx && c.ck_cy == cy && c.ck_cz == cz) { return; }
    m.m_chunks.push(build_chunk(cx, cy, cz));
}

fn map_set_hex(m: &mut Map, q: i64, r: i64, cy: i64, h: Hex) {
    map_ensure_chunk(m, q, r, cy);
    let (cx, cz) = (chunk_idx_32(q), chunk_idx_32(r));
    let idx = (hex_idx_32(q) * 32 + hex_idx_32(r)) as usize;
    for c in &mut m.m_chunks {
        if c.ck_cx == cx && c.ck_cy == cy && c.ck_cz == cz {
            c.ck_hexes[idx] = h;
            return;
        }
    }
}

fn map_get_hex(m: &Map, q: i64, r: i64, cy: i64) -> Hex {
    let (cx, cz) = (chunk_idx_32(q), chunk_idx_32(r));
    let idx = hex_idx_32(q) * 32 + hex_idx_32(r);
    for c in &m.m_chunks {
        if c.ck_cx == cx && c.ck_cy == cy && c.ck_cz == cz && idx >= 0 && (idx as usize) < c.ck_hexes.len() {
            return c.ck_hexes[idx as usize];
        }
    }
    Hex::default()
}

fn map_set_height(m: &mut Map, q: i64, r: i64, cy: i64, height: i64) {
    let mut cur = map_get_hex(m, q, r, cy);
    cur.h_height = height;
    map_set_hex(m, q, r, cy, cur);
}

fn hex_distance(q1: i64, r1: i64, q2: i64, r2: i64) -> i64 {
    let dq = (q2 - q1).abs();
    let dr = (r2 - r1).abs();
    let ds = ((q2 + r2) - (q1 + r1)).abs();
    dq.max(dr.max(ds))
}

fn lerp_int(a: i64, b: i64, i: i64, n: i64) -> i64 {
    if n == 0 { return a; }
    a + (b - a) * i / n
}

fn make_map(salt: i64) -> Map {
    let mut m = Map { m_name: "harbour \"east\" v2".to_string(), ..Map::default() };
    for cz in 0..2i64 {
        for cx in 0..2i64 {
            let hexes = (0..1024i64)
                .map(|i| {
                    let q = cx * 32 + i / 32;
                    let r = cz * 32 + i % 32;
                    Hex {
                        h_height: 10 + ((q * 3 + r * 5 + salt) % 17) / 4,
                        h_material: (q + r * 3 + salt) % 10,
                        h_item: (q * 7 + r) % 5,
                        h_item_rotation: (q + r) % 24,
                        h_wall_n: if (q * 5 + r * 3) % 11 == 0 { 1 + salt } else { 0 },
                        h_wall_ne: if (q + r * 7 + salt) % 13 == 0 { 2 } else { 0 },
                        h_wall_se: if (q * 2 + r + salt) % 17 == 0 { 3 } else { 0 },
                    }
                })
                .collect();
            m.m_chunks.push(Chunk { ck_cx: cx, ck_cy: 0, ck_cz: cz, ck_hexes: hexes });
        }
    }
    for i in 0..10i64 {
        m.m_material_palette.push(MaterialDef {
            md_name: format!("mat{i}"),
            md_category: if i % 4 == 3 { "stair" } else { "ground" }.to_string(),
            md_stair_kind: "none".to_string(), md_texture: i + 1, md_tint_r: 200 + i, md_tint_g: 150 + i,
            md_tint_b: 100 + i, md_walkable: 1, md_swimmable: i % 2, md_climbable: 0, md_slippery: i % 3 / 2, md_loud: 0,
        });
    }
    for i in 0..5i64 {
        m.m_wall_palette.push(WallDef {
            wd_name: format!("wall{i}"), wd_body: "SOLID".to_string(), wd_base: "stone".to_string(),
            wd_thickness: 0.25 * (i + 1) as f64, wd_texture: 20 + i, wd_tint_r: 90, wd_tint_g: 80 + i, wd_tint_b: 70,
        });
    }
    for i in 0..4i64 {
        m.m_item_palette.push(ItemDef {
            id_name: format!("item{i}"), id_kind: "crate".to_string(), id_model: 30 + i, id_symmetric: i % 2,
            id_arc_radius: 0.5 + i as f64,
        });
    }
    for i in 0..6i64 {
        m.m_spawn_points.push(SpawnPoint {
            sp_hex: HexAddress { ha_q: 3 + i * 9, ha_r: 5 + i * 7, ha_cy: 0 }, sp_kind: "npc".to_string(),
            sp_creature: i % 3, sp_npc_id: 100 + i, sp_count: 1 + i % 2, sp_facing: i % 6, sp_condition: "day".to_string(),
        });
    }
    for i in 0..3i64 {
        let nr_waypoints = (0..4i64)
            .map(|k| NpcWaypoint {
                wp_hex: HexAddress { ha_q: 10 + k * 5, ha_r: 20 + i * 3, ha_cy: 0 }, wp_activity: "walk".to_string(),
                wp_time_start: k * 60, wp_time_end: k * 60 + 45, wp_facing: k % 6, wp_note: String::new(),
            })
            .collect();
        m.m_npc_routines.push(NpcRoutine { nr_npc_id: 100 + i, nr_name: format!("npc{i}"), nr_creature: i, nr_waypoints });
    }
    m
}

// ── moros_render: world <-> hex ───────────────────────────────────────────────────────
fn hex_to_world(q: i64, r: i64, height: i64) -> Vec3 {
    let x = q as f64 * HEX_WIDTH + (r % 2) as f64 * (HEX_WIDTH / 2.0);
    let y = height as f64 * HEIGHT_SCALE;
    let z = r as f64 * HEX_ROW_HEIGHT;
    vec3(x, y, z)
}

fn world_to_hex(wx: f64, wz: f64) -> HexAddress {
    let r_int = (wz / HEX_ROW_HEIGHT).round() as i64;
    let off = if r_int % 2 == 0 { 0.0 } else { HEX_WIDTH / 2.0 };
    let q = ((wx - off) / HEX_WIDTH).round() as i64;
    HexAddress { ha_q: q, ha_r: r_int, ha_cy: 0 }
}

// ── moros_sim collide: resolve_move ───────────────────────────────────────────────────
fn edge_direction(from_q: i64, from_r: i64, to_q: i64, to_r: i64) -> i64 {
    match (to_q - from_q, to_r - from_r) {
        (0, -1) => 0,
        (1, -1) => 1,
        (1, 0) => 2,
        (0, 1) => 3,
        (-1, 1) => 4,
        (-1, 0) => 5,
        _ => -1,
    }
}

fn wall_value_on_edge(map: &Map, fq: i64, fr: i64, tq: i64, tr: i64, cy: i64) -> i64 {
    match edge_direction(fq, fr, tq, tr) {
        0 => map_get_hex(map, fq, fr, cy).h_wall_n,
        1 => map_get_hex(map, fq, fr, cy).h_wall_ne,
        2 => map_get_hex(map, fq, fr, cy).h_wall_se,
        3 => map_get_hex(map, fq, fr + 1, cy).h_wall_n,
        4 => map_get_hex(map, fq - 1, fr + 1, cy).h_wall_ne,
        5 => map_get_hex(map, fq - 1, fr, cy).h_wall_se,
        _ => 0,
    }
}

fn blocked_by_wall(map: &Map, fq: i64, fr: i64, tq: i64, tr: i64, cy: i64) -> bool {
    wall_value_on_edge(map, fq, fr, tq, tr, cy) != 0
}

fn floor_y_at(map: &Map, wx: f64, wz: f64) -> f64 {
    let a = world_to_hex(wx, wz);
    map_get_hex(map, a.ha_q, a.ha_r, 0).h_height as f64 * HEIGHT_SCALE
}

fn hex_at(wx: f64, wz: f64) -> HexAddress { world_to_hex(wx, wz) }

fn move_blocked(map: &Map, fr: Vec3, to: Vec3) -> bool {
    let a = hex_at(fr.x, fr.z);
    let b = hex_at(to.x, to.z);
    if a.ha_q == b.ha_q && a.ha_r == b.ha_r { return false; }
    let fy_to = floor_y_at(map, to.x, to.z);
    let fy_fr = floor_y_at(map, fr.x, fr.z);
    if (fy_to - fy_fr) > STEP_UP_LIMIT { return true; }
    blocked_by_wall(map, a.ha_q, a.ha_r, b.ha_q, b.ha_r, 0)
}

struct MoveResult { mr_pos: Vec3, mr_vel_y: f64, mr_on_ground: bool }

fn resolve_move(map: &Map, from: Vec3, vel_y: f64, dxyz: Vec3) -> MoveResult {
    let mut new_x = from.x;
    let new_y;
    let mut new_z = from.z;
    let mut on_ground = false;
    let mut vy = vel_y;
    let cand_x = from.x + dxyz.x;
    if !move_blocked(map, from, vec3(cand_x, from.y, from.z)) { new_x = cand_x; }
    let cand_z = from.z + dxyz.z;
    if !move_blocked(map, vec3(new_x, from.y, from.z), vec3(new_x, from.y, cand_z)) { new_z = cand_z; }
    let cand_y = from.y + dxyz.y;
    let floor = floor_y_at(map, new_x, new_z);
    if cand_y <= floor {
        new_y = floor;
        on_ground = true;
        vy = 0.0;
    } else {
        new_y = cand_y;
    }
    MoveResult { mr_pos: vec3(new_x, new_y, new_z), mr_vel_y: vy, mr_on_ground: on_ground }
}

fn c_resolve_move(map: &Map, salt: i64) -> i64 {
    let mut state = 777 + salt;
    let mut acc = 0i64;
    for _ in 0..10000 {
        state = (state * 1103515245 + 12345) & 2147483647;
        let q = 2 + (state >> 4) % 60;
        let r = 2 + (state >> 12) % 60;
        let centre = hex_to_world(q, r, 0);
        let lift = ((state >> 18) % 5) as f64 * 0.25 - 0.25;
        let from = vec3(centre.x + 0.1, floor_y_at(map, centre.x, centre.z) + lift, centre.z - 0.1);
        let dxyz = vec3(((state >> 20) % 9 - 4) as f64 * 0.25, -0.3, ((state >> 24) % 9 - 4) as f64 * 0.25);
        let vel = ((state >> 8) % 7) as f64 * 0.5 - 1.5;
        let mv = resolve_move(map, from, vel, dxyz);
        acc += (mv.mr_pos.x * 16.0).round() as i64 + (mv.mr_pos.y * 16.0).round() as i64 * 3
            + (mv.mr_pos.z * 16.0).round() as i64 * 7 + (mv.mr_vel_y * 16.0).round() as i64;
        if mv.mr_on_ground { acc += 1; }
    }
    acc
}

// ── moros_ui: panel_build + panel_hit_test ────────────────────────────────────────────
#[derive(Clone, Copy)]
struct Rect { r_x: i64, r_y: i64, r_w: i64, r_h: i64 }

fn rect(r_x: i64, r_y: i64, r_w: i64, r_h: i64) -> Rect { Rect { r_x, r_y, r_w, r_h } }

fn rect_contains(r: &Rect, mx: i64, my: i64) -> bool { mx >= r.r_x && mx < r.r_x + r.r_w && my >= r.r_y && my < r.r_y + r.r_h }

#[allow(dead_code)]
struct Button { btn_id: i64, btn_rect: Rect, btn_label: String, btn_hotkey_label: String, btn_selected: bool }
struct ListBox { lb_rect: Rect, lb_items: Vec<String>, lb_selected: i64, lb_scroll: i64, lb_item_height: i64 }
#[allow(dead_code)]
struct StatusStrip { ss_rect: Rect, ss_text: String }
struct Panel { p_rect: Rect, p_toolbar: Vec<Button>, p_list: ListBox, p_status: StatusStrip }

#[derive(Clone, Copy, PartialEq)]
enum ToolKind { None, RaiseHex, LowerHex, PlaceStencil, PlaceItem, PlaceWall }

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct ToolState {
    ts_current: ToolKind, ts_selected_stencil: i64, ts_selected_item: i64, ts_selected_wall: i64,
    ts_wall_direction: i64, ts_height_step: i64,
}

fn tool_state_default() -> ToolState {
    ToolState { ts_current: ToolKind::None, ts_selected_stencil: 0, ts_selected_item: 0, ts_selected_wall: 0,
                ts_wall_direction: 0, ts_height_step: 4 }
}

const PANEL_WIDTH: i64 = 240;
const TOOLBAR_ROW_HEIGHT: i64 = 32;
const TOOLBAR_GAP: i64 = 4;
const TOOLBAR_TOP: i64 = 8;
const SEPARATOR_HEIGHT: i64 = 2;
const LIST_ITEM_HEIGHT: i64 = 20;
const STATUS_HEIGHT: i64 = 24;
const TOOLBAR_BUTTONS: i64 = 6;

const WALL_PALETTE_NAMES: [&str; 5] = ["none", "solid", "half", "thick_flat", "thick_curved"];
const ITEM_PALETTE_NAMES_PLACEHOLDER: [&str; 4] = ["item_0", "item_1", "item_2", "item_3"];
const HEIGHT_STEP_LABELS: [&str; 4] = ["1", "2", "4", "8"];

fn panel_rect(_window_w: i64, window_h: i64) -> Rect { rect(0, 0, PANEL_WIDTH, window_h) }

fn toolbar_button_rect(idx: i64) -> Rect {
    rect(8, TOOLBAR_TOP + idx * (TOOLBAR_ROW_HEIGHT + TOOLBAR_GAP), PANEL_WIDTH - 16, TOOLBAR_ROW_HEIGHT)
}

fn list_rect(window_h: i64) -> Rect {
    let top = TOOLBAR_TOP + TOOLBAR_BUTTONS * (TOOLBAR_ROW_HEIGHT + TOOLBAR_GAP) + SEPARATOR_HEIGHT + 8;
    let bottom = window_h - STATUS_HEIGHT - 4;
    rect(8, top, PANEL_WIDTH - 16, bottom - top)
}

fn status_rect(window_h: i64) -> Rect { rect(0, window_h - STATUS_HEIGHT, PANEL_WIDTH, STATUS_HEIGHT) }

fn tool_of_id(id: i64) -> ToolKind {
    match id {
        1 => ToolKind::RaiseHex,
        2 => ToolKind::LowerHex,
        3 => ToolKind::PlaceStencil,
        4 => ToolKind::PlaceItem,
        5 => ToolKind::PlaceWall,
        _ => ToolKind::None,
    }
}

fn stencil_palette_size() -> i64 { 3 }

fn stencil_palette_name(i: i64) -> &'static str {
    match i {
        1 => "house_small",
        2 => "spiral_stair",
        _ => "flat",
    }
}

fn strings(names: &[&str]) -> Vec<String> { names.iter().map(|s| s.to_string()).collect() }

fn palette_items_for_tool(tools: &ToolState) -> Vec<String> {
    match tools.ts_current {
        ToolKind::RaiseHex | ToolKind::LowerHex => strings(&HEIGHT_STEP_LABELS),
        ToolKind::PlaceStencil => (0..stencil_palette_size()).map(|i| stencil_palette_name(i).to_string()).collect(),
        ToolKind::PlaceItem => strings(&ITEM_PALETTE_NAMES_PLACEHOLDER),
        ToolKind::PlaceWall => strings(&WALL_PALETTE_NAMES),
        ToolKind::None => Vec::new(),
    }
}

fn current_palette_selection(tools: &ToolState) -> i64 {
    match tools.ts_current {
        ToolKind::RaiseHex | ToolKind::LowerHex => match tools.ts_height_step {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            _ => -1,
        },
        ToolKind::PlaceStencil => tools.ts_selected_stencil,
        ToolKind::PlaceItem => tools.ts_selected_item,
        ToolKind::PlaceWall => tools.ts_selected_wall,
        ToolKind::None => -1,
    }
}

fn make_button(id: i64, label: &str, hotkey: &str, selected: bool) -> Button {
    Button { btn_id: id, btn_rect: toolbar_button_rect(id), btn_label: label.to_string(),
             btn_hotkey_label: hotkey.to_string(), btn_selected: selected }
}

#[allow(clippy::too_many_arguments)]
fn panel_build(tools: &ToolState, window_w: i64, window_h: i64, hex_q: i64, hex_r: i64, hex_cy: i64, altitude: f64, fps: i64) -> Panel {
    let cur = tools.ts_current;
    let buttons = vec![
        make_button(0, "None", "1", cur == ToolKind::None),
        make_button(1, "Raise", "2", cur == ToolKind::RaiseHex),
        make_button(2, "Lower", "3", cur == ToolKind::LowerHex),
        make_button(3, "Stencil", "4", cur == ToolKind::PlaceStencil),
        make_button(4, "Item", "5", cur == ToolKind::PlaceItem),
        make_button(5, "Wall", "6", cur == ToolKind::PlaceWall),
    ];
    let list = ListBox {
        lb_rect: list_rect(window_h),
        lb_items: palette_items_for_tool(tools),
        lb_selected: current_palette_selection(tools),
        lb_scroll: 0,
        lb_item_height: LIST_ITEM_HEIGHT,
    };
    let status = StatusStrip {
        ss_rect: status_rect(window_h),
        ss_text: format!("q={hex_q} r={hex_r} cy={hex_cy}  y={altitude:4.1}  FPS{fps:3}"),
    };
    Panel { p_rect: panel_rect(window_w, window_h), p_toolbar: buttons, p_list: list, p_status: status }
}

enum UiHit { UhWorld, UhToolButton { tb_id: i64 }, UhListItem { li_idx: i64 }, UhNone }

fn panel_hit_test(p: &Panel, mx: i64, my: i64) -> UiHit {
    if mx >= p.p_rect.r_x + p.p_rect.r_w { return UiHit::UhWorld; }
    for b in &p.p_toolbar {
        if rect_contains(&b.btn_rect, mx, my) { return UiHit::UhToolButton { tb_id: b.btn_id }; }
    }
    if rect_contains(&p.p_list.lb_rect, mx, my) {
        let local_y = my - p.p_list.lb_rect.r_y + p.p_list.lb_scroll;
        let idx = local_y / p.p_list.lb_item_height;
        if idx >= 0 && (idx as usize) < p.p_list.lb_items.len() { return UiHit::UhListItem { li_idx: idx }; }
    }
    UiHit::UhNone
}

fn c_panel_build(salt: i64) -> i64 {
    let mut tools = tool_state_default();
    let mut acc = 0i64;
    for f in 0..2000i64 {
        tools.ts_current = tool_of_id(f % 6);
        tools.ts_selected_stencil = f % 3;
        tools.ts_selected_item = f % 4;
        tools.ts_selected_wall = f % 5;
        let p = panel_build(&tools, 1280, 720, f % 64 + salt, (f * 7) % 64, 0, (f % 50) as f64 * 0.37 + salt as f64, 55 + f % 10);
        acc += match panel_hit_test(&p, (f * 37) % 300, (f * 53) % 720) {
            UiHit::UhWorld => 1,
            UiHit::UhToolButton { tb_id } => 10 + tb_id,
            UiHit::UhListItem { li_idx } => 100 + li_idx,
            UiHit::UhNone => 1000,
        };
        acc += p.p_status.ss_text.len() as i64 + p.p_list.lb_items.len() as i64 * 3 + p.p_list.lb_selected;
    }
    acc
}

// ── moros_editor: slope_path_with_undo ────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct UndoEntry { ue_q: i64, ue_r: i64, ue_cy: i64, ue_batch_id: i64, ue_prev: Hex }
struct UndoStack { us_entries: Vec<UndoEntry>, us_redo: Vec<UndoEntry>, us_batch_id: i64, us_next_id: i64 }

fn undo_empty() -> UndoStack { UndoStack { us_entries: Vec::new(), us_redo: Vec::new(), us_batch_id: 0, us_next_id: 1 } }

fn batch_begin(s: &mut UndoStack) {
    if s.us_batch_id == 0 {
        s.us_batch_id = s.us_next_id;
        s.us_next_id += 1;
    }
}

fn batch_end(s: &mut UndoStack) { s.us_batch_id = 0; }

fn undo_push(s: &mut UndoStack, m: &Map, q: i64, r: i64, cy: i64) {
    let prev = map_get_hex(m, q, r, cy);
    s.us_entries.push(UndoEntry { ue_q: q, ue_r: r, ue_cy: cy, ue_batch_id: s.us_batch_id, ue_prev: prev });
    s.us_redo.clear();
}

fn set_height_with_undo(s: &mut UndoStack, m: &mut Map, q: i64, r: i64, cy: i64, height: i64) {
    undo_push(s, m, q, r, cy);
    map_set_height(m, q, r, cy, height);
}

#[allow(clippy::too_many_arguments)]
fn slope_path_with_undo(s: &mut UndoStack, m: &mut Map, q1: i64, r1: i64, q2: i64, r2: i64, cy: i64, h_start: i64, h_end: i64) {
    batch_begin(s);
    let dist = hex_distance(q1, r1, q2, r2);
    if dist == 0 {
        set_height_with_undo(s, m, q1, r1, cy, h_start);
        batch_end(s);
        return;
    }
    for i in 0..=dist {
        let q = lerp_int(q1, q2, i, dist);
        let r = lerp_int(r1, r2, i, dist);
        let h = lerp_int(h_start, h_end, i, dist);
        set_height_with_undo(s, m, q, r, cy, h);
    }
    batch_end(s);
}

fn c_slope_path(m: &mut Map, salt: i64) -> i64 {
    let mut st = undo_empty();
    for p in 0..32i64 {
        let sq = 3 + (p % 8) * 7;
        slope_path_with_undo(&mut st, m, sq, 1, sq + 4, 62, 0, 20 + salt, 40 + p);
    }
    let mut acc = st.us_entries.len() as i64 * 1000 + st.us_next_id;
    for (i, e) in st.us_entries.iter().enumerate() {
        acc += e.ue_prev.h_height * (i as i64 % 7 + 1) + e.ue_batch_id * 3 + e.ue_q + e.ue_r * 5 + e.ue_prev.h_wall_n;
    }
    acc += map_get_hex(m, 17, 30, 0).h_height * 100000;
    for e in st.us_entries.iter().rev() {
        map_set_hex(m, e.ue_q, e.ue_r, e.ue_cy, e.ue_prev);
    }
    acc
}

// ── moros_render: emit_to_material + emit_hex_surface ─────────────────────────────────
fn hex_corner_offset(i: i64) -> Vec2 {
    let half_w = HEX_WIDTH / 2.0;
    match i {
        0 => vec2(0.0, 1.0),
        1 => vec2(half_w, 0.5),
        2 => vec2(half_w, -0.5),
        3 => vec2(0.0, -1.0),
        4 => vec2(0.0 - half_w, -0.5),
        _ => vec2(0.0 - half_w, 0.5),
    }
}

fn emit_hex_surface(m: &mut Mesh, q: i64, r: i64, height: i64) -> i64 {
    let centre = hex_to_world(q, r, height);
    let up = vec3(0.0, 1.0, 0.0);
    let base = m.vertices.len() as i64;
    m.vertices.push(vertex(centre, up, vec2(0.5, 0.5)));
    for ci in 0..6 {
        let off = hex_corner_offset(ci);
        let pos = vec3(centre.x + off.x, centre.y, centre.z + off.y);
        let u = (off.x / HEX_WIDTH) + 0.5;
        let v = (off.y / 1.0) * 0.5 + 0.5;
        m.vertices.push(vertex(pos, up, vec2(u, v)));
    }
    for ti in 0..6 {
        let next = (ti + 1) % 6;
        m.triangles.push(Triangle { a: base, b: base + 1 + ti, c: base + 1 + next });
    }
    base
}

fn emit_to_material(meshes: &mut Vec<Mesh>, mat: i64, q: i64, r: i64, height: i64) {
    let mat_name = mat.to_string();
    if let Some(em) = meshes.iter_mut().find(|em| em.name == mat_name) {
        emit_hex_surface(em, q, r, height);
        return;
    }
    meshes.push(mesh(mat_name));
    let nm = meshes.last_mut().unwrap();
    emit_hex_surface(nm, q, r, height);
}

fn c_emit_to_material(map: &Map) -> i64 {
    let mut meshes: Vec<Mesh> = Vec::new();
    for c in &map.m_chunks {
        if c.ck_cx != 0 || c.ck_cz != 0 { continue; }
        for (i, h) in c.ck_hexes.iter().enumerate() {
            let q = c.ck_cx * 32 + i as i64 / 32;
            let r = c.ck_cz * 32 + i as i64 % 32;
            emit_to_material(&mut meshes, h.h_material, q, r, h.h_height);
        }
    }
    let mut acc = meshes.len() as i64 * 1000000;
    for (i, mm) in meshes.iter().enumerate() {
        let lv = &mm.vertices[mm.vertices.len() - 1];
        acc += mm.vertices.len() as i64 * (i as i64 + 1) + mm.triangles.len() as i64 * 3
            + mm.name.parse::<i64>().unwrap_or(0) * 7
            + (lv.pos.x * 16.0).round() as i64 + (lv.pos.y * 16.0).round() as i64 + (lv.uv.x * 64.0).round() as i64
            + mm.triangles[mm.triangles.len() - 1].c;
    }
    acc
}

// ── moros_map: map_to_json / map_from_json, by hand ───────────────────────────────────
fn map_empty() -> Map { Map { m_name: "untitled".to_string(), ..Map::default() } }

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn key(out: &mut String, first: bool, k: &str) {
    if !first { out.push(','); }
    out.push('"');
    out.push_str(k);
    out.push_str("\":");
}

fn kint(out: &mut String, first: bool, k: &str, v: i64) {
    key(out, first, k);
    let _ = write!(out, "{v}");
}

fn kfloat(out: &mut String, first: bool, k: &str, v: f64) {
    key(out, first, k);
    let _ = write!(out, "{v}");
}

fn kstr(out: &mut String, first: bool, k: &str, v: &str) {
    key(out, first, k);
    json_str(out, v);
}

fn karr<T>(out: &mut String, first: bool, k: &str, v: &[T], each: impl Fn(&mut String, &T)) {
    key(out, first, k);
    out.push('[');
    for (i, e) in v.iter().enumerate() {
        if i > 0 { out.push(','); }
        each(out, e);
    }
    out.push(']');
}

fn addr_json(out: &mut String, a: &HexAddress) {
    out.push('{');
    kint(out, true, "ha_q", a.ha_q);
    kint(out, false, "ha_r", a.ha_r);
    kint(out, false, "ha_cy", a.ha_cy);
    out.push('}');
}

fn map_to_json(m: &Map) -> String {
    let mut o = String::with_capacity(1 << 19);
    o.push('{');
    kstr(&mut o, true, "m_name", &m.m_name);
    karr(&mut o, false, "m_chunks", &m.m_chunks, |o, c| {
        o.push('{');
        kint(o, true, "ck_cx", c.ck_cx);
        kint(o, false, "ck_cy", c.ck_cy);
        kint(o, false, "ck_cz", c.ck_cz);
        karr(o, false, "ck_hexes", &c.ck_hexes, |o, h| {
            o.push('{');
            kint(o, true, "h_height", h.h_height);
            kint(o, false, "h_material", h.h_material);
            kint(o, false, "h_item", h.h_item);
            kint(o, false, "h_item_rotation", h.h_item_rotation);
            kint(o, false, "h_wall_n", h.h_wall_n);
            kint(o, false, "h_wall_ne", h.h_wall_ne);
            kint(o, false, "h_wall_se", h.h_wall_se);
            o.push('}');
        });
        o.push('}');
    });
    karr(&mut o, false, "m_material_palette", &m.m_material_palette, |o, d| {
        o.push('{');
        kstr(o, true, "md_name", &d.md_name);
        kstr(o, false, "md_category", &d.md_category);
        kstr(o, false, "md_stair_kind", &d.md_stair_kind);
        kint(o, false, "md_texture", d.md_texture);
        kint(o, false, "md_tint_r", d.md_tint_r);
        kint(o, false, "md_tint_g", d.md_tint_g);
        kint(o, false, "md_tint_b", d.md_tint_b);
        kint(o, false, "md_walkable", d.md_walkable);
        kint(o, false, "md_swimmable", d.md_swimmable);
        kint(o, false, "md_climbable", d.md_climbable);
        kint(o, false, "md_slippery", d.md_slippery);
        kint(o, false, "md_loud", d.md_loud);
        o.push('}');
    });
    karr(&mut o, false, "m_wall_palette", &m.m_wall_palette, |o, d| {
        o.push('{');
        kstr(o, true, "wd_name", &d.wd_name);
        kstr(o, false, "wd_body", &d.wd_body);
        kstr(o, false, "wd_base", &d.wd_base);
        kfloat(o, false, "wd_thickness", d.wd_thickness);
        kint(o, false, "wd_texture", d.wd_texture);
        kint(o, false, "wd_tint_r", d.wd_tint_r);
        kint(o, false, "wd_tint_g", d.wd_tint_g);
        kint(o, false, "wd_tint_b", d.wd_tint_b);
        o.push('}');
    });
    karr(&mut o, false, "m_item_palette", &m.m_item_palette, |o, d| {
        o.push('{');
        kstr(o, true, "id_name", &d.id_name);
        kstr(o, false, "id_kind", &d.id_kind);
        kint(o, false, "id_model", d.id_model);
        kint(o, false, "id_symmetric", d.id_symmetric);
        kfloat(o, false, "id_arc_radius", d.id_arc_radius);
        o.push('}');
    });
    karr(&mut o, false, "m_spawn_points", &m.m_spawn_points, |o, s| {
        o.push('{');
        key(o, true, "sp_hex");
        addr_json(o, &s.sp_hex);
        kstr(o, false, "sp_kind", &s.sp_kind);
        kint(o, false, "sp_creature", s.sp_creature);
        kint(o, false, "sp_npc_id", s.sp_npc_id);
        kint(o, false, "sp_count", s.sp_count);
        kint(o, false, "sp_facing", s.sp_facing);
        kstr(o, false, "sp_condition", &s.sp_condition);
        o.push('}');
    });
    karr(&mut o, false, "m_npc_routines", &m.m_npc_routines, |o, r| {
        o.push('{');
        kint(o, true, "nr_npc_id", r.nr_npc_id);
        kstr(o, false, "nr_name", &r.nr_name);
        kint(o, false, "nr_creature", r.nr_creature);
        karr(o, false, "nr_waypoints", &r.nr_waypoints, |o, w| {
            o.push('{');
            key(o, true, "wp_hex");
            addr_json(o, &w.wp_hex);
            kstr(o, false, "wp_activity", &w.wp_activity);
            kint(o, false, "wp_time_start", w.wp_time_start);
            kint(o, false, "wp_time_end", w.wp_time_end);
            kint(o, false, "wp_facing", w.wp_facing);
            kstr(o, false, "wp_note", &w.wp_note);
            o.push('}');
        });
        o.push('}');
    });
    o.push('}');
    o
}

struct Parser<'a> { b: &'a [u8], p: usize }

type PResult<T> = Result<T, ()>;

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.p < self.b.len() && matches!(self.b[self.p], b' ' | b'\n' | b'\r' | b'\t') { self.p += 1; }
    }
    fn eat(&mut self, c: u8) -> PResult<()> {
        self.ws();
        if self.p < self.b.len() && self.b[self.p] == c { self.p += 1; Ok(()) } else { Err(()) }
    }
    fn peek(&mut self) -> u8 {
        self.ws();
        if self.p < self.b.len() { self.b[self.p] } else { 0 }
    }
    fn string(&mut self) -> PResult<String> {
        self.eat(b'"')?;
        let mut s = String::new();
        loop {
            let start = self.p;
            while self.p < self.b.len() && self.b[self.p] != b'"' && self.b[self.p] != b'\\' { self.p += 1; }
            s.push_str(std::str::from_utf8(&self.b[start..self.p]).map_err(|_| ())?);
            if self.p >= self.b.len() { return Err(()); }
            if self.b[self.p] == b'"' { self.p += 1; return Ok(s); }
            self.p += 1;
            let e = *self.b.get(self.p).ok_or(())?;
            self.p += 1;
            match e {
                b'"' => s.push('"'),
                b'\\' => s.push('\\'),
                b'/' => s.push('/'),
                b'n' => s.push('\n'),
                b't' => s.push('\t'),
                b'r' => s.push('\r'),
                b'b' => s.push('\u{8}'),
                b'f' => s.push('\u{c}'),
                b'u' => {
                    let hex = std::str::from_utf8(self.b.get(self.p..self.p + 4).ok_or(())?).map_err(|_| ())?;
                    let cp = u32::from_str_radix(hex, 16).map_err(|_| ())?;
                    s.push(char::from_u32(cp).ok_or(())?);
                    self.p += 4;
                }
                _ => return Err(()),
            }
        }
    }
    fn number(&mut self) -> PResult<&'a str> {
        self.ws();
        let start = self.p;
        while self.p < self.b.len() && matches!(self.b[self.p], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') { self.p += 1; }
        std::str::from_utf8(&self.b[start..self.p]).map_err(|_| ())
    }
    fn int(&mut self) -> PResult<i64> { self.number()?.parse().map_err(|_| ()) }
    fn float(&mut self) -> PResult<f64> { self.number()?.parse().map_err(|_| ()) }
    fn skip(&mut self) -> PResult<()> {
        match self.peek() {
            b'"' => { self.string()?; }
            b'{' => self.object(|p, _| p.skip())?,
            b'[' => self.array(|p| p.skip())?,
            b't' | b'f' | b'n' => {
                while self.p < self.b.len() && self.b[self.p].is_ascii_alphabetic() { self.p += 1; }
            }
            _ => { self.number()?; }
        }
        Ok(())
    }
    fn object(&mut self, mut field: impl FnMut(&mut Self, &str) -> PResult<()>) -> PResult<()> {
        self.eat(b'{')?;
        if self.peek() == b'}' { self.p += 1; return Ok(()); }
        loop {
            let k = self.string()?;
            self.eat(b':')?;
            field(self, &k)?;
            if self.peek() == b',' { self.p += 1; continue; }
            return self.eat(b'}');
        }
    }
    fn array(&mut self, mut elem: impl FnMut(&mut Self) -> PResult<()>) -> PResult<()> {
        self.eat(b'[')?;
        if self.peek() == b']' { self.p += 1; return Ok(()); }
        loop {
            elem(self)?;
            if self.peek() == b',' { self.p += 1; continue; }
            return self.eat(b']');
        }
    }
    fn vec<T>(&mut self, one: impl Fn(&mut Self) -> PResult<T>) -> PResult<Vec<T>> {
        let mut v = Vec::new();
        self.array(|p| { v.push(one(p)?); Ok(()) })?;
        Ok(v)
    }
    fn addr(&mut self) -> PResult<HexAddress> {
        let mut a = HexAddress::default();
        self.object(|p, k| {
            match k {
                "ha_q" => a.ha_q = p.int()?,
                "ha_r" => a.ha_r = p.int()?,
                "ha_cy" => a.ha_cy = p.int()?,
                _ => p.skip()?,
            }
            Ok(())
        })?;
        Ok(a)
    }
    fn hex(&mut self) -> PResult<Hex> {
        let mut h = Hex::default();
        self.object(|p, k| {
            match k {
                "h_height" => h.h_height = p.int()?,
                "h_material" => h.h_material = p.int()?,
                "h_item" => h.h_item = p.int()?,
                "h_item_rotation" => h.h_item_rotation = p.int()?,
                "h_wall_n" => h.h_wall_n = p.int()?,
                "h_wall_ne" => h.h_wall_ne = p.int()?,
                "h_wall_se" => h.h_wall_se = p.int()?,
                _ => p.skip()?,
            }
            Ok(())
        })?;
        Ok(h)
    }
    fn chunk(&mut self) -> PResult<Chunk> {
        let mut c = Chunk::default();
        self.object(|p, k| {
            match k {
                "ck_cx" => c.ck_cx = p.int()?,
                "ck_cy" => c.ck_cy = p.int()?,
                "ck_cz" => c.ck_cz = p.int()?,
                "ck_hexes" => c.ck_hexes = p.vec(Self::hex)?,
                _ => p.skip()?,
            }
            Ok(())
        })?;
        Ok(c)
    }
    fn material(&mut self) -> PResult<MaterialDef> {
        let mut d = MaterialDef::default();
        self.object(|p, k| {
            match k {
                "md_name" => d.md_name = p.string()?,
                "md_category" => d.md_category = p.string()?,
                "md_stair_kind" => d.md_stair_kind = p.string()?,
                "md_texture" => d.md_texture = p.int()?,
                "md_tint_r" => d.md_tint_r = p.int()?,
                "md_tint_g" => d.md_tint_g = p.int()?,
                "md_tint_b" => d.md_tint_b = p.int()?,
                "md_walkable" => d.md_walkable = p.int()?,
                "md_swimmable" => d.md_swimmable = p.int()?,
                "md_climbable" => d.md_climbable = p.int()?,
                "md_slippery" => d.md_slippery = p.int()?,
                "md_loud" => d.md_loud = p.int()?,
                _ => p.skip()?,
            }
            Ok(())
        })?;
        Ok(d)
    }
    fn wall(&mut self) -> PResult<WallDef> {
        let mut d = WallDef::default();
        self.object(|p, k| {
            match k {
                "wd_name" => d.wd_name = p.string()?,
                "wd_body" => d.wd_body = p.string()?,
                "wd_base" => d.wd_base = p.string()?,
                "wd_thickness" => d.wd_thickness = p.float()?,
                "wd_texture" => d.wd_texture = p.int()?,
                "wd_tint_r" => d.wd_tint_r = p.int()?,
                "wd_tint_g" => d.wd_tint_g = p.int()?,
                "wd_tint_b" => d.wd_tint_b = p.int()?,
                _ => p.skip()?,
            }
            Ok(())
        })?;
        Ok(d)
    }
    fn item(&mut self) -> PResult<ItemDef> {
        let mut d = ItemDef::default();
        self.object(|p, k| {
            match k {
                "id_name" => d.id_name = p.string()?,
                "id_kind" => d.id_kind = p.string()?,
                "id_model" => d.id_model = p.int()?,
                "id_symmetric" => d.id_symmetric = p.int()?,
                "id_arc_radius" => d.id_arc_radius = p.float()?,
                _ => p.skip()?,
            }
            Ok(())
        })?;
        Ok(d)
    }
    fn spawn(&mut self) -> PResult<SpawnPoint> {
        let mut s = SpawnPoint::default();
        self.object(|p, k| {
            match k {
                "sp_hex" => s.sp_hex = p.addr()?,
                "sp_kind" => s.sp_kind = p.string()?,
                "sp_creature" => s.sp_creature = p.int()?,
                "sp_npc_id" => s.sp_npc_id = p.int()?,
                "sp_count" => s.sp_count = p.int()?,
                "sp_facing" => s.sp_facing = p.int()?,
                "sp_condition" => s.sp_condition = p.string()?,
                _ => p.skip()?,
            }
            Ok(())
        })?;
        Ok(s)
    }
    fn waypoint(&mut self) -> PResult<NpcWaypoint> {
        let mut w = NpcWaypoint::default();
        self.object(|p, k| {
            match k {
                "wp_hex" => w.wp_hex = p.addr()?,
                "wp_activity" => w.wp_activity = p.string()?,
                "wp_time_start" => w.wp_time_start = p.int()?,
                "wp_time_end" => w.wp_time_end = p.int()?,
                "wp_facing" => w.wp_facing = p.int()?,
                "wp_note" => w.wp_note = p.string()?,
                _ => p.skip()?,
            }
            Ok(())
        })?;
        Ok(w)
    }
    fn routine(&mut self) -> PResult<NpcRoutine> {
        let mut r = NpcRoutine::default();
        self.object(|p, k| {
            match k {
                "nr_npc_id" => r.nr_npc_id = p.int()?,
                "nr_name" => r.nr_name = p.string()?,
                "nr_creature" => r.nr_creature = p.int()?,
                "nr_waypoints" => r.nr_waypoints = p.vec(Self::waypoint)?,
                _ => p.skip()?,
            }
            Ok(())
        })?;
        Ok(r)
    }
    fn map(&mut self) -> PResult<Map> {
        let mut m = Map::default();
        self.object(|p, k| {
            match k {
                "m_name" => m.m_name = p.string()?,
                "m_chunks" => m.m_chunks = p.vec(Self::chunk)?,
                "m_material_palette" => m.m_material_palette = p.vec(Self::material)?,
                "m_wall_palette" => m.m_wall_palette = p.vec(Self::wall)?,
                "m_item_palette" => m.m_item_palette = p.vec(Self::item)?,
                "m_spawn_points" => m.m_spawn_points = p.vec(Self::spawn)?,
                "m_npc_routines" => m.m_npc_routines = p.vec(Self::routine)?,
                _ => p.skip()?,
            }
            Ok(())
        })?;
        Ok(m)
    }
}

fn map_from_json(json: &str) -> Map {
    if json.is_empty() { return map_empty(); }
    Parser { b: json.as_bytes(), p: 0 }.map().unwrap_or_else(|_| map_empty())
}

fn c_map_json(m: &Map) -> i64 {
    let js = map_to_json(m);
    let m2 = map_from_json(&js);
    let mut acc = js.len() as i64 * 1000;
    for c in &m2.m_chunks {
        for h in &c.ck_hexes {
            acc += h.h_height + h.h_material * 3 + h.h_item * 5 + h.h_item_rotation * 7 + h.h_wall_n * 11
                + h.h_wall_ne * 13 + h.h_wall_se * 17 + c.ck_cx * 19 + c.ck_cz * 23;
        }
    }
    for d in &m2.m_material_palette { acc += (d.md_name.len() + d.md_category.len()) as i64 + d.md_tint_g + d.md_slippery; }
    for d in &m2.m_wall_palette { acc += (d.wd_thickness * 100.0).round() as i64 + d.wd_tint_g; }
    for d in &m2.m_item_palette { acc += (d.id_arc_radius * 10.0).round() as i64 + d.id_model; }
    for s in &m2.m_spawn_points { acc += s.sp_hex.ha_q * 3 + s.sp_hex.ha_r + s.sp_npc_id + s.sp_condition.len() as i64; }
    for r in &m2.m_npc_routines {
        acc += r.nr_npc_id + r.nr_name.len() as i64;
        for w in &r.nr_waypoints { acc += w.wp_hex.ha_q + w.wp_time_end + w.wp_activity.len() as i64; }
    }
    acc + m2.m_name.len() as i64
}

// ── dryopea: painted / marker worlds ──────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct PaintedHex { q: i64, r: i64, kind: u8 }
struct PaintedWorld { painted: HashMap<(i64, i64), PaintedHex> }

fn paint(w: &mut PaintedWorld, q: i64, r: i64, kind: u8) {
    if kind == 0 { w.painted.remove(&(q, r)); } else { w.painted.insert((q, r), PaintedHex { q, r, kind }); }
}

fn lookup_painted(w: &PaintedWorld, q: i64, r: i64) -> u8 { w.painted.get(&(q, r)).map_or(0, |e| e.kind) }

const MARKER_KIND_SPAWN: i64 = 0;
const MARKER_KIND_TARGET: i64 = 1;

#[derive(Clone, Copy)]
struct MarkerEntry { q: i64, r: i64, kind: u8, direction: u8 }
struct MarkerWorld { markers: HashMap<(i64, i64), MarkerEntry> }

fn place_spawn(w: &mut MarkerWorld, q: i64, r: i64, direction: u8) {
    w.markers.insert((q, r), MarkerEntry { q, r, kind: MARKER_KIND_SPAWN as u8, direction });
}

fn place_target(w: &mut MarkerWorld, q: i64, r: i64) {
    w.markers.insert((q, r), MarkerEntry { q, r, kind: MARKER_KIND_TARGET as u8, direction: 0 });
}

fn has_marker(w: &MarkerWorld, q: i64, r: i64) -> bool { w.markers.contains_key(&(q, r)) }
fn marker_kind(w: &MarkerWorld, q: i64, r: i64) -> i64 { w.markers.get(&(q, r)).map_or(-1, |e| e.kind as i64) }
fn marker_direction(w: &MarkerWorld, q: i64, r: i64) -> u8 { w.markers.get(&(q, r)).map_or(0, |e| e.direction) }

// ── dryopea history: the timeline ─────────────────────────────────────────────────────
struct PaintedDelta { q: i64, r: i64, old_kind: i64, new_kind: i64 }
struct MarkerDelta {
    q: i64, r: i64, old_present: bool, old_kind: i64, old_direction: i64, new_present: bool, new_kind: i64, new_direction: i64,
}
struct Stroke { painted: Vec<PaintedDelta>, markers: Vec<MarkerDelta> }
struct Timeline { entries: Vec<Stroke>, cursor: i64 }

const HISTORY_MAX_DEPTH: i64 = 50;

fn undo_entry_empty() -> Stroke { Stroke { painted: Vec::new(), markers: Vec::new() } }
fn history_empty() -> Timeline { Timeline { entries: Vec::new(), cursor: 0 } }

fn truncate_to(h: &mut Timeline, n: i64) {
    if n >= h.entries.len() as i64 { return; }
    h.entries.truncate(n as usize);
}

fn drop_oldest(h: &mut Timeline, n: i64) {
    if n <= 0 || n > h.entries.len() as i64 { return; }
    h.entries.drain(..n as usize);
    h.cursor = (h.cursor - n).max(0);
}

fn history_push(h: &mut Timeline, e: Stroke) {
    if e.painted.is_empty() && e.markers.is_empty() { return; }
    truncate_to(h, h.cursor);
    h.entries.push(e);
    h.cursor += 1;
    if h.entries.len() as i64 > HISTORY_MAX_DEPTH {
        let n = h.entries.len() as i64 - HISTORY_MAX_DEPTH;
        drop_oldest(h, n);
    }
}

fn reload_and_record(pw_cur: &PaintedWorld, mw_cur: &MarkerWorld, pw_ld: &PaintedWorld, mw_ld: &MarkerWorld, history: &mut Timeline) {
    let mut e = undo_entry_empty();
    for cp in pw_cur.painted.values() {
        let lk = lookup_painted(pw_ld, cp.q, cp.r);
        if lk != cp.kind { e.painted.push(PaintedDelta { q: cp.q, r: cp.r, old_kind: cp.kind as i64, new_kind: lk as i64 }); }
    }
    for lp in pw_ld.painted.values() {
        if lookup_painted(pw_cur, lp.q, lp.r) == 0 {
            e.painted.push(PaintedDelta { q: lp.q, r: lp.r, old_kind: 0, new_kind: lp.kind as i64 });
        }
    }
    for cm in mw_cur.markers.values() {
        if has_marker(mw_ld, cm.q, cm.r) {
            let lk = marker_kind(mw_ld, cm.q, cm.r);
            let ld = marker_direction(mw_ld, cm.q, cm.r);
            if lk != cm.kind as i64 || ld != cm.direction {
                e.markers.push(MarkerDelta { q: cm.q, r: cm.r, old_present: true, old_kind: cm.kind as i64,
                    old_direction: cm.direction as i64, new_present: true, new_kind: lk, new_direction: ld as i64 });
            }
        } else {
            e.markers.push(MarkerDelta { q: cm.q, r: cm.r, old_present: true, old_kind: cm.kind as i64,
                old_direction: cm.direction as i64, new_present: false, new_kind: 0, new_direction: 0 });
        }
    }
    for lm in mw_ld.markers.values() {
        if !has_marker(mw_cur, lm.q, lm.r) {
            e.markers.push(MarkerDelta { q: lm.q, r: lm.r, old_present: false, old_kind: 0, old_direction: 0,
                new_present: true, new_kind: lm.kind as i64, new_direction: lm.direction as i64 });
        }
    }
    history_push(history, e);
}

fn make_painted(lo: i64, twist: i64) -> PaintedWorld {
    let mut w = PaintedWorld { painted: HashMap::new() };
    for i in lo..lo + 5000 { paint(&mut w, i % 100, i / 100, ((1 + (i * 7 + (i / 3) * twist) % 10) & 15) as u8); }
    w
}

fn make_markers(lo: i64, twist: i64) -> MarkerWorld {
    let mut w = MarkerWorld { markers: HashMap::new() };
    for i in lo..lo + 300 {
        if (i + twist) % 4 == 0 { place_target(&mut w, (i * 13) % 100, (i * 7) % 50); }
        else { place_spawn(&mut w, (i * 13) % 100, (i * 7) % 50, (((i + twist) % 6) & 7) as u8); }
    }
    w
}

fn delta_sum(h: &Timeline) -> i64 {
    let mut acc = h.entries.len() as i64 * 1000000 + h.cursor * 100000;
    for e in &h.entries {
        for d in &e.painted { acc += d.q * 7 + d.r * 13 + d.old_kind * 101 + d.new_kind * 1009; }
        for d in &e.markers {
            acc += d.q * 3 + d.r * 5 + d.old_kind * 17 + d.old_direction * 19 + d.new_kind * 23 + d.new_direction * 29;
            if d.old_present { acc += 31; }
            if d.new_present { acc += 37; }
        }
    }
    acc
}

fn c_reload_and_record(pa: &PaintedWorld, ma: &MarkerWorld, pb: &PaintedWorld, mb: &MarkerWorld) -> i64 {
    let mut hist = history_empty();
    reload_and_record(pa, ma, pb, mb, &mut hist);
    delta_sum(&hist)
}

// ── dryopea save: mapfile_to_painted ──────────────────────────────────────────────────
struct GroundEntry { q: i64, r: i64, kind: String }
#[allow(dead_code)]
struct MapFile { version: i64, name: String, cam_q: i64, cam_r: i64, cam_zoom: i64, ground: Vec<GroundEntry> }
#[allow(dead_code)]
struct GroundType {
    name: String, color: String, sub_palette: String, slope: i64, drop: i64, drainage: bool, walk_ground: bool,
    walk_vehicle: bool, buildable: bool, extrusion_kind: String, height_override: f64,
}

fn palette_index_of(palette: &[GroundType], name: &str) -> i64 {
    palette.iter().position(|g| g.name == name).map_or(-1, |i| i as i64)
}

fn mapfile_to_painted(m: &MapFile, palette: &[GroundType]) -> PaintedWorld {
    let mut out = PaintedWorld { painted: HashMap::new() };
    for e in &m.ground {
        let k = palette_index_of(palette, &e.kind);
        if k > 0 { paint(&mut out, e.q, e.r, k as u8); }
    }
    out
}

const GROUND_NAMES: [&str; 16] = ["sea", "water", "rapids", "waterfall", "sand", "grass", "hill", "rock",
                                  "steep_rock", "wall", "wall_high", "lava", "ice", "mud", "snow", "road"];

fn make_palette() -> Vec<GroundType> {
    GROUND_NAMES
        .iter()
        .enumerate()
        .map(|(gi, gn)| GroundType {
            name: gn.to_string(), color: "#0a2c5e".to_string(),
            sub_palette: if gi < 4 { "water" } else { "land" }.to_string(), slope: gi as i64 % 3, drop: 0,
            drainage: gi < 4, walk_ground: gi >= 4, walk_vehicle: gi != 9, buildable: gi == 5,
            extrusion_kind: "flat".to_string(), height_override: 0.0,
        })
        .collect()
}

fn make_mapfile(pal: &[GroundType], salt: i64) -> MapFile {
    let ground = (0..5000i64)
        .map(|i| {
            let j = if salt == 0 { i } else { 4999 - i };
            GroundEntry { q: i % 100 - 50, r: i / 100 - 25, kind: pal[((j * 7 + j / 13) % 16) as usize].name.clone() }
        })
        .collect();
    MapFile { version: 1, name: "exit".to_string(), cam_q: 0, cam_r: 0, cam_zoom: 1, ground }
}

fn c_mapfile_to_painted(mf: &MapFile, pal: &[GroundType]) -> i64 {
    let pw = mapfile_to_painted(mf, pal);
    let mut acc = pw.painted.len() as i64 * 1000000;
    for ph in pw.painted.values() { acc += (ph.q + 50) * 31 + (ph.r + 25) * 17 + ph.kind as i64 * (ph.q + 101); }
    acc
}

// ── dryopea history: truncate_to / drop_oldest through history_push ───────────────────
fn c_truncate_to(salt: i64) -> i64 {
    let mut tl = history_empty();
    for k in 0..300i64 {
        if k % 10 == 9 && tl.cursor >= 3 { tl.cursor -= 3; }
        let mut e = undo_entry_empty();
        for j in 0..3i64 {
            e.painted.push(PaintedDelta { q: k + j, r: salt + j * 2, old_kind: (k + j) % 11, new_kind: (k * 3 + j) % 11 });
        }
        e.markers.push(MarkerDelta { q: k, r: salt, old_present: k % 2 == 0, old_kind: k % 2, old_direction: k % 6,
                                     new_present: true, new_kind: (k + 1) % 2, new_direction: (k + salt) % 6 });
        history_push(&mut tl, e);
    }
    let mut acc = tl.entries.len() as i64 * 1000000 + tl.cursor * 10000;
    for (i, te) in tl.entries.iter().enumerate() {
        let w = i as i64 + 1;
        for pd in &te.painted { acc += w * (pd.q * 3 + pd.r + pd.old_kind * 7 + pd.new_kind * 11); }
        for md in &te.markers { acc += w * (md.q + md.new_direction * 5 + md.old_kind * 13); }
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
    let maps = [make_map(0), make_map(1)];
    let mut smap = make_map(0);
    let (pa, pb) = (make_painted(0, 0), make_painted(1000, 1));
    let (ma, mb) = (make_markers(0, 0), make_markers(100, 1));
    let pal = make_palette();
    let mfs = [make_mapfile(&pal, 0), make_mapfile(&pal, 1)];
    if std::env::var("BENCH17_DUMP_JSON").is_ok() {
        print!("{}", map_to_json(&maps[0]));
        return;
    }
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let mut sink: i64 = 0;

    let (us, s) = timed(n, |r| c_resolve_move(black_box(&maps[0]), r & 1));
    sink = sink.wrapping_add(s);
    row("resolve_move", n, us, 10000, c_resolve_move(&maps[0], 0));
    let (us, s) = timed(n, |r| c_panel_build(r & 1));
    sink = sink.wrapping_add(s);
    row("panel_build", n, us, 2000, c_panel_build(0));
    let (us, s) = timed(n, |r| c_slope_path(black_box(&mut smap), r & 1));
    sink = sink.wrapping_add(s);
    row("slope_path_with_undo", n, us, 2112, c_slope_path(&mut smap, 0));
    let (us, s) = timed(n, |r| c_emit_to_material(black_box(&maps[(r & 1) as usize])));
    sink = sink.wrapping_add(s);
    row("emit_to_material", n, us, 1024, c_emit_to_material(&maps[0]));
    let (us, s) = timed(n, |r| c_map_json(black_box(&maps[(r & 1) as usize])));
    sink = sink.wrapping_add(s);
    row("map_json", n, us, 4096, c_map_json(&maps[0]));
    let (us, s) = timed(n, |r| {
        if r & 1 == 0 { c_reload_and_record(black_box(&pa), &ma, &pb, &mb) } else { c_reload_and_record(black_box(&pb), &mb, &pa, &ma) }
    });
    sink = sink.wrapping_add(s);
    row("reload_and_record", n, us, 10000, c_reload_and_record(&pa, &ma, &pb, &mb));
    let (us, s) = timed(n, |r| c_mapfile_to_painted(black_box(&mfs[(r & 1) as usize]), &pal));
    sink = sink.wrapping_add(s);
    row("mapfile_to_painted", n, us, 5000, c_mapfile_to_painted(&mfs[0], &pal));
    let (us, s) = timed(n, |r| c_truncate_to(r & 1));
    sink = sink.wrapping_add(s);
    row("truncate_to", n, us, 300, c_truncate_to(0));

    println!("time: 0ms sink={}", black_box(sink));
}
