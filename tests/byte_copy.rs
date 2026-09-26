// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-ByteCopy` — the admission of a byte-wise text copy, PINNED on the IR of the cells
//! (`tests/scripts/a-byte-wise-text-copy-is-one-append.loft`): an admitted loop is one
//! `OpAppendTextBytes` behind its guard, a declined one keeps the loop.  The cells' values
//! pass on either form, so only this pin sees an admission drift.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-byte-wise-text-copy-is-one-append.loft";

/// Per function: the bulk appends with the rewrite ON (and OFF: always none).
const EXPECTED: &[(&str, usize)] = &[
    ("n_copy_all", 1), // the encoder's spelling
    ("n_b3", 1),       // literal bounds
    ("n_b4", 1),       // variable bounds
    ("n_b5", 1),       // an end past the text: admitted, the guard refuses it at run time
    ("n_b6", 1),       // an empty range
    ("n_b7", 1),       // onto a filled vector
    ("n_b8", 0),       // an integer destination is no byte push
    ("n_b9", 0),       // a second statement in the body
    ("n_b10", 0),      // a text field as the source
    ("n_b11", 1),      // the bound spelled `size(t)` in the range
    ("n_b12", 2),      // the same text twice
    ("n_b14", 0),      // a bound that is a call
];

fn introspect(env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_BYTE_COPY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains("fn n_b3("),
        "no IR printed (exit {:?}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    text
}

/// The IR of `fn <name>(` up to the next top-level `fn `.
fn body_of<'a>(ir: &'a str, name: &str) -> &'a str {
    let head = format!("fn {name}(");
    let start = ir
        .find(&head)
        .unwrap_or_else(|| panic!("{name} was not printed"));
    let rest = &ir[start + head.len()..];
    let end = rest.find("\nfn ").unwrap_or(rest.len());
    &rest[..end]
}

fn bulk_appends(body: &str) -> usize {
    body.matches("OpAppendTextBytes(").count()
}

#[test]
fn a_byte_wise_copy_is_one_append_and_anything_else_keeps_its_loop() {
    let ir = introspect(&[]);
    for (name, admitted) in EXPECTED {
        assert_eq!(
            bulk_appends(body_of(&ir, name)),
            *admitted,
            "{name}: bulk appends — the cell's admission moved"
        );
    }
}

#[test]
fn the_switch_keeps_every_loop() {
    let ir = introspect(&[("LOFT_NO_BYTE_COPY", "1")]);
    for (name, _) in EXPECTED {
        assert_eq!(
            bulk_appends(body_of(&ir, name)),
            0,
            "{name}: LOFT_NO_BYTE_COPY=1 must leave every loop"
        );
    }
}
