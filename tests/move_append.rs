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
//! loop, a rebound destination, a non-struct element) keeps the deep copy — and the switch
//! (`LOFT_NO_MOVE_APPEND=1`).  A struct named like a stdlib type variable was a seventh
//! declining shape until loft#1519; `hoist.rs`'s wrapper-identity check still stands as a
//! net, but nothing reaches it now, so c19 pins the PAIRING.  Read off
//! `--native-emit`.  Every paired cell counts TWO record frees per pair: the placed
//! record dies WITH ITS HOST STORE — a release injected before every free of the host
//! `__vdb` (early dead-after-last-read sites included, the 2026-09-11 gate corruption's
//! soundness line) — and the buffer attr's own scope-end free is the null-guarded
//! backstop that covers an adopted `__retbuf` host, which no in-function site frees.
//! Between those, an enclosing loop REUSES the placement (the −16.6 % fronds ceiling);
//! a host recreated per iteration re-arms the guard at its `OpDatabase` site, whose
//! reuse arm clears the whole store (the c21 corruption when unarmed).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-j-move-append-cells.loft";

/// `(function, placements, moves, record frees)` — the predictions written beside the
/// cells before the emitters existed.
const EXPECTED: &[(&str, usize, usize, usize)] = &[
    ("n_c1", 1, 1, 2), // the base shape pairs
    ("n_c2", 0, 0, 0), // f read after the append — declines
    ("n_c3", 0, 0, 0), // the same f appended twice — declines
    ("n_c4", 0, 0, 0), // a NAMED source (no call in the For header) — declines
    ("n_c5", 1, 1, 2), // break: pairs; the leftovers die in the record free
    ("n_c6", 1, 1, 2), // a view-returning call still DELIVERS a copy into the placed
    // buffer, so the move relocates that copy; the same-store dispatch guards the rest
    ("n_c7", 0, 0, 0),       // two destinations for one f — declines
    ("n_c8", 1, 1, 2),       // the INNER call pairs; the outer f (under the inner loop) declines
    ("n_c9", 0, 0, 0),       // an inner record of the temporary — the source is not the loop var
    ("n_c10", 0, 0, 0),      // vector<R?> — not a plain struct element
    ("n_c11", 1, 1, 2),      // a text field rides the moved bytes (same-store handles)
    ("n_c12", 1, 1, 2),      // the append under an `if` arm runs at most once per iteration
    ("n_c13", 0, 0, 0),      // a container element — declines
    ("n_c14", 0, 0, 0),      // the append under a FURTHER loop — declines (c8's falsifier)
    ("n_c15", 0, 0, 0),      // the destination is reassigned later — declines
    ("n_c16", 1, 1, 2),      // a no-heap element: composes with § V-t's record push
    ("n_collect2", 0, 0, 0), // c17: the callee re-inits its buffer store — declines
    ("n_collectr", 0, 0, 0), // c18: the same through direct recursion — declines
    ("n_c19", 1, 1, 2),      // a struct NAMED `T`.  This cell pinned the DECLINE while the
    // name-based wrapper lookup found the generic template (typevar field); loft#1519 fixed
    // that at its root, so the wrapper is the element's own and the shape pairs like c11 —
    // a text field riding the moved bytes.  Kept as the regression for the naming AND for
    // the pairing being value-neutral here: the cell prints the same under either outcome
    ("n_c20", 1, 1, 2), // loop 1 pairs and its dest dies early; loop 2 declines (read-after)
    ("n_c21", 1, 1, 2), // an enclosing loop re-enters: the host's per-iteration OpDatabase
    // re-arms the placement (its reuse arm cleared the store), the host's one free site
    // and the backstop carry the two frees
    ("n_c22", 2, 2, 4), // TWO paired loops, ONE host: the host's death releases both placed
                        // records (2 injected) and each attr keeps its backstop (2 more)
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
