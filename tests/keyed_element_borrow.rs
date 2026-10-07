// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#882 — a keyed element read BORROWS the collection it was read out of.
//!
//! `v[i]` on a vector has always typed its result with a dep naming the container, and
//! that dep is the entire reason the vector shape is safe: `return_views_local` sees a
//! borrow from a local and `materialize_view_return` copies the element into the return
//! buffer before the container is freed. Every keyed read carried none, so
//! `return make_hash()[k]` returned a pointer into a store the same function freed on
//! the way out.
//!
//! This needs its own binary for the reason `ref_param_publish` does: freed bytes are
//! usually still intact, so `tests/scripts/882-…loft` passes on value alone whether or
//! not the defect is present — the whole suite was green over it. Two oracles are added,
//! answering different questions:
//!
//!   - **Static** ([`keyed_element_is_typed_as_a_borrow`]). The compiler must NAME the
//!     container and materialise the element at the return. Deterministic in an ordinary
//!     release build: it does not depend on whether a freed store happened to be reused,
//!     which is exactly what let this through every gate the project runs.
//!   - **Behavioural** (`..._poison_…`). The same shapes under `LOFT_POISON=1`, which
//!     overwrites a freed store, on both backends. Set per-test, so it costs nothing
//!     suite-wide and does not rely on anyone remembering a poison sweep.
//!
//! [`harness_can_fail`] is the control for the harness itself: a script whose assertion
//! is deliberately false must fail, otherwise "the script printed OK" proves nothing.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/882-keyed-element-read-borrows-its-container.loft")
}

/// Run `file` on `backend` with extra env; return `(ok, stdout, stderr)`.
fn run(backend: &str, file: &PathBuf, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const OK: &str = "882 keyed-element borrow OK";

fn assert_cells_green(backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, &probe(), env);
    assert!(
        ok && stdout.contains(OK),
        "[{backend}/{tag}] every keyed-element cell must be green\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// The smallest program with the defect: a call mints a hash, the tail reads one element
/// out of it, and the function returns that element as a record. Kept separate from the
/// full probe so the static assertions below cannot be satisfied by some OTHER function
/// in the file (the vector sibling materialises too, and would mask a keyed regression).
const MINIMAL: &str = "struct Z { q: integer, r: integer }\n\
fn zmake(n: integer) -> hash<Z[q, r]> {\n\
\x20 zv: hash<Z[q, r]> = [];\n\
\x20 for zi in 0..n { zv[zi + 7, 0] = Z { q: zi + 7, r: 0 }; }\n\
\x20 zv\n}\n\
fn ztail(n: integer) -> Z { zmake(n)[7, 0] ?? Z { q: -1, r: -1 } }\n\
fn main() { println(\"{ztail(3).q}\"); }\n";

fn write_temp(tag: &str, src: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("loft_882_{tag}_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    path
}

// ── Static: the container is named, and the element is copied before it dies ─────────

/// The deterministic half. Two facts must both hold in the emitted IR, and each fails
/// differently: without the first, the element's type says it owns what it points at;
/// without the second, nothing copies it out before `OpFreeRef` runs.
#[test]
fn keyed_element_is_typed_as_a_borrow() {
    let path = write_temp("static", MINIMAL);
    let out = Command::new(loft_bin())
        .arg("introspect")
        .arg(&path)
        .env("LOFT_TIMEOUT", "300")
        .output()
        .expect("failed to invoke loft introspect");
    let _ = fa::remove_file(&path);
    assert!(out.status.success(), "introspect must exit 0");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    assert!(
        text.contains("inline_container"),
        "the collection a keyed element is read out of must be bound to a variable the \
         element can depend on — loft#882 regressed at the naming step\n{text}"
    );
    assert!(
        text.contains("materialized_view_return"),
        "a returned keyed element must be COPIED into the return buffer before its \
         container is freed — loft#882 regressed at the materialisation step\n{text}"
    );
}

// ── Value, both backends ────────────────────────────────────────────────────────────

#[test]
fn keyed_cells_interpret() {
    assert_cells_green("--interpret", &[], "value");
}

#[test]
fn keyed_cells_native() {
    assert_cells_green("--native", &[], "value");
}

// ── The gate that sees the read land on a freed store ───────────────────────────────

#[test]
fn keyed_cells_poison_clean_interpret() {
    assert_cells_green("--interpret", &[("LOFT_POISON", "1")], "poison");
}

#[test]
fn keyed_cells_poison_clean_native() {
    assert_cells_green("--native", &[("LOFT_POISON", "1")], "poison");
}

/// Naming the container introduces a work-ref that OWNS the lifted collection, so it
/// must still be freed. A fix that stopped the use-after-free by never freeing anything
/// would pass every cell above and leak instead; this is the neighbour that catches it.
#[test]
fn keyed_cells_leak_clean_native() {
    let (_ok, _out, stderr) = run("--native", &probe(), &[("LOFT_NATIVE_LEAK_CHECK", "1")]);
    assert!(
        !stderr.to_lowercase().contains("not freed"),
        "[native] a store was not freed at exit — the named container leaked\n{stderr}"
    );
    assert_cells_green("--native", &[("LOFT_NATIVE_LEAK_CHECK", "1")], "leak");
}

// ── Control: the harness can fail ───────────────────────────────────────────────────

/// The same shape with the expected value deliberately wrong. If this reports success,
/// a green run above means only "the program executed", not "the element survived".
#[test]
fn harness_can_fail() {
    let src = MINIMAL.replace(
        "fn main() { println(\"{ztail(3).q}\"); }",
        "fn main() { assert(ztail(3).q == 999, \"deliberately wrong: it is 7\"); \
         println(\"882 keyed-element borrow OK\"); }",
    );
    let path = write_temp("control", &src);
    let (ok, stdout, _stderr) = run("--interpret", &path, &[]);
    let _ = fa::remove_file(&path);
    assert!(
        !(ok && stdout.contains(OK)),
        "a false assertion must fail the script — the OK line is not self-validating\n{stdout}"
    );
}
