// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-TextBorrow`'s versioned clause — PINNED on
//! `tests/scripts/a-walk-borrows-its-word-in-the-versioned-copy.loft`.  A borrowed word read
//! after a growth of its store answers right until the store happens to move, so this runs
//! the cells where a stale borrow cannot hide — under `LOFT_HOIST_VERIFY=1` (every borrow
//! re-read at its release) and `LOFT_POISON=1` (every growth moves and scribbles) — with the
//! versioned copy and the borrow each switched off, and pins which walks borrow, read off
//! `LOFT_TRACE_TEXT_BORROW`.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/a-walk-borrows-its-word-in-the-versioned-copy.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_DISTINCT_VERSION")
        .env_remove("LOFT_NO_TEXT_BORROW")
        .env_remove("LOFT_HOIST_VERIFY")
        .env_remove("LOFT_TRACE_TEXT_BORROW");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

#[test]
fn the_cells_hold_under_the_falsifiers_with_each_switch() {
    for mode in ["--native", "--native-release"] {
        for switch in [
            None,
            Some(("LOFT_NO_DISTINCT_VERSION", "1")),
            Some(("LOFT_NO_TEXT_BORROW", "1")),
        ] {
            for falsifier in [("LOFT_HOIST_VERIFY", "1"), ("LOFT_POISON", "1")] {
                let mut env = vec![falsifier];
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
}

#[test]
fn a_walk_that_writes_only_the_result_borrows_and_one_that_writes_its_parameter_does_not() {
    let borrows = |env: &[(&str, &str)]| -> String {
        let mut e = vec![("LOFT_TRACE_TEXT_BORROW", "1")];
        e.extend_from_slice(env);
        let out = loft(&["--native-release", "--native-emit", "/dev/null"], &e);
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let on = borrows(&[]);
    for (f, v) in [("n_lex", "arg"), ("n_measure", "a")] {
        assert!(
            on.contains(&format!(
                "{f}: `{v}` borrows its element in the copy versioned on `argv`"
            )),
            "{f} borrows in the copy:\n{on}"
        );
    }
    assert!(
        on.contains(
            "n_logged: `w` no versioned borrow — a writer to a store the frame did not mint"
        ),
        "logged's walk writes its parameter's store:\n{on}"
    );
    let off = borrows(&[("LOFT_NO_DISTINCT_VERSION", "1")]);
    assert!(
        !off.contains("in the copy versioned"),
        "no versioned copy, no versioned borrow:\n{off}"
    );
}
