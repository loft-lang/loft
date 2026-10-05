#!/usr/bin/env python3
"""Ceiling: (R-Apart) for a scalar vector local that never leaves the frame — draw_bezier's two
stacks as Rust Vec<i64> (no store minted, reads keep @FR-H-Index: negative from the end, out of
range answers the `?? 0` default), with draw_line through its __inv variant.  usage: price_apart_vec.py base098.rs"""
import re, sys
src = open(sys.argv[1]).read()
i = src.find('fn t_6Canvas_draw_bezier('); j = src.find('\n  } /*block_1: void*/\n', i) + len('\n  } /*block_1: void*/\n')
f = src[i:j]; n0 = f
helpers = '''
#[inline(always)] fn a_get(v: &Vec<i64>, idx: i64) -> i64 { let n = v.len() as i64; let k = if idx < 0 { idx + n } else { idx }; if k >= 0 && k < n { v[k as usize] } else { 0 } }
'''
# pre-loop: the two store vectors become Vec locals (line by line: the emitter interleaves `// loft:` lines)
k = f.find('let mut var_bz_top: i64 = 4_i64;'); head, f = f[:k], f[k:]
out = []
for line in head.splitlines(keepends=True):
    t = line.strip()
    if re.match(r'(let mut )?var___vdb_[12](: DbRef)? = ', t) or (t.startswith('{{ let _v_val = (0_i64);') and 'var___vdb_' in t) or t.startswith('{vector::pre_alloc_vector(&(var_bz_s'):
        continue
    m = re.match(r'let mut var_bz_s([xy]): DbRef = ', t)
    if m: out.append('  let mut bz_%s: Vec<i64> = Vec::with_capacity(64);\n' % m.group(1)); continue
    m = re.match(r'\{stores.append_i64\(&\(var_bz_s([xy])\), \((var_b[xy]\d)\)\);\};', t)
    if m: out.append('  bz_%s.push(%s);\n' % (m.group(1), m.group(2))); continue
    out.append(line)
head = ''.join(out)
f, n = re.subn(r'  if var___vdb_[12].store_nr != u16::MAX \{ OpFreeRef\(cell,var___vdb_[12], "var___vdb_[12]"\); var___vdb_[12].store_nr = u16::MAX; \};\n', '', f); assert n >= 2, n
f = head + f
assert 'OpDatabaseNP' not in f and 'var___vdb' not in f, 'pre-loop rewrite incomplete'
f = f.replace('let mut var_bz_top: i64 = 4_i64;\n', 'let mut var_bz_top: i64 = 4_i64;\n'
 '  let __vh_c = vector::vec_header(&({{ let _v_v1 = (var_self); DbRef {store_nr: _v_v1.store_nr, rec: _v_v1.rec, pos: _v_v1.pos + (16_i64) as u32} }}), &stores.allocations);\n'
 '  let __vb_c: *const u8 = vector::vec_base(&__vh_c, &stores.allocations);\n'
 '  let __vs_w = {{let db = (var_self); if db.rec == 0 { i64::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i64) as u32)} }};\n'
 '  let __vs_h = {{let db = (var_self); if db.rec == 0 { i64::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i64) as u32)} }};\n', 1)
pat = re.compile(r'let _pre_(\d+) = \{\{ let _v_index = \((.*?)\); \{ let _v_r = \(var_bz_s([xy])\); \{ let el = vector::get_vector\(&_v_r, \(8_i64\) as u32, _v_index, &stores.allocations\); ops::note_format_fault\(3, [^;]*\); el \} \} \}\};\n(\s*)let mut (var___ncc_\d+): i64 = \{\{let db = \(_pre_\1\); if db.rec == 0 \{ i64::MIN \} else \{ stores.store\(&db\).get_int\(db.rec, db.pos \+ \(0_i64\) as u32\)\} \}\};')
f, n = pat.subn(lambda m: f'let mut {m.group(5)}: i64 = a_get(&bz_{m.group(3)}, {m.group(2)});', f); assert n == 8, n
f, n = re.subn(r't_6Canvas_draw_line\(cell, var_self, (var_bz_p0x), (var_bz_p0y), (var_bz_p3x), (var_bz_p3y), var_color\);',
               r't_6Canvas_draw_line__inv(cell, var_self, \1, \2, \3, \4, var_color, __vs_w, __vs_h, __vh_c, __vb_c);', f); assert n == 1, n
f, n = re.subn(r'i64::from\(vector::length_vector\(&\(var_bz_s([xy])\), &stores.allocations\)\)', r'(bz_\1.len() as i64)', f); assert n >= 2, n
f, n = re.subn(r'\{stores.vector_keep_range\(&\(var_bz_s([xy])\), \((var__slice_lo_\d)\), \((var__slice_hi_\d)\), \(0_u16\)\);\};',
               r'{ bz_\1.drain(..(\2 as usize)); bz_\1.truncate((\3 - \2) as usize); }', f); assert n >= 2, n
f, n = re.subn(r'\{vector::pre_alloc_vector\(&\(var_bz_s([xy])\), \(4_i64\) as u32, \(8_i64\) as u32, &mut stores.allocations\);\};', r'bz_\1.reserve(4);', f); assert n == 4, n
f, n = re.subn(r'\{stores.append_i64\(&\(var_bz_s([xy])\), \((var_bz_\w+)\)\);\};', r'bz_\1.push(\2);', f); assert n == 16, n
assert 'var_bz_sx' not in f and 'var_bz_sy' not in f, 'a store op survived'
sys.stdout.write(src[:i] + helpers + f + src[j:])
