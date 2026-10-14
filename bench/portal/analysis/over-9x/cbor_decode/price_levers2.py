#!/usr/bin/env python3
"""Levers beyond D7, cumulative on v_d.rs (the D1–D7 form):
   e = d + D4' every byte read of `bytes` goes through the function-clause header's BASE
   f = e + a PUSH window for `items += [sub.value]` across the recursive call (the vector's
       record, length and capacity held in locals; a growth step is the runtime append and a
       fresh header) — in the three decoders' array arms
   g = f + the return record written through ONE resolved store (major 0/1 arms of dp) and
       no call-line frame in dp (ceiling for the per-call overhead)
usage: price_levers2.py v_d.rs e|f|g out.rs"""
import re, sys
src = open(sys.argv[1]).read(); level = sys.argv[2]
HELPER = r'''
#[inline(always)]
fn pw_open(stores: &Stores, v: DbRef, size: u32) -> (u32, u32, u32) { // the vector's record, length, capacity
  if v.store_nr == u16::MAX || v.rec == 0 { return (0, 0, 0); }
  let st = stores.store(&v);
  let vec_rec = st.collection_rec(v.rec, v.pos);
  if vec_rec == 0 { return (0, 0, 0); }
  let words = st.read::<i32>(vec_rec, 0) as u32;
  let len = st.get_u32_raw(vec_rec, 4);
  (vec_rec, len, (words * 8 - 8) / size)
}
'''
# e — D4': base reads
n_e = 0
hdr = 'let __vh_1 = vector::vec_header(&(var_bytes), &stores.allocations); //@FR-R-Header function clause\n'
assert src.count(hdr) == 3
src = src.replace(hdr, hdr + '  let __vb_f: *const u8 = vector::vec_base(&__vh_1, &stores.allocations); // D4: the function clause carries its base\n')
pat = re.compile(r'let _pre_(\d+) = vector::get_vector_hoisted::<false>\(&__vh_1, &\(var_bytes\), \(1_i64\) as u32, (.*?), &stores\.allocations\);\n(\s*)let mut (var___\w+): i64 = \{\{let db = \(_pre_\1\); if db\.rec == 0 \{ i64::MIN \} else \{ i64::from\(stores\.store\(&db\)\.get_byte\(db\.rec, db\.pos \+ \(0_i64\) as u32, \(\(0_i32\)\)\)\) \}\}\}')
def e_repl(m):
    return (f'let mut {m.group(4)}: i64 = {{ let __bi: i64 = {m.group(2)}; let __bl = i64::from(__vh_1.len); let __bf = if __bi < 0 {{ __bi + __bl }} else {{ __bi }}; '
            f'if __bf >= 0 && __bf < __bl {{ i64::from(unsafe {{ __vb_f.add(__bf as usize).read() }}) }} else {{ i64::MIN }} }}')
