#!/usr/bin/env python3
"""Lever C+D — a run's text is built once and reaches its store slot from the work String
(no Str -> String through Display), and the work String keeps its capacity (clear(), not a
fresh empty String) in buf_slice_text and slice_runs."""
import sys; from price_lib import edit
edit(sys.argv[1], sys.argv[2], [
 (934,  '*var_out = ("").to_string();', "  var_out.clear();"),
 (982,  '*var_out = ("").to_string();', "  var_out.clear();"),
 (1027, '*var_out = ("").to_string();', "  var_out.clear();"),
 (1126, 'var___work_c1 = "".to_string();', "            var___work_c1.clear();"),
 (1129, "let mut var_s: String = n_buf_slice_text(cell", "          { let _ = n_buf_slice_text(cell, {{ let _v_v1 = (var_d); DbRef {store_nr: _v_v1.store_nr, rec: _v_v1.rec, pos: _v_v1.pos + (0_i64) as u32} }}, _pre_1, ops::op_min_int((var_hi), (var_lo)), _pre_2); }"),
 (1133, "AsRef::<str>::as_ref(&*(&var_s))", "          {{{let db = (var__elm_1); let s_val = AsRef::<str>::as_ref(&*(&var___work_c1)); if db.rec != 0 {{ let store = stores.store_mut(&db); let s_pos = store.set_str(s_val); store.set_u32_raw(db.rec, db.pos + (8_i64) as u32, s_pos); }}}}};"),
])
