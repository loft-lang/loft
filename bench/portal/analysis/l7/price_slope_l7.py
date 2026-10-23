#!/usr/bin/env python3
"""The L7 hand prices on lane 17's `slope_path_with_undo` (worklist.md § The two rows over
4×), applied to the emitted Rust of `bench/17_consumer_moros_dryopea/bench.loft`.  Every edit
asserts the text it replaces, so a stale emission fails loudly instead of pricing nothing.

    L7     `m.m_chunks`' header and base taken once in `slope_path_with_undo` and handed down
           (`_l7` twins of set_height_with_undo, undo_push, map_set_height, map_get_hex,
           map_set_hex, map_ensure_chunk); map_ensure_chunk re-derives them after its append
    N      the matched chunk's `ck_hexes` record and length read through the held chunk
           address (`__pa_2`), not resolved again by length_vector / get_vector / rec_and_len
    TABLE  L7 with every chunk's `ck_hexes` (record, length) in the caller's held pair, up
           to eight chunks (a ceiling, not a rule); excludes N

    L=target/install-lib/release
    $L/loft --native-emit /tmp/b.rs --lean --path $PWD/ bench/17_consumer_moros_dryopea/bench.loft
    price_slope_l7.py /tmp/b.rs /tmp/b_l7n.rs L7 N
    rustc -C opt-level=3 -C codegen-units=1 -C llvm-args=-align-loops=64 --edition=2024 \\
        --extern loft=$L/libloft.rlib -L $L/deps -o /tmp/b_l7n /tmp/b_l7n.rs   # and /tmp/b.rs
    taskset -c 2 /tmp/b --n 1000 | grep slope; taskset -c 2 /tmp/b_l7n --n 1000 | grep slope
"""
import re
import sys

TUPLE = "(vector::VecHeader, *const u8)"


def func_span(lines, name):
    st = next(i for i, l in enumerate(lines) if l.startswith("fn %s(" % name))
    en = next(i for i in range(st, len(lines)) if lines[i].startswith("  } /*block_1"))
    return st, en


def func(src, name):
    lines = src.split("\n")
    st, en = func_span(lines, name)
    return "\n".join(lines[st:en + 1])


def one(body, old, new):
    assert body.count(old) == 1, old[:60]
    return body.replace(old, new)


def sub1(pat, rep, body):
    body, k = re.subn(pat, rep, body, count=1)
    assert k == 1, pat[:60]
    return body


def drop_text_span(body):
    return re.sub(r"\n *let __ts_\d+: \(\*const u8, u32\) = vector::text_span_of\([^\n]*", "", body, count=1)


def hold(body):
    """A walk's header and base from the caller's held pair."""
    m = re.search(r"      let __vh_(\d+) = vector::vec_header\(&\(var__vector_1\), &stores\.allocations\);[^\n]*", body)
    assert m
    n = m.group(1)
    body = body.replace(m.group(0), "      let __vh_%s = l7h.0;" % n, 1)
    body = sub1(r"let __vb_(\d+): \*const u8 = vector::vec_base\(&__vh_%s, &stores\.allocations\);" % n,
                lambda m: "let __vb_%s: *const u8 = l7h.1;" % m.group(1), body)
    return drop_text_span(body)


def hold_ensure(body):
    """map_ensure_chunk's header sits outside its inner block; its append re-derives."""
    body = sub1(r"    let __vh_1 = vector::vec_header\(&\(var__vector_1\), &stores\.allocations\);[^\n]*",
                "    let __vh_1 = l7h.0;", body)
    body = sub1(r"let __vb_2: \*const u8 = vector::vec_base\(&__vh_1, &stores\.allocations\);",
                "let __vb_2: *const u8 = l7h.1;", body)
    body = drop_text_span(body)
    return sub1(r"(  stores\.field_vector_finish\(&\(\*var_m\), 4\);)", r"\1\n  *l7h = l7_hdr(&*var_m, stores);", body)


def twin(body, name, mref, inline):
    body = sub1(r"^fn %s\(cell: &std::cell::UnsafeCell<Stores>, " % name,
                "fn %s_l7(cell: &std::cell::UnsafeCell<Stores>, l7h: &%s%s, " % (name, mref, TUPLE), body)
    return ("#[inline]\n" if inline else "") + body


L7_HDR = '''
#[inline]
fn l7_hdr(m: &DbRef, stores: &Stores) -> (vector::VecHeader, *const u8) {
  let d = DbRef { store_nr: m.store_nr, rec: m.rec, pos: m.pos + 4 };
  let h = vector::vec_header(&d, &stores.allocations);
  let b = vector::vec_base(&h, &stores.allocations);
  (h, b)
}'''


