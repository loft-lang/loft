// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#980 — a struct-enum field access answers for the variant the value HOLDS.
//!
//! `c.field` resolved at COMPILE time to the first variant declaring the name and read
//! that offset whatever the tag said, so `a.n` on an `Anon` answered `Anon.k`'s value as
//! if it were `Named.n`, and `a.label = "x"` wrote into a record whose tag still said
//! `Anon` — after which `match` still reported `Anon`. Both backends, exit 0, and until
//! `variant-field-unchecked` shipped, no diagnostic either.
//!
//! Direct payload access STAYS: C89 decided permanently that enum payloads are named
//! fields you read straight, with matching for DISPATCH and never for extraction. What
//! this pins is the CHECK — a partial access reads null (the C80 sentinel, the same
//! answer a hash miss and an out-of-range index give) and a write to a field the value
//! does not have is suppressed.
//!
//! [`tests/variant_field.rs`] pins the DIAGNOSTIC; this pins the BEHAVIOUR. They are
//! separate because the diagnostic stays even where the access is now answerable — a
//! suppressed write is a lost write, which is the two-tier rule's own gating example.
//!
//! [`harness_can_fail`] is the control for the harness itself.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/980-variant-field-answers-its-own-variant.loft")
}

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

const OK: &str = "980 variant field OK";

fn assert_cells_green(backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, &probe(), env);
    assert!(
        ok && stdout.contains(OK),
        "[{backend}/{tag}] every variant-field cell must be green\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn variant_field_cells_interpret() {
    assert_cells_green("--interpret", &[], "value");
}

#[test]
fn variant_field_cells_native() {
    assert_cells_green("--native", &[], "value");
}

/// The guard is SEMANTICS, and semantics must not depend on a diagnostic switch.
/// `LOFT_NO_VARIANT_FIELD=1` silences the message; the tag check stays.
#[test]
fn the_diagnostic_opt_out_does_not_change_the_answer() {
    let (ok, stdout, stderr) = run("--interpret", &probe(), &[("LOFT_NO_VARIANT_FIELD", "1")]);
    assert!(
        ok && stdout.contains(OK),
        "silencing the warning must not restore the unchecked access — a diagnostic \
         switch that moves the answer is not a diagnostic switch (loft#980)\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("variant-field-unchecked"),
        "…and it must still silence the message\nstderr:\n{stderr}"
    );
}

#[test]
fn harness_can_fail() {
    let src = "fn main() {\n  assert(1 == 2, \"deliberately false\");\n  \
               print(\"980 variant field OK\\n\");\n}\n";
    let path =
        std::env::temp_dir().join(format!("loft_980_cannotpass_{}.loft", std::process::id()));
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
