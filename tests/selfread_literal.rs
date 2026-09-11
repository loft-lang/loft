// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-w (loft#1426) — a vector LITERAL that reads its own destination hoists
//! those reads into TEMPS evaluated before the build (`build_val`), instead of
//! snapshotting the whole vector (`build_src`) — `(I-Comp)` / `@FR-O-Detach` kept, and
//! the prefix-sum accumulator (`cum += [cum[len(cum)-1]? + d]`) drops from O(n²) to
//! O(n).  The snapshot stays wherever a temp cannot carry the rule: a part with a user
//! call, record elements, a `&`-linked or field destination, and every comprehension.
//!
//! The cell corpus (`bytecode-comparisons/V-w-selfread-literal-cells.loft`) can only say
//! the VALUES hold; this pins WHICH route each cell takes, read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-w-selfread-literal-cells.loft";

/// `(function, whole-vector snapshots, hoisted read temps)`.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_q1", 0, 2), // the prefix-sum accumulator rides a temp
    ("n_q2", 0, 4), // the swap: both reads pre-evaluated (2 temps × decl+use)
    ("n_q3", 0, 4), // both length reads pre-evaluated
    ("n_q4", 4, 0), // a USER call in the part — the snapshot stays
    ("n_q5", 4, 0), // record elements — the snapshot stays
    ("n_q6", 4, 0), // a `&`-linked destination — the snapshot stays
    ("n_q7", 4, 0), // a field destination — the snapshot stays
    ("n_q8", 3, 0), // the comprehension keeps its own SNAPSHOT lowering untouched
    ("n_q9", 0, 2), // the long prefix sum — the linearity carrier
];

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn emit(src: &Path, out: &Path) -> String {
    let status = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read_to_string(out).expect("read the emitted Rust")
}

#[test]
fn each_cell_takes_exactly_the_route_predicted() {
    let out = std::env::temp_dir().join("loft_selfread_literal.rs");
    let rust = emit(&cells(), &out);
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        let row = map.entry(current.clone()).or_default();
        row.0 += line.matches("build_src").count();
        row.1 += line.matches("build_val").count();
    }
    for (name, snap, temps) in EXPECTED {
        let (s, t) = map
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(s, *snap, "{name}: whole-vector snapshot mentions");
        assert_eq!(t, *temps, "{name}: hoisted read-temp mentions");
    }
    let _ = std::fs::remove_file(&out);
}
