// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Test fixture for native extension loading and marshalling.
//!
//! Tests all common patterns: scalar, vector, struct ref, const borrowing.
//! Every function validates its inputs and returns a checkable result.
//!
//! Plan-74: each `n_ext_*` impl carries `#[loft_native]`, so the interpreter
//! dispatches it through the generated uniform marshal bridge
//! (`<fn>__loft_bridge`), not the deleted legacy `dispatch_call` arms.  The
//! loft decls bind the `#native "loft_ext_*"` symbols, so the registration and
//! bridge tables key the bridges under those `loft_ext_*` names (pointing at the
//! `n_ext_*__loft_bridge` siblings).  Vector args arrive as a `LoftRef` to the
//! vector data record, read here via `LoftStore::vector_len`/`vector_data_ptr`.

#![allow(clippy::missing_safety_doc)]

use loft_ffi::{LoftRef, LoftStore};
use loft_ffi_macros::loft_native;

/// Borrow a vector argument's elements as a `&[T]` for the call's duration.
/// Empty when the ref is null or the vector is empty.
#[inline]
unsafe fn vec_slice<'a, T>(store: &LoftStore, vec: &LoftRef) -> &'a [T] {
    if vec.rec == 0 {
        return &[];
    }
    let count = unsafe { store.vector_len(vec) } as usize;
    if count == 0 {
        return &[];
    }
    let ptr = unsafe { store.vector_data_ptr(vec) }.cast::<T>();
    unsafe { std::slice::from_raw_parts(ptr, count) }
}

// ── Pattern 1: Scalar → Scalar (baseline) ────────────────────────────

#[loft_native]
#[unsafe(no_mangle)]
pub extern "C" fn n_ext_add_one(x: i64) -> i64 {
    x + 1
}

/// dlsym bait — returns a different value so tests detect wrong dispatch
/// (A7.2.4 registry-vs-dlsym priority).  No bridge: reached only via the raw
/// `loft_register_v1` / dlsym path under test.
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_add_one(x: i64) -> i64 {
    x + 1000
}

/// Unregistered — for the guard test (A7.2.5): exported as a raw C-ABI symbol
/// but never registered, so a `#native` decl naming it must panic at wire time.
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_unregistered(x: i64) -> i64 {
    x + 9999
}

// ── Pattern 2: Vector<integer> as input ──────────────────────────────
// Post-2c: loft `integer` is i64 at rest, so the elements are i64.

/// Sum all elements of a vector<integer>.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_vec_sum(store: LoftStore, vec: LoftRef) -> i64 {
    let data = unsafe { vec_slice::<i64>(&store, &vec) };
    if data.is_empty() {
        return -1;
    }
    eprintln!("[ext_vec_sum] count={} first={} last={}", data.len(), data[0], data[data.len() - 1]);
    data.iter().sum::<i64>()
}

// ── Pattern 3: Vector<single> as input (single-precision) ────────────

/// Sum of vector<single> elements.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_vec_sum_f32(store: LoftStore, vec: LoftRef) -> i64 {
    let data = unsafe { vec_slice::<f32>(&store, &vec) };
    if data.is_empty() {
        return -1;
    }
    eprintln!("[ext_vec_sum_f32] count={} first={} last={}", data.len(), data[0], data[data.len() - 1]);
    data.iter().sum::<f32>() as i64
}

// ── Pattern 4: Scalars before vector ─────────────────────────────────

/// offset + sum(data).
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_offset_sum(store: LoftStore, offset: i64, vec: LoftRef) -> i64 {
    let data = unsafe { vec_slice::<i64>(&store, &vec) };
    if data.is_empty() {
        return offset;
    }
    eprintln!("[ext_offset_sum] offset={} count={}", offset, data.len());
    offset + data.iter().sum::<i64>()
}

// ── Pattern 5: Vector between scalars ────────────────────────────────

