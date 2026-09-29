// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Cross-backend store-leak regressions.
//!
//! These run the same loft program under both the **interpreter**
//! (`--interpret`, which prints a leak warning unconditionally) and the
//! **native** Rust-codegen pipeline (`--native`, with
//! `LOFT_NATIVE_LEAK_CHECK` set so the generated binary runs the same
//! exit-time check) and assert NEITHER backend leaks a store.
//!
//! Until @P297 added the native leak check, `--native` had no exit-time
//! leak detection at all — a whole class of leaks that the interpreter's
//! in-process `collect_store_leaks` guards (`tests/leak.rs`) went
//! unverified on the codegen path.  This file closes that gap.
//!
//! **Every test is `#[ignore]` by default** — the harness shells out to
//! the `loft` binary twice and `--native` invokes `rustc`, too heavy for
//! the default `cargo test` path.  Run them explicitly:
//!
//! ```bash
//! cargo test --release --test leak_cross_mode -- --ignored
//! ```

mod common;
use common::cross_mode::run_cross_mode_leak_free;

/// @P297 — struct-returning call passed directly as a function argument
/// (anonymous temporary) leaked one store per call on BOTH backends.
/// The interp half is also guarded in-process by
/// `tests/leak.rs::p297_nested_call_arg_temp_no_leak`; this verifies the
/// native codegen path frees the temporary too.
#[test]
#[ignore = "heavy (shells out + rustc) — runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --test leak_cross_mode -- --ignored`"]
fn p297_nested_call_arg_temp_leak_free_both_backends() {
    run_cross_mode_leak_free(
        "p297_nested_call_arg_temp",
        r#"
struct V3 { x: float, y: float, z: float }
fn mk(a: float, b: float, c: float) -> V3 { V3 { x: a, y: b, z: c } }
fn add(p: const V3, q: const V3) -> V3 { V3 { x: p.x + q.x, y: p.y + q.y, z: p.z + q.z } }
fn sx(p: const V3) -> float { p.x }
fn test() {
  a = mk(1.0, 2.0, 3.0);
  b = mk(0.5, 0.5, 0.5);
  total = 0.0;
  for i in 0..50 { total += sx(add(a, b)); }
  assert(total == 75.0, "total={total}");
  print("ok {total}");
}
"#,
    );
}

/// Both-backend control — a struct-returning call with plain (non-struct)
/// arguments, result bound to a loop local, frees correctly on interp AND
/// native.  This is the genuinely leak-free shape that bounds the two
/// bugs from below: it differs from @P297 (no anonymous arg temporary)
/// and from @P298 (callee takes no `const`-struct arg).  A regression
/// that broke this plain path would be caught here.
#[test]
#[ignore = "heavy (shells out + rustc) — runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --test leak_cross_mode -- --ignored`"]
fn plain_call_to_loop_local_leak_free_both_backends() {
    run_cross_mode_leak_free(
        "plain_call_to_loop_local",
        r#"
struct V3 { x: float, y: float, z: float }
fn mk(a: float, b: float, c: float) -> V3 { V3 { x: a, y: b, z: c } }
fn test() {
  total = 0.0;
  for i in 0..50 { d = mk(1.0, 2.0, 3.0); total += d.x; }
  assert(total == 50.0, "total={total}");
  print("ok {total}");
}
"#,
    );
}

/// @P298 — on NATIVE only, a struct-returning call whose callee takes a
/// `const`-struct (reference) argument leaked its return store even when
/// the result was bound to a local.  The interpreter freed it correctly.
///
/// **Closed 2026-05-21** — native `dispatch.rs` now OR-s the `0x8000`
/// source-free bit into the deep-copy `OpCopyRecord` (with a borrowed-view
/// guard), mirroring the interpreter's `gen_set_first_ref_call_copy`.
#[test]
#[ignore = "heavy (shells out + rustc) — runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --test leak_cross_mode -- --ignored`"]
fn p298_const_struct_arg_call_leak_free_both_backends() {
    run_cross_mode_leak_free(
        "p298_const_struct_arg_call",
        r#"
struct V3 { x: float, y: float, z: float }
fn mk(a: float, b: float, c: float) -> V3 { V3 { x: a, y: b, z: c } }
fn add(p: const V3, q: const V3) -> V3 { V3 { x: p.x + q.x, y: p.y + q.y, z: p.z + q.z } }
fn test() {
  a = mk(1.0, 2.0, 3.0);
  b = mk(0.5, 0.5, 0.5);
  d = add(a, b);
  assert(d.x == 1.5, "d.x={d.x}");
  print("ok {d.x}");
}
"#,
    );
}

/// loft#1369 — a nullable heap local REBOUND from a source that is ABSENT at run time
/// orphaned the record it displaced, on `--native` only.
///
/// `@FR-O-Borrow` and `@FR-O-Proxy`: the destination owns the store it holds, so writing
/// `DbRef::NULL` over it has to release it.  Native's nullable record bind emits two arms —
/// `if src.rec == 0 { dest = NULL } else { dest = OpDatabase(dest); OpCopyRecord(…) }` — and
/// the ELSE arm is safe because `OpDatabase` recycles the slot's existing store, while the
/// null arm displaces it.  The release was emitted only at a FIRST bind: at a reassignment
/// each of the two native sites named the OTHER as the one that frees, so neither did.
///
/// The leak is per CALL WHOSE SOURCE IS ABSENT AT RUN TIME, not per rebind — the same
/// function with the argument present is clean, which is why it reads as a nullability bug
/// and is really a missing displacement release.  `absent` and `present` are both exercised
/// here so the fix cannot be a blanket free: a present source must still recycle, not free.
#[test]
#[ignore = "heavy (shells out + rustc) — runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --test leak_cross_mode -- --ignored`"]
fn issue1369_nullable_rebind_absent_source_leak_free_both_backends() {
    run_cross_mode_leak_free(
        "issue1369_nullable_rebind_absent_source",
        r#"
struct S1369 { n: integer }
fn join_ref(y: S1369?, z: S1369) -> integer {
  x: S1369? = z;
  x = y;
  if x != null { return x.n; }
  return -1;
}
// The control the fix must not break: the destination only VIEWS a parameter, owns no
// store, and must free nothing (@FR-O-Borrow).
fn read_only(y: S1369?) -> integer { if y != null { return y.n; } return -1; }
fn test() {
  // absent twice — one displaced record each before the fix
  assert(join_ref(null, S1369 { n: 4 }) == -1, "absent 1");
  assert(join_ref(null, S1369 { n: 5 }) == -1, "absent 2");
  // present — the else arm recycles; must stay clean
  assert(join_ref(S1369 { n: 8 }, S1369 { n: 4 }) == 8, "present");
  assert(read_only(null) == -1, "view absent");
  assert(read_only(S1369 { n: 3 }) == 3, "view present");
  print("ok 1369");
}
"#,
    );
}
