// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Base` × `@FR-I-Range` — which loop bounds read the held header's length, PINNED on
//! the emitted Rust of the cells (`tests/scripts/a-length-bound-reads-the-held-header.loft`):
//! per function, how many loop tests still compare against the prelude's `_range_end` local.
//! The cells answer the same on either form, so only this pin sees the admission move — above
//! all a growing loop taking the substitution, which is the one wrong answer it could give.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-length-bound-reads-the-held-header.loft";

/// Per function: the loop tests against a range-end local with the rewrite ON, then OFF.
// Since `(R-Header)`'s function clause (2026-09-30) a parameter read twice or more outside any
// loop holds ONE header from entry, and a loop over it reads its end off that header
// (`var__range_end_1 = i64::from(__vh_1.len)`): the range-end local is then a plain `i64`
// read once, and the substitution has nothing left to buy, so `n_update` and `n_square_sum`
// keep their two local tests with the rewrite on — the pin reads them against that header,
// not against a runtime lookup.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_update", 2, 2), // in place, growth-free — the function clause holds the header
    ("n_doubled", 0, 2), // through a field path
    ("n_square_sum", 2, 2), // nested — the function clause holds the header
    ("n_grows", 1, 1),  // pushes onto v: grows, keeps its local
    ("n_into_other", 0, 1), // pushes onto ANOTHER vector: since `@FR-R-Base`'s growth clause it holds v's base and reads the header
    ("n_empty_sum", 0, 1),  // growth-free
    ("n_all_but_last", 1, 1), // an end that is not a plain length
];

fn emit(off: bool) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!(
        "range_end_header_{}_{}.rs",
        std::process::id(),
        u8::from(off)
    ));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_RANGE_END_HEADER");
    if off {
        cmd.env("LOFT_NO_RANGE_END_HEADER", "1");
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    assert!(
        text.contains("fn n_update("),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    text
}

/// The emitted `fn <name>(` up to the next top-level `fn `.
fn body_of<'a>(rs: &'a str, name: &str) -> &'a str {
    let head = format!("\nfn {name}(");
    let start = rs
        .find(&head)
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rs[start + head.len()..];
    &rest[..rest.find("\nfn ").unwrap_or(rest.len())]
}

fn range_end_tests(body: &str) -> usize {
    body.match_indices("var__range_end_")
        .filter(|(i, _)| {
            let tail = &body[*i..];
            let end = tail.find(')').unwrap_or(0);
            tail[end..].starts_with(") as i64) <= ")
        })
        .count()
}

#[test]
fn a_growth_free_loop_bounds_itself_by_the_held_header() {
    let on = emit(false);
    let off = emit(true);
    for (name, with, without) in EXPECTED {
        assert_eq!(
            range_end_tests(body_of(&on, name)),
            *with,
            "{name}: range-end tests with the rewrite — its admission moved"
        );
        assert_eq!(
            range_end_tests(body_of(&off, name)),
            *without,
            "{name}: range-end tests under LOFT_NO_RANGE_END_HEADER=1"
        );
    }
}
