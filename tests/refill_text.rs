// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RefillText`, PINNED on its cells (`tests/scripts/a-pooled-buffers-texts-are-refilled-in-their-slots.loft`):
//! which pooled call sites refill their callee's texts in place, and why each other site
//! declines, read off `LOFT_TRACE_REFILL_TEXT` during `--native-emit` (code generation only,
//! no rustc).  The cells' values pass with either form, so only this pin sees a site that
//! stops being taken or one that starts being taken for the wrong reason.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-pooled-buffers-texts-are-refilled-in-their-slots.loft";

/// Every trace line the cells produce, in order.
const TRACE: &[&str] = &[
    "refill-text: n_c1 → n_mk_sz admitted",
    "refill-text: n_c2 → n_mk_env admitted",
    "refill-text: n_c3 → n_mk_sz admitted",
    "refill-text: n_c5 → n_mk_tv declined — the result owns heap other than text",
    "refill-text: n_c6 → n_mk_fwd declined — the buffer is named outside its literals",
    "refill-text: n_c8 → n_mk_td admitted",
    "refill-text: n_c9 → n_mk_sz declined — the result is not only read",
    "refill-text: n_c10 → n_mk_lit admitted",
    "refill-text: n_m1_env → n_mk_env admitted",
    "refill-text: n_m1_sz → n_mk_sz admitted",
    "refill-text: n_m1_fwd → n_mk_fwd declined — the buffer is named outside its literals",
    "refill-text: n_m1_lit → n_mk_lit admitted",
];

fn emit(env: &[(&str, &str)]) -> (String, bool) {
    emit_file(CELLS, env)
}

fn emit_file(cells: &str, env: &[(&str, &str)]) -> (String, bool) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // One file per test: the tests are threads of one process.
    let stem = Path::new(cells)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("cells");
    let tag = env.iter().map(|(k, _)| *k).collect::<Vec<_>>().join("_");
    let out = std::env::temp_dir().join(format!(
        "loft_refill_text_{}_{stem}_{tag}.rs",
        std::process::id()
    ));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(root.join(cells))
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1");
    for k in [
        "LOFT_NO_REFILL_TEXT",
        "LOFT_NO_REFILL_ELEMENTS",
        "LOFT_TRACE_REFILL_TEXT",
    ] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let res = cmd.output().expect("spawn loft");
    let _ = std::fs::remove_file(&out);
    (
        String::from_utf8_lossy(&res.stderr).into_owned(),
        res.status.success(),
    )
}

fn trace(err: &str) -> Vec<&str> {
    err.lines()
        .filter(|l| l.starts_with("refill-text: "))
        .collect()
}

#[test]
fn the_cells_refill_where_the_rule_admits() {
    let (err, ok) = emit(&[("LOFT_TRACE_REFILL_TEXT", "1")]);
    assert!(ok, "emission failed:\n{err}");
    assert_eq!(trace(&err), TRACE, "the refilled sites moved:\n{err}");
}

/// The collection clause's guard: which builders keep their elements, and why the other
/// declines.
const KEPT: &str = "tests/scripts/a-kept-buffers-elements-are-refilled-in-their-slots.loft";
const KEPT_TRACE: &[&str] = &[
    "refill-text: n_runs_of keeps its elements",
    "refill-text: n_twos_of keeps its elements",
    "refill-text: n_trimmed_of keeps no elements — the vector or an element is used another way",
];

#[test]
fn the_builders_keep_their_elements_where_the_clause_admits() {
    let (err, ok) = emit_file(KEPT, &[("LOFT_TRACE_REFILL_TEXT", "1")]);
    assert!(ok, "emission failed:\n{err}");
    assert_eq!(trace(&err), KEPT_TRACE, "the kept builders moved:\n{err}");
}

#[test]
fn the_switch_takes_no_site() {
    let (err, ok) = emit(&[
        ("LOFT_TRACE_REFILL_TEXT", "1"),
        ("LOFT_NO_REFILL_TEXT", "1"),
    ]);
    assert!(ok, "emission failed:\n{err}");
    assert!(
        trace(&err).is_empty(),
        "a site was asked with the switch off:\n{err}"
    );
}
