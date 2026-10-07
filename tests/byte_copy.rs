// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-ByteCopy` — the admission of a byte-wise text copy, PINNED on the IR of the cells
//! (`tests/scripts/a-byte-wise-text-copy-is-one-append.loft`): an admitted loop is one
//! `OpAppendTextBytes` behind its guard, a declined one keeps the loop.  The cells' values
//! pass on either form, so only this pin sees an admission drift.
use std::path::{Path, PathBuf};

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
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
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

/// `(R-ByteCopy)`'s vector clause, pinned on its own cells
/// (`tests/scripts/a-byte-range-of-a-vector-copied-one-at-a-time-is-one-append.loft`): an
/// admitted loop is one `OpSliceVector` behind its guard, a declined one keeps the loop.
const VECTOR_CELLS: &str =
    "tests/scripts/a-byte-range-of-a-vector-copied-one-at-a-time-is-one-append.loft";

/// Per function: the slice appends with the rewrite ON (and OFF: always none).
const VECTOR_EXPECTED: &[(&str, usize)] = &[
    ("n_rd", 1),     // the decoder's spelling, `v[a + k] ?? 0`
    ("n_rd_rec", 1), // the destination placed in the return buffer's store
    ("n_v2", 1),     // `v[k + a]`
    ("n_v3", 1),     // the plain index, literal bounds
    ("n_v4", 1),     // a computed range
    ("n_v5", 1),     // an end past the source: admitted, the guard refuses it at run time
    ("n_v6", 1),     // a negative offset: likewise
    ("n_v7", 1),     // onto a filled vector, a literal offset
    ("n_v8", 1),     // an empty range
    ("n_v9", 0),     // the destination is the source
    ("n_v10", 0),    // an integer vector is no byte push
    ("n_v11", 0),    // a second statement in the body
    ("n_v12", 1),    // 5000 bytes
    ("n_v15", 1),    // a default unlike every byte
    ("n_into", 0),   // a `&` destination
    ("n_v17", 0),    // a field as the source
    ("n_v18", 0),    // a default that is a variable
    ("n_v19", 0),    // a float run: its element can hold null
    ("n_v20", 1),    // the copy inside an `if` arm
];

fn introspect_cells(cells: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(cells);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
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
        text.contains("fn n_rd("),
        "no IR printed (exit {:?}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    text
}

#[test]
fn a_byte_run_of_a_vector_is_one_slice_append_and_anything_else_keeps_its_loop() {
    let ir = introspect_cells(VECTOR_CELLS, &[]);
    for (name, admitted) in VECTOR_EXPECTED {
        assert_eq!(
            body_of(&ir, name).matches("OpSliceVector(").count(),
            *admitted,
            "{name}: slice appends — the cell's admission moved"
        );
    }
    let off = introspect_cells(VECTOR_CELLS, &[("LOFT_NO_BYTE_COPY", "1")]);
    for (name, _) in VECTOR_EXPECTED {
        assert_eq!(
            body_of(&off, name).matches("OpSliceVector(").count(),
            0,
            "{name}: LOFT_NO_BYTE_COPY=1 must leave every loop"
        );
    }
}
