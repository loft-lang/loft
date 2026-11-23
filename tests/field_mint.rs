// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Mint`'s field clause — the EMISSION pins.  A record append to a vector FIELD of a
//! plain record (`m.verts += [Vertex { … }]`, lowered as `OpNewRecord(m, parent_tp, fld)`)
//! holds a record-push header keyed by the path a read of the field already has, and emits
//! its mint and its finish through it.  `LOFT_NO_FIELD_MINT=1` restores the templates.  The
//! cell corpus (`tests/scripts/158-field-mint.loft`) says the VALUES hold on both backends,
//! in every switch state and under the falsifiers; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/158-field-mint.loft";

const SWITCHES: [&str; 5] = [
    "LOFT_NO_FIELD_MINT",
    "LOFT_NO_RECORD_PUSH",
    "LOFT_NO_HEAP_RECORD_PUSH",
    "LOFT_NO_ELEMENT_FIRST",
    "LOFT_HOIST_VERIFY",
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    // These pins count `(R-Mint)`'s field clause and its templates: `(R-FieldAppend)`, which
    // spells a template mint without a header differently, is off.
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_FIELD_APPEND", "1");
    for s in SWITCHES {
        cmd.env_remove(s);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn run(args: &[&str], env: &[(&str, &str)]) -> String {
    let mut a: Vec<&str> = args.to_vec();
    let s = cells().to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.trim_end().ends_with("field mint ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// The interpreter's answer, computed once and shared by the five switch-state tests below.
fn oracle() -> &'static str {
    static ORACLE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ORACLE.get_or_init(|| run(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]))
}

/// Native under every falsifier must print what the interpreter printed, in one switch state.
/// One test per switch state, so the five run in parallel: as one test the 25 native builds
/// passed the test-duration gate's 300 s on a loaded runner (328.8 s), and the limit is not
/// raised (CI_BUDGET.md).
fn cells_answer_the_interpreter(switch: Option<(&str, &str)>) {
    {
        // Every detector at once: one build per switch state, each detector still on
        // (falsifying a guard is a one-time receipt, not a cost of every run).
        let mut env = vec![
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ("LOFT_HOIST_VERIFY", "1"),
        ];
        env.extend(switch);
        let got = run(&["--native"], &env);
        assert_eq!(
            got,
            oracle(),
            "native under {env:?} must print what the interpreter printed"
        );
    }
}

#[test]
fn the_cells_answer_the_interpreter_under_the_falsifiers() {
    cells_answer_the_interpreter(None);
}

#[test]
fn the_cells_answer_the_interpreter_without_field_mint() {
    cells_answer_the_interpreter(Some(("LOFT_NO_FIELD_MINT", "1")));
}

#[test]
fn the_cells_answer_the_interpreter_without_record_push() {
    cells_answer_the_interpreter(Some(("LOFT_NO_RECORD_PUSH", "1")));
}

#[test]
fn the_cells_answer_the_interpreter_without_heap_record_push() {
    cells_answer_the_interpreter(Some(("LOFT_NO_HEAP_RECORD_PUSH", "1")));
}

#[test]
fn the_cells_answer_the_interpreter_without_element_first() {
    cells_answer_the_interpreter(Some(("LOFT_NO_ELEMENT_FIRST", "1")));
}
