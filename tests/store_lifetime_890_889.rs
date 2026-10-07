// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#890 and loft#889 — two store-lifetime defects that a green suite could not see.
//!
//! Both scripts pass on value alone in an ordinary run: a freed store usually still holds
//! the bytes it held before, so the wrong answer arrives only once the allocator hands the
//! slot to somebody else. `LOFT_POISON=1` overwrites a freed store, which turns "usually
//! right" into "always wrong" — and it has to be set per-test, because the suite does not
//! run these files under poison and nothing would remind a later reader to.
//!
//! Three oracles, each answering a question the others cannot:
//!
//!   - **value**, both backends — the ordinary run, so a plain regression still reports.
//!   - **poison**, both backends — the read lands on a store somebody freed.
//!   - **leak** (`LOFT_NATIVE_LEAK_CHECK`) — the neighbour that catches the cheap fix.
//!     Both issues are about a free happening at the wrong time, and "never free it" ends
//!     the use-after-free while passing every value cell.
//!
//! [`a_false_assertion_fails_the_script`] is the control for the harness itself: without
//! it, a green run proves only that the script executed.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn script(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts")
        .join(name)
}

const I890: &str = "890-consumed-lift-double-free.loft";
const I890_OK: &str = "890 consumed-lift double free OK";
const I889: &str = "889-collection-through-a-call-s-field.loft";
const I889_OK: &str = "889 collection through a call's field OK";

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

fn assert_green(name: &str, ok_line: &str, backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, &script(name), env);
    assert!(
        ok && stdout.contains(ok_line),
        "[{backend}/{tag}] every cell of {name} must be green\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

// ── loft#890 — a lift freed the store its consuming op had already released ──────────

#[test]
fn consumed_lift_cells_interpret() {
    assert_green(I890, I890_OK, "--interpret", &[], "value");
}

#[test]
fn consumed_lift_cells_native() {
    assert_green(I890, I890_OK, "--native", &[], "value");
}

#[test]
fn consumed_lift_cells_poison_interpret() {
    assert_green(
        I890,
        I890_OK,
        "--interpret",
        &[("LOFT_POISON", "1")],
        "poison",
    );
}

/// The reported cell. `--native` recycled the freed slot for the return buffer, so the
/// function answered the poison pattern; the interpreter's allocator did not, which is the
/// only reason it looked correct.
#[test]
fn consumed_lift_cells_poison_native() {
    assert_green(I890, I890_OK, "--native", &[("LOFT_POISON", "1")], "poison");
}

#[test]
fn consumed_lift_stays_leak_free() {
    assert_leak_free(I890, I890_OK);
}

// ── loft#889 — a collection reached through a field of a call's result ───────────────

#[test]
fn call_field_cells_interpret() {
    assert_green(I889, I889_OK, "--interpret", &[], "value");
}

#[test]
fn call_field_cells_native() {
    assert_green(I889, I889_OK, "--native", &[], "value");
}

#[test]
fn call_field_cells_poison_interpret() {
    assert_green(
        I889,
        I889_OK,
        "--interpret",
        &[("LOFT_POISON", "1")],
        "poison",
    );
}

#[test]
fn call_field_cells_poison_native() {
    assert_green(I889, I889_OK, "--native", &[("LOFT_POISON", "1")], "poison");
}

/// Naming the container introduces a work-ref that HOLDS the call's result, so it must
/// still be released. The script's own round-over-round `store_memory()` check covers the
/// per-iteration half; this covers the whole program.
#[test]
fn call_field_stays_leak_free() {
    assert_leak_free(I889, I889_OK);
}

fn assert_leak_free(name: &str, ok_line: &str) {
    let (_ok, _out, stderr) = run(
        "--native",
        &script(name),
        &[("LOFT_NATIVE_LEAK_CHECK", "1")],
    );
    assert!(
        !stderr.to_lowercase().contains("not freed"),
        "[native] a store was not freed at exit in {name} — the fix traded a \
         use-after-free for a leak\n{stderr}"
    );
    assert_green(
        name,
        ok_line,
        "--native",
        &[("LOFT_NATIVE_LEAK_CHECK", "1")],
        "leak",
    );
}

// ── Control: the harness can fail ────────────────────────────────────────────────────

/// The smallest shape of loft#890 with the expected value deliberately wrong. If this
/// reports success, every green run above means only "the program executed".
#[test]
fn a_false_assertion_fails_the_script() {
    let src = "struct Y { q: integer, r: integer }\n\
fn ymake(n: integer) -> hash<Y[q, r]> {\n\
\x20 yv: hash<Y[q, r]> = [];\n\
\x20 for yi in 0..n { yv[yi + 7, 0] = Y { q: yi + 7, r: 0 }; }\n\
\x20 yv\n}\n\
fn ybound(n: integer) -> Y { yb = ymake(n); yb[7, 0] ?? Y { q: -1, r: -1 } }\n\
fn main() {\n\
\x20 assert(ybound(3).q == 999, \"deliberately wrong: it is 7\");\n\
\x20 println(\"890 consumed-lift double free OK\");\n}\n";
    let path = std::env::temp_dir().join(format!("loft_890_control_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    let (ok, stdout, _stderr) = run("--interpret", &path, &[]);
    let _ = fa::remove_file(&path);
    assert!(
        !(ok && stdout.contains(I890_OK)),
        "a false assertion must fail the script — the OK line is not self-validating\n{stdout}"
    );
}
