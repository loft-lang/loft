// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-TextBorrow`'s call clause — PINNED on
//! `tests/scripts/a-call-text-is-borrowed-from-its-work-buffer.loft`.  A borrowed
//! text read after a growth of its store answers right until the store happens to move, so
//! this runs the cells where a stale borrow cannot hide — under `LOFT_POISON=1` (every growth
//! moves and scribbles) — in both switch states, and pins which bindings borrow, read off
//! `LOFT_TRACE_TEXT_BORROW`.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/a-call-text-is-borrowed-from-its-work-buffer.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_CALL_TEXT_BORROW")
        .env_remove("LOFT_NO_TEXT_BORROW")
        .env_remove("LOFT_TRACE_TEXT_BORROW");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

#[test]
fn the_cells_hold_under_poison_in_both_switch_states() {
    for mode in ["--native", "--native-release"] {
        for switch in [None, Some(("LOFT_NO_CALL_TEXT_BORROW", "1"))] {
            let mut env = vec![("LOFT_POISON", "1")];
            env.extend(switch);
            let out = loft(&[mode], &env);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.trim_end().ends_with("ok"),
                "{mode} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}

#[test]
fn a_call_answering_its_work_text_is_borrowed_and_one_that_may_answer_a_parameter_is_not() {
    let trace = |env: &[(&str, &str)]| -> String {
        let mut e = vec![("LOFT_TRACE_TEXT_BORROW", "1")];
        e.extend_from_slice(env);
        let out = loft(&["--native-release", "--native-emit", "/dev/null"], &e);
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let on = trace(&[]);
    assert!(
        on.contains("n_runs: `s` borrows a format's or a call's work text"),
        "runs borrows its cut:\n{on}"
    );
    assert!(
        !on.contains("n_tagged: `s` borrows"),
        "tagged's pick may answer a parameter's text:\n{on}"
    );
    let off = trace(&[("LOFT_NO_CALL_TEXT_BORROW", "1")]);
    assert!(
        !off.contains("n_runs: `s` borrows"),
        "the switch removes the call borrow:\n{off}"
    );
}
