// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 P4c (loft#1426) — a loop's INVARIANT record scalar field reads are read ONCE, into
//! a local the loop body then uses, on the native backend.
//!
//! The cell corpus (`bytecode-comparisons/P4c-scalar-hoist-cells.loft`) can only say the VALUES
//! hold, because a hoisted read and a per-iteration read of an invariant field answer the same
//! number.  This pins the EMISSION per cell — which cells hoist, how many scalars each — and the
//! switch (`LOFT_NO_SCALAR_HOIST=1`) that restores the per-iteration reads, which is what makes
//! it red on the build before the hoist and on one that lost it.  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/P4c-scalar-hoist-cells.loft";

/// `(function, hoisted scalars, hoisted vector headers)` — the predictions written beside
/// the cells before the emitter existed, and confirmed by the first run.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_raster", 2, 1), // c1: x0 + lw beside the px header
    ("n_c2", 0, 0),     // writes the hoisted field itself
    ("n_c3", 1, 0),     // another type at the same offset
    ("n_c4", 0, 0),     // a callee writes the field
    ("n_c5", 0, 0),     // an alias writes it
    ("n_c6", 0, 0),     // the view is rebound
    ("n_c7", 0, 1),     // an element write of the same type at the offset (pts keeps its header)
    ("n_c8", 1, 0),     // the inner loop only
    ("n_c9", 1, 0), // a growing write that is a fusable PUSH hoists since § V-q: x0 kept, px takes a push header (`__ph_`, not counted here)
    ("n_c10", 4, 0), // boolean, enum, character, float
    ("n_c11", 1, 0), // lw evicted through the callee, x0 kept
    ("n_c12", 1, 0), // a retbuf writer beside a rebound result
    ("n_c13", 0, 0), // a path read is not a candidate; h.n is written
    ("n_c14", 1, 1), // an element write of ANOTHER type at the offset
    ("n_c15", 0, 1), // an element write of the SAME type: conservative eviction (pts keeps its header)
    ("n_c16", 1, 0), // a nested-path write of another root
    ("n_c17", 0, 0), // a text read keeps the loop off the allow-list
    ("n_c18", 2, 1), // narrow integers
    ("n_c19", 2, 0), // a read as a call argument in an if-arm
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

/// Per emitted function: how many `__vs_` scalars and `__vh_` headers its preludes bind.
fn counts(rust: &str) -> HashMap<String, (usize, usize)> {
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        if line.contains("let __vs_") {
            map.entry(current.clone()).or_default().0 += 1;
        }
        if line.contains("let __vh_") {
            map.entry(current.clone()).or_default().1 += 1;
        }
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_hoists_exactly_the_scalars_predicted() {
    let out = std::env::temp_dir().join("loft_scalar_hoist_on.rs");
    let rust = emit(&cells(), &out, &[]);
    let got = counts(&rust);
    for (name, scalars, headers) in EXPECTED {
        let (s, h) = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            (s, h),
            (*scalars, *headers),
            "{name}: (scalars, headers) — the cell's prediction vs the emitted preludes"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_verify_form_re_reads_every_hoisted_scalar() {
    let out = std::env::temp_dir().join("loft_scalar_hoist_verify.rs");
    let rust = emit(&cells(), &out, &[("LOFT_HOIST_VERIFY", "1")]);
    let hoisted: usize = EXPECTED.iter().map(|(_, s, _)| *s).sum();
    let checked = rust.matches("vector::hoisted_scalar_verify(").count();
    assert!(
        checked >= hoisted,
        "under LOFT_HOIST_VERIFY=1 every hoisted read is compared against a fresh one: \
         {hoisted} scalars hoisted, {checked} checking reads emitted"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_the_per_iteration_reads() {
    let out = std::env::temp_dir().join("loft_scalar_hoist_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_SCALAR_HOIST", "1")]);
    assert!(
        !rust.contains("let __vs_"),
        "LOFT_NO_SCALAR_HOIST=1 must bind no scalar local"
    );
    // The vector headers are a separate mechanism and stay.
    let got = counts(&rust);
    assert_eq!(
        got.get("n_raster").map(|c| c.1),
        Some(1),
        "the vector header hoist is untouched by the scalar switch"
    );
    let _ = std::fs::remove_file(&out);
}
