// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-ForwardWalk` — PINNED on the cells of
//! `tests/scripts/a-walk-the-body-cannot-resize-is-a-counted-loop.loft`.  The cells check values,
//! which pass whether or not a walk becomes counted; this pins WHICH walks do, read off
//! `LOFT_TRACE_FORWARD_WALK`, so a walk that stops being rewritten — or one rewritten where the
//! body can resize the vector — is a red test.  It also runs the cells with the rewrite off.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-walk-the-body-cannot-resize-is-a-counted-loop.loft";

/// `(function, loop variable, rewritten)` for every walk of the cells.
const WALKS: &[(&str, &str, bool)] = &[
    ("n_sum_walk", "x", true),
    ("n_record_walk", "p", true),
    ("n_index_walk", "x", true),
    ("n_continue_break_walk", "x", true),
    // `out` is a result buffer the scope pass made a parameter — never the walked vector.
    ("n_into_another", "x", true),
    ("n_nested_walk", "x", true),
    ("n_nested_walk", "y", true),
    // `x#remove` shrinks the vector under the walk.
    ("n_remove_walk", "x", false),
    // `b` is a `&` parameter: the caller passes the walked vector as it (`both(w, w)`).
    ("n_both", "x", false),
];

fn loft(env: &[(&str, &str)]) -> (String, String, bool) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_FORWARD_WALK")
        .env_remove("LOFT_TRACE_FORWARD_WALK");
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
fn the_walks_the_body_cannot_resize_are_the_ones_rewritten() {
    let (_, err, ok) = loft(&[("LOFT_TRACE_FORWARD_WALK", "1")]);
    assert!(ok, "the cells failed:\n{err}");
    for (f, v, counted) in WALKS {
        let want = if *counted {
            format!("forward-walk: {f} {v} counted")
        } else {
            format!("forward-walk: {f} {v} declined")
        };
        assert!(
            err.lines().any(|l| l.starts_with(&want)),
            "{f} {v}: expected `{want}…` in the trace:\n{err}"
        );
    }
}

#[test]
fn the_cells_hold_with_the_rewrite_off() {
    let (out, err, ok) = loft(&[("LOFT_NO_FORWARD_WALK", "1")]);
    assert!(ok && out.trim_end().ends_with("ok"), "{out}\n{err}");
}
