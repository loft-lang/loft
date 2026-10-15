// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-TextBorrow`'s tested clause — PINNED on
//! `tests/scripts/a-match-binding-borrows-its-text-while-the-stores-differ.loft`.  A borrowed
//! text read after a growth of its store answers right until the store happens to move, so
//! this runs the cells where a stale borrow cannot hide — under `LOFT_POISON=1` (every growth
//! moves and scribbles) — in both switch states, and pins which bindings borrow, read off
//! `LOFT_TRACE_TEXT_BORROW`.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/a-match-binding-borrows-its-text-while-the-stores-differ.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_TESTED_BORROW")
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
        for switch in [None, Some(("LOFT_NO_TESTED_BORROW", "1"))] {
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
fn an_arm_writing_only_the_result_borrows_and_one_writing_its_parameter_copies() {
    let trace = |env: &[(&str, &str)]| -> String {
        let mut e = vec![("LOFT_TRACE_TEXT_BORROW", "1")];
        e.extend_from_slice(env);
        let out = loft(&["--native-release", "--native-emit", "/dev/null"], &e);
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let on = trace(&[]);
    assert!(
        on.contains(
            "n_enc: `_mv_s_1` borrows a parameter's text while its store is not the return buffer's"
        ),
        "enc's text arm borrows under the test:\n{on}"
    );
    assert!(
        on.contains(
            "n_tagged: `_mv_s_1` not tested either — a writer to a store the frame did not mint"
        ),
        "tagged's arm writes its parameter's store:\n{on}"
    );
    let off = trace(&[("LOFT_NO_TESTED_BORROW", "1")]);
    assert!(
        !off.contains("while its store is not the return buffer's"),
        "the switch removes the tested borrow:\n{off}"
    );
}
