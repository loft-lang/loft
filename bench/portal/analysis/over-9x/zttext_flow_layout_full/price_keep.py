#!/usr/bin/env python3
"""Hand-price the collection clause on the full zttext bench's emission at 5bec988cb
   (`loft --native-release --native-emit zt.rs bench/bench.loft` in the library checkout;
   the line numbers are that file's, each asserted before it is edited).
   A: token_width's slice_runs buffer pooled across calls (one static, never freed).
   F: slice_runs refills its kept elements: element i's text through refill_str, style
      rewritten, new elements appended past the old length, a shorter result truncated
      (its surplus texts released).  usage: edit.py in.rs out.rs A|F|AF"""
import sys
src, dst, mode = sys.argv[1:4]
L = open(src).read().split('\n')
def at(n, needle):
    assert needle in L[n-1], (n, needle, L[n-1][:120])
def sub(n, needle, new):
    at(n, needle); L[n-1] = new
if 'A' in mode:
    # token_width: 2348 early-return free, 2355-2356 the lazy mint, 2391 exit free
    sub(2348, 'OpFreeRef(cell,var___ref_1', '')
    sub(2391, 'OpFreeRef(cell,var___ref_1', '')
    at(2355, 'var___ref_1 = OpDatabase(cell,var___ref_1, 106_i32)')
    L[2354] = '  var___ref_1 = unsafe { if TW_POOL.store_nr == u16::MAX { TW_POOL = OpDatabase(cell, DbRef::NULL, 106_i32); } TW_POOL };'
    at(2356, '} else {()};'); L[2355] = ''
    i = next(k for k in range(2300, 2360) if L[k].startswith('fn n_token_width('))
    L[i] = 'static mut TW_POOL: DbRef = DbRef::NULL;\n' + L[i]
if 'F' in mode:
    TRUNC = ('  if __produced < __old_len { let __v = stores.store(&var_runs).collection_rec(var_runs.rec, var_runs.pos); '
             'for j in __produced..__old_len { let el = vector::get_vector(&var_runs, 12u32, j as i64, &stores.allocations); '
             'let st = stores.store_mut(&el); let old = st.get_u32_raw(el.rec, el.pos + 8); if old != 0 { st.delete(old); st.set_u32_raw(el.rec, el.pos + 8, 0); } } '
             'stores.store_mut(&var_runs).set_u32_raw(__v, 4, __produced); }')
    sub(1614, 'stores.clear_vector_release(&var___vdb_1)', '')
    sub(1618, '//@FR-R-RefillBuffer', '  let __old_len: u32 = vector::length_vector(&var_runs, &stores.allocations); let mut __produced: u32 = 0;')
    at(1621, 'vector_replace'); L[1620] = TRUNC + '\n' + L[1620]
    at(1670, 'pre_alloc_vector'); at(1671, 'OpNewRecordNP'); at(1672, 'set_str(s_val)')
    L[1669] = ('          let __i = __produced; __produced += 1;\n'
               '          if __i < __old_len { var__elm_1 = vector::get_vector(&var_runs, 12u32, __i as i64, &stores.allocations);'
               ' { let db = var__elm_1; stores.store_mut(&db).refill_str(db.rec, db.pos + 8, &var_s); } } else {\n' + L[1669])
    L[1671] = L[1671] + ' }'
    sub(1675, 'OpFinishRecord(cell, var_runs, var__elm_1, 107', '          if __i >= __old_len { OpFinishRecord(cell, var_runs, var__elm_1, 107_i32, 65535_i32); }')
    at(1684, 'vector_replace'); L[1683] = TRUNC + '\n' + L[1683]
open(dst, 'w').write('\n'.join(L))
