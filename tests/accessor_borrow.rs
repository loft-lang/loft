// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#974 — a record an ACCESSOR hands back out of one of its parameters is a VIEW
//! into that parameter's store, and the signature must say so.
//!
//! `it = b.items[k]?` inline types `it` as `ref(Item)["b"]`, so scope exit leaves it
//! alone. The same lookup behind `fn get(b: Bag, k: text) -> Item?` declared
//! `Item?` with an EMPTY dep list — no dep at all — so the caller read the result
//! as OWNED and emitted `OpFreeRef(it)`, releasing a store the CALLER's `b` still owned.
//! The next unrelated allocation claimed the recycled slot, and every later lookup
//! answered out of it: `2, 0, 0` where the inline spelling reads `2, 2, 2`. Silent, both
//! backends, and it survived a 1361-test consumer suite.
//!
//! Two oracles, answering different questions:
//!
//!   - **Static** ([`an_accessors_returned_view_names_its_parameter`]). The declared
//!     return type must NAME the parameter it borrows. Deterministic: it does not depend
//!     on whether a freed slot happened to be reused, which is exactly what let this
//!     through every gate the project runs (`LOFT_NO_SLOT_REUSE=1` reads correctly WITH
//!     the defect present).
//!   - **Behavioural** (the script cells). `tests/scripts/974-…loft` on both backends,
//!     plus a strict-store run — the shape is calibrated so it FAILS against the defect
//!     on each backend, which the first version of it did not (see the ⚠ in the script).
//!
//! [`harness_can_fail`] is the control for the harness itself: a script whose assertion
//! is deliberately false must be reported as a failure.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/974-accessor-returned-record-borrows-its-container.loft")
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

const OK: &str = "974 accessor borrow OK";

fn assert_cells_green(backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, &probe(), env);
    assert!(
        ok && stdout.contains(OK),
        "[{backend}/{tag}] every accessor-borrow cell must be green\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// The smallest program with the defect. Deliberately ONE accessor: the file-wide probe
/// has several shapes, and a static assertion satisfied by a neighbouring function would
/// mask a regression in the one under test.
const MINIMAL: &str = "struct Y974 { name: text, limbs: vector<float> }\n\
struct B974 { items: hash<Y974[name]> }\n\
fn y974_get(b: B974, n: text) -> Y974? { b.items[n] }\n\
fn main() {\n\
\x20 b = B974 { items: [] };\n\
\x20 one = Y974 { name: \"one\", limbs: [] };\n\
\x20 one.limbs += [1.0];\n\
\x20 b.items += [one];\n\
\x20 it = y974_get(b, \"one\")?;\n\
\x20 println(\"{len(it.limbs)}\");\n\
}\n";

fn write_temp(tag: &str, src: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("loft_974_{tag}_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    path
}

// ── Static: the signature names what the return borrows ─────────────────────────────

/// The deterministic half, and the one that states the invariant: a returned view of a
/// parameter must appear in the declared return deps.
///
/// `Y974?` with an EMPTY dep list — the shape before the fix — is a return type that claims
/// to own what it points at. The caller believes it, frees the caller's own record at scope
/// exit, and nothing anywhere reports a thing.
///
/// The dump renders a dep list as the VARIABLE NAMES it holds, so the assertion below reads
/// the parameter's own name out of the signature rather than an attribute index.
#[test]
fn an_accessors_returned_view_names_its_parameter() {
    let path = write_temp("static", MINIMAL);
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("introspect")
        .arg(&path)
        .env("LOFT_TIMEOUT", "300")
        .output()
        .expect("failed to invoke loft introspect");
    let _ = fa::remove_file(&path);
    assert!(out.status.success(), "introspect must exit 0");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    let sig = text
        .lines()
        .find(|l| l.starts_with("fn n_y974_get("))
        .unwrap_or_default()
        .to_string();
    let ret = sig.split("->").nth(1).unwrap_or_default().to_string();
    assert!(
        ret.contains("[\"b\"]"),
        "the accessor's return must name the parameter it is a view of (`b`) — \
         an empty dep list reads as OWNED at the call site, and the caller then frees a \
         store it does not own (loft#974)\nsignature: {sig}\n{text}"
    );
    assert!(
        ret.contains('?'),
        "and it must still be NULLABLE — the borrow fact is about the storage, the `?` \
         about the value, and re-typing one must not drop the other\nsignature: {sig}"
    );

    // The other end of the same fact: the caller must not free what it borrowed.
    let read = text
        .split("fn n_main(")
        .nth(1)
        .unwrap_or_default()
        .split("}#block")
        .next()
        .unwrap_or_default()
        .to_string();
    assert!(
        !read.contains("OpFreeRef(it("),
        "the caller must not free a borrowed record at scope exit (loft#974)\n{read}"
    );
}

// ── Value, both backends ────────────────────────────────────────────────────────────

#[test]
fn accessor_cells_interpret() {
    assert_cells_green("--interpret", &[], "value");
}

#[test]
fn accessor_cells_native() {
    assert_cells_green("--native", &[], "value");
}

/// Strict store lifetime: a freed store stays dead and any access through a reference
/// naming it is an error. It also implies `LOFT_NO_SLOT_REUSE`, which is why it is a
/// SEPARATE cell and not the whole gate — with reuse off, this program answered
/// correctly WITH the defect present.
#[test]
fn accessor_cells_strict_stores_interpret() {
    assert_cells_green("--interpret", &[("LOFT_STRICT_STORES", "1")], "strict");
}

/// A fix that stopped the over-free by never freeing anything would pass every cell
/// above and leak instead; this is the neighbour that catches it.
#[test]
fn accessor_cells_leak_clean_native() {
    let (_ok, _out, stderr) = run("--native", &probe(), &[("LOFT_NATIVE_LEAK_CHECK", "1")]);
    assert!(
        !stderr.to_lowercase().contains("not freed"),
        "[native] a store was not freed at exit — the borrow fix must not turn an \
         over-free into a leak\n{stderr}"
    );
}

// ── The control for the harness ─────────────────────────────────────────────────────

/// A script whose assertion is deliberately false must be REPORTED as a failure —
/// otherwise "the cells printed OK" proves only that the file ran.
#[test]
fn harness_can_fail() {
    let path = write_temp(
        "canfail",
        "fn main() { assert(1 == 2, \"deliberate\"); print(\"974 accessor borrow OK\\n\"); }\n",
    );
    let (ok, stdout, _stderr) = run("--interpret", &path, &[]);
    let _ = fa::remove_file(&path);
    assert!(
        !(ok && stdout.contains(OK)),
        "the harness must report a failing script as failing"
    );
}
