// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 null-flow Phase 2 — `(N-Prop)`: nullability propagates through arithmetic.
//!
//! Under `LOFT_NULLFLOW`, a binary op with a nullable operand yields a nullable result
//! (`integer? + integer → integer?`, `float - float? → float?`), because the runtime already
//! carries the null sentinel / NaN through. The propagation is observed as a Phase-1 store
//! WARNING that appears ON but not OFF. C85 is the complement — two non-null operands stay
//! non-null (overflow is a produced sentinel, not a propagated input). See
//! `doc/claude/plans/102-stability-contract/float-null-domain-typing.md` § Implementation plan.

use loft::file_access as fa;

mod common;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `(success, stdout, warning_count)`.  `tag` keeps the temp script unique across parallel tests.
/// The count is loft's OWN warnings — see `common::loft_warnings` for why rustc's must not count.
fn run(body: &str, backend: &str, nullflow: bool, tag: &str) -> (bool, String, usize) {
    let name = format!("loft_nf2_{}_{tag}.loft", std::process::id());
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
    let script_removed = fa::remove_file(&script).is_ok();
    let _ = script_removed;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let warns = common::loft_warnings(&stderr, &name);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        warns,
    )
}

/// `n = 10/y` is `integer?`; `x = n + 1` must stay `integer?` (N-Prop); storing `x` into the
/// non-null field `f` is then a Phase-1 warning.  `y = 2` so the value is a real `6` (the store
/// still completes) — the point is the TYPE, observed via the warning.
const PROP_INT: &str = "struct S { f: integer }\n\
fn main() {\n  y = 2;\n  n = 10 / y;\n  x = n + 1;\n  s = S { f: 0 };\n  s.f = x;\n  print(\"f={s.f}\\n\");\n}\n";

/// `1.0 - x` with `x: float?` must stay `float?` (N-Prop on the RIGHT operand).
const PROP_FLOAT: &str = "struct S { g: float }\n\
fn main() {\n  x: float? = null;\n  m = 1.0 - x;\n  s = S { g: 0.0 };\n  s.g = m;\n  print(\"g={s.g}\\n\");\n}\n";

/// Two non-null operands — C85: the result stays non-null, so NO store warning.
const C85: &str = "struct S { f: integer }\n\
fn main() {\n  a = 3; b = 4;\n  s = S { f: 0 };\n  s.f = a * b;\n  print(\"f={s.f}\\n\");\n}\n";

#[test]
fn prop_int_off_launders_no_warning() {
    let (ok, out, warns) = run(PROP_INT, "--interpret", false, "int_off");
    assert!(ok, "OFF should compile+run: {out}");
    assert_eq!(
        warns, 0,
        "OFF: n+1 launders to non-null, so the store must NOT warn"
    );
    assert!(out.contains("f=6"));
}

#[test]
fn prop_int_on_propagates_and_warns_interpret() {
    let (ok, out, warns) = run(PROP_INT, "--interpret", true, "int_on_i");
    assert!(ok, "ON should compile+run: {out}");
    assert_eq!(
        warns, 1,
        "ON: n+1 is integer? (N-Prop), so the store must warn once"
    );
    assert!(out.contains("f=6"), "value still flows: {out}");
}

#[test]
fn prop_int_on_propagates_and_warns_native() {
    let (ok, out, warns) = run(PROP_INT, "--native", true, "int_on_n");
    assert!(ok, "ON native: {out}");
    assert_eq!(
        warns, 1,
        "ON native: the store must warn once (N-Prop): {out}"
    );
    assert!(out.contains("f=6"));
}

#[test]
fn prop_float_on_right_operand_propagates() {
    let (ok, out, warns) = run(PROP_FLOAT, "--interpret", true, "flt_on");
    assert!(ok, "ON: {out}");
    assert_eq!(
        warns, 1,
        "ON: 1.0 - float? is float? (N-Prop on the right operand)"
    );
    assert!(out.contains("g=null"), "the null flows through: {out}");
}

/// The counting oracle itself, against the transcript shape that broke it.
///
/// Every warning assertion in this suite is only as good as its attribution: a `--native` run
/// relays rustc's stderr verbatim, so a host whose toolchain warns inflates the count and fails
/// a test about loft's type system.  That is exactly what `windows-latest` hit — an MSVC linker
/// warning plus rustc's summary line made a 1-warning program report 3.  Feed the oracle both
/// renderings of a loft warning surrounded by rustc's, and prove it can report zero.
#[test]
fn oracle_counts_loft_warnings_not_the_toolchains() {
    let script = "loft_nf2_1234_probe.loft";
    let rustc_noise = "warning: linker stderr: LINK : warning LNK4044: unrecognized option \
                       '/Wl,--allow-multiple-definition'; ignored\n\
                       warning: 1 warning emitted\n\
                       warning: unused variable: `x`\n \
                       --> /tmp/loft_native_1234.rs:9:5\n";
    let pretty = format!(
        "{rustc_noise}warning: a nullable `integer?` is stored into the assignment target of \
         the non-null type `integer`\n  \
         --> /tmp/{script}:7:11\n  |\n7 |   s.f = x;\n  |           ^\n"
    );
    assert_eq!(
        common::loft_warnings(&pretty, script),
        1,
        "pretty: only the warning whose `-->` names the script counts"
    );
    let compact =
        format!("{rustc_noise}Warning: a nullable `integer?` is stored at /tmp/{script}:7:11\n");
    assert_eq!(
        common::loft_warnings(&compact, script),
        1,
        "compact (LOFT_ERRORS=compact): the location rides on the header line"
    );
    assert_eq!(
        common::loft_warnings(rustc_noise, script),
        0,
        "the oracle must be able to report zero — rustc's warnings are never loft's"
    );
}

#[test]
fn c85_non_null_arithmetic_stays_non_null() {
    let (ok, out, warns) = run(C85, "--interpret", true, "c85");
    assert!(ok, "ON: {out}");
    assert_eq!(
        warns, 0,
        "C85: two non-null operands → non-null result, no store warning"
    );
    assert!(out.contains("f=12"));
}
