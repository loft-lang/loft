// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#1357 — every text buffer a frame mints is released, by the frame or by the caller
//! it is delivered to.
//!
//! The values were always right on both backends, which is why nothing said anything: a
//! lambda's `??` return, a nullable text local, a generic's loop-variable return, a par
//! loop's text discard, a tail that reads its own buffer and a `??` temp consumed by a scalar
//! each left one `String` behind per call.  The exit and assertion channels cannot see that,
//! so the leak half is scored on the interpreter's own text ledger (`LOFT_TEXT_TIMELINE=1`):
//! it counted 99 orphaned buffers on the build this guard was written against, none after.
//! The value half runs the same script on both backends.  The third test drives a
//! frame-yielding `main` through the TEST RUNNER, which used to score the first frame as the
//! whole test and abandon the frame's buffers.

use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/1357-every-text-buffer-a-frame-mints-is-released.loft")
}

/// Run `loft` with `args` plus extra env; return `(exit-ok, stdout, stderr)`.
fn run(args: &[&str], env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.args(args)
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

/// The leak half: the ledger is process-wide, so a `par` worker's buffers count too.
#[test]
fn every_text_buffer_a_frame_mints_is_released() {
    let path = script().to_string_lossy().into_owned();
    let (ok, stdout, stderr) = run(&["--interpret", &path], &[("LOFT_TEXT_TIMELINE", "1")]);
    assert!(
        ok && stdout.contains("1357 text buffers ok"),
        "the guard must run every cell on --interpret\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("NO text leak"),
        "a text buffer a frame minted was never released — a delivery site, a scope-exit \
         free or the `??` temp's consumer-free did not reach one of the guard's shapes\
         \nstderr:\n{stderr}"
    );
}

/// The value half, on both backends.
#[test]
fn both_backends_answer_the_same() {
    let path = script().to_string_lossy().into_owned();
    for backend in ["--interpret", "--native"] {
        let (ok, stdout, stderr) = run(&[backend, &path], &[]);
        assert!(
            ok && stdout.contains("1357 text buffers ok"),
            "{backend}: every cell must answer its hand-computed value\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

/// A `main` that yields frames is resumed to its end under `--tests`, as the CLI resumes it:
/// the runner used to return after the first `yield_frame` and leave the frame's formatted
/// texts unreleased (the release sweep runs every script under `--tests`, so this was the
/// one file it kept reporting).
#[test]
fn a_yielding_main_finishes_under_the_test_runner() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/85-yield-resume.loft")
        .to_string_lossy()
        .into_owned();
    let (ok, stdout, stderr) = run(
        &["--interpret", "--tests", &path],
        &[("LOFT_TEXT_TIMELINE", "1")],
    );
    assert!(
        ok && stdout.contains("test result: ok"),
        "the yielding program must run to its end under the runner\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("NO text leak"),
        "a frame abandoned after its first yield leaves its formatted texts behind\nstderr:\n{stderr}"
    );
}
