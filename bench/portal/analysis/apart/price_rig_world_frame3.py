#!/usr/bin/env python3
"""Price (R-Apart) instance 3 on hex_body's rig_world_frame3: the twelve `vector<float>`
locals (today (R-WorkBuffer) parameters from the caller's pool) carried as Rust `Vec<f64>`
values that never touch a store.  Reads keep H-Index (negative from the end, out of range =
null -> the `??` default); the Rig and `values` parameters stay store values; the Frame result
is still built into the return buffer as emitted.

usage: apart_edit.py base.rs out.rs [cap]     cap = "new" (Vec::new) | "n" (with_capacity(n))
"""
import re, sys

src = open(sys.argv[1]).read()
cap = sys.argv[3] if len(sys.argv) > 3 else "new"
BUFS = ["bx", "by", "bz", "m00", "m01", "m02", "m10", "m11", "m12", "m20", "m21", "m22"]

HELPERS = r'''
// ---- (R-Apart) instance 3, hand-priced: a vector<float> local as a Rust Vec<f64> ----
#[inline(always)]
fn a_getv(v: &[f64], i: i64) -> f64 { // @FR-H-Index
    let n = v.len() as i64;
    let i = if i < 0 { i + n } else { i };
    if i >= 0 && i < n { v[i as usize] } else { f64::NAN }
}
struct ABuf { d: [f64; 64], n: usize }
impl ABuf {
    #[inline(always)] fn push(&mut self, v: f64) { if self.n < 64 { self.d[self.n] = v; self.n += 1; } }
}
impl std::ops::Deref for ABuf { type Target = [f64]; #[inline(always)] fn deref(&self) -> &[f64] { &self.d[..self.n] } }
'''

def edit_fn(body):
    # 1. the twelve buffer prologues -> Vec locals
    for b in BUFS:
        pat = re.compile(
            r"  if \(\(\(var_" + b + r"\)\.store_nr == u16::MAX\) as u8\) == 1 \{      if \(var_" + b
            + r"\)\.store_nr != .*?\n\} else \{      if var_" + b + r"\.rec != 0 \{ stores\.clear_vector_release\(&var_"
            + b + r"\); \}\n\};\n", re.S)
        if cap == "arr":   # an inline stack buffer with a length (a SmallVec with no spill — the ceiling)
            decl = "  let mut a_" + b + ": ABuf = ABuf { d: [0.0; 64], n: 0 };\n"
        else:
            init = "Vec::new()" if cap == "new" else "Vec::with_capacity(var_n.max(0) as usize)"
            decl = "  let mut a_" + b + ": Vec<f64> = " + init + ";\n"
        body, k = pat.subn(decl, body)
        assert k == 1, (b, k)
    # 2. the push-header hoists of those buffers
    body, k = re.subn(r"\n\s*let mut __ph_\d+ = vector::push_header\(&\(var_(" + "|".join(BUFS) + r")\), &stores\.allocations\);[^\n]*", "", body)
    assert k == 12, k
    # 3. pushes
    body, k = re.subn(r"stores\.push_hoisted::<f64, false>\(&mut __ph_\d+, &\(var_(" + "|".join(BUFS) + r")\), 8, __pv\)",
                      r"a_\1.push(__pv)", body)
    assert k == 24, k
    # 4. hoisted reads inside the loop
    body, k = re.subn(r"vector::get_elem_hoisted::<f64, false>\(&__ph_\d+\.h, &\(var_(" + "|".join(BUFS) + r")\), \(8_i64\) as u32, (var_p), \(0_i64\) as u32, f64::NAN, &stores\.allocations\)",
                      r"a_getv(&a_\1, \2)", body)
    assert k == 12, k
    # 5. the exit reads `X[i] ?? d`
    body, k = re.subn(r"let _pre_(\d+) = \{\{ let _v_index = \(var_i\); \{ let _v_r = \(var_(" + "|".join(BUFS) + r")\);[^\n]*\n\s*let mut var___ncc_(\d+): f64 = \{\{let db = \(_pre_\1\);[^\n]*",
                      r"let mut var___ncc_\3: f64 = a_getv(&a_\2, var_i);", body)
    assert k == 12, k
    return body

out = src
for name in ["n_rig_world_frame3", "n_rig_world_frame3__rg"]:
    start = out.index("\nfn " + name + "(") + 1
    end = out.index("\n// loft:", start) + 1
    out = out[:start] + edit_fn(out[start:end]) + out[end:]
out = out.replace("\nfn n_rig_world_frame3(", HELPERS + "\nfn n_rig_world_frame3(", 1)
open(sys.argv[2], "w").write(out)
print("ok", cap, len(src), "->", len(out))
