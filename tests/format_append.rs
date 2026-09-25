// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-FormatAppend` — the admission of a format string appended to a text, PINNED on the
//! IR of the cells (`tests/scripts/a-format-appended-to-a-text-is-written-into-it.loft`): an
//! admitted append has no `Formatted string` block under its `OpAppendText`, a declined one
//! keeps it.  The cells' values pass on either form, so only this pin sees an admission drift.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-format-appended-to-a-text-is-written-into-it.loft";

/// Per function: the appends that keep their buffer with the rewrite ON, and with it OFF.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_fa1", 0, 1),  // a character hole into a local
    ("n_fa2", 0, 4),  // text, bracketed, float and spec'd holes
    ("n_add", 0, 1),  // a `&text` parameter
    ("n_esc", 0, 1),  // the promoted return buffer
    ("n_fa5", 2, 2),  // a part reads the destination: kept
    ("n_fa6", 0, 2),  // a null character
    ("n_fa7", 2, 2),  // a FIELD destination: kept
    ("n_fa8", 0, 1),  // two holes and a tail
    ("n_fa9", 0, 1),  // an expression hole
    ("n_fa10", 2, 2), // a call reading the destination: kept
    ("n_fa11", 0, 4), // null integer and text holes
    ("n_fa12", 0, 3), // a boolean hole; the same character twice a pass
];

fn introspect(env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_FORMAT_APPEND");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains("fn n_fa1("),
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

/// An append that keeps its buffer: the `Formatted string` block is the append's operand.
fn buffered_appends(body: &str) -> usize {
    body.lines()
        .filter(|l| {
            (l.contains("OpAppendText(") || l.contains("OpAppendStackText("))
                && l.contains("{#Formatted string")
        })
        .count()
}

#[test]
fn an_append_is_written_into_its_destination_and_a_reader_of_it_keeps_the_buffer() {
    let ir = introspect(&[]);
    for (name, kept, _) in EXPECTED {
        assert_eq!(
            buffered_appends(body_of(&ir, name)),
            *kept,
            "{name}: appends keeping their buffer — the cell's admission moved"
        );
    }
}

#[test]
fn the_switch_keeps_every_buffer() {
    let ir = introspect(&[("LOFT_NO_FORMAT_APPEND", "1")]);
    for (name, _, all) in EXPECTED {
        assert_eq!(
            buffered_appends(body_of(&ir, name)),
            *all,
            "{name}: LOFT_NO_FORMAT_APPEND=1 must leave every append on its buffer"
        );
    }
}
