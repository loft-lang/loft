// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-SingleUse`'s statement clause — PINNED on the cells of
//! `tests/scripts/a-comprehension-element-goes-straight-into-its-push.loft`: exactly the
//! comprehensions with a pure element move it into the append, read off
//! `LOFT_TRACE_SINGLE_USE`; an element computed by a function with a side effect keeps its
//! temporary.  And the cells hold with the clause off.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-comprehension-element-goes-straight-into-its-push.loft";

fn loft(env: &[(&str, &str)]) -> (String, String, bool) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_SINGLE_USE")
        .env_remove("LOFT_TRACE_SINGLE_USE");
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
fn only_pure_elements_move_into_their_append() {
    let (_, err, ok) = loft(&[("LOFT_TRACE_SINGLE_USE", "1")]);
    assert!(ok, "the cells failed:\n{err}");
    let mut moved: Vec<&str> = err
        .lines()
        .filter_map(|l| l.strip_prefix("single-use: "))
        .filter(|l| l.starts_with("n_"))
        .collect();
    moved.sort_unstable();
    assert_eq!(
        moved,
        [
            "n_grid_diagonal — 1 temporary(ies) moved into their read",
            "n_halves — 1 temporary(ies) moved into their read",
            "n_squares — 1 temporary(ies) moved into their read",
        ],
        "an element with a side effect keeps its temporary"
    );
}

#[test]
fn the_cells_hold_with_the_clause_off() {
    let (out, err, ok) = loft(&[("LOFT_NO_SINGLE_USE", "1")]);
    assert!(ok && out.trim_end().ends_with("ok"), "{out}\n{err}");
}
