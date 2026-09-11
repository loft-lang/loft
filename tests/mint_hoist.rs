// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-s (loft#1426) — a loop that APPENDS a record element (the mint group:
//! `OpPreAllocVector · OpNewRecord · in-place sets or a retbuf callee · OpFinishRecord`)
//! still hoists its invariant record scalars on the native backend.
//!
//! The mint is a mover admitted like a push (`@FR-R-Mint` beside `@FR-R-Alias`); a write
//! into the freshly minted element — a literal element's direct sets, § V-d's
//! `OpCopyRecord` delivery tail — evicts nothing, because a record minted inside the body
//! cannot be named by a variable whose getter the prelude already ran.  The cell corpus
//! (`bytecode-comparisons/V-s-mint-hoist-cells.loft`) can only say the VALUES hold; this
//! pins the EMISSION per cell — which cells hoist, how many scalars each — and the switch
//! (`LOFT_NO_MINT_HOIST=1`) that restores the per-iteration reads, which is what makes it
//! red on the build before the admission and on one that lost it.  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-s-mint-hoist-cells.loft";

/// `(function, hoisted scalars)` — the predictions written beside the cells before the
/// admission existed, and confirmed by the first run.
const EXPECTED: &[(&str, usize)] = &[
    ("n_seg1", 4),   // c1: the literal append — a.ptx, a.pty, b.ptx, b.pty
    ("n_seg2", 4),   // c2: the call append (smooth's shape), § V-d delivery admitted
    ("n_mint3", 2),  // c3: an owned local out vector
    ("n_mint4", 1),  // c4: growth across 100 elements
    ("n_mint5", 2),  // c5: a mint and a fused push, two exclusive movers
    ("n_mint6", 3),  // c6: boolean, integer, float kinds
    ("n_mint7", 0),  // c7: a keyed container is a keyed insert — not admitted
    ("n_mint8", 0),  // c8: index of one-field records — keyed, and a field path
    ("n_mint9", 0),  // c9: the loop writes the field through the parameter
    ("n_mint10", 0), // c10: the write arrives through a `&` alias — evicted by type
    ("n_mint11", 0), // c11: the pushed vector is rebound — the loop declines
    ("n_mint12", 0), // c12: the `v[i]?` discharge's default-record OpDatabase blocks
    ("n_mint13", 2), // c13: same-type local survives the mint's own element writes
    ("n_mint14", 0), // c14: a field-path mint is outside this unit's admission
    ("n_mint16", 1), // c16: outer mints and hoists a.ptx; a.pty evicted by the element write
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

/// Per emitted function: how many `__vs_` scalars its preludes bind.
fn scalar_counts(rust: &str) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        if line.contains("let __vs_") {
            *map.entry(current.clone()).or_default() += 1;
        }
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_hoists_exactly_the_scalars_predicted() {
    let out = std::env::temp_dir().join("loft_mint_hoist_on.rs");
    let rust = emit(&cells(), &out, &[]);
    let got = scalar_counts(&rust);
    for (name, scalars) in EXPECTED {
        let s = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            s, *scalars,
            "{name}: hoisted scalars — the cell's prediction vs the emitted prelude"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_the_per_iteration_reads() {
    let out = std::env::temp_dir().join("loft_mint_hoist_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_MINT_HOIST", "1")]);
    let got = scalar_counts(&rust);
    for (name, _) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or(0),
            0,
            "{name}: under LOFT_NO_MINT_HOIST=1 a record-appending loop hoists nothing"
        );
    }
    let _ = std::fs::remove_file(&out);
}
