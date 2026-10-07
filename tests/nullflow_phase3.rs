// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 null-flow Phase 3 — `(N-Domain)` for floats.
//!
//! Step 3.1: float / single `/` and `%` fault on a zero divisor, so they type `τ?` (like
//! integer `/`) under `LOFT_NULLFLOW`; the `divisor_provably_nonzero` elision keeps
//! `x / 2.0` non-null. Observed via the Phase-1 store warning (ON but not OFF).

use loft::file_access as fa;

mod common;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `(success, warning_count, stdout)`.  The count is loft's OWN warnings — see
/// `common::loft_warnings` for why rustc's must not count.
fn run(body: &str, backend: &str, nullflow: bool, tag: &str) -> (bool, usize, String) {
    let name = format!("loft_nf3_{}_{tag}.loft", std::process::id());
    let script = std::env::temp_dir().join(&name);
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
    let stderr = String::from_utf8_lossy(&out.stderr);
    let warns = common::loft_warnings(&stderr, &name);
    (
        out.status.success(),
        warns,
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// float `/` a VARIABLE divisor — `float?` under nullflow.
const DIV_VAR: &str = "struct S { g: float }\n\
fn main() {\n  b = 0.0;\n  s = S { g: 0.0 };\n  s.g = 1.0 / b;\n  print(\"f={s.g}\\n\");\n}\n";
/// float `/` a CONSTANT divisor — non-null (provably non-zero).
const DIV_CONST: &str = "struct S { g: float }\n\
fn main() {\n  s = S { g: 0.0 };\n  s.g = 1.0 / 2.0;\n  print(\"f={s.g}\\n\");\n}\n";

#[test]
fn float_div_var_off_launders() {
    let (ok, warns, out) = run(DIV_VAR, "--interpret", false, "var_off");
    assert!(ok, "{out}");
    assert_eq!(warns, 0, "OFF: float / stays non-null");
}
#[test]
fn float_div_var_on_is_nullable_interpret() {
    let (ok, warns, out) = run(DIV_VAR, "--interpret", true, "var_on_i");
    assert!(ok, "{out}");
    assert_eq!(
        warns, 1,
        "ON: float / var is float?, so the store warns; out={out}"
    );
    // The TYPE is unchanged — `/` can still yield null, because `0.0 / 0.0` is NaN — so
    // the (N-Store) warning above still fires. Only the VALUE moved (loft#983): `1.0 /
    // 0.0` is IEEE `inf` wherever it lands, instead of `inf` inline and `null` once bound.
    assert!(out.contains("f=inf"), "out={out}");
}
#[test]
fn float_div_var_on_is_nullable_native() {
    let (ok, warns, out) = run(DIV_VAR, "--native", true, "var_on_n");
    assert!(ok, "{out}");
    assert_eq!(warns, 1, "ON native: float / var is float?; out={out}");
    assert!(out.contains("f=inf"), "out={out}");
}
#[test]
fn float_div_const_stays_non_null() {
    let (ok, warns, out) = run(DIV_CONST, "--interpret", true, "const_on");
    assert!(ok, "{out}");
    assert_eq!(
        warns, 0,
        "ON: a constant non-zero divisor is provably safe → non-null"
    );
    assert!(out.contains("f=0.5"));
}

/// A NAMED-constant divisor (and a cast of one) const-folds to its non-zero literal, so it is
/// provably safe just like a bare literal — closing the gap where `x / SFX_RATE as single` (real
/// graphics-lib code) spuriously typed `single?` and broke under the flip.
const DIV_NAMED_CONST: &str = "pub RATE = 44100;\nstruct S { g: single }\n\
fn main() {\n  s = S { g: 0.0f };\n  s.g = 1.0f / RATE as single;\n  print(\"f={s.g}\\n\");\n}\n";

#[test]
fn float_div_named_const_stays_non_null_interpret() {
    let (ok, warns, out) = run(DIV_NAMED_CONST, "--interpret", true, "named_i");
    assert!(ok, "{out}");
    assert_eq!(
        warns, 0,
        "ON: `1.0f / RATE as single` (RATE a nonzero constant) is provably safe → non-null; out={out}"
    );
}
#[test]
fn float_div_named_const_stays_non_null_native() {
    let (ok, warns, out) = run(DIV_NAMED_CONST, "--native", true, "named_n");
    assert!(ok, "{out}");
    assert_eq!(
        warns, 0,
        "ON native: named-const divisor is non-null; out={out}"
    );
}

/// `sqrt` of a negative — `float?` under nullflow (decl-gate), non-null when off.
const SQRT: &str = "struct S { g: float }\n\
fn main() {\n  x = -1.0;\n  s = S { g: 0.0 };\n  s.g = sqrt(x);\n  print(\"f={s.g}\\n\");\n}\n";

#[test]
fn sqrt_off_non_null_no_warning() {
    let (ok, warns, out) = run(SQRT, "--interpret", false, "sqrt_off");
    assert!(
        ok,
        "OFF: stdlib must load with sqrt stripped to non-null float: {out}"
    );
    assert_eq!(warns, 0, "OFF: sqrt returns non-null float");
}
#[test]
fn sqrt_on_nullable_warns_interpret() {
    let (ok, warns, out) = run(SQRT, "--interpret", true, "sqrt_on_i");
    assert!(ok, "{out}");
    assert_eq!(
        warns, 1,
        "ON: sqrt returns float?, the store warns; out={out}"
    );
    assert!(out.contains("f=null"));
}
#[test]
fn sqrt_on_nullable_warns_native() {
    let (ok, warns, out) = run(SQRT, "--native", true, "sqrt_on_n");
    assert!(ok, "{out}");
    assert_eq!(warns, 1, "ON native: sqrt returns float?; out={out}");
}

// --- 3.3/3.4: ln/log/pow nullable; exp TOTAL stays non-null.  3.5: constant-in-domain elision. ---

fn store(expr: &str) -> String {
    format!(
        "struct S {{ g: float }}\nfn main() {{ s = S {{ g: 0.0 }}; s.g = {expr}; print(\"r={{s.g}}\\n\"); }}\n"
    )
}

#[test]
fn exp_total_stays_non_null_even_on() {
    // exp is defined off the raw non-null OpPow, so it never gets a spurious `?`.
    let (ok, warns, out) = run(&store("exp(2.0)"), "--interpret", true, "exp_on");
    assert!(ok, "{out}");
    assert_eq!(
        warns, 0,
        "exp is total → non-null, no store warning; out={out}"
    );
    assert!(out.contains("r=7.389"), "{out}");
}
#[test]
fn ln_nullable_warns_on() {
    let (ok, warns, out) = run(
        &store("ln(x)").replace("s = S", "x = -1.0; s = S"),
        "--interpret",
        true,
        "ln_on",
    );
    assert!(ok, "{out}");
    assert_eq!(warns, 1, "ln(var) is float? → store warns; out={out}");
}
#[test]
fn pow_variable_nullable_warns_on() {
    let body = store("pow(b, e)").replace("s = S", "b = -2.0; e = 0.5; s = S");
    let (ok, warns, out) = run(&body, "--interpret", true, "pow_on");
    assert!(ok, "{out}");
    assert_eq!(warns, 1, "pow(var,var) is float? → store warns; out={out}");
}
#[test]
fn elision_const_in_domain_is_non_null() {
    for (expr, tag) in [
        ("sqrt(4.0)", "e1"),
        ("pow(2.0, 3.0)", "e2"),
        ("pow(-2.0, 3.0)", "e3"),
        ("ln(2.0)", "e4"),
        ("asin(0.5)", "e5"),
    ] {
        let (ok, warns, out) = run(&store(expr), "--interpret", true, tag);
        assert!(ok, "{expr}: {out}");
        assert_eq!(
            warns, 0,
            "{expr} is provably in-domain → non-null, no warning; out={out}"
        );
    }
}
#[test]
fn elision_out_of_domain_stays_nullable() {
    let (ok, warns, out) = run(&store("sqrt(-1.0)"), "--interpret", true, "oob");
    assert!(ok, "{out}");
    assert_eq!(
        warns, 1,
        "sqrt(-1.0) is out of domain → stays float?; out={out}"
    );
}
#[test]
fn exp_ln_load_off() {
    // OFF: the flipped stdlib must still load and behave non-null.
    let (ok, warns, _out) = run(&store("exp(1.0)"), "--interpret", false, "off_load");
    assert!(ok, "OFF: stdlib must load with the flipped decls stripped");
    assert_eq!(warns, 0);
}