def apply_l7(src):
    g = twin(hold(func(src, "n_map_get_hex")), "n_map_get_hex", "", False)
    e = twin(hold_ensure(func(src, "n_map_ensure_chunk")), "n_map_ensure_chunk", "mut ", False)
    s = twin(hold(func(src, "n_map_set_hex")), "n_map_set_hex", "mut ", False)
    s = one(s, "n_map_ensure_chunk(cell, var_m,", "n_map_ensure_chunk_l7(cell, l7h, var_m,")
    h = twin(func(src, "n_map_set_height"), "n_map_set_height", "mut ", True)
    h = one(h, "n_map_get_hex(cell,", "n_map_get_hex_l7(cell, l7h,")
    h = one(h, "n_map_set_hex(cell,", "n_map_set_hex_l7(cell, l7h,")
    u = twin(func(src, "n_undo_push"), "n_undo_push", "", True)
    u = one(u, "n_map_get_hex(cell,", "n_map_get_hex_l7(cell, l7h,")
    w = twin(func(src, "n_set_height_with_undo"), "n_set_height_with_undo", "mut ", True)
    w = one(w, "n_undo_push(cell,", "n_undo_push_l7(cell, l7h,")
    w = one(w, "n_map_set_height(cell,", "n_map_set_height_l7(cell, l7h,")
    sp = func(src, "n_slope_path_with_undo")
    new_sp = one(sp, "  { //For block_3: void\n", "  let mut l7h = l7_hdr(&*var_m, stores);\n  { //For block_3: void\n")
    new_sp = one(new_sp, "n_set_height_with_undo(cell, var_s, var_m, var_q, var_r, var_cy, var_h);",
                 "n_set_height_with_undo_l7(cell, &mut l7h, var_s, var_m, var_q, var_r, var_cy, var_h);")
    src = one(src, sp, new_sp)
    return src + "\n" + "\n\n".join([L7_HDR, g, e, s, h, u, w]) + "\n"


R_LEN = ("vector::length_vector(&({{ let _v_v1 = (var_gh_c); DbRef {store_nr: _v_v1.store_nr, rec: _v_v1.rec, "
         "pos: _v_v1.pos + (24_i64) as u32} }}), &stores.allocations)")
R_GET = ("vector::get_vector(&({{ let _v_v1 = (var_gh_c); DbRef {store_nr: _v_v1.store_nr, rec: _v_v1.rec, "
         "pos: _v_v1.pos + (24_i64) as u32} }}), (56_i64) as u32, (var_idx), &stores.allocations)")
W_RL = "let (__er, __el) = vector::rec_and_len(&__vr, &stores.allocations);"

NESTED = '''
#[inline(always)]
fn l7_nested(pa: *const u8, nr: u16, stores: &Stores) -> (u32, u32) {
  let r = unsafe { pa.add(24).cast::<u32>().read_unaligned() };
  if r == 0 || r == DbRef::ABSENT_REC { (0, 0) } else { (r, stores.allocations[nr as usize].get_u32_raw(r, 4)) }
}
'''


def nested_edits(get_len, get_rec, set_pair):
    return [("get", R_LEN, get_len), ("get", R_GET, "DbRef { store_nr: var_gh_c.store_nr, rec: %s, pos: 8 + var_idx as u32 * 56 }" % get_rec),
            ("set", W_RL, "let (__er, __el) = %s;" % set_pair)]


def edit_funcs(src, names, edits):
    lines = src.split("\n")
    for key, name in names.items():
        st, en = func_span(lines, name)
        body = "\n".join(lines[st:en + 1])
        for k, old, new in edits:
            if k == key:
                body = one(body, old, new)
        lines[st:en + 1] = body.split("\n")
    return "\n".join(lines)


def apply_nested(src, l7):
    names = {"get": "n_map_get_hex_l7", "set": "n_map_set_hex_l7"} if l7 else {"get": "n_map_get_hex", "set": "n_map_set_hex"}
    edits = nested_edits("l7_nested(__pa_2, var_gh_c.store_nr, stores).1", "l7_nested(__pa_2, var_gh_c.store_nr, stores).0",
                         "l7_nested(__pa_2, __vr.store_nr, stores)")
    return edit_funcs(src, names, edits) + NESTED


TABLE_HDR = '''
#[inline]
fn l7_hdr(m: &DbRef, stores: &Stores) -> L7H {
  let d = DbRef { store_nr: m.store_nr, rec: m.rec, pos: m.pos + 4 };
  let h = vector::vec_header(&d, &stores.allocations);
  let b = vector::vec_base(&h, &stores.allocations);
  let mut t = [(0u32, 0u32); 8];
  for i in 0..(h.len as usize).min(8) {
    let c = DbRef { store_nr: h.store_nr, rec: h.rec, pos: 8 + i as u32 * 32 + 24 };
    t[i] = vector::rec_and_len(&c, &stores.allocations);
  }
  (h, b, t)
}
type L7H = (vector::VecHeader, *const u8, [(u32, u32); 8]);'''


def apply_table(src):
    src = one(src, L7_HDR, TABLE_HDR)
    src = src.replace("&mut " + TUPLE, "&mut L7H").replace("&" + TUPLE, "&L7H")
    edits = nested_edits("l7h.2[var_gh_c__index as usize].1", "l7h.2[var_gh_c__index as usize].0",
                         "l7h.2[var_sh_c__index as usize]")
    return edit_funcs(src, {"get": "n_map_get_hex_l7", "set": "n_map_set_hex_l7"}, edits)


def main():
    if len(sys.argv) < 3 or sys.argv[1] in ("-h", "--help"):
        print(__doc__ or "usage: price_slope_l7.py <emitted.rs> <out.rs> [L7] [N | TABLE]")
        sys.exit(0 if sys.argv[1:2] in (["-h"], ["--help"]) else 2)
    src = open(sys.argv[1], encoding="utf-8").read()
    levers = set(sys.argv[3:])
    assert levers <= {"L7", "N", "TABLE"} and not ({"N", "TABLE"} <= levers), levers
    if "L7" in levers or "TABLE" in levers:
        src = apply_l7(src)
    if "N" in levers:
        src = apply_nested(src, "L7" in levers)
    if "TABLE" in levers:
        src = apply_table(src)
    open(sys.argv[2], "w", encoding="utf-8").write(src)


if __name__ == "__main__":
    main()
