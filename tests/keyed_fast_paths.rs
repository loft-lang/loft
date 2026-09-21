// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The keyed fast paths (@PLN158, `bench/portal/analysis/keyed.md` L1–L4) answer what the
//! general paths answer.
//!
//! `tests/scripts/158-keyed-fast-paths.loft` states every answer by hand, and the corpus
//! runners already run it as built on both backends.  That leaves two readings nothing else
//! takes, and this file takes them:
//!
//! * with `LOFT_NO_FAST_ORDER=1 LOFT_NO_ONE_PROBE_INSERT=1` — the GENERAL paths must give
//!   the same hand-computed answers, or the cells would be pinning the fast paths to
//!   themselves;
//! * with `LOFT_KEYED_VERIFY=1` — every pre-resolved comparison, every exact `index`
//!   lookup and every one-probe `hash` insert is checked against the general form as it
//!   is made, and a disagreement panics.  The cells are the workload; the check is per
//!   operation, so it sees a wrong comparison that happens not to change a cell's answer.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/158-keyed-fast-paths.loft";

fn run(backend: &str, env: &[(&str, &str)]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg(backend)
        .arg("--tests")
        .arg(&src)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_FAST_ORDER")
        .env_remove("LOFT_NO_ONE_PROBE_INSERT")
        .env_remove("LOFT_KEYED_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft --tests");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && text.contains("test result: ok. 14 passed"),
        "{backend} {env:?}: the keyed cells did not all pass\n{text}"
    );
}

const GENERAL: [(&str, &str); 2] = [
    ("LOFT_NO_FAST_ORDER", "1"),
    ("LOFT_NO_ONE_PROBE_INSERT", "1"),
];
const VERIFY: [(&str, &str); 1] = [("LOFT_KEYED_VERIFY", "1")];

#[test]
fn the_general_paths_give_the_same_answers_on_the_interpreter() {
    run("--interpret", &GENERAL);
}

#[test]
fn the_general_paths_give_the_same_answers_on_native() {
    run("--native", &GENERAL);
}

#[test]
fn every_fast_answer_verifies_against_the_general_one_on_the_interpreter() {
    run("--interpret", &VERIFY);
}

#[test]
fn every_fast_answer_verifies_against_the_general_one_on_native() {
    run("--native", &VERIFY);
}
