#!/usr/bin/env python3
"""Lever F — the pooled Run buffer is REFILLED in place: its records are kept across calls,
a run's text is overwritten in its existing slot when it fits (else the slot is replaced),
the style int rewritten, and only a SHORTER result truncates; plus the callee's work String
pooled (one static, capacity kept) instead of a per-call local.  Applied on top of v_all."""
import sys; from price_lib import edit
APPEND = r'''          { let __i = __produced; __produced += 1;
            if __i < __old_len {
              var__elm_1 = vector::get_vector(&var_runs, 12u32, __i as i64, &stores.allocations);
              { let db = var__elm_1; let store = stores.store_mut(&db); let old = store.get_u32_raw(db.rec, db.pos + 8); let s = wc.as_bytes();
                let cap = if old != 0 { (store.get_u32_raw(old, 0) as usize) * 8 - 8 } else { 0 };
                if old != 0 && s.len() <= cap { store.set_u32_raw(old, 4, s.len() as u32); unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), store.ptr.offset(old as isize * 8 + 8), s.len()); } }
                else { if old != 0 { store.delete(old); } let p = store.set_str(&*wc); store.set_u32_raw(db.rec, db.pos + 8, p); } }
            } else {
              {vector::pre_alloc_vector(&(var_runs), (1_i64) as u32, (12_i64) as u32, &mut stores.allocations);};
              var__elm_1 = OpNewRecordNP(cell,var_runs, 107_i32, 65535_i32);
              {{{let db = (var__elm_1); let s_val: &str = &*wc; if db.rec != 0 {{ let store = stores.store_mut(&db); let s_pos = store.set_str(s_val); store.set_u32_raw(db.rec, db.pos + (8_i64) as u32, s_pos); }}}}};
            }'''
FINISH = r'''          if __i >= __old_len { OpFinishRecord(cell, var_runs, var__elm_1, 107_i32, 65535_i32); } }'''
TRUNC = r'''  if __produced < __old_len { let mut j = __old_len; while j > __produced { j -= 1; let el = vector::get_vector(&var_runs, 12u32, j as i64, &stores.allocations); { let store = stores.store_mut(&el); let old = store.get_u32_raw(el.rec, el.pos + 8); if old != 0 { store.delete(old); } } vector::remove_vector(&var_runs, 12u32, j as i64, &mut stores.allocations); } }
  {stores.vector_replace(&(var___vdb_1), &(var_runs), (102_u16));};'''
edit(sys.argv[1], sys.argv[2], [
 (1074, 'let mut var___work_c1: String = "".to_string();', '  static mut WC1: String = String::new(); let wc: &mut String = unsafe { &mut *std::ptr::addr_of_mut!(WC1) }; let mut __produced: u32 = 0;'),
 (1075, 'stores.clear_vector_release(&var___vdb_1)', '  let __old_len: u32 = if var___vdb_1.rec != 0 { vector::length_vector(&var___vdb_1, &stores.allocations) } else { 0 };'),
 (1079, '//@FR-R-RefillBuffer', '  let _ = &var___vdb_1; // kept: the buffer keeps its records for the refill'),
 (1082, 'stores.vector_replace(&(var___vdb_1), &(var_runs), (102_u16))', '    if var___vdb_1.rec != 0 { stores.clear_vector_release(&var___vdb_1); } {stores.vector_replace(&(var___vdb_1), &(var_runs), (102_u16));};'),
 (1126, 'var___work_c1.clear();', '            wc.clear();'),
 (1127, '&mut var___work_c1', '            &mut *wc'),
 (1129, 'n_buf_slice_text(cell', '          { let _ = n_buf_slice_text(cell, {{ let _v_v1 = (var_d); DbRef {store_nr: _v_v1.store_nr, rec: _v_v1.rec, pos: _v_v1.pos + (0_i64) as u32} }}, _pre_1, ops::op_min_int((var_hi), (var_lo)), _pre_2); }'),
 (1131, 'vector::pre_alloc_vector', APPEND),
 (1132, 'OpNewRecordNP', ''),
 (1133, 'set_str(s_val)', ''),
 (1136, 'OpFinishRecord(cell, var_runs, var__elm_1', FINISH),
(1145, 'stores.vector_replace(&(var___vdb_1), &(var_runs), (102_u16))', TRUNC),
])
