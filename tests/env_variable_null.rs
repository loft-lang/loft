// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1302 — `env_variable` answers null for a variable that is not set.
//!
//! `default/02_files.loft` promises *"the value of the environment variable name, or null if
//! it is not set"*, and the sentence reaches the published Standard Library.  It never did:
//! `Stores::os_variable` ended in `.unwrap_or_default()`, so an unset variable and one set to
//! the empty string were the same answer, the `== null` test the doc invites could not fire,
//! and a program could not tell a variable it must be given from one deliberately blanked.
//!
//! **This runner IS the guard**, not a wrapper around one.  The distinction under test is
//! between three states of the environment — absent, set, set-empty — and a `.loft` program
//! cannot put its own process into them.  So the cells live in
//! `tests/fixtures/1302-env-variable-answers-null-when-unset.loft` (deliberately NOT under
//! `tests/scripts/`, which `loft_suite` sweeps with the ambient environment) and this file
//! supplies the three states on BOTH backends.
//!
//! [`the_harness_can_fail`] is the control for the harness itself: with the variables not
//! supplied, the SET rows must FAIL.  Without it a runner that silently forgot to pass the
//! environment would report green over a program measuring nothing.

use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/1302-env-variable-answers-null-when-unset.loft")
}

/// The three states the guard measures.  `LOFT_1302_ABSENT` is deliberately absent — and
/// `env_remove`d rather than merely unmentioned, so an outer environment that happens to
/// define it cannot turn the first cell green for the wrong reason.
fn run(backend: &str, with_env: bool) -> (bool, String, String) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg(probe())
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_1302_ABSENT");
    if with_env {
        cmd.env("LOFT_1302_SET", "hello").env("LOFT_1302_EMPTY", "");
    } else {
        cmd.env_remove("LOFT_1302_SET")
            .env_remove("LOFT_1302_EMPTY");
    }
    let out = cmd.output().expect("failed to invoke the loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const OK: &str = "1302 env ok";

fn assert_green(backend: &str) {
    let (ok, stdout, stderr) = run(backend, true);
    assert!(
        ok && stdout.contains(OK),
        "[{backend}] every env-variable cell must be green\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn env_variable_null_cells_are_green_on_the_interpreter() {
    assert_green("--interpret");
}

#[test]
fn env_variable_null_cells_are_green_on_native() {
    assert_green("--native");
}

/// The harness control.  Run the same program WITHOUT the two variables and the SET rows must
/// fail — which is what says the passes above measure the environment this file supplies and
/// not whatever the machine happened to export.
#[test]
fn the_harness_can_fail() {
    let (ok, stdout, stderr) = run("--interpret", false);
    assert!(
        !ok && !stdout.contains(OK),
        "the guard must FAIL when the environment is not supplied, or it is measuring \
         nothing\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
