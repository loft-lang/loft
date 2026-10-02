// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-InRange` — PINNED on the cells of
//! `tests/scripts/a-record-element-its-loop-proves-in-range-reads-plainly.loft`: only the loop
//! that reads its own vector's record elements, with a body that cannot resize it, drops its
//! discharges — read off `LOFT_TRACE_IN_RANGE`.  And the cells hold with the rewrite off.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-record-element-its-loop-proves-in-range-reads-plainly.loft";

fn loft(env: &[(&str, &str)]) -> (String, String, bool) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_IN_RANGE")
        .env_remove("LOFT_TRACE_IN_RANGE");
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
fn only_the_in_range_record_reads_drop_their_discharge() {
    let (_, err, ok) = loft(&[("LOFT_TRACE_IN_RANGE", "1")]);
    assert!(ok, "the cells failed:\n{err}");
    let rewritten: Vec<&str> = err
        .lines()
        .filter_map(|l| l.strip_prefix("in-range: "))
        .filter(|l| l.starts_with("n_"))
        .collect();
    assert_eq!(
        rewritten,
        ["n_update i — 1 discharge(s) dropped"],
        "a cleared vector, another vector and a scalar element keep their discharges"
    );
}

#[test]
fn the_cells_hold_with_the_rewrite_off() {
    let (out, err, ok) = loft(&[("LOFT_NO_IN_RANGE", "1")]);
    assert!(ok && out.trim_end().ends_with("ok"), "{out}\n{err}");
}
