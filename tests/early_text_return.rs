// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#1338 — a text function delivers an owned result through its caller's hidden `&text`
//! buffer from EVERY `return`, not only from the block tail.
//!
//! The values were always right, on both backends, which is why nothing said anything: the
//! early return copied its text into a frame-local `__ret_N` String that the frame never
//! freed, and the caller read a view of the orphan.  The exit and assertion channels cannot
//! see that, so the leak half is scored on the interpreter's own text ledger
//! (`LOFT_TEXT_TIMELINE=1`, the loft#568 instrument): 106 orphaned buffers on the build this
//! guard was written against, none after.  The value half runs the same script on both
//! backends; its `d5` cell is the native silent-wrong the same census turned up — the
//! emitter took a USER `&text` parameter for the return buffer and wrote the returned text
//! into the caller's variable.

use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/1338-an-early-text-return-is-delivered-through-the-caller-buffer.loft")
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

/// The leak half.  The count is what this asserts, not the size: every cell orphaned one
/// 8-byte buffer per call, unbounded in a loop.
#[test]
fn an_early_text_return_orphans_no_buffer() {
    let path = script().to_string_lossy().into_owned();
    let (ok, stdout, stderr) = run(&["--interpret", &path], &[("LOFT_TEXT_TIMELINE", "1")]);
    assert!(
        ok && stdout.contains("1338 early text returns ok"),
        "the guard must run every cell on --interpret\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("NO text leak"),
        "an early `return <call>` / `return a ?? b` / `return t[0][0]`, and a tail that views \
         a local, must deliver through the caller's `&text` buffer — the frame-local `__ret_N` \
         copy is an orphan\nstderr:\n{stderr}"
    );
}

/// The value half, on both backends.  `d5` is the cell that fails on the control build's
/// `--native`: the caller's `&text` came back holding the returned text.
#[test]
fn both_backends_answer_the_same_and_leave_the_callers_text_alone() {
    let path = script().to_string_lossy().into_owned();
    for backend in ["--interpret", "--native"] {
        let (ok, stdout, stderr) = run(&[backend, &path], &[]);
        assert!(
            ok && stdout.contains("1338 early text returns ok"),
            "{backend}: every cell must answer its hand-computed value and a user `&text` \
             parameter must not be written by a return\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}
