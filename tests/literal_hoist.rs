// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-x (`@FR-R-LitHoist`) — an invariant loop-body vector literal builds ONCE:
//! the local is pre-declared at function top and its declaration (the one wrapped `Set`,
//! or the flat `OpDatabase · Set · pushes` statement run) is guarded on the local being
//! unbound, so every later iteration and re-entry reuses the store.  The cell corpus
//! (`bytecode-comparisons/V-x-invariant-literal-cells.loft`) says the VALUES hold; this
//! pins the EMISSION per cell — which declarations earn the guard, and that every
//! declining shape (an append, a loop-var part, an element write, an escape, a heap
//! element, a non-const param read, a sanitized-name collision) keeps the per-iteration
//! build — and the switch (`LOFT_NO_LITERAL_HOIST=1`).  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-x-invariant-literal-cells.loft";

/// `(function, guarded builds)` — the predictions written beside the cells.
const EXPECTED: &[(&str, usize)] = &[
    ("n_c1", 1),           // the base flat literal
    ("n_mirror_total", 1), // the fronds shape: if-of-literals on a const-param field
    ("n_c3", 0),           // the body appends — the push joins the run and its part varies
    ("n_c4", 0),           // a loop-var part
    ("n_c5", 0),           // an element write (the context-blind OpGetVector lvalue)
    ("n_c6", 0),           // a whole-value escape
    ("n_c7", 0),           // a text element owns heap
    ("n_sum_nc", 0),       // a non-const parameter read
    ("n_c9", 1),           // nested loops: one activation-level build
    ("n_scaled_total", 1), // const-param arithmetic parts
    ("n_c11", 0),          // two same-named locals would share the one fn-top binding
];

const MARK: &str = "V-x invariant literal";

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

fn guard_counts(rust: &str) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        *map.entry(current.clone()).or_default() += line.matches(MARK).count();
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_guards_exactly_the_builds_predicted() {
    let out = std::env::temp_dir().join("loft_literal_hoist_on.rs");
    let rust = emit(&cells(), &out, &[]);
    let got = guard_counts(&rust);
    for (name, guards) in EXPECTED {
        let g = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(g, *guards, "{name}: guarded invariant-literal builds");
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_the_per_iteration_build() {
    let out = std::env::temp_dir().join("loft_literal_hoist_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_LITERAL_HOIST", "1")]);
    assert_eq!(
        rust.matches(MARK).count(),
        0,
        "under LOFT_NO_LITERAL_HOIST=1 every literal rebuilds per iteration"
    );
    let _ = std::fs::remove_file(&out);
}
