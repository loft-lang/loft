// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-DischargeInto` — PINNED on the cells of
//! `tests/scripts/a-text-discharge-reads-straight-into-its-local.loft`: exactly the text binds
//! whose target appears in neither the read nor the default are read straight in, read off
//! `LOFT_TRACE_DISCHARGE_INTO`.  And the cells hold with the rewrite off.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-text-discharge-reads-straight-into-its-local.loft";

fn loft(env: &[(&str, &str)]) -> (String, String, bool) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_DISCHARGE_INTO")
        .env_remove("LOFT_TRACE_DISCHARGE_INTO");
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
fn only_binds_that_never_read_their_target_are_rewritten() {
    let (_, err, ok) = loft(&[("LOFT_TRACE_DISCHARGE_INTO", "1")]);
    assert!(ok, "the cells failed:\n{err}");
    let mut rewritten: Vec<&str> = err
        .lines()
        .filter_map(|l| l.strip_prefix("discharge-into: "))
        .filter(|l| l.starts_with("n_"))
        .collect();
    rewritten.sort_unstable();
    assert_eq!(
        rewritten,
        [
            "n_bind — 1 read(s) straight into their local",
            "n_lengths — 1 read(s) straight into their local",
            "n_named_default — 1 read(s) straight into their local",
        ],
        "a read indexed by `len(w)` and a default built from `w` keep their temporaries"
    );
}

#[test]
fn the_cells_hold_with_the_rewrite_off() {
    let (out, err, ok) = loft(&[("LOFT_NO_DISCHARGE_INTO", "1")]);
    assert!(ok && out.trim_end().ends_with("ok"), "{out}\n{err}");
}
