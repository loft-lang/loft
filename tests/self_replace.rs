// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-SelfReplace` — PINNED on the cells of
//! `tests/scripts/a-vector-delivered-into-its-own-buffer-is-not-copied.loft`: exactly the
//! builder whose result is its buffer's own view drops its deliveries (an early return and the
//! tail), read off `LOFT_TRACE_SELF_REPLACE`.  The cells check values; this pins where the
//! rewrite fires, and runs the cells with it off.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-vector-delivered-into-its-own-buffer-is-not-copied.loft";

fn loft(env: &[(&str, &str)]) -> (String, String, bool) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_SELF_REPLACE")
        .env_remove("LOFT_TRACE_SELF_REPLACE");
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
fn only_the_buffer_s_own_view_drops_its_deliveries() {
    let (_, err, ok) = loft(&[("LOFT_TRACE_SELF_REPLACE", "1")]);
    assert!(ok, "the cells failed:\n{err}");
    let dropped: Vec<&str> = err
        .lines()
        .filter_map(|l| l.strip_prefix("self-replace: "))
        .filter(|l| l.starts_with("n_"))
        .collect();
    assert_eq!(
        dropped,
        ["n_build drops 2"],
        "`swap_in` and `pick` return a view of a store other than the buffer and must keep \
         their deliveries"
    );
}

#[test]
fn the_cells_hold_with_the_rewrite_off() {
    let (out, err, ok) = loft(&[("LOFT_NO_SELF_REPLACE", "1")]);
    assert!(ok && out.trim_end().ends_with("ok"), "{out}\n{err}");
}
