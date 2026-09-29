// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// plan-25 F2 — apply #[loft_native] to representative native-fn signatures
// and drive the generated `<fn>__loft_bridge` directly, asserting it decodes
// each LoftValue arg into the impl's real typed parameter, calls the real fn,
// and tags the return into *ret.

use loft_ffi::{LoftRef, LoftStore, LoftStoreCtx, LoftStr, LoftValue};
use loft_ffi_macros::loft_native;

fn dummy_store() -> LoftStore {
    LoftStore {
        ptr: std::ptr::null_mut(),
        size: 0,
        ctx: LoftStoreCtx {
            _opaque: std::ptr::null_mut(),
        },
        claim_fn: None,
        reload_fn: None,
        resize_fn: None,
        foreign_fn: None,
    }
}

fn store_with_size(size: u32) -> LoftStore {
    LoftStore {
        size,
        ..dummy_store()
    }
}

// ── scalars: width cast comes from the REAL Rust signature ───────────────

#[loft_native]
extern "C" fn n_add(a: i32, b: i32) -> i32 {
    a + b
}

#[loft_native]
extern "C" fn n_trunc_u16(x: u16) -> i32 {
    i32::from(x)
}

#[test]
fn scalar_in_out() {
    let args = [LoftValue::int(2), LoftValue::int(40)];
    let mut ret = LoftValue::VOID;
    unsafe { n_add__loft_bridge(dummy_store(), args.as_ptr(), args.len(), &mut ret) };
    assert_eq!(ret.as_i64(), 42);
}

#[test]
fn scalar_width_from_impl_signature() {
    // 70000 as u16 wraps to 4464 — proves the cast width is the impl's u16,
    // not the loft i64 cell.
    let args = [LoftValue::int(70_000)];
    let mut ret = LoftValue::VOID;
    unsafe { n_trunc_u16__loft_bridge(dummy_store(), args.as_ptr(), args.len(), &mut ret) };
    assert_eq!(ret.as_i64(), 4464);
}

// ── i32 null-sentinel widening ───────────────────────────────────────────

#[loft_native]
extern "C" fn n_null() -> i32 {
    i32::MIN
}

#[test]
fn i32_min_widens_to_i64_min_sentinel() {
    let mut ret = LoftValue::VOID;
    unsafe { n_null__loft_bridge(dummy_store(), std::ptr::null(), 0, &mut ret) };
    assert_eq!(
        ret.as_i64(),
        i64::MIN,
        "null sentinel preserved across widen"
    );
}

// ── bool / float ─────────────────────────────────────────────────────────

#[loft_native]
extern "C" fn n_not(b: bool) -> bool {
    !b
}

#[loft_native]
extern "C" fn n_half(x: f64) -> f64 {
    x / 2.0
}

#[loft_native]
extern "C" fn n_single(x: f32) -> f32 {
    x * 2.0
}

#[test]
fn bool_and_float() {
    let mut ret = LoftValue::VOID;
    let a = [LoftValue::boolean(true)];
    unsafe { n_not__loft_bridge(dummy_store(), a.as_ptr(), 1, &mut ret) };
    assert!(!ret.as_bool());

    let a = [LoftValue::float(9.0)];
    unsafe { n_half__loft_bridge(dummy_store(), a.as_ptr(), 1, &mut ret) };
    assert!((ret.as_f64() - 4.5).abs() < 1e-9);

    let a = [LoftValue::float(2.5)];
    unsafe { n_single__loft_bridge(dummy_store(), a.as_ptr(), 1, &mut ret) };
    assert!((ret.as_f64() - 5.0).abs() < 1e-6);
}

// ── text: one loft arg → (*const u8, usize) ──────────────────────────────

#[loft_native]
extern "C" fn n_textlen(p: *const u8, l: usize) -> i32 {
    let _ = p;
    l as i32
}

#[test]
fn text_arg_expands_to_ptr_len() {
    let s = "hello!";
    let args = [LoftValue::text(LoftStr {
        ptr: s.as_ptr(),
        len: s.len(),
    })];
    let mut ret = LoftValue::VOID;
    unsafe { n_textlen__loft_bridge(dummy_store(), args.as_ptr(), 1, &mut ret) };
    assert_eq!(ret.as_i64(), 6);
}

// ── LoftStr return ───────────────────────────────────────────────────────

#[loft_native]
extern "C" fn n_greet() -> LoftStr {
    loft_ffi::ret("hi".to_string())
}

#[test]
fn loftstr_return_tags_text() {
    let mut ret = LoftValue::VOID;
    unsafe { n_greet__loft_bridge(dummy_store(), std::ptr::null(), 0, &mut ret) };
    let t = ret.as_text();
    assert_eq!(t.len, 2);
}

// ── LoftStore first param + LoftRef param + LoftRef return ───────────────

#[loft_native]
extern "C" fn n_store_ref(s: LoftStore, r: LoftRef) -> LoftRef {
    LoftRef {
        store_nr: r.store_nr,
        rec: r.rec + s.size, // prove the bridge forwarded `store`
        pos: r.pos,
    }
}

#[test]
fn store_first_then_ref() {
    let args = [LoftValue::reference(LoftRef {
        store_nr: 2,
        rec: 5,
        pos: 8,
    })];
    let mut ret = LoftValue::VOID;
    unsafe { n_store_ref__loft_bridge(store_with_size(10), args.as_ptr(), 1, &mut ret) };
    let r = ret.as_ref();
    assert_eq!((r.store_nr, r.rec, r.pos), (2, 15, 8)); // 5 + size(10)
}

// ── void return + side effect ────────────────────────────────────────────

static SINK: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

#[loft_native]
extern "C" fn n_sink(v: i64) {
    SINK.store(v, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn void_return_runs_call() {
    let args = [LoftValue::int(777)];
    let mut ret = LoftValue::int(1); // should be overwritten with VOID
    unsafe { n_sink__loft_bridge(dummy_store(), args.as_ptr(), 1, &mut ret) };
    assert_eq!(SINK.load(std::sync::atomic::Ordering::SeqCst), 777);
    assert_eq!(ret.tag, loft_ffi::LoftTag::Void);
}
