#!/usr/bin/env python3
"""Price the server/header levers on the emitted base.rs, cumulatively:
  A   (R-TextBorrow) admits `h`: the walked header line is a &str borrow, not a String copy
  AB  + a case-folded prefix test: `h.to_lowercase().starts_with(prefix)` compares without
        building the lowercase text (ASCII fast path, full fold only where a non-ASCII byte is met)
  ABD + the prefix itself is never built: `name.to_lowercase() + ":"` is folded into the same test
  ABDC + the hit path: the split table slices the borrowed `h` (no copy), and the value is
        returned once (no trim().to_string() + to_string() + Str::new + caller to_string chain)
Only t_7Request_header is edited.  usage: price_levers.py base.rs <A|AB|ABD|ABDC> out.rs
"""
import sys
src = open(sys.argv[1]).read(); level = sys.argv[2]
start = src.index("fn t_7Request_header(")
end = src.index("\n// loft:", start)
f = src[start:end]
def rep(old, new):
    global f
    assert f.count(old) == 1, (old[:60], f.count(old))
    f = f.replace(old, new)

HELPERS = r'''
#[inline(always)]
fn __fold_starts_with(h: &str, prefix_lower: &str) -> bool { // h.to_lowercase().starts_with(prefix_lower), prefix already lowercase
    let hb = h.as_bytes(); let pb = prefix_lower.as_bytes();
    if hb.len() >= pb.len() && hb[..pb.len()].is_ascii() {
        return hb[..pb.len()].eq_ignore_ascii_case(pb);
    }
    h.to_lowercase().starts_with(prefix_lower)
}
#[inline(always)]
fn __fold_starts_with_name(h: &str, name: &str) -> bool { // h.to_lowercase().starts_with(name.to_lowercase() + ":")
    let hb = h.as_bytes(); let nb = name.as_bytes();
    if nb.is_ascii() && hb.len() > nb.len() && hb[..nb.len() + 1].is_ascii() {
        return hb[..nb.len()].eq_ignore_ascii_case(nb) && hb[nb.len()] == b':';
    }
    let mut p = name.to_lowercase(); p.push(':');
    h.to_lowercase().starts_with(&p)
}
'''
# A: the walked element is a borrow
rep('''        {{let db = (_pre_1); if db.rec == 0 { loft::state::STRING_NULL } else { let store = stores.store(&db); store.get_str(store.get_u32_raw(db.rec, db.pos + (0_i64) as u32)) } }}
        } /*iter next_5: text*/.to_string();''',
    '''        {{let db = (_pre_1); if db.rec == 0 { loft::state::STRING_NULL } else { let store = stores.store(&db); store.get_str(store.get_u32_raw(db.rec, db.pos + (0_i64) as u32)) } }}
        } /*iter next_5: text*/;''')
if "B" in level:
    rep('''        let _pre_2 = &*({ //synth text dest_9: text["__work_p2_1"]
          var___work_p2_1 = (&var_h).to_lowercase().to_string();
          &var___work_p2_1
          } /*synth text dest_9: text["__work_p2_1"]*/);
        if (((_pre_2).starts_with((&var_prefix))) as u8) == 1 {''',
        '''        if __fold_starts_with(var_h, &var_prefix) {''' if "D" not in level else
        '''        if __fold_starts_with_name(var_h, var_name) {''')
if "D" in level:
    rep('''  let mut var_prefix = { //Add text_2: text["__work_1"]
    { let _cap = 2_usize * 8; if var___work_1.capacity() < _cap { var___work_1 = String::with_capacity(_cap); } else { var___work_1.clear(); } };
    var___work_1 += &*((var_name).to_lowercase());
    var___work_1 += &*(":");
    &var___work_1
    } /*Add text_2: text["__work_1"]*/.to_string();''', '')
if "C" in level:
    rep('''let __st_src_7: String = (&var_h).to_string(); let __st_7: Vec<&str> = loft::codegen_runtime::lazy_split(&__st_src_7, ':').collect()''',
        '''let __st_7: Vec<&str> = loft::codegen_runtime::lazy_split(var_h, ':').collect()''')
    rep('''          let mut var___ret_1: String = (&*var_value).trim().to_string();
          *var_value = (&var___ret_1).to_string();''', '')
    rep('''          return Str::new(&*var_value)
          } /*block_10: never*/''', '''          return Str::new((&*var_value).trim())
          } /*block_10: never*/''')
out = src[:start] + f + src[end:]
out = out.replace("fn t_7Request_header(", HELPERS + "\nfn t_7Request_header(", 1)
open(sys.argv[3], "w").write(out)
