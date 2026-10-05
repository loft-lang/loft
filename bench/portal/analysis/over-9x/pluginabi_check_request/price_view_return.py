#!/usr/bin/env python3
"""L1 — pa_get answers a VIEW of the entry it found, and pa_text reads through it.

Today n_pa_text mints a CborValue store (OpDatabase 97) as pa_get's return buffer, n_pa_get
DEEP-COPIES the found entry's value into it (OpCopyRecord: a claim of the text, copy_claims),
and frees it at exit.  The hand form: n_pa_get returns the entry's own address (DbRef::NULL
when nothing matched — the tag read already answers 0 for rec == 0), n_pa_text mints nothing.
Edits exactly n_pa_get and n_pa_text; every other function is untouched."""
import re, sys
src = open(sys.argv[1]).read()

def replace_fn(src, name, body):
    m = re.search(r'(?:#\[inline\]\n)?fn ' + name + r'\(.*?\n  \} /\*block_1: [^*]*\*/\n', src, re.S)
    assert m, name
    return src[:m.start()] + body + src[m.end():]

PA_GET = r'''fn n_pa_get(cell: &std::cell::UnsafeCell<Stores>, mut var_m: DbRef, mut var_key: &str, mut var___retbuf: DbRef) -> DbRef { //L1 view return
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  let _pre_2 = {{let db = (var_m); if db.rec == 0 { 0u8 } else { let r = stores.store(&db).get_byte(db.rec, db.pos + (0_i64) as u32, 0); if r < 0 { 255u8 } else { r as u8 } }}};
  if _pre_2 == 7 {
    let var__mv_entries_1 = DbRef {store_nr: var_m.store_nr, rec: var_m.rec, pos: var_m.pos + 4};
    let var__range_end_1: i64 = i64::from(vector::length_vector(&(var__mv_entries_1), &stores.allocations));
    let mut var_i: i64 = 0;
    while var_i < var__range_end_1 {
      let var_e: DbRef = vector::get_vector(&var__mv_entries_1, 32u32, var_i, &stores.allocations);
      let tag = {{let db = var_e; if db.rec == 0 { 0u8 } else { let r = stores.store(&db).get_byte(db.rec, db.pos, 0); if r < 0 { 255u8 } else { r as u8 } }}};
      if tag == 5 {
        let var__mv_value_1: String = {{let db = var_e; if db.rec == 0 { loft::state::STRING_NULL } else { let store = stores.store(&db); store.get_str(store.get_u32_raw(db.rec, db.pos + 4)) } }}.to_string();
        if ops::op_eq_text(&var__mv_value_1, var_key) {
          return DbRef {store_nr: var_e.store_nr, rec: var_e.rec, pos: var_e.pos + 16};
        }
      }
      var_i += 1;
    }
  }
  DbRef::NULL
  } /*block_1: L1*/
'''

PA_TEXT = r'''#[inline]
fn n_pa_text(cell: &std::cell::UnsafeCell<Stores>, mut var_m: DbRef, mut var_key: &str, mut var___work_1: &mut String) -> Str { //L1 view return
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  *var___work_1 = ("").to_string();
  let var__match_subj_1: DbRef = n_pa_get(cell, var_m, var_key, DbRef::NULL);
  let _pre_3 = {{let db = (var__match_subj_1); if db.rec == 0 { 0u8 } else { let r = stores.store(&db).get_byte(db.rec, db.pos + (0_i64) as u32, 0); if r < 0 { 255u8 } else { r as u8 } }}};
  if _pre_3 == 5 {
    let var__mv_value_1: String = {{let db = (var__match_subj_1); if db.rec == 0 { loft::state::STRING_NULL } else { let store = stores.store(&db); store.get_str(store.get_u32_raw(db.rec, db.pos + (4_i64) as u32)) } }}.to_string();
    *var___work_1 = ("").to_string();
    ops::format_text(&mut var___work_1, &var__mv_value_1, 0_i64, 2, 32);
    return Str::new(&*var___work_1)
  }
  return Str::new("")
  } /*block_1: L1*/
'''
src = replace_fn(src, 'n_pa_get', PA_GET)
src = replace_fn(src, 'n_pa_text', PA_TEXT)
open(sys.argv[2], 'w').write(src)
