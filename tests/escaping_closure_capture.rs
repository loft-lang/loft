// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#1308 — a closure that ESCAPES its defining frame keeps the store behind every
//! capture, whatever heap kind that capture is.
//!
//! `(L-Escape)` says a returned closure "keeps its captures (it escapes cleanly)". It did
//! not: `fn mk() -> fn()->integer { v = [7,2,3]; fn()->integer { … v … } }` returned a
//! closure over a store its own frame had already freed.
//!
//! ⚠ **Every value here is correct in every state of the bug.** The cells answer `12`, `7`
//! and `3` whether or not the store is live, because the freed bytes have not been reused
//! yet — which is why an ordinary build saw nothing and why the guard that first caught this
//! claimed "there is no build on which it fails". `LOFT_STRICT_STORES=1` is the only witness.
//!
//! ⚠ **And BOTH channels have to be asserted, not just the use-after-free.** The fix has two
//! halves and either alone leaves a defect the other hides:
//!
//!  1. `mark_backing_stores_captured` stops the frame freeing the backing local. Alone, the
//!     store is never freed at all — the UAF becomes a LEAK, and a UAF-only assertion goes
//!     green over it. Measured: disabling half 2 turns cells 1 and 3 into `NEVER FREED`.
//!  2. `frame_owns_capture_store` makes the record ADOPT what half 1 stopped freeing, so the
//!     closure's own death reclaims it.
//!
//! So `assert_clean` fails on the strict checker's TOTAL, which counts every channel, rather
//! than grepping for one of them.
//!
//! Why two mechanisms were needed at all: a capture names ONE local, but the store is not
//! always in it. A struct local owns its store outright (`s: ref(726) OWNS`), while a
//! collection local is a VIEW whose deps name a separate backing local
//! (`v: vec<int> deps=[__vdb_1(2)]` beside `__vdb_1: ref(467) OWNS`). Both the free
//! suppression and the adoption verdict were reading only the named local.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test binary path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("loft")
}

/// Run `src` on `backend` under the store oracle. Returns combined output.
///
/// The scratch file is keyed on `tag` as well as the pid: tests in one binary share a pid,
/// so keying on that alone lets them clobber each other's source.
fn run(src: &str, backend: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("loft1308_{}_{tag}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("p.loft");
    fa::write(&file, src).expect("write probe");
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(&file)
        .env("LOFT_STRICT_STORES", "1")
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("run loft");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&dir);
    text
}

/// The strict checker's own total, which counts EVERY channel — use-after-free, never-freed
/// and double-free alike. Grepping for one channel is how a leak passes a UAF-only guard.
fn violations(out: &str) -> u32 {
    out.find("FAILED: ")
        .map(|i| {
            out[i + 8..]
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

fn assert_clean(src: &str, expect: &str, tag: &str) {
    for backend in ["--interpret", "--native"] {
        let out = run(src, backend, tag);
        assert!(
            out.contains(expect),
            "{tag} on {backend}: expected {expect:?} in output:\n{out}"
        );
        assert_eq!(
            violations(&out),
            0,
            "{tag} on {backend}: the store oracle reported violations:\n{out}"
        );
    }
}

/// A vector built in the frame that returns the closure. The original report.
#[test]
fn a_returned_closure_keeps_a_captured_vector() {
    assert_clean(
        r#"fn mk() -> fn() -> integer { v = [7,2,3]; fn() -> integer { s=0; for e in v { s+=e; } s } }
fn main() { g = mk(); println("got {g()}"); }"#,
        "got 12",
        "vector",
    );
}

/// A KEYED collection. Its local owns the store outright, so the adoption verdict was already
/// right and only the free suppression was wrong — the reverse of the vector cell, which is
/// why one fix alone closed neither.
#[test]
fn a_returned_closure_keeps_a_captured_keyed_collection() {
    assert_clean(
        r#"struct R { k: text, n: integer }
fn mk() -> fn() -> integer { h: hash<R[k]> = [R{k:"a",n:7}]; fn() -> integer { c=0; for e in h { c+=e.n; } c } }
fn main() { g = mk(); println("got {g()}"); }"#,
        "got 7",
        "keyed",
    );
}

/// The captured vector comes from a CALL, so the backing local is a `__ref_N` rather than the
/// literal's `__vdb_N` — a different name and a different type for the same relationship.
#[test]
fn a_returned_closure_keeps_a_captured_call_result() {
    assert_clean(
        r#"fn src() -> vector<integer> { [7,2,3] }
fn mk() -> fn() -> integer { v = src(); fn() -> integer { s=0; for e in v { s+=e; } s } }
fn main() { g = mk(); println("got {g()}"); }"#,
        "got 12",
        "call",
    );
}

/// An element READ rather than a walk. `len(v)` reads the captured header and never touched
/// the store, so it stayed green throughout and is why the boundary looked narrower than it
/// was; `v[0]` dereferences and did not.
#[test]
fn a_returned_closure_indexes_its_captured_vector() {
    assert_clean(
        r#"fn mk() -> fn() -> integer { v = [7,2,3]; fn() -> integer { v[0] } }
fn main() { g = mk(); println("got {g()}"); }"#,
        "got 7",
        "index",
    );
}

/// The OVER-FIRING control, and the reason the fix follows ownership rather than the dep edge.
///
/// A captured PARAMETER must stay a BORROW: the caller owns that store and outlives this
/// frame, so adopting it would cascade a second free onto a live store — #682's defect, and
/// strictly worse than the one being fixed here. A walk that stopped at "has a dep" instead
/// of "roots at an argument" would break this cell.
#[test]
fn a_captured_parameter_is_still_borrowed() {
    assert_clean(
        r#"fn mk(v: vector<integer>) -> fn() -> integer { fn() -> integer { s=0; for e in v { s+=e; } s } }
fn main() { v=[7,2,3]; g = mk(v); println("got {g()}"); assert(len(v) == 3, "caller intact"); }"#,
        "got 12",
        "param",
    );
}

/// The harness must be able to fail, or every green above is vacuous. A program that really
/// does leak has to be reported — otherwise `assert_clean` is asserting nothing.
#[test]
fn the_harness_can_fail() {
    let out = run(
        r#"fn main() { println("got 1"); }"#,
        "--interpret",
        "control",
    );
    assert_eq!(violations(&out), 0, "the control program is clean:\n{out}");
    assert!(
        out.contains("got 1"),
        "the control program must actually run:\n{out}"
    );
    // …and the counter reads a real report rather than defaulting to zero.
    assert_eq!(
        violations("[strict-store] FAILED: 6 store-lifetime violation(s)"),
        6,
        "violations() must read the checker's total, not default to 0"
    );
}
