#!/usr/bin/env python3
"""L2 on top of L1 — a text match binding that is only COMPARED or FORMATTED in its arm borrows
the store's bytes: no `.to_string()` per scanned entry in pa_get, none for the found value in
pa_text (formatted straight into the work buffer), and check_request keeps `op` as a `&str`."""
import re, sys
src = open(sys.argv[1]).read()
a = src.count('let var__mv_value_1: String = {{let db = var_e; if db.rec == 0 { loft::state::STRING_NULL } else { let store = stores.store(&db); store.get_str(store.get_u32_raw(db.rec, db.pos + 4)) } }}.to_string();\n        if ops::op_eq_text(&var__mv_value_1, var_key) {')
assert a == 1, a
src = src.replace('let var__mv_value_1: String = {{let db = var_e; if db.rec == 0 { loft::state::STRING_NULL } else { let store = stores.store(&db); store.get_str(store.get_u32_raw(db.rec, db.pos + 4)) } }}.to_string();\n        if ops::op_eq_text(&var__mv_value_1, var_key) {',
 'let var__mv_value_1: &str = {{let db = var_e; if db.rec == 0 { loft::state::STRING_NULL } else { let store = stores.store(&db); store.get_str(store.get_u32_raw(db.rec, db.pos + 4)) } }};\n        if ops::op_eq_text(var__mv_value_1, var_key) {')
old = '''    let var__mv_value_1: String = {{let db = (var__match_subj_1); if db.rec == 0 { loft::state::STRING_NULL } else { let store = stores.store(&db); store.get_str(store.get_u32_raw(db.rec, db.pos + (4_i64) as u32)) } }}.to_string();
    *var___work_1 = ("").to_string();
    ops::format_text(&mut var___work_1, &var__mv_value_1, 0_i64, 2, 32);'''
assert src.count(old) == 1
src = src.replace(old, '''    let var__mv_value_1: &str = {{let db = (var__match_subj_1); if db.rec == 0 { loft::state::STRING_NULL } else { let store = stores.store(&db); store.get_str(store.get_u32_raw(db.rec, db.pos + (4_i64) as u32)) } }};
    var___work_1.clear();
    var___work_1.push_str(var__mv_value_1);''')
# check_request: op stays a borrowed Str
old2 = 'let mut var_op: String = n_pa_text(cell, {{ let _v_v1 = (var__reuse_4); DbRef {store_nr: _v_v1.store_nr, rec: _v_v1.rec, pos: _v_v1.pos + (0_i64) as u32} }}, "op", _pre_4).to_string();'
assert src.count(old2) == 1
src = src.replace(old2, 'let var_op: Str = n_pa_text(cell, {{ let _v_v1 = (var__reuse_4); DbRef {store_nr: _v_v1.store_nr, rec: _v_v1.rec, pos: _v_v1.pos + (0_i64) as u32} }}, "op", _pre_4);')
src = src.replace('n_valid_op(cell, &var_op)', 'n_valid_op(cell, &*var_op)')
open(sys.argv[2], 'w').write(src)
