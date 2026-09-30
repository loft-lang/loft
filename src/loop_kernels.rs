// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Loop kernels: a whole loop as ONE Rust function both backends call.
//!
//! Each kernel is an ordinary stdlib function — declared without a loft body in
//! `default/*.loft`, implemented here, and reached by the interpreter through its single
//! native-call operator (`OpStaticCall`, a row in `native::FUNCTIONS`) and by `--native`
//! through `codegen_runtime` (a row in `CODEGEN_RUNTIME_FNS`).  Adding a kernel therefore adds
//! a function, never an operator: the opcode table does not grow with the library.
//!
//! What a kernel buys is the interpreter's per-element cost.  A loop the interpreter
//! dispatches element by element costs several operators per element; the kernel runs the
//! loop in compiled Rust, the way native already does (@PLN180 § Kernels).  A kernel answers
//! exactly what the loop it replaces answers — null propagation, overflow reports and all —
//! so replacing a loop by a call is a representation change, never a semantics one.

use crate::keys::DbRef;
use crate::store::Store;
use crate::vector;

/// `LOFT_HOIST_VERIFY=1` re-checks every plain block the sum admits against the checked add,
/// as the native emission does under the same switch.
fn verify() -> bool {
    crate::env_once!(std::env::var_os("LOFT_HOIST_VERIFY").is_some_and(|v| v != "0"))
}

/// `acc + v[0] + v[1] + … + v[len-1]` over an integer vector, with loft's integer add:
/// a null operand gives null and stays null, an overflow reports once and gives null.
///
/// The plain part runs [`vector::sum_blocks_i64`] — blocks whose elements and running total
/// are provably far from the i64 edge, summed vectorised — and the elements it leaves are
/// added one by one with the checked add, which is where a null or an overflow is met.
#[must_use]
pub fn vector_sum_int(stores: &[Store], v: &DbRef, acc: i64) -> i64 {
    let len = i64::from(vector::length_vector(v, stores));
    if len == 0 {
        return acc;
    }
    let header = vector::vec_header(v, stores);
    let base = vector::vec_base(&header, stores);
    // SAFETY: `base` is `vec_base` of the header just read; nothing below writes a store.
    let (mut acc, at) = unsafe {
        if verify() {
            vector::sum_blocks_i64::<true>(base, header.len, 0, len, acc)
        } else {
            vector::sum_blocks_i64::<false>(base, header.len, 0, len, acc)
        }
    };
    for i in at..len {
        // SAFETY: as above; `i < len`.
        let x = unsafe {
            vector::get_elem_at::<i64, false>(&header, base, v, 8, i, 0, i64::MIN, stores)
        };
        acc = crate::ops::op_add_int(acc, x);
    }
    acc
}
