// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-CopyView` — the admission of a read-only record copy, PINNED on the trace of the
//! cells (`tests/scripts/a-read-only-record-copy-is-a-view.loft`): the cells' values pass on
//! either form, so only this pin sees an admission drift.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-read-only-record-copy-is-a-view.loft";

/// Per function: the record locals made a view (the rewrite ON; OFF: none).
const EXPECTED: &[(&str, &[&str])] = &[
    ("n_k1", &["t"]),   // the plain bind, only read
    ("n_k2", &["t"]),   // the join
    ("n_k3", &[]),      // t written
    ("n_k4", &[]),      // the record written through another parameter in t's range
    ("n_k5", &[]),      // a callee handed a writable vector of the record's type
    ("n_k6", &["t"]),   // a push into a vector<integer> cannot reach the record
    ("n_k7", &[]),      // t returned
    ("n_k8", &[]),      // the source is a record the frame owns
    ("n_k10", &["lo"]), // the join per loop iteration
    ("n_k11", &[]),     // the source is a parameter itself: R-ValueRecord's
];

fn trace(env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_TRACE_COPY_VIEW", "1")
        .env_remove("LOFT_NO_COPY_VIEW");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    assert!(out.status.success(), "introspect failed: {:?}", out.status);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn viewed_in(trace: &str, name: &str) -> Vec<String> {
    let head = format!("[copy-view] fn={name} local=");
    trace
        .lines()
        .filter_map(|l| l.strip_prefix(&head))
        .filter_map(|rest| rest.strip_suffix(" VIEWS its source instead of copying it"))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_read_only_copy_of_an_undisturbed_record_is_a_view_and_nothing_else_is() {
    let t = trace(&[]);
    for (name, expected) in EXPECTED {
        assert_eq!(
            viewed_in(&t, name),
            expected.to_vec(),
            "{name}: the viewed locals — the cell's admission moved"
        );
    }
}

#[test]
fn the_switch_keeps_every_copy() {
    let t = trace(&[("LOFT_NO_COPY_VIEW", "1")]);
    assert!(
        !t.contains("VIEWS"),
        "LOFT_NO_COPY_VIEW=1 must view nothing:\n{t}"
    );
}
