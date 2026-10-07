// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1235 — a `text` RETURNED from a function is read before the callee's stores are
//! released.
//!
//! Three shapes returned a text whose bytes live in a container the function owns, and on
//! `--native` each was read AFTER the epilogue dropped that container: an element of a local
//! `vector<text>`, an element of a nested-vector SLICE, and a boxed `text` parameter a closure
//! mutated. The value was right on every one of them, because nothing had reused the bytes yet.
//!
//! This needs its own binary for the reason [`keyed_element_borrow`] does: freed bytes are
//! usually intact, so `tests/scripts/1235-…loft` passes on value alone whether or not the defect
//! is present — the whole suite was green over it, on a shipped release, with three test files
//! sitting on top of the shapes. Two oracles, answering different questions:
//!
//!   - **Behavioural** (`..._poison_…`). `LOFT_POISON=1` overwrites a freed store, so a read
//!     that lands on one stops being right by luck: two of the three shapes panic on
//!     `0xDEADBEEF` and the third answers `null`. Set per-test, so it costs nothing suite-wide
//!     and does not depend on anyone remembering to arm a sweep.
//!   - **Static** ([`the_returned_text_is_read_before_the_free`]). The emitted Rust must read
//!     the element BEFORE the `OpFreeRef` of its container, which is deterministic in an
//!     ordinary build — it does not depend on whether a freed store happened to be reused,
//!     which is exactly what let this through every gate the project runs.
//!
//! [`harness_can_fail`] is the control for the harness itself: a script whose assertion is
//! deliberately false must fail, otherwise "the script printed OK" proves nothing.
//!
//! Falsified against the pre-fix build (`65d995d1` plus this file): **4 of the 6 tests pass
//! there**, and the two that move are exactly the two oracles above —
//! `returned_text_poison_clean_native` and `the_returned_text_is_read_before_the_free`. Both
//! value tests and the interpreter's poison run are green on the broken build, which is the
//! measurement that says a value-only guard for this defect would be worthless.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/1235-a-returned-text-outlives-its-store.loft")
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

const OK: &str = "1235 returned-text lifetime OK";

fn assert_cells_green(backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, &probe(), env);
    assert!(
        ok && stdout.contains(OK),
        "[{backend}/{tag}] every returned-text cell must be green\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// The smallest program with the defect. Kept separate from the full probe so the static
/// assertion below cannot be satisfied by some OTHER function in the file.
const MINIMAL: &str = "fn m1235() -> text {\n\
\x20 tv: vector<text> = [\"a\", \"b\"];\n\
\x20 return tv[0];\n}\n\
fn main() { println(\"{m1235()}\"); }\n";

fn write_temp(tag: &str, src: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("loft_1235_{tag}_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    path
}

// ── Value, both backends ────────────────────────────────────────────────────────────

#[test]
fn returned_text_cells_interpret() {
    assert_cells_green("--interpret", &[], "value");
}

#[test]
fn returned_text_cells_native() {
    assert_cells_green("--native", &[], "value");
}

// ── The gate that sees the read land on a freed store ───────────────────────────────

#[test]
fn returned_text_poison_clean_interpret() {
    assert_cells_green("--interpret", &[("LOFT_POISON", "1")], "poison");
}

#[test]
fn returned_text_poison_clean_native() {
    assert_cells_green("--native", &[("LOFT_POISON", "1")], "poison");
}

// ── Static: the order of the two statements, which no value can show ─────────────────

/// The deterministic half. In the emitted Rust the element read must come BEFORE the
/// `OpFreeRef` that releases the container it reads through. A value oracle cannot see this
/// at all: both orders answer `"a"` until something reuses the bytes.
#[test]
fn the_returned_text_is_read_before_the_free() {
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
    let body = text
        .split("fn n_m1235(cell:")
        .nth(1)
        .expect("the generated function must be in the introspect output");
    let read = body
        .find("vec_get_or_raise_runtime")
        .expect("the element read must be emitted");
    let free = body
        .find("OpFreeRef(cell,var___vdb_1")
        .expect("the container's free must be emitted");
    assert!(
        read < free,
        "the element is read AFTER its container is freed — the B5-L3 collapse hoisted the \
         read past the scope's frees (loft#1235)\n{body}"
    );
}

// ── Control: the harness can fail ───────────────────────────────────────────────────

/// The same shape with the expected value deliberately wrong. If this reports success, a
/// green run above means only "the program executed", not "the text survived".
#[test]
fn harness_can_fail() {
    let src = MINIMAL.replace(
        "fn main() { println(\"{m1235()}\"); }",
        "fn main() { assert(m1235() == \"zzz\", \"deliberately wrong: it is a\"); \
         println(\"1235 returned-text lifetime OK\"); }",
    );
    let path = write_temp("control", &src);
    let (ok, stdout, _stderr) = run("--interpret", &path, &[]);
    let _ = fa::remove_file(&path);
    assert!(
        !(ok && stdout.contains(OK)),
        "a false assertion must fail the script — the OK line is not self-validating\n{stdout}"
    );
}
