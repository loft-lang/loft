// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 null-flow Phase 1 — the `(N-Store)` warn/error split, end-to-end.
//!
//! Under `LOFT_NULLFLOW`, storing a `τ?` into a non-null slot is a WARNING (the store
//! proceeds and the slot holds the null sentinel, which reads back as `null`) for a type
//! that reserves its null DISTINCTLY in the non-null form — full `integer`, `float`, … —
//! and stays a hard ERROR only for a NARROW width (`u8`…`u32`), whose non-null form spends
//! its whole width on real values so a null has nowhere to go. OFF keeps the current
//! uniform hard error. See
//! `doc/claude/plans/102-stability-contract/float-null-domain-typing.md` § Implementation plan.

use loft::file_access as fa;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run `body` on `backend` (`--interpret` / `--native`), optionally with `LOFT_NULLFLOW`.
/// Returns `(success, stdout, stderr)`.  `tag` keeps the temp script unique across the
/// parallel tests.
fn run(body: &str, backend: &str, nullflow: bool, tag: &str) -> (bool, String, String) {
    let script = std::env::temp_dir().join(format!("loft_nf_{}_{tag}.loft", std::process::id()));
    fa::write(&script, body).expect("write script");
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_TIMEOUT", "120");
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

/// `10 / y` (y a variable) is `integer?`; storing it into the non-null `integer` field `f`
/// is the DN3 store site.  `y == 0` makes the value actually null so the slot's read is observable.
const FULL: &str = "struct S { f: integer }\n\
fn main() {\n  y = 0;\n  s = S { f: 0 };\n  s.f = 10 / y;\n  print(\"f={s.f}\\n\");\n}\n";

/// Same store into a NARROW `u8` field — no room for the null sentinel.
const NARROW: &str = "struct N { x: u8 }\n\
fn main() {\n  y = 2;\n  n = N { x: 0 };\n  n.x = 10 / y;\n  print(\"x={n.x}\\n\");\n}\n";

#[test]
fn off_full_integer_stays_hard_error() {
    let (ok, _out, err) = run(FULL, "--interpret", false, "off_full");
    assert!(!ok, "OFF: a nullable store must stay a hard error");
    assert!(err.contains("cannot be stored"), "OFF stderr: {err}");
}

#[test]
fn on_full_integer_warns_and_runs_interpret() {
    let (ok, out, err) = run(FULL, "--interpret", true, "on_full_i");
    assert!(
        ok,
        "ON: the full-integer store should compile + run; stderr: {err}"
    );
    assert!(
        err.contains("is stored into") && !err.contains("cannot be stored"),
        "expected a WARNING, not an error: {err}"
    );
    assert!(out.contains("f=null"), "the slot should hold null: {out}");
}

#[test]
fn on_full_integer_warns_and_runs_native() {
    let (ok, out, err) = run(FULL, "--native", true, "on_full_n");
    assert!(
        ok,
        "ON native: the store should compile + run; stderr: {err}"
    );
    assert!(
        out.contains("f=null"),
        "native: the slot should hold null: {out}"
    );
}

#[test]
fn on_narrow_u8_stays_hard_error() {
    let (ok, _out, err) = run(NARROW, "--interpret", true, "on_narrow");
    assert!(
        !ok,
        "ON: a narrow store must stay a hard error (no room for the null)"
    );
    assert!(err.contains("cannot be stored"), "narrow stderr: {err}");
}