/// a + sum(data) + b.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_sandwich_sum(store: LoftStore, a: i64, vec: LoftRef, b: i64) -> i64 {
    let data = unsafe { vec_slice::<i64>(&store, &vec) };
    if data.is_empty() {
        return a + b;
    }
    eprintln!("[ext_sandwich_sum] a={} count={} b={}", a, data.len(), b);
    a + data.iter().sum::<i64>() + b
}

// ── Pattern 6: Vector from struct field ──────────────────────────────

/// Read a vector<integer> that comes from a struct field access (e.g. canvas.data).
/// This tests the indirect reference path where the DbRef on the stack
/// points to a struct field containing the vector record number.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_struct_vec_len(store: LoftStore, vec: LoftRef) -> i64 {
    let data = unsafe { vec_slice::<i64>(&store, &vec) };
    eprintln!("[ext_struct_vec_len] count={}", data.len());
    data.len() as i64
}

// ── Pattern 7: Vector inside loop with if ────────────────────────────

/// Same as vec_sum but with logging to detect store issues in loops.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_loop_vec_sum(store: LoftStore, vec: LoftRef) -> i64 {
    let data = unsafe { vec_slice::<i64>(&store, &vec) };
    if data.is_empty() {
        eprintln!("[ext_loop_vec_sum] EMPTY");
        return -1;
    }
    let sum: i64 = data.iter().sum();
    eprintln!("[ext_loop_vec_sum] count={} sum={}", data.len(), sum);
    sum
}

// ── Pattern 8: Vector<u8> RETURN via the FFI alloc path (#409) ────────
// The FFI-bridge equivalent of a `vector<u8>`-returning native: builds the
// bytes [0, 1, 2, …] with `alloc_vector_from_bytes` (the null-store alloc the
// crypto/imaging libs use for a heap return).  Exercises the path where a
// returned FFI vector is later mutated in place (`v += …`) by the caller —
// loft-lang/loft#409, where that `+=` used to drop the returned elements.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_make_bytes(store: LoftStore, n: i64) -> LoftRef {
    let mut store = store;
    let len = n.max(0) as u32;
    let data: Vec<u8> = (0..len).map(|i| (i & 0xff) as u8).collect();
    unsafe { store.alloc_vector_from_bytes(1, len, data.as_ptr(), data.len()) }
}

// ── Pattern 8b/8c: Vector<u8> RETURN as a FOREIGN store (@PLN174 F5) ────
// The same bytes as `n_ext_make_bytes`, handed over with no copy: the host
// serves the cdylib's own `Vec<u8>` read-only and releases it (in this
// crate's allocator) with the last loft handle.  8b has no argument, so the
// handle's store is the one the dispatcher minted for the return; 8c takes a
// vector argument, whose store the handle pins, so the answer gets its own.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_make_bytes_foreign(store: LoftStore, n: i64) -> LoftRef {
    let mut store = store;
    let len = n.max(0) as u32;
    let data: Vec<u8> = (0..len).map(|i| (i & 0xff) as u8).collect();
    unsafe { store.foreign_vector_from_owned(data) }
}

#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_reverse_foreign(store: LoftStore, vec: LoftRef) -> LoftRef {
    let mut store = store;
    let mut data: Vec<u8> = unsafe { vec_slice::<u8>(&store, &vec) }.to_vec();
    data.reverse();
    unsafe { store.foreign_vector_from_owned(data) }
}

#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_reverse_owned(store: LoftStore, vec: LoftRef) -> LoftRef {
    let mut store = store;
    let mut data: Vec<u8> = unsafe { vec_slice::<u8>(&store, &vec) }.to_vec();
    data.reverse();
    unsafe { store.alloc_vector_from_bytes(1, data.len() as u32, data.as_ptr(), data.len()) }
}

