// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RefillText`, PINNED on its cells (`tests/scripts/a-pooled-buffers-texts-are-refilled-in-their-slots.loft`):
//! which pooled call sites refill their callee's texts in place, and why each other site
//! declines, read off `LOFT_TRACE_REFILL_TEXT` during `--native-emit` (code generation only,
//! no rustc).  The cells' values pass with either form, so only this pin sees a site that
//! stops being taken or one that starts being taken for the wrong reason.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-pooled-buffers-texts-are-refilled-in-their-slots.loft";

fn emit(env: &[(&str, &str)]) -> (String, bool) {
    emit_file(CELLS, env)
}

fn emit_file(cells: &str, env: &[(&str, &str)]) -> (String, bool) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // One file per test: the tests are threads of one process.
    let stem = fa::file_stem(cells).unwrap_or_else(|| "cells".to_string());
    let tag = env.iter().map(|(k, _)| *k).collect::<Vec<_>>().join("_");
    let out = std::env::temp_dir().join(format!(
        "loft_refill_text_{}_{stem}_{tag}.rs",
        std::process::id()
    ));
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native")
        .arg("--native-emit")
        .arg(&out)
        .arg(root.join(cells))
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1");
    for k in [
        "LOFT_NO_REFILL_TEXT",
        "LOFT_NO_REFILL_ELEMENTS",
        "LOFT_NO_REFILL_VECTORS",
        "LOFT_TRACE_REFILL_TEXT",
    ] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let res = cmd.output().expect("spawn loft");
    let _ = fa::remove_file(&out);
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

/// The vector clause's guard: which pooled sites skip their release because the callee empties
/// each vector field in place, and why the others decline.
const VECTORS: &str = "tests/scripts/a-pooled-buffers-vectors-are-emptied-in-place.loft";

#[test]
fn the_vector_switch_releases_first_again() {
    let (err, ok) = emit_file(
        VECTORS,
        &[
            ("LOFT_TRACE_REFILL_TEXT", "1"),
            ("LOFT_NO_REFILL_VECTORS", "1"),
        ],
    );
    assert!(ok, "emission failed:\n{err}");
    assert!(
        !trace(&err)
            .iter()
            .any(|l| l.ends_with("admitted (vectors)")),
        "LOFT_NO_REFILL_VECTORS=1 must admit no vector site:\n{err}"
    );
}
