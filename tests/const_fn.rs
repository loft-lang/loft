// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-Const` — the admission of a literal-bodied function's calls, PINNED on the emitted
//! Rust of the cells (`tests/scripts/a-literal-bodied-function-is-a-constant.loft`): a call
//! in a read position reads `const_ref_at_runtime`, a call anywhere else stays `n_codes(…)`.
//! The cells' values pass on either form, so only this pin sees an admission drift.

use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-literal-bodied-function-is-a-constant.loft";

/// Per cell function: (constant reads, calls of `n_codes`/`n_rows`/`n_names`/`n_with_call` left).
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_c1", 1, 0),   // the read-only bind
    ("n_c2", 1, 0),   // the direct index
    ("n_c3", 1, 0),   // the iteration
    ("n_c4", 1, 0),   // the length
    ("n_c5", 1, 1),   // the written local keeps its call; the direct read beside it is served
    ("n_c6", 1, 1),   // an append keeps the call
    ("n_c7", 1, 1),   // handed to a callee that writes: the call; the direct read beside it served
    ("n_c8", 0, 1), // handed to a callee that only reads: still the call (an argument may be written through)
    ("n_c9", 1, 1), // stored into a field: the call
    ("n_c10", 0, 1), // returned: the call
    ("n_c11", 0, 1), // copied into a local that is written: the call
    ("n_c12", 1, 0), // iterated through the bound local: served through the alias
    ("n_c13", 0, 1), // a record table read through `?`: declined (the discharge mints)
    ("n_c14", 1, 0), // a text table
    ("n_c16b", 0, 1), // a body with a call before its literal is a function: the call stays
    ("n_c18", 1, 0), // a folded literal
    ("n_c19", 1, 0), // the glyph-table shape
    ("n_c21", 2, 0), // two calls in one frame
    ("n_c22", 1, 0), // a call inside a loop
    ("n_c23", 1, 1), // a written copy beside a read-only one
];

fn emit(env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out =
        std::env::temp_dir().join(format!("const_fn_{}_{}.rs", std::process::id(), env.len()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_CONST_VIEW");
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

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

#[test]
fn a_read_only_call_reads_the_constant_and_any_other_keeps_the_call() {
    let rust = emit(&[]);
    for (name, reads, calls) in EXPECTED {
        let body = body_of(&rust, name);
        let got_reads = count(body, "const_ref_at_runtime(");
        let got_calls = count(body, "n_codes(cell")
            + count(body, "n_rows(cell")
            + count(body, "n_names(cell")
            + count(body, "n_with_call(cell");
        assert_eq!(
            (got_reads, got_calls),
            (*reads, *calls),
            "{name}: (constant reads, calls left) — the cell's admission moved"
        );
    }
}

#[test]
fn the_switch_keeps_every_call() {
    let rust = emit(&[("LOFT_NO_CONST_VIEW", "1")]);
    assert_eq!(
        count(&rust, "const_ref_at_runtime("),
        0,
        "LOFT_NO_CONST_VIEW=1 must leave no call reading the constant store"
    );
    assert!(
        count(body_of(&rust, "n_c1"), "n_codes(cell") == 1,
        "c1 keeps its call under the switch"
    );
}