src, n_e = pat.subn(e_repl, src)
assert n_e == 36, n_e
n_f = n_g1 = n_g2 = 0
if level in ('f', 'g'):
    # f — the push window around the dp array arm (3 arms: plain, ranged, dp)
    arm = re.compile(r"(\n\s*)var__elm_3 = stores\.record_new\(&var_items, 104_u16, 65535_u16\);\n"
                     r"(\s*)let \(__nx, __okc\) = n_read_value_dp\(cell, var_bytes, var_p, (.*?), var__elm_3\);\n"
                     r"\s*if __okc == 1 \{ OpFinishRecord\(cell, var_items, var__elm_3, 104_i32, 65535_i32\); var_p = __nx; \} else \{ var_aok = \(false\) as u8; \}\n")
    def f_repl(m):
        i1, i2, depth = m.group(1), m.group(2), m.group(3)
        return (f"{i1}if __pw_len >= __pw_cap {{ var__elm_3 = vector::vector_append(&var_items, 16, &mut stores.allocations); let __t = pw_open(stores, var_items, 16); __pw_rec = __t.0; __pw_len = __t.1; __pw_cap = __t.2; }}"
                f" else {{ var__elm_3 = DbRef {{ store_nr: var_items.store_nr, rec: __pw_rec, pos: 8 + __pw_len * 16 }}; }}\n"
                f"{i2}let (__nx, __okc) = n_read_value_dp(cell, var_bytes, var_p, {depth}, var__elm_3);\n"
                f"{i2}if __okc == 1 {{ __pw_len += 1; stores.store_mut(&var_items).set_u32_raw(__pw_rec, 4, __pw_len); var_p = __nx; }} else {{ var_aok = (false) as u8; }}\n")
    src, n_f = arm.subn(f_repl, src)
    assert n_f == 3, n_f
    # open the window where the loop starts: before `let mut var_aok: u8 = (true) as u8;` in those arms
    k = src.count('    let mut var_aok: u8 = (true) as u8;\n')
    assert k == 3, k
    src = src.replace('    let mut var_aok: u8 = (true) as u8;\n',
                      '    let mut var_aok: u8 = (true) as u8;\n    let (mut __pw_rec, mut __pw_len, mut __pw_cap) = pw_open(stores, var_items, 16);\n')
if level == 'g':
    m = re.search(r'\nfn n_read_value_dp\(cell.*?\n \}  \} /\*block_1: ref\(Decoded\)\*/\n', src, re.S)
    dp = m.group(0); dp0 = dp
    dp = dp.replace('  cr_call_push_lean("/Users/Jurjen/workspace/loft-bench-libs/loft-libs-core/cbor/src/cbor.loft", 207);\n  let _call_guard = codegen_runtime::CallGuard;\n', '', 1)
    assert dp != dp0
    three = re.compile(r'\{\{ let _v_val = \(0_i64\); \{let db = \(var___retbuf\); let v = if _v_val == i64::MIN \{ i32::MIN \} else \{ _v_val as i32 \}; if db\.rec != 0 \{ stores\.store_mut\(&db\)\.set_i32_raw\(db\.rec, db\.pos \+ \(0_i64\) as u32, v\); \}\} \}\};\n'
                       r'(\s*)\{\{let db = \(\{\{ let _v_v1 = \(var___retbuf\); DbRef \{store_nr: _v_v1\.store_nr, rec: _v_v1\.rec, pos: _v_v1\.pos \+ \(0_i64\) as u32\} \}\}\); let v = (.*?); if db\.rec != 0 \{ stores\.store_mut\(&db\)\.set_int\(db\.rec, db\.pos \+ \(8_i64\) as u32, v\); \}\}\};\n'
                       r'\s*\{\{let db = \(\{\{ let _v_v1 = \(var___retbuf\); DbRef \{store_nr: _v_v1\.store_nr, rec: _v_v1\.rec, pos: _v_v1\.pos \+ \(0_i64\) as u32\} \}\}\); let v = \(\(3_u8\) as u8\); if db\.rec != 0 \{ stores\.store_mut\(&db\)\.set_byte\(db\.rec, db\.pos \+ \(0_i64\) as u32, 0, i32::from\(v\)\); \}\}\};\n')
    def g_repl(m):
        return (f"{{ let db = var___retbuf; if db.rec != 0 {{ let __v: i64 = {m.group(2)}; let st = stores.store_mut(&db); st.set_i32_raw(db.rec, db.pos, 0); st.set_int(db.rec, db.pos + 8, __v); st.set_byte(db.rec, db.pos, 0, 3); }} }}\n")
    dp, n_g1 = three.subn(g_repl, dp)
    assert n_g1 == 2, n_g1
    src = src.replace(dp0, dp, 1)
src = src.replace('\nfn n_read_value(cell', HELPER + '\nfn n_read_value(cell', 1)
open(sys.argv[3], 'w').write(src)
print(f"level {level}: base reads {n_e}, push arms {n_f}, retbuf-once arms {n_g1}")
