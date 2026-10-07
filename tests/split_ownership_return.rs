// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#981 / loft#982 — a RETURN whose paths disagree about ownership is decided at
//! RUN TIME, not by a static bit that cannot express the disagreement.
//!
//! A heap return carries one static answer to "may the caller free this?", read off the
//! return deps. A return that is a view of a parameter on one path and a freshly minted
//! store on the other has no correct static answer, and the one it got — BORROW, never
//! free — orphaned the minted store: one leaked store per call, unbounded in a loop,
//! silent on both backends with every value correct.
//!
//! Two oracles, answering different questions:
//!
//!   - **Leak** (`..._leaks_*`). The primary one, and DETERMINISTIC: the store census at
//!     exit counts what was never freed regardless of whether a freed slot happened to
//!     be reused. This is the defect itself.
//!   - **Over-free** (`..._poison_*`, `..._strict_*`). The opposite direction, and the
//!     reason the fix is a decision rather than a widening: a genuine borrow must still
//!     never be freed. Under `LOFT_POISON` a store freed early answers `0xDEADBEEF`
//!     instead of passing on intact bytes. Freeing a keyed-collection parameter's
//!     element is exactly what an earlier draft of the fix did, and it broke
//!     `tests/scripts/882-…` rather than anything here — which is why that shape is a
//!     control in this probe too.
//!
//! [`harness_can_fail`] is the control for the harness itself: a script whose assertion
//! is deliberately false must fail, otherwise "the script printed OK" proves nothing.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/981-split-ownership-return.loft")
}

/// Run `file` on `backend` with extra env; return `(ok, stdout, stderr)`.
fn run(backend: &str, file: &PathBuf, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
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

const OK: &str = "981 split-ownership return OK";

fn assert_cells_green(backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, &probe(), env);
    assert!(
        ok && stdout.contains(OK),
        "[{backend}/{tag}] every split-ownership-return cell must be green\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

// ── Value, both backends ────────────────────────────────────────────────────────────

#[test]
fn split_return_cells_interpret() {
    assert_cells_green("--interpret", &[], "value");
}

#[test]
fn split_return_cells_native() {
    assert_cells_green("--native", &[], "value");
}

// ── The leak: the defect itself, and deterministic ──────────────────────────────────

/// Every call down a split return's OWNED path hands the caller a store the callee
/// minted. Reading the return as a plain borrow left it owned by nobody — 40 calls, 40
/// orphans, once per shape in the probe. The census at exit is exact.
#[test]
fn split_return_leaks_nothing_interpret() {
    let (ok, stdout, stderr) = run("--interpret", &probe(), &[("LOFT_STRICT_STORES", "1")]);
    assert!(
        ok && stdout.contains(OK),
        "[--interpret/leak] the probe must run clean\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("NEVER FREED"),
        "a split-ownership return must hand its minted store to the caller to free — \
         the store census names what was orphaned (loft#981/#982)\nstderr:\n{stderr}"
    );
}

#[test]
fn split_return_leaks_nothing_native() {
    let (ok, stdout, stderr) = run("--native", &probe(), &[("LOFT_NATIVE_LEAK_CHECK", "1")]);
    assert!(
        ok && stdout.contains(OK),
        "[--native/leak] the probe must run clean\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("stores not freed"),
        "the native backend must reach the same decision as the interpreter — one fact, \
         two translations (loft#981/#982)\nstderr:\n{stderr}"
    );
}

// ── The other direction: a genuine borrow must still never be freed ─────────────────

/// The control that keeps the fix a DECISION. Freeing on the borrow arm would trade a
/// leak for a use-after-free, which is strictly worse; `LOFT_POISON` overwrites a freed
/// store so the borrowed record answers `0xDEADBEEF` instead of passing on bytes that
/// merely happen to still be intact.
#[test]
fn split_return_poison_clean_interpret() {
    assert_cells_green("--interpret", &[("LOFT_POISON", "1")], "poison");
}

#[test]
fn split_return_poison_clean_native() {
    assert_cells_green("--native", &[("LOFT_POISON", "1")], "poison");
}

/// Strict store lifetime: a freed store stays dead, and any access through a reference
/// naming it is an error rather than a read of whatever now occupies the slot.
#[test]
fn split_return_strict_stores_interpret() {
    assert_cells_green("--interpret", &[("LOFT_STRICT_STORES", "1")], "strict");
}

// ── The harness control ─────────────────────────────────────────────────────────────

/// A script whose assertion is FALSE must fail on both backends. Without this, a green
/// cell above could mean the probe never ran its asserts at all.
#[test]
fn harness_can_fail() {
    let src = "fn main() {\n  assert(1 == 2, \"deliberately false\");\n  \
               print(\"981 split-ownership return OK\\n\");\n}\n";
    let path =
        std::env::temp_dir().join(format!("loft_981_cannotpass_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    for backend in ["--interpret", "--native"] {
        let (ok, stdout, _) = run(backend, &path, &[]);
        assert!(
            !ok && !stdout.contains(OK),
            "[{backend}] a false assertion must fail the script — otherwise the green \
             cells above prove nothing"
        );
    }
    let _ = fa::remove_file(&path);
}
