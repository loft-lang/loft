// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-RepeatRun` — a run of one constant spelled out in a literal, PINNED on the IR of the
//! cells (`tests/scripts/a-run-of-one-constant-is-one-repeat-fill.loft`): an admitted run is
//! one template push and one `OpAppendCopy`, a declined one keeps its pushes.  The cells'
//! values pass on either form, so only this pin sees an admission drift.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-run-of-one-constant-is-one-repeat-fill.loft";

/// Per function: the fills with the rewrite ON (and OFF: always none).
const EXPECTED: &[(&str, usize)] = &[
    ("n_zero", 1), // a record field: sixteen zeros
    ("n_r2", 1),   // a local
    ("n_r3", 1),   // onto a filled vector, after a different element
    ("n_r4", 0),   // three equal elements stay pushes
    ("n_r5", 2),   // two runs in one literal
    ("n_r6", 2),   // a different constant breaks the run in two
    ("n_r7", 1),   // a nested field path
    ("n_r8", 1),   // single precision
    ("n_r9", 0),   // a computed element
    ("n_r11", 1),  // a long run
];

fn introspect(env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_REPEAT_RUN");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains("fn n_zero("),
        "no IR printed (exit {:?}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    text
}

/// The IR of `fn <name>(` up to the next top-level `fn `.
fn body_of<'a>(ir: &'a str, name: &str) -> &'a str {
    let head = format!("fn {name}(");
    let start = ir
        .find(&head)
        .unwrap_or_else(|| panic!("{name} was not printed"));
    let rest = &ir[start + head.len()..];
    let end = rest.find("\nfn ").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn a_run_of_one_constant_is_one_fill_and_anything_else_keeps_its_pushes() {
    let ir = introspect(&[]);
    for (name, fills) in EXPECTED {
        assert_eq!(
            body_of(&ir, name).matches("OpAppendCopy(").count(),
            *fills,
            "{name}: repeat fills — the cell's admission moved"
        );
    }
}

#[test]
fn the_switch_keeps_every_push() {
    let ir = introspect(&[("LOFT_NO_REPEAT_RUN", "1")]);
    for (name, _) in EXPECTED {
        assert_eq!(
            body_of(&ir, name).matches("OpAppendCopy(").count(),
            0,
            "{name}: LOFT_NO_REPEAT_RUN=1 must keep every push"
        );
    }
}