// Pattern 9: STRUCT (non-vector heap) RETURN via alloc_record (@PLN85).
// `ExtBox { v: integer }` — one i64 field.  alloc_record(2) = 1 word header
// (pos=8) + 1 word for the field; write the field at (rec, pos, offset 0).
// Returns a foreign null-store reference exactly like the vector path; the
// probe checks whether a caller mutating / appending it corrupts (the #410
// foreign-store-borrow desync, which the vector-only assignment-side fix does
// not cover for a struct return).
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_make_box(store: LoftStore, v: i64) -> LoftRef {
    let mut store = store;
    let rec = unsafe { store.alloc_record(2) };
    unsafe { store.set_int(rec.rec, rec.pos, 0, v as i32) };
    rec
}

// Pattern 10: NULLABLE scalar return (`-> integer?`).  `Optional(integer)`
// rides the plain-integer sentinel layout, so the marshal signature is a bare
// i64 and null crosses as i64::MIN.  Negative input → null; else x + 1.
#[loft_native]
#[unsafe(no_mangle)]
pub extern "C" fn n_ext_maybe(x: i64) -> i64 {
    if x < 0 { i64::MIN } else { x + 1 }
}

// Pattern 11: WIDE/NEGATIVE integer round-trip.  A plain loft `integer` is
// 64-bit at the boundary; values beyond i32 and negatives must arrive and
// return untruncated (the i32-extern regression class).
#[loft_native]
#[unsafe(no_mangle)]
pub extern "C" fn n_ext_echo(x: i64) -> i64 {
    x
}

// ── Registration ─────────────────────────────────────────────────────
// The loft decls bind `#native "loft_ext_*"`, so register the raw `n_ext_*`
// symbols under those names (`loft_register_v1`, for the dlsym-priority and
// guard tests) AND register the generated bridges under the same names
// (`loft_register_bridges_v1`) — the interpreter prefers the bridge.

loft_ffi::loft_register! {
    loft_ext_add_one => n_ext_add_one,
    loft_ext_vec_sum => n_ext_vec_sum,
    loft_ext_vec_sum_f32 => n_ext_vec_sum_f32,
    loft_ext_offset_sum => n_ext_offset_sum,
    loft_ext_sandwich_sum => n_ext_sandwich_sum,
    loft_ext_struct_vec_len => n_ext_struct_vec_len,
    loft_ext_loop_vec_sum => n_ext_loop_vec_sum,
    loft_ext_make_bytes => n_ext_make_bytes,
    loft_ext_make_bytes_foreign => n_ext_make_bytes_foreign,
    loft_ext_reverse_foreign => n_ext_reverse_foreign,
    loft_ext_reverse_owned => n_ext_reverse_owned,
    loft_ext_make_box => n_ext_make_box,
    loft_ext_maybe => n_ext_maybe,
    loft_ext_echo => n_ext_echo,
}

loft_ffi::loft_register_bridges! {
    "loft_ext_add_one" => n_ext_add_one__loft_bridge,
    "loft_ext_vec_sum" => n_ext_vec_sum__loft_bridge,
    "loft_ext_vec_sum_f32" => n_ext_vec_sum_f32__loft_bridge,
    "loft_ext_offset_sum" => n_ext_offset_sum__loft_bridge,
    "loft_ext_sandwich_sum" => n_ext_sandwich_sum__loft_bridge,
    "loft_ext_struct_vec_len" => n_ext_struct_vec_len__loft_bridge,
    "loft_ext_loop_vec_sum" => n_ext_loop_vec_sum__loft_bridge,
    "loft_ext_make_bytes" => n_ext_make_bytes__loft_bridge,
    "loft_ext_make_bytes_foreign" => n_ext_make_bytes_foreign__loft_bridge,
    "loft_ext_reverse_foreign" => n_ext_reverse_foreign__loft_bridge,
    "loft_ext_reverse_owned" => n_ext_reverse_owned__loft_bridge,
    "loft_ext_make_box" => n_ext_make_box__loft_bridge,
    "loft_ext_maybe" => n_ext_maybe__loft_bridge,
    "loft_ext_echo" => n_ext_echo__loft_bridge,
}
