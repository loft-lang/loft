// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-j (loft#1426) — the move-append: in `for f in call(…) { v += [f] }` where
//! the loop variable's single use after binding is that one append, the call's hidden
//! `__ref` buffer is PLACED as a record inside `v`'s own store
//! (`Stores::place_record_in`), the append relocates the element's bytes and zeroes the
//! source (`move_record_shallow` — the deep copy's two claims and two frees per element
//! gone), and the buffer's free is a record-level release (`free_record_in`) inside the
//! store that lives on.
//!
//! The cell corpus (`bytecode-comparisons/V-j-move-append-cells.loft`) can only say the
//! VALUES hold; this pins the EMISSION per cell — which loops pair, and that every
//! declining shape (a read after the append, a double append, a named source, a nested
//! loop, a rebound destination, a non-struct element) keeps the deep copy — and the
//! switch (`LOFT_NO_MOVE_APPEND=1`).  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-j-move-append-cells.loft";

/// `(function, placements, moves, record frees)` — the predictions written beside the
/// cells before the emitters existed.
const EXPECTED: &[(&str, usize, usize, usize)] = &[
    ("n_c1", 1, 1, 1), // the base shape pairs
    ("n_c2", 0, 0, 0), // f read after the append — declines
    ("n_c3", 0, 0, 0), // the same f appended twice — declines
    ("n_c4", 0, 0, 0), // a NAMED source (no call in the For header) — declines
    ("n_c5", 1, 1, 1), // break: pairs; the leftovers die in the record free
    ("n_c6", 1, 1, 1), // a view-returning call still DELIVERS a copy into the placed
    // buffer, so the move relocates that copy; the same-store dispatch guards the rest
    ("n_c7", 0, 0, 0),  // two destinations for one f — declines
    ("n_c8", 1, 1, 1),  // the INNER call pairs; the outer f (under the inner loop) declines
    ("n_c9", 0, 0, 0),  // an inner record of the temporary — the source is not the loop var
    ("n_c10", 0, 0, 0), // vector<R?> — not a plain struct element
    ("n_c11", 1, 1, 1), // a text field rides the moved bytes (same-store handles)
    ("n_c12", 1, 1, 1), // the append under an `if` arm runs at most once per iteration
    ("n_c13", 0, 0, 0), // a container element — declines
    ("n_c14", 0, 0, 0), // the append under a FURTHER loop — declines (c8's falsifier)
    ("n_c15", 0, 0, 0), // the destination is reassigned later — declines
    ("n_c16", 1, 1, 1), // a no-heap element: composes with § V-t's record push
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

/// Per emitted function: `(placements, moves, record frees)`.
fn counts(rust: &str) -> HashMap<String, (usize, usize, usize)> {
    let mut map: HashMap<String, (usize, usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        let row = map.entry(current.clone()).or_default();
        row.0 += line.matches("place_record_in").count();
        row.1 += line.matches("move_record_shallow").count();
        row.2 += line.matches("free_record_in").count();
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_pairs_exactly_the_loops_predicted() {
    let out = std::env::temp_dir().join("loft_move_append_on.rs");
    let rust = emit(&cells(), &out, &[]);
    let got = counts(&rust);
    for (name, place, mv, free) in EXPECTED {
        let (p, m, f) = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(p, *place, "{name}: placed buffers");
        assert_eq!(m, *mv, "{name}: move sites");
        assert_eq!(f, *free, "{name}: record-level frees");
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_the_deep_copy() {
    let out = std::env::temp_dir().join("loft_move_append_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_MOVE_APPEND", "1")]);
    let got = counts(&rust);
    for (name, ..) in EXPECTED {
        let (p, m, f) = got.get(*name).copied().unwrap_or((0, 0, 0));
        assert_eq!(
            p + m + f,
            0,
            "{name}: under LOFT_NO_MOVE_APPEND=1 every append keeps the deep copy"
        );
    }
    let _ = std::fs::remove_file(&out);
}
