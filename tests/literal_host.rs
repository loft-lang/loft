// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-MoveLast`'s host clause — which cells it ADMITS.  The cell corpus
//! (`tests/scripts/a-literal-takes-a-dead-vector-into-its-store.loft`) says the VALUES hold on
//! both backends in both switch states; a declined cell keeps the copy and answers the same
//! values, so only the emission can say the clause fired.  An admitted literal takes its vector
//! by one `OpMoveVector`; a declined one, or the clause switched off (`LOFT_NO_LITERAL_HOST=1`),
//! emits none.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-literal-takes-a-dead-vector-into-its-store.loft";

/// Per function: the handle moves it emits.  c8 is `Args::parse`'s cursor, matched by slice
/// patterns: its match subject is a view of the literal, which names the literal's own field
/// and so does not decline the move as a view of the vector would (d2's `_elm_` view).
const EXPECTED: &[(&str, usize)] = &[
    ("c1", 1),
    ("c2", 0),
    ("c3", 1),
    ("c5", 1),
    ("c6", 0),
    ("c7", 1),
    ("c8", 1),
    ("c9", 1),
    ("d1", 0),
    ("d2", 0),
    ("d3", 0),
    ("d4", 0),
    ("d5", 0),
];

fn introspect(env: &[(&str, &str)]) -> String {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "240")
        .env_remove("LOFT_NO_LITERAL_HOST");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "introspect failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The handle moves in one function's IR: from its `{#block` header to the next function's.
fn moves_in(text: &str, func: &str) -> usize {
    let head = format!("fn n_{func}(");
    let start = text
        .match_indices(&head)
        .map(|(i, _)| i)
        .find(|i| {
            text[*i..]
                .lines()
                .next()
                .is_some_and(|l| l.contains("{#block"))
        })
        .unwrap_or_else(|| panic!("{func}'s IR"));
    let rest = &text[start + head.len()..];
    let end = rest.find("\nfn ").unwrap_or(rest.len());
    rest[..end].matches("OpMoveVector(").count()
}

#[test]
fn the_host_clause_admits_exactly_its_cells() {
    let text = introspect(&[]);
    let got: Vec<(&str, usize)> = EXPECTED
        .iter()
        .map(|(f, _)| (*f, moves_in(&text, f)))
        .collect();
    assert_eq!(got, EXPECTED, "handle moves per cell");
}

#[test]
fn switched_off_the_host_clause_moves_nothing() {
    let text = introspect(&[("LOFT_NO_LITERAL_HOST", "1")]);
    for (f, _) in EXPECTED {
        assert_eq!(moves_in(&text, f), 0, "{f} under LOFT_NO_LITERAL_HOST=1");
    }
}
