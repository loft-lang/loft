// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-TextRun` — a byte run read once as text, PINNED on the IR of the cells
//! (`tests/scripts/a-byte-run-read-once-as-text-is-read-in-place.loft`): an admitted vector is
//! read as `text_from_byte_range` behind its guard, a declined one keeps its vector.  The
//! cells' values pass on either form, so only this pin sees an admission drift.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-byte-run-read-once-as-text-is-read-in-place.loft";

/// Per function: the in-place reads with the rewrite ON (and OFF: always none).
const EXPECTED: &[(&str, usize)] = &[
    ("n_txt", 1), // a local vector read once as text
    ("n_rec", 1), // the decoder's record: the vector placed in the return buffer
    ("n_t5", 0),  // a second use of the vector
    ("n_t6", 0),  // the offset reassigned between the copy and the read
    ("n_t9", 0),  // a vector not empty before the copy
    ("n_t13", 0), // the read under an `if` whose condition is a call
    ("n_t14", 1), // the read inside an `if` arm
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
        text.contains("fn n_txt("),
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

fn in_place_reads(body: &str) -> usize {
    body.matches("n_text_from_byte_range(").count()
}

#[test]
fn a_run_read_once_as_text_is_read_in_place_and_anything_else_keeps_its_vector() {
    let ir = introspect(&[]);
    for (name, admitted) in EXPECTED {
        assert_eq!(
            in_place_reads(body_of(&ir, name)),
            *admitted,
            "{name}: in-place reads — the cell's admission moved"
        );
    }
}

#[test]
fn the_switch_keeps_every_vector() {
    let ir = introspect(&[("LOFT_NO_BYTE_COPY", "1")]);
    for (name, _) in EXPECTED {
        assert_eq!(
            in_place_reads(body_of(&ir, name)),
            0,
            "{name}: LOFT_NO_BYTE_COPY=1 must keep every vector"
        );
    }
}
