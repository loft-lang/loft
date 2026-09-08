// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A nullable local bound from a view-returning call gets ONE lowering on both backends,
//! and that lowering never mints a store nobody owns.
//!
//! The script cells in `tests/scripts/1468-…loft` answer the VALUE question, and the suite
//! already runs them on both backends. This file exists for the other channel: the defect
//! this guard was written against was a **leak** on `--native` and a wrong value only on the
//! one shape the corpus does not contain, and the corpus runner does not arm store checks.
//! Measured — the emit diff moved eight corpus files, all eight passed on both backends
//! before and after, and three of them (1181, 1202, 1414) were leaking under
//! `LOFT_STRICT_STORES=1` the whole time with nothing reporting it.
//!
//! So a value-only gate cannot fail against the defect, which is the property a guard has to
//! have. `LOFT_STRICT_STORES=1` is the witness, and it is a separate cell rather than the
//! whole gate because it also implies `LOFT_NO_SLOT_REUSE` — a program can read correctly
//! with an over-free present once reuse is off, so the plain cells stay.

use std::path::PathBuf;
use std::process::Command;

fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/1468-a-nullable-call-result-binds-without-an-ownerless-copy.loft")
}

fn run(mode: &str, envs: &[(&str, &str)]) -> (bool, String) {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg(mode).arg(probe()).env("LOFT_TIMEOUT", "120");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run loft");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn assert_green(mode: &str, envs: &[(&str, &str)], label: &str) {
    let (ok, text) = run(mode, envs);
    assert!(
        ok && text.contains("1468 OK"),
        "{label} cells failed on {mode}:\n{text}"
    );
}

#[test]
fn nullable_first_bind_cells_interpret() {
    assert_green("--interpret", &[], "value");
}

#[test]
fn nullable_first_bind_cells_native() {
    assert_green("--native", &[], "value");
}

/// The channel the defect actually moved. A copy emitted at a first bind whose TYPE names
/// the argument's dep has no owner — `owns_freeable_store` emits no free for it — so the
/// ten-iteration loop in the script orphans one store per call. The value cells beside it
/// stayed green throughout, on both backends.
#[test]
fn nullable_first_bind_leaks_nothing_native() {
    let (_ok, text) = run("--native", &[("LOFT_STRICT_STORES", "1")]);
    assert!(
        !text.contains("NEVER FREED"),
        "a nullable first bind from a view-returning call orphaned a store:\n{text}"
    );
}

#[test]
fn nullable_first_bind_leaks_nothing_interpret() {
    let (_ok, text) = run("--interpret", &[("LOFT_STRICT_STORES", "1")]);
    assert!(
        !text.contains("NEVER FREED"),
        "a nullable first bind from a view-returning call orphaned a store:\n{text}"
    );
}

/// The two backends must agree about the SOURCE, whichever answer loft#1468 settles on.
/// The script asserts each backend against `{93, 7}`; only a cross-backend comparison can
/// see them landing on different members of that set, which is exactly what the defect did
/// (`--native` 7, `--interpret` 99 on the filed repro).
#[test]
fn nullable_first_bind_agrees_across_backends() {
    fn source(text: &str) -> String {
        text.lines()
            .find_map(|l| l.trim().strip_prefix("1468 OK source=").map(str::to_string))
            .unwrap_or_else(|| panic!("the guard printed no `source=` line:\n{text}"))
    }
    let (_a, interp) = run("--interpret", &[]);
    let (_b, native) = run("--native", &[]);
    assert_eq!(
        source(&interp),
        source(&native),
        "the two backends read a different source after a nullable first bind from a \
         view-returning call — one copied and the other aliased"
    );
}
