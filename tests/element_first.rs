// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-z (`@FR-R-ElemFirst`) — the element-first build: a local vector consumed
//! exactly once as a record-literal field of an append is built INSIDE the appended
//! element (minted at the first temp's declaration; the length bump stays at the
//! append's finish), and the reservation, the mint, the paired handle-zeros and the
//! paired `OpAppendVector` copies vanish from the append site.  The cell corpus
//! (`bytecode-comparisons/V-z-element-first-cells.loft`) says the VALUES hold; this
//! pins the EMISSION — which appends earn the early mint, and that every declining
//! shape (a read after the append, an `out` read in between, an append under an `if`
//! arm) keeps the temp-store build — and the switch (`LOFT_NO_ELEMENT_FIRST=1`).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-z-element-first-cells.loft";

/// `(function, elements minted at a declaration site)`.
const EXPECTED: &[(&str, usize)] = &[
    ("n_c1", 1), // the base shape: two temps, one early mint
    ("n_c2", 1), // the fronds shape: builds under if arms, append after
    ("n_c3", 0), // a temp read AFTER the append
    ("n_c4", 1), // two appends: the SECOND pairs (the first keeps its copy and merely
    // reads the slot), the first is declined by the use-reconciliation
    ("n_c5", 0), // `len(out)` read between declaration and append
    ("n_c6", 1), // a single-field wrapper
    ("n_c7", 0), // the append under an if arm — the early mint would strand elements
    ("n_c8", 1), // text elements ride the in-place build
    ("n_c9", 0), // the temp declared BEFORE the destination — the prelude would
                 // precede the destination's own binding (the sqldb fixture's E0425)
];

const MARK: &str = "V-z element minted";

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

fn counts(rust: &str) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        *map.entry(current.clone()).or_default() += line.matches(MARK).count();
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_mints_exactly_the_elements_predicted() {
    let out = std::env::temp_dir().join("loft_element_first_on.rs");
    let rust = emit(&cells(), &out, &[]);
    let got = counts(&rust);
    for (name, mints) in EXPECTED {
        let g = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(g, *mints, "{name}: elements minted at a declaration site");
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_the_temp_store_build() {
    let out = std::env::temp_dir().join("loft_element_first_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_ELEMENT_FIRST", "1")]);
    assert_eq!(
        rust.matches(MARK).count(),
        0,
        "under LOFT_NO_ELEMENT_FIRST=1 every literal field keeps its temp-store build"
    );
    let _ = std::fs::remove_file(&out);
}

/// @PLN164 E-2 — the element-first build reaches a record's COLLECTION FIELD and a temp a CALL
/// fills: `tests/scripts/164-element-place.loft`.  `(function, elements minted at a declaration)`.
const PLACE_CELLS: &str = "tests/scripts/164-element-place.loft";
const PLACE_MARK: &str = "E-2 element minted";
const PLACE_EXPECTED: &[(&str, usize)] = &[
    ("n_g1", 1),  // a call result appended once, viewed by the result
    ("n_g2", 1),  // a literal-built local and a second local into one element
    ("n_g3", 0),  // a `return` between the call and the append
    ("n_g4", 0),  // the container named in between
    ("n_g5", 0),  // the callee mints into its buffer
    ("n_g6", 0),  // an argument reads the destination
    ("n_g7", 0),  // the local read after the append
    ("n_g8", 0),  // a record-form exit copies the local: a second consumer
    ("n_g9", 1),  // repeated by a loop
    ("n_g10", 1), // two calls into one element
    ("n_g11", 1), // a local record's collection
    ("n_g13", 0), // the local rebound under a condition (loft#1552)
    ("n_g14", 0), // a `continue` between the call and the append
];

fn place_counts(rust: &str) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        *map.entry(current.clone()).or_default() += line.matches(PLACE_MARK).count();
    }
    map
}

#[test]
fn a_parameter_collection_and_a_call_take_the_element_first_build() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(PLACE_CELLS);
    let out = std::env::temp_dir().join("loft_element_place_on.rs");
    let rust = emit(&src, &out, &[]);
    let got = place_counts(&rust);
    for (name, mints) in PLACE_EXPECTED {
        let g = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(g, *mints, "{name}: elements minted at a declaration site");
    }
    // The call is handed the element's field as its buffer.
    assert!(
        rust.contains("= n_mkpts(cell, var_n, DbRef { store_nr: var__elm_1.store_nr, rec: var__elm_1.rec, pos: var__elm_1.pos + 20 });"),
        "g1's call builds into the element"
    );
    let _ = std::fs::remove_file(&out);
    let off = emit(
        &src,
        &std::env::temp_dir().join("loft_element_place_off.rs"),
        &[("LOFT_NO_ELEMENT_PLACE", "1")],
    );
    assert_eq!(
        off.matches(PLACE_MARK).count(),
        0,
        "under LOFT_NO_ELEMENT_PLACE=1 no parameter collection or call takes the build"
    );
    let _ = std::fs::remove_file(std::env::temp_dir().join("loft_element_place_off.rs"));
}
