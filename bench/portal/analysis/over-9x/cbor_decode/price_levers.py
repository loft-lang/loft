#!/usr/bin/env python3
"""Price cbor/decode's levers on the emitted base.rs, cumulatively:
   a = D2 (16-byte same-store move inline) + D3 (NULL-inline exit test) + D5 (no 1-element reserve)
   b = a + D6 (an appended moved element skips the prefill)
   d = b + D7 (the child is built in its element: n_read_value_dp writes `value` at the
       caller's element and answers (next, ok) as a tuple; the array arms of both the plain
       and the ranged decoder call it)
Only n_read_value / n_read_value__rg and the helper are touched; the hash must stay 1aec9537.
usage: price_levers.py base.rs a|b|d out.rs"""
import re, sys
src = open(sys.argv[1]).read(); level = sys.argv[2]
HELPER = r'''
// ---- hand-priced levers (cbor decode) ----
#[inline(always)]
fn mv16(stores: &mut Stores, s: DbRef, d: DbRef) { // D2: a 16-byte same-store field move is one copy and one zero
  if s.store_nr == d.store_nr && s.store_nr != u16::MAX && s.rec != 0 && d.rec != 0 {
    let st = stores.store_mut(&s);
    let b: [u8; 16] = st.read::<[u8; 16]>(s.rec, s.pos);
    st.write::<[u8; 16]>(d.rec, d.pos, b);
    st.write::<[u8; 16]>(s.rec, s.pos, [0u8; 16]);
  } else { stores.move_field_out(&s, &d, 97_u16); }
}
'''
# D3 — the free-unless-returned exit tests NULL inline
n3 = len(re.findall(r'if \((var___ref_\d+)\)\.store_nr != \(var___retbuf\)\.store_nr \{ OpFreeRef\(', src))
src = re.sub(r'if \((var___ref_\d+)\)\.store_nr != \(var___retbuf\)\.store_nr \{ OpFreeRef\(',
             r'if (\1).store_nr != u16::MAX && (\1).store_nr != (var___retbuf).store_nr { OpFreeRef(', src)
# D5 — no one-element reserve before an append
n5 = len(re.findall(r'\{vector::pre_alloc_vector\(&\(var_(items|entries)\), \(1_i64\) as u32, \(\d+_i64\) as u32, &mut stores\.allocations\);\};', src))
src = re.sub(r'\{vector::pre_alloc_vector\(&\(var_(items|entries)\), \(1_i64\) as u32, \(\d+_i64\) as u32, &mut stores\.allocations\);\};', '', src)
# D2 — the moved CborValue (type 97, 16 bytes) into its element
n2 = len(re.findall(r'\{stores\.move_field_out\(&\((.*?)\), &\((var__elm_\d+)\), \(97_u16\)\);\};', src))
src = re.sub(r'\{stores\.move_field_out\(&\((.*?)\), &\((var__elm_\d+)\), \(97_u16\)\);\};', r'{mv16(stores, \1, \2);};', src)
n6 = n7 = 0
if level in ('b', 'd'):
    # D6 — the element an OpMoveField fully overwrites is minted without its prefill
    n6 = len(re.findall(r'OpNewRecord\(cell, var_items, 104_i32, 65535_i32\)', src))
    src = src.replace('OpNewRecord(cell, var_items, 104_i32, 65535_i32)', 'stores.record_new(&var_items, 104_u16, 65535_u16)')
if level == 'd':
    # D7 — destination passing: a copy of n_read_value that writes `value` at `dest` and answers (next, ok)
    m = re.search(r'\nfn n_read_value\(cell.*?\n \}  \} /\*block_1: ref\(Decoded\)\*/\n', src, re.S)
    body = m.group(0)
    dp = body.replace('fn n_read_value(cell', 'fn n_read_value_dp(cell', 1)
    dp = re.sub(r'mut var___retbuf: DbRef\) -> DbRef \{', 'mut var___retbuf: DbRef) -> (i64, u8) {', dp, count=1)
    dp = dp.replace('let stores: &mut Stores = unsafe { &mut *cell.get() };',
                    'let stores: &mut Stores = unsafe { &mut *cell.get() };\n  let mut __next: i64 = 0; let mut __ok: u8 = 0;', 1)
    k_next = len(re.findall(r'\{\{let db = \(var___retbuf\); let v = (.*?); if db\.rec != 0 \{ stores\.store_mut\(&db\)\.set_int\(db\.rec, db\.pos \+ \(16_i64\) as u32, v\); \}\}\};', dp))
    dp = re.sub(r'\{\{let db = \(var___retbuf\); let v = (.*?); if db\.rec != 0 \{ stores\.store_mut\(&db\)\.set_int\(db\.rec, db\.pos \+ \(16_i64\) as u32, v\); \}\}\};', r'__next = \1;', dp)
    k_ok = len(re.findall(r'\{\{let db = \(var___retbuf\); let v = (.*?); if db\.rec != 0 \{ stores\.store_mut\(&db\)\.set_byte\(db\.rec, db\.pos \+ \(24_i64\) as u32, 0, i32::from\(v\)\); \}\}\};', dp))
    dp = re.sub(r'\{\{let db = \(var___retbuf\); let v = (.*?); if db\.rec != 0 \{ stores\.store_mut\(&db\)\.set_byte\(db\.rec, db\.pos \+ \(24_i64\) as u32, 0, i32::from\(v\)\); \}\}\};', r'__ok = \1;', dp)
    k_ret = dp.count('return var___retbuf')
    dp = dp.replace('return var___retbuf', 'return (__next, __ok)')
    assert k_next == k_ok == k_ret == 16, (k_next, k_ok, k_ret)
    src = src.replace(body, body + dp, 1)
    # both array arms (plain and ranged decoder, and the dp copy) call dp with the element as destination
    arm = re.compile(r'            if \(\(\(var___ref_5\)\.store_nr == u16::MAX\) as u8\) == 1 \{.*?'
                     r'n_read_value\(cell, var_bytes, var_p, (.*?), var___ref_5\);.*?'
                     r'if \(var_sub\)\.store_nr != \(var___ref_5\)\.store_nr \{ OpFreeRef\(cell,var_sub, "var_sub"\); var_sub\.store_nr = u16::MAX \};\n', re.S)
    def repl(mm):
        return ('            var__elm_3 = stores.record_new(&var_items, 104_u16, 65535_u16);\n'
                '            let (__nx, __okc) = n_read_value_dp(cell, var_bytes, var_p, ' + mm.group(1) + ', var__elm_3);\n'
                '            if __okc == 1 { OpFinishRecord(cell, var_items, var__elm_3, 104_i32, 65535_i32); var_p = __nx; } else { var_aok = (false) as u8; }\n')
    src, n7 = arm.subn(repl, src)
    assert n7 == 3, n7
src = src.replace('\nfn n_read_value(cell', HELPER + '\nfn n_read_value(cell', 1)
open(sys.argv[3], 'w').write(src)
print(f"level {level}: D3 sites {n3}, D5 sites {n5}, D2 sites {n2}, D6 sites {n6}, D7 arms {n7}")
