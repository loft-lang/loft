// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-Compact` — which rebuilds are compacted, PINNED on the emitted Rust of the cells
//! (`tests/scripts/a-vector-rebuilt-from-a-run-of-its-own-elements-is-compacted-in-place.loft`):
//! an admitted rebuild reads `keep_vector_range(`, a declined one does not.  The cells' values
//! pass on either form, so only this pin sees an admission drift.

use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "tests/scripts/a-vector-rebuilt-from-a-run-of-its-own-elements-is-compacted-in-place.loft";

/// Per cell function: how many in-place keeps its body emits.
const EXPECTED: &[(&str, usize)] = &[
    ("n_c1", 1),  // the prefix of a field
    ("n_c2", 1),  // the suffix of a field
    ("n_c3", 1),  // the prefix of a local
    ("n_c4", 1),  // both bounds computed
    ("n_c10", 0), // a bound that is an expression keeps the rebuild
    ("n_c11", 0), // an inclusive range keeps the rebuild
    ("n_c12", 0), // a scalar element keeps the rebuild
    ("n_c13", 0), // a statement beside the append
    ("n_c14", 0), // the local read after the rebind
    ("n_c15", 0), // a run of another vector
    ("n_c16", 2), // two rebuilds in one function
    ("n_c17", 1), // a rebuild in a loop
    ("n_c18", 1), // elements with declared defaults
];

fn emit(env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!("compact_{}_{}.rs", std::process::id(), env.len()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_COMPACT");
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
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    let _ = std::fs::remove_file(&out);
    rust
}

/// The body of `fn <name>(` up to the next top-level `fn `.
fn body_of<'a>(rust: &'a str, name: &str) -> &'a str {
    let head = format!("fn {name}(");
    let start = rust
        .find(&head)
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + head.len()..];
    let end = rest.find("\nfn ").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn a_run_of_the_vectors_own_elements_is_kept_in_place_and_anything_else_is_rebuilt() {
    let rust = emit(&[]);
    for (name, keeps) in EXPECTED {
        let got = body_of(&rust, name).matches("keep_vector_range(").count();
        assert_eq!(
            got, *keeps,
            "{name}: in-place keeps — the cell's admission moved"
        );
    }
}

#[test]
fn the_switch_keeps_every_rebuild() {
    let rust = emit(&[("LOFT_NO_COMPACT", "1")]);
    assert_eq!(
        rust.matches("keep_vector_range(").count(),
        0,
        "LOFT_NO_COMPACT=1 must leave no rebuild compacted"
    );
}
