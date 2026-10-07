// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 null-flow Phase 4 — `(N-Cast)`: a cast `as τ` is an assertion; the text parse folds in.
//!
//! Under `LOFT_NULLFLOW`, a BARE `text as τ` is a compile error (a parse can't be proven),
//! directing to the checked `as τ?` (value or null) or the assert-or-default `as τ ?? d`.
//! OFF keeps the DN3 auto-`τ?` parse. See float-null-domain-typing.md § Implementation plan.

use loft::file_access as fa;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run(body: &str, backend: &str, nullflow: bool, tag: &str) -> (bool, String, String) {
    run_inner(body, backend, nullflow, tag, false)
}

/// Same, with `--explain` — so a test can assert the FIX a diagnostic offers (@PLN131),
/// which is where "what to write instead" now lives.
fn run_explained(body: &str, backend: &str, nullflow: bool, tag: &str) -> (bool, String, String) {
    run_inner(body, backend, nullflow, tag, true)
}

fn run_inner(
    body: &str,
    backend: &str,
    nullflow: bool,
    tag: &str,
    explain: bool,
) -> (bool, String, String) {
    let script = std::env::temp_dir().join(format!("loft_nf4_{}_{tag}.loft", std::process::id()));
    fa::write(&script, body).expect("write script");
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_TIMEOUT", "120");
    if explain {
        cmd.arg("--explain");
    }
    // @PLN102 flip — the null-flow model is default-ON; the OFF case opts out with LOFT_NO_NULLFLOW.
    if nullflow {
        cmd.env_remove("LOFT_NO_NULLFLOW");
    } else {
        cmd.env("LOFT_NO_NULLFLOW", "1");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = fa::remove_file(&script);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const BARE: &str = "fn main() { s = \"3.14\"; x = s as float; print(\"r={x}\\n\"); }\n";
const COAL: &str = "fn main() { s = \"3.14\"; x = s as float ?? 0.0; print(\"r={x}\\n\"); }\n";
const CHECKED: &str = "fn main() { s = \"abc\"; x = s as float?; print(\"r={x ?? -1.0}\\n\"); }\n";

#[test]
fn bare_text_cast_off_is_auto_nullable() {
    let (ok, out, _e) = run(BARE, "--interpret", false, "bare_off");
    assert!(
        ok,
        "OFF: bare `text as float` stays the DN3 auto-nullable parse: {out}"
    );
    assert!(out.contains("r=3.14"));
}
#[test]
fn bare_text_cast_on_is_a_compile_error() {
    let (ok, _o, err) = run(BARE, "--interpret", true, "bare_on");
    assert!(!ok, "ON: a bare `text as float` must be a compile error");
    assert!(err.contains("may fail"), "stderr: {err}");
    // @PLN131 moved the second half of this assertion, and it is worth keeping rather than
    // dropping: "the error DIRECTS to the checked form" is the contract this module's header
    // states. The message now says only what is wrong, and `--explain` says what to write
    // instead — so the direction is asserted where it now lives.
    let (_ok, _o, explained) = run_explained(BARE, "--interpret", true, "bare_on_explain");
    assert!(
        explained.contains("as float?"),
        "the error must still direct to the checked cast, now via its fix line: {explained}"
    );
}
#[test]
fn text_cast_coalesce_on_ok_interpret() {
    let (ok, out, err) = run(COAL, "--interpret", true, "coal_i");
    assert!(
        ok,
        "ON: `as float ?? d` is the assert-or-default form; stderr: {err}"
    );
    assert!(out.contains("r=3.14"), "{out}");
}
#[test]
fn text_cast_coalesce_on_ok_native() {
    let (ok, out, err) = run(COAL, "--native", true, "coal_n");
    assert!(ok, "ON native: {err}");
    assert!(out.contains("r=3.14"), "{out}");
}
#[test]
fn text_cast_checked_on_yields_null_on_bad_parse() {
    let (ok, out, err) = run(CHECKED, "--interpret", true, "chk_i");
    assert!(ok, "ON: `as float?` is the checked cast; stderr: {err}");
    assert!(
        out.contains("r=-1"),
        "bad parse → null → ?? -1.0; out={out}"
    );
}

// @PLN102 — a nullable SCALAR source can take the CHECKED cast `as τ?`: the null rides in-band
// (NaN is the float null, C90) and the cast op propagates it, so `float? as integer?` resolves to
// the base `OpCast` instead of reporting "Unknown cast". A bare `float? as integer` still errors
// (DN5). This is what makes the "use `as integer?`" advice on a `float?` source actually work.
const NULLABLE_SRC_VALUE: &str =
    "fn main(){ a = \"5.7\" as float?; b = a as integer?; print(\"r={b ?? -99}\\n\"); }\n";
const NULLABLE_SRC_NULL: &str =
    "fn main(){ n = sqrt(-1.0); m = n as integer?; print(\"r={m ?? -1}\\n\"); }\n";
const NULLABLE_SRC_BARE: &str =
    "fn main(){ a = \"5.7\" as float?; b = a as integer; print(\"r={b}\\n\"); }\n";

#[test]
fn nullable_scalar_checked_cast_value_interpret() {
    let (ok, out, err) = run(NULLABLE_SRC_VALUE, "--interpret", true, "nsv_i");
    assert!(ok, "ON: `float? as integer?` must resolve; stderr: {err}");
    assert!(out.contains("r=5"), "5.7 → 5: {out}");
}
#[test]
fn nullable_scalar_checked_cast_value_native() {
    let (ok, out, err) = run(NULLABLE_SRC_VALUE, "--native", true, "nsv_n");
    assert!(ok, "ON native: {err}");
    assert!(out.contains("r=5"), "native 5.7 → 5: {out}");
}
#[test]
fn nullable_scalar_checked_cast_propagates_null() {
    let (ok, out, err) = run(NULLABLE_SRC_NULL, "--interpret", true, "nsn_i");
    assert!(
        ok,
        "ON: a null `float?` casts to null, not a value; stderr: {err}"
    );
    assert!(out.contains("r=-1"), "null → null → ?? -1: {out}");
}
#[test]
fn nullable_scalar_bare_cast_still_errors() {
    // The over-reach guard: only the CHECKED form is enabled; a bare cast keeps the DN5 error.
    let (ok, _o, err) = run(NULLABLE_SRC_BARE, "--interpret", true, "nsb_i");
    assert!(!ok, "ON: a bare `float? as integer` must still be rejected");
    assert!(
        err.contains("possibly-null") && err.contains("integer?"),
        "expected the DN5 error advising `as integer?`: {err}"
    );
}
