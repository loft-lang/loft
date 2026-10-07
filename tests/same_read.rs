// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-SameRead` — PINNED on the cells of
//! `tests/scripts/a-read-a-statement-spells-twice-is-read-once.loft`: exactly the statements that
//! read one element twice with nothing between that can change it are bound, read off
//! `LOFT_TRACE_SAME_READ`.  The cells check values; this pins where the rewrite fires, and runs
//! the cells with it off.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-read-a-statement-spells-twice-is-read-once.loft";

fn loft(env: &[(&str, &str)]) -> (String, String, bool) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_SAME_READ")
        .env_remove("LOFT_TRACE_SAME_READ");
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
fn only_the_reads_nothing_can_change_are_bound() {
    let (_, err, ok) = loft(&[("LOFT_TRACE_SAME_READ", "1")]);
    assert!(ok, "the cells failed:\n{err}");
    let mut bound: Vec<&str> = err
        .lines()
        .filter_map(|l| l.strip_prefix("same-read: "))
        .filter(|l| l.starts_with("n_"))
        .collect();
    bound.sort_unstable();
    assert_eq!(
        bound,
        [
            "n_twice binds 1 read(s) of one statement",
            "n_twice_float binds 1 read(s) of one statement",
        ],
        "two different indexes, two defaults, the arms of an `if` and a call that writes \
         between the reads must each keep two reads"
    );
}

#[test]
fn the_cells_hold_with_the_rewrite_off() {
    let (out, err, ok) = loft(&[("LOFT_NO_SAME_READ", "1")]);
    assert!(ok && out.trim_end().ends_with("ok"), "{out}\n{err}");
}
