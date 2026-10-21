// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-BoundAppend`, PINNED on its cells
//! (`tests/scripts/a-bound-result-appended-later-is-appended-where-it-is-made.loft`): which
//! bodies move an append to its call, read off `LOFT_TRACE_BOUND_APPEND`, and that the cells
//! answer the same on both backends with the rule on and off.  The declined cells answer the
//! same either way, so only this pin sees a decline become an admission.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-bound-result-appended-later-is-appended-where-it-is-made.loft";

/// Only c1 moves its append: c2 reads an element of the destination between, c3 writes it,
/// c4 may leave between, c5 reads `k` after the append, c6's destination is a `&` link.
const TRACE: &[&str] = &["[bound-append] n_c1: n_piece appended where it is made"];

fn loft(args: &[&str], env: &[(&str, &str)]) -> (String, String, bool) {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_BOUND_APPEND")
        .env_remove("LOFT_TRACE_BOUND_APPEND");
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

fn cells() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CELLS)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn only_the_admitted_body_moves_its_append() {
    let (out, err, ok) = loft(
        &["--interpret", &cells()],
        &[("LOFT_TRACE_BOUND_APPEND", "1")],
    );
    assert!(ok && out.contains("done"), "cells failed:\n{out}\n{err}");
    let got: Vec<&str> = err
        .lines()
        .filter(|l| l.starts_with("[bound-append]"))
        .collect();
    assert_eq!(got, TRACE, "the admitted bodies moved:\n{err}");
}

#[test]
fn the_cells_answer_the_same_on_both_backends_either_way() {
    for b in ["--interpret", "--native"] {
        for env in [&[][..], &[("LOFT_NO_BOUND_APPEND", "1")][..]] {
            let (out, err, ok) = loft(&[b, &cells()], env);
            assert!(ok && out.contains("done"), "{b} {env:?}:\n{out}\n{err}");
        }
    }
}
