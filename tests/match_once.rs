// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-M-Once` — a `match` is lowered once per arm, so its lowered size grows linearly with its
//! source.  A guarded arm folds its guard into its own condition (`parser::control::guarded_arm`),
//! and a failed guard reaches the next arm through control flow, never through a copy of the
//! arms after it.  The cells lower an eight-arm plain-enum match with 0 to 6 of its first arms
//! guarded and count the subject tests in its IR: one per arm, at every guard count.  The
//! values a guarded match answers are the language suite's (`rules/matching/M-Once/`).
use std::path::PathBuf;

fn loft() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// The eight-arm match, its first `guarded` arms carrying `if x > i`.
fn program(guarded: usize) -> String {
    let mut src = String::from("enum E { V0, V1, V2, V3, V4, V5, V6, V7 }\n\n");
    src.push_str("fn f(e: E, x: integer) -> integer {\n  match e {\n");
    for i in 0..8 {
        let guard = if i < guarded {
            format!(" if x > {i}")
        } else {
            String::new()
        };
        src.push_str(&format!("    V{i}{guard} => {i},\n"));
    }
    src.push_str("    _ => 99,\n  }\n}\n\nfn main() { println(\"{f(V7, 1)}\"); }\n");
    src
}

/// The number of subject tests (`OpEqInt`) in `fn n_f`'s IR, lowered with `env` set.
fn subject_tests(guarded: usize, env: &[(&str, &str)]) -> usize {
    let dir =
        std::env::temp_dir().join(format!("loft_match_once_{}_{guarded}", std::process::id()));
    let _ = loft::file_access::remove_dir_all(&dir);
    loft::file_access::create_dir_all(&dir).expect("scratch");
    let file = dir.join("m.loft");
    loft::file_access::write(&file, program(guarded)).expect("program");
    let mut cmd = loft::platform::process::harness_command(loft());
    cmd.arg("introspect").arg(&file).env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run loft introspect");
    let _ = loft::file_access::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let start = text.find("fn n_f(").expect("the dump holds n_f");
    let end = text[start..]
        .find("byte-code for")
        .map_or(text.len(), |e| start + e);
    text[start..end].matches("OpEqInt").count()
}

#[test]
fn every_arm_is_lowered_once_at_every_guard_count() {
    let got: Vec<usize> = (0..=6).map(|g| subject_tests(g, &[])).collect();
    assert_eq!(
        got,
        vec![8; 7],
        "subject tests per guard count, 0..=6 guarded arms"
    );
}

/// The control: the switch builds the copying lowering (`D-match-26`'s), and the same count
/// reads its exponential growth — so a regression to it cannot pass this file.
#[test]
fn the_copying_lowering_is_what_the_count_catches() {
    assert_eq!(subject_tests(6, &[("LOFT_NO_GUARD_ARM", "1")]), 191);
}
