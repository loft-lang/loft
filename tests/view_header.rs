// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-n (loft#1426) — a `&`-bound vector VIEW (`d = &cv.data`) derives its header
//! ONCE, at the binding, for the rest of the block that indexes it, on the native backend.
//!
//! The cell corpus (`bytecode-comparisons/V-n-view-def-header-cells.loft`) can only say the
//! VALUES hold, because a header-served read and a runtime read of an unmoved vector answer
//! the same number.  This pins the EMISSION per helper — which bindings earn a header — and
//! the switch (`LOFT_NO_VIEW_HOIST=1`), which is what makes it red on the build before the
//! unit and on one that lost it.  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-n-view-def-header-cells.loft";

/// `(helper, view headers bound)` — the predictions written beside the cells.
const EXPECTED: &[(&str, usize)] = &[
    ("n_setp", 1), // c1: the set_pixel shape
    ("n_getp", 1), // c2: the get_pixel shape
    ("n_h3", 0),   // a growing write through the view
    ("n_h4", 1),   // rebound in the block: only the second binding
    ("n_h5", 1),   // an in-place callee between
    ("n_h6", 0),   // a growing callee between
    ("n_h7", 1),   // bound inside an if-arm
    ("n_h8", 1),   // bound inside a loop body
    ("n_h9", 0),   // the root reassigned (a store write)
    ("n_h10", 2),  // two sibling views
    ("n_h11", 1),  // a nested path
    ("n_h12", 0),  // only the length is read
    ("n_h13", 1),  // reads inside a nested loop and if
    ("n_last", 0), // the binding is the last statement
    ("n_h15", 0),  // a keyed collection has no element address
    ("n_h16", 1),  // a float vector with an in-place write
    ("n_h17", 0),  // the `?? []` default allocates, which blocks the gate
];

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read_to_string(out).expect("read the emitted Rust")
}

/// Per emitted function: how many view headers its blocks bind.
fn counts(rust: &str) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        if line.contains("V-n view header") {
            *map.entry(current.clone()).or_default() += 1;
        }
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_binding_earns_exactly_the_header_predicted() {
    let out = std::env::temp_dir().join("loft_view_header_on.rs");
    let got = counts(&emit(&cells(), &out, &[]));
    for (name, headers) in EXPECTED {
        let h = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            h, *headers,
            "{name}: view headers — the prediction vs the emission"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_the_runtime_reads() {
    let out = std::env::temp_dir().join("loft_view_header_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_VIEW_HOIST", "1")]);
    assert!(
        !rust.contains("V-n view header"),
        "LOFT_NO_VIEW_HOIST=1 must bind no view header"
    );
    let _ = std::fs::remove_file(&out);
}
