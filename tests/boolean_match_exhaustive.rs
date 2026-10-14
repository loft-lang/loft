// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#1343 — a `match` on a boolean whose arms spell out `true` and `false` is exhaustive,
//! so it warns nothing.
//!
//! The values were always right; the defect was a WARNING (nullable-into-non-null on the
//! function's return) that gates a library's CI under `LOFT_DENY_WARNINGS=1`.  No corpus
//! channel scores a diagnostic that must NOT fire, so this test reads the warning stream.

use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/1343-a-boolean-match-with-both-arms-is-exhaustive.loft")
}

fn run(args: &[&str]) -> (bool, String, String) {
    let out = loft::platform::process::harness_command(loft_bin())
        .args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Both backends: the values, and an EMPTY warning stream.  The control build printed
/// three `warning: a nullable text? is stored into the return value …` lines here.
#[test]
fn a_boolean_match_with_both_arms_warns_nothing() {
    let path = script().to_string_lossy().into_owned();
    for backend in ["--interpret", "--native"] {
        let (ok, stdout, stderr) = run(&[backend, &path]);
        assert!(
            ok && stdout.contains("1343 boolean match exhaustive ok"),
            "{backend}: every cell must answer its value\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        let warnings: Vec<&str> = stderr
            .lines()
            .filter(|l| l.starts_with("warning"))
            .collect();
        assert!(
            warnings.is_empty(),
            "{backend}: a boolean match with both arms is exhaustive and must not warn — \
             got:\n{}",
            warnings.join("\n")
        );
    }
}
