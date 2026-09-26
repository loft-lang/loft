// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-VecCopy` — the admission of an element-wise vector copy, PINNED on the IR of the
//! cells (`tests/scripts/a-vector-copied-element-by-element-is-one-append.loft`): an admitted
//! walk is one `OpAppendVector`, a declined one keeps the loop.  The cells' values pass on
//! either form, so only this pin sees an admission drift — above all the `&` destination,
//! whose admission is value-identical until a caller hands the source in twice.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-vector-copied-element-by-element-is-one-append.loft";

/// Per function: the bulk appends with the rewrite ON (and OFF: always none).
const EXPECTED: &[(&str, usize)] = &[
    ("n_grow", 1), // the editor's spelling, a local returned inside a record
    ("n_cp", 1),   // the hidden return buffer
    ("n_v3", 1),   // integer? with a null
    ("n_v4", 1),   // float? with a null
    ("n_v5", 1),   // single
    ("n_v6", 1),   // boolean
    ("n_v7", 1),   // enum
    ("n_v8", 2),   // raw and biased bytes
    ("n_v9", 1),   // i32
    ("n_v10", 1),  // onto a filled local
    ("n_onto", 0), // a `&` destination with a second statement
    ("n_into", 0), // a `&` destination alone: its type declines it
    ("n_tail", 0), // a by-value parameter destination: not exclusive
    ("n_v12", 0),  // a source that is a call
    ("n_v13", 0),  // a second statement
    ("n_v14", 0),  // a transformed value
    ("n_v15", 0),  // text and record elements
    ("n_v16", 1),  // an empty source
    ("n_v17", 1),  // re-run in an outer loop
    ("n_v18", 0),  // a slice source
    ("n_v20", 1),  // a field of a field
    ("n_v22", 0),  // a narrower source into a wider destination
];

fn introspect(env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_VEC_COPY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains("fn n_grow("),
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
    body.matches("OpAppendVector(").count()
}

#[test]
fn an_element_wise_copy_is_one_append_and_anything_else_keeps_its_loop() {
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
    let ir = introspect(&[("LOFT_NO_VEC_COPY", "1")]);
    for (name, _) in EXPECTED {
        assert_eq!(
            bulk_appends(body_of(&ir, name)),
            0,
            "{name}: LOFT_NO_VEC_COPY=1 must leave every loop"
        );
    }
}
