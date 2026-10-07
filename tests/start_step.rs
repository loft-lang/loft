// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-StartStep`, PINNED on its cells (`tests/scripts/a-computed-range-start-steps-one-counter.loft`):
//! which counted loops with a computed start step one counter, read off
//! `LOFT_TRACE_START_STEP`.  The cells' values pass with either form, so only this pin sees a
//! loop that stops being taken; and the cells run under every switch state, because the
//! default run never reaches the two-counter path.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-computed-range-start-steps-one-counter.loft";

/// Every trace line the cells produce, in order.
const TRACE: &[&str] = &[
    "start-step: n_range_sum i steps one counter",
    "start-step: n_rounds ___1 steps one counter",
    "start-step: n_odd_sum i steps one counter",
    "start-step: n_index_sum i steps one counter",
    "start-step: n_written i steps one counter",
    "start-step: n_written_and_index i declined — the body names a counter",
    "start-step: n_triangle i steps one counter",
    "start-step: n_triangle ___1 steps one counter",
    "start-step: n_first_square i steps one counter",
    "start-step: n_ends i steps one counter",
];

fn loft(env: &[(&str, &str)]) -> (String, String, bool) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1");
    for k in [
        "LOFT_NO_START_STEP",
        "LOFT_TRACE_START_STEP",
        "LOFT_NO_LOOP_ROTATE",
        "LOFT_NO_LOOP_VAR_ALIAS",
    ] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn the_cells_step_one_counter_where_the_rule_admits() {
    let (out, err, ok) = loft(&[("LOFT_TRACE_START_STEP", "1")]);
    assert!(ok && out.contains("done"), "cells failed:\n{out}\n{err}");
    let got: Vec<&str> = err
        .lines()
        .filter(|l| l.starts_with("start-step: "))
        .collect();
    assert_eq!(got, TRACE, "the loops taking one counter moved:\n{err}");
}

#[test]
fn the_cells_answer_the_same_under_every_switch() {
    for env in [
        &[][..],
        &[("LOFT_NO_START_STEP", "1")][..],
        &[("LOFT_NO_LOOP_ROTATE", "1")][..],
        &[("LOFT_NO_LOOP_VAR_ALIAS", "1")][..],
        &[("LOFT_NO_LOOP_VAR_ALIAS", "1"), ("LOFT_NO_START_STEP", "1")][..],
    ] {
        let (out, err, ok) = loft(env);
        assert!(ok && out.contains("done"), "{env:?}:\n{out}\n{err}");
    }
}

#[test]
fn the_switch_and_a_rotation_off_take_no_loop() {
    for env in [
        &[("LOFT_NO_START_STEP", "1")][..],
        &[("LOFT_NO_LOOP_ROTATE", "1")][..],
    ] {
        let mut env = env.to_vec();
        env.push(("LOFT_TRACE_START_STEP", "1"));
        let (_, err, ok) = loft(&env);
        assert!(ok, "{env:?}:\n{err}");
        assert!(
            !err.contains("start-step: "),
            "{env:?} still took a loop:\n{err}"
        );
    }
}
