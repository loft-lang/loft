// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-LiteralWalk` — a `for` over a scalar vector literal of constant length walks its
//! items as scalar temps behind a counted select, and builds no vector.
//!
//! The cell corpus (`tests/scripts/a-walk-of-a-scalar-literal-builds-no-vector.loft`) can
//! only say the VALUES hold — both backends take the lowering at parse time, so their
//! agreement is no evidence of the shape.  This pins the EMISSION per cell: which walks are
//! the select (`Iter literal`) and build no vector, which declines keep the literal's block
//! (`Vector_`), and that `LOFT_NO_LITERAL_WALK=1` lowers none.  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-walk-of-a-scalar-literal-builds-no-vector.loft";

/// `(function, literal walks, vector literal blocks)` — the predictions beside the cells.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_c1", 1, 0),  // three field reads
    ("n_c2", 1, 0),  // a widening literal: the integers converted to floats
    ("n_c3", 1, 0),  // booleans
    ("n_c4", 1, 0),  // characters
    ("n_c5", 1, 0),  // one item
    ("n_c6", 1, 0),  // break / continue
    ("n_c7", 1, 0),  // x#index
    ("n_c8", 1, 0),  // items are calls, evaluated once each before the loop
    ("n_c9", 1, 0),  // the body writes the loop variable
    ("n_c10", 1, 0), // the items' sources change in the body
    ("n_c11", 2, 0), // two nested literal walks
    ("n_c12", 1, 0), // a filter
    ("n_c13", 0, 1), // rev(…) keeps the vector walk
    ("n_c14", 0, 1), // seventeen items: over the bound
    ("n_c15", 1, 0), // the discard `_`
    ("n_c16", 1, 0), // an early return
    ("n_c17", 0, 1), // text items are no scalars
    ("n_c18", 1, 0), // the items index another vector
    ("n_c19", 1, 0), // a float first, an integer after
    ("n_c20", 1, 0), // the literal walk inside a record walk
    ("n_c21", 0, 1), // a widening literal under rev(…)
];

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

/// Per emitted function: `(literal walks, vector literal blocks)`.
fn counts(rust: &str) -> HashMap<String, (usize, usize)> {
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
            continue;
        }
        let e = map.entry(current.clone()).or_default();
        if line.contains("//Iter literal_") {
            e.0 += 1;
        }
        if line.contains("//Vector_") {
            e.1 += 1;
        }
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_walk_is_the_select_or_the_vector_predicted() {
    let out = std::env::temp_dir().join("loft_literal_walk_on.rs");
    let got = counts(&emit(&cells(), &out, &[]));
    for (name, walks, vectors) in EXPECTED {
        let (w, v) = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            (w, v),
            (*walks, *vectors),
            "{name}: (literal walks, vector blocks)"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_walks_every_literal_as_a_vector() {
    let out = std::env::temp_dir().join("loft_literal_walk_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_LITERAL_WALK", "1")]);
    assert!(
        !rust.contains("//Iter literal_"),
        "LOFT_NO_LITERAL_WALK=1 must lower no literal walk"
    );
    let got = counts(&rust);
    for (name, walks, vectors) in EXPECTED {
        let (_, v) = got[*name];
        assert_eq!(v, walks + vectors, "{name}: every walk builds its vector");
    }
    let _ = std::fs::remove_file(&out);
}
