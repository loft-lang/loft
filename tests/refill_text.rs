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
    "refill-text: n_m1_env → n_mk_env admitted",
    "refill-text: n_m1_sz → n_mk_sz admitted",
];

fn emit(env: &[(&str, &str)]) -> (String, bool) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = std::env::temp_dir().join(format!("loft_refill_text_{}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(root.join(CELLS))
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1");
    for k in ["LOFT_NO_REFILL_TEXT", "LOFT_TRACE_REFILL_TEXT"] {
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
