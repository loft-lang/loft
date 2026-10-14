#!/usr/bin/env python3
"""Price the held-header form of draw_bezier (the 0.9.8 library form, base098.rs):
the two stack vectors keep a push header + element base across the subdivision loop,
OpKeepRange refreshes the held length at its own site ((R-Refresh) extended to OpKeepRange),
the 16 pushes go through push_hoisted (the base re-taken only after a growth), and
draw_line is called through its __inv variant with the canvas header held once.
Edits ONE function: t_6Canvas_draw_bezier.  usage: price_held_stacks.py base098.rs > out.rs
"""
import re, sys
MODE = sys.argv[2] if len(sys.argv) > 2 else 'all'
src = open(sys.argv[1]).read()
i = src.find('fn t_6Canvas_draw_bezier(')
j = src.find('\n  } /*block_1: void*/\n', i) + len('\n  } /*block_1: void*/\n')
f = src[i:j]
n0 = f
# 1. prelude after the stack pointer is set
f = f.replace('let mut var_bz_top: i64 = 4_i64;\n',
 'let mut var_bz_top: i64 = 4_i64;\n'
 '  let mut __ph_x = vector::push_header(&(var_bz_sx), &stores.allocations); let mut __vb_x: *const u8 = vector::vec_base(&__ph_x.h, &stores.allocations);\n'
 '  let mut __ph_y = vector::push_header(&(var_bz_sy), &stores.allocations); let mut __vb_y: *const u8 = vector::vec_base(&__ph_y.h, &stores.allocations);\n'
 '  let __vh_c = vector::vec_header(&({{ let _v_v1 = (var_self); DbRef {store_nr: _v_v1.store_nr, rec: _v_v1.rec, pos: _v_v1.pos + (16_i64) as u32} }}), &stores.allocations);\n'
 '  let __vb_c: *const u8 = vector::vec_base(&__vh_c, &stores.allocations);\n'
 '  let __vs_w = {{let db = (var_self); if db.rec == 0 { i64::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i64) as u32)} }};\n'
 '  let __vs_h = {{let db = (var_self); if db.rec == 0 { i64::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i64) as u32)} }};\n', 1)
# the loop half only: everything after the stack pointer's definition
k = f.find('let mut var_bz_top: i64 = 4_i64;'); head, f = f[:k], f[k:]
# 2. the eight reads
pat = re.compile(r'let _pre_(\d+) = \{\{ let _v_index = \((.*?)\); \{ let _v_r = \(var_bz_s([xy])\); \{ let el = vector::get_vector\(&_v_r, \(8_i64\) as u32, _v_index, &stores.allocations\); ops::note_format_fault\(3, [^;]*\); el \} \} \}\};\n(\s*)let mut (var___ncc_\d+): i64 = \{\{let db = \(_pre_\1\); if db.rec == 0 \{ i64::MIN \} else \{ stores.store\(&db\).get_int\(db.rec, db.pos \+ \(0_i64\) as u32\)\} \}\};')
def rd(m):
    idx, ax, ind, var = m.group(2), m.group(3), m.group(4), m.group(5)
    return (f'let mut {var}: i64 = unsafe {{ vector::get_elem_at::<i64, false>(&__ph_{ax}.h, __vb_{ax}, &(var_bz_s{ax}), 8_u32, {idx}, 0_u32, i64::MIN, &stores.allocations) }};')
f, n = pat.subn(rd, f); assert n == 8, n
# 3. draw_line through its invariant-input variant
if MODE in ('all', 'inv'):
  f, n = re.subn(r't_6Canvas_draw_line\(cell, var_self, (var_bz_p0x), (var_bz_p0y), (var_bz_p3x), (var_bz_p3y), var_color\);',
               r't_6Canvas_draw_line__inv(cell, var_self, \1, \2, \3, \4, var_color, __vs_w, __vs_h, __vh_c, __vb_c);', f); assert n == 1, n
if MODE == 'inv':
  f = head + f
  sys.stdout.write(src[:i] + f + src[j:]); sys.exit(0)
# 4. keep range refreshes the held length at its own site
f, n = re.subn(r'\{stores.vector_keep_range\(&\(var_bz_s([xy])\), \((var__slice_lo_\d)\), \((var__slice_hi_\d)\), \(0_u16\)\);\};',
               r'{stores.vector_keep_range(&(var_bz_s\1), (\2), (\3), (0_u16));}; __ph_\1.h.len = (\3 - \2) as u32;', f); assert n == 2, n
# 5. pushes through the held push header; the base re-taken after a growth
f, n = re.subn(r'\{vector::pre_alloc_vector\(&\(var_bz_s([xy])\), \(4_i64\) as u32, \(8_i64\) as u32, &mut stores.allocations\);\};',
               r'let __cap_\1 = __ph_\1.cap;', f); assert n == 4, n
f, n = re.subn(r'\{stores.append_i64\(&\(var_bz_s([xy])\), \((var_bz_\w+)\)\);\};',
               r'{ stores.push_hoisted::<i64, false>(&mut __ph_\1, &(var_bz_s\1), 8, \2) };', f); assert n == 16, n
f, n = re.subn(r'var_bz_top = \(\(var_bz_top\).wrapping_add\(4_i64\)\);',
               r'var_bz_top = ((var_bz_top).wrapping_add(4_i64)); if __ph_x.cap != __cap_x { __vb_x = vector::vec_base(&__ph_x.h, &stores.allocations); } if __ph_y.cap != __cap_y { __vb_y = vector::vec_base(&__ph_y.h, &stores.allocations); }', f); assert n == 2, n
f = head + f
assert f != n0
sys.stdout.write(src[:i] + f + src[j:])
