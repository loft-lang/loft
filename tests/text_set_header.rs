// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Header`'s text clause — PINNED on `tests/scripts/a-loop-holds-its-header-across-a-text-write.loft`.
//! The cells' values pass whether or not a loop holds its header, and a base kept across a
//! growth answers right until the store happens to move, so this runs them where a stale hold
//! cannot hide — under `LOFT_HOIST_VERIFY=1` (every held read checked) and `LOFT_POISON=1`
//! (every growth moves and scribbles) — in both switch states, and pins which loops hold,
//! read off `LOFT_TRACE_HOIST_DECLINE`.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/a-loop-holds-its-header-across-a-text-write.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_TEXT_SET_HEADER")
        .env_remove("LOFT_HOIST_VERIFY")
        .env_remove("LOFT_TRACE_HOIST_DECLINE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

#[test]
fn the_cells_hold_under_the_hoist_falsifiers_in_both_switch_states() {
    for mode in ["--native", "--native-release"] {
        for switch in [None, Some(("LOFT_NO_TEXT_SET_HEADER", "1"))] {
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
fn a_loop_that_writes_texts_holds_and_one_that_also_pushes_declines() {
    // The decline trace lines of the cells' functions.
    let declines = |env: &[(&str, &str)]| -> Vec<String> {
        let mut e = vec![("LOFT_TRACE_HOIST_DECLINE", "1")];
        e.extend_from_slice(env);
        let out = loft(&["--native-release", "--native-emit", "/dev/null"], &e);
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| l.starts_with("hoist: n_") && l.contains(" loop "))
            .map(str::to_string)
            .collect()
    };
    let held = ["n_reset", "n_relabel", "n_shift", "n_tally"];
    // On: the text-writing loops hold; `grow` (a text set beside a push) declines.
    let on = declines(&[]);
    for f in held {
        assert!(
            !on.iter()
                .any(|l| l.starts_with(&format!("hoist: {f} loop"))),
            "{f}'s loop holds: {on:#?}"
        );
    }
    assert!(
        on.iter()
            .any(|l| l.starts_with("hoist: n_grow loop") && l.contains("also pushes")),
        "grow's loop declines for its push: {on:#?}"
    );
    // Off: every text-writing loop declines again.
    let off = declines(&[("LOFT_NO_TEXT_SET_HEADER", "1")]);
    for f in held {
        assert!(
            off.iter()
                .any(|l| l.starts_with(&format!("hoist: {f} loop"))),
            "the switch restores {f}'s decline: {off:#?}"
        );
    }
}
