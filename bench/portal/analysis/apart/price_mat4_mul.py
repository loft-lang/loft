#!/usr/bin/env python3
"""Price (R-Apart) instance 2 on mesh3d's mat4_mul: Mat4 { m: [16 floats] } carried as a
[f64; 16] value across mat4_identity / rotate_x / rotate_y / mat4_mul inside mul_op, converted
to the store form ONCE at mul_op's return (the boundary: the caller reads `.m` through the store).

Edits base.rs by replacing n_mul_op and n_mul_op__rg; every other function is untouched.
"""
import re, sys

src = open(sys.argv[1]).read()

HELPERS = r'''
// ---- (R-Apart) instance 2, hand-priced: Mat4 as a [f64; 16] value ----
#[inline(always)]
fn a_get16(m: &[f64; 16], i: i64) -> f64 { // @FR-H-Index: negative counts from the end, out of range answers null (NaN)
    let i = if i < 0 { i + 16 } else { i };
    if (0..16).contains(&i) { m[i as usize] } else { f64::NAN }
}
#[inline(always)]
fn a_set16(m: &mut [f64; 16], i: i64, v: f64) { // @FR-H-WriteOOB: out of range writes nothing
    let i = if i < 0 { i + 16 } else { i };
    if (0..16).contains(&i) { m[i as usize] = v; }
}
#[inline]
fn a_mat4_identity() -> [f64; 16] {
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
}
#[inline]
fn a_mat4_rotate_y(angle: f64) -> [f64; 16] {
    let c = angle.cos(); let s = angle.sin();
    [c, 0.0, s, 0.0, 0.0, 1.0, 0.0, 0.0, -s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0]
}
#[inline]
fn a_mat4_rotate_x(angle: f64) -> [f64; 16] {
    let c = angle.cos(); let s = angle.sin();
    [1.0, 0.0, 0.0, 0.0, 0.0, c, -s, 0.0, 0.0, s, c, 0.0, 0.0, 0.0, 0.0, 1.0]
}
// The loft body as written: the same loops, the same index arithmetic (wrapping, as emitted),
// the same checked read/write semantics, plain f64 ops as the emission has them.
fn a_mat4_mul(ma: &[f64; 16], mb: &[f64; 16]) -> [f64; 16] {
    let mut r: [f64; 16] = [0.0; 16];
    let mut col: i64 = 0;
    while col < 4 {
        let mut row: i64 = 0;
        while row < 4 {
            let mut sum: f64 = 0.0;
            let mut k: i64 = 0;
            while k < 4 {
                let a = a_get16(ma, k.wrapping_mul(4).wrapping_add(row));
                let b = a_get16(mb, col.wrapping_mul(4).wrapping_add(k));
                sum = sum + a * b;
                k += 1;
            }
            a_set16(&mut r, col.wrapping_mul(4).wrapping_add(row), sum);
            row += 1;
        }
        col += 1;
    }
    r
}
// The BOUNDARY: the ordinary store constructor for `Mat4 { m: [...] }` into the return buffer,
// the exact sequence n_mat4_identity emits (refill guard, clear, 16 appends).
fn a_mat4_to_store(cell: &std::cell::UnsafeCell<Stores>, mut buf: DbRef, m: &[f64; 16]) -> DbRef {
    let stores: &mut Stores = unsafe { &mut *cell.get() };
    if !(buf.store_nr != u16::MAX && buf.rec != 0) { buf = OpDatabaseRefill(cell, buf, 100_i32); }
    { let _rf = buf; vector::clear_vector(&DbRef { store_nr: _rf.store_nr, rec: _rf.rec, pos: _rf.pos + 0_u32 }, &mut stores.allocations); }
    let v = DbRef { store_nr: buf.store_nr, rec: buf.rec, pos: buf.pos + 0_u32 };
    for i in 0..16 { stores.append_f64(&v, m[i]); }
    buf
}
'''

def mul_op_body(name):
    return f'''fn {name}(cell: &std::cell::UnsafeCell<Stores>, mut var_r: i64, mut var_mo_c: DbRef) -> DbRef {{ //(R-Apart) instance 2, hand form
  let mo_a: [f64; 16] = a_mat4_mul(&a_mat4_rotate_y((0.3_f64) + ((ops::op_conv_float_from_int((ops::op_logical_and_int((var_r), (1_i64))))) * (0.01_f64))), &a_mat4_rotate_x(0.2_f64));
  let mut mo_c: [f64; 16] = a_mat4_identity();
  let mut i: i64 = 0;
  while i < 100000 {{ mo_c = a_mat4_mul(&mo_a, &mo_c); i += 1; }}
  a_mat4_to_store(cell, var_mo_c, &mo_c)
}}
'''

def replace_fn(src, name):
    # from "fn <name>(" to the next top-level "// loft:" comment (the next function's header)
    start = src.index(f"\nfn {name}(") + 1
    end = src.index("\n// loft:", start) + 1
    return src[:start] + mul_op_body(name) + "\n" + src[end:]

out = replace_fn(src, "n_mul_op")
out = replace_fn(out, "n_mul_op__rg")
out = out.replace("\nfn n_mul_op(", HELPERS + "\nfn n_mul_op(", 1)
open(sys.argv[2], "w").write(out)
print("ok", len(src), "->", len(out))
