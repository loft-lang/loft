// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `LOFT_POISON_CLAIM=1` — the claim-side twin of `LOFT_POISON`'s poison-on-free, and the
//! falsifier for *"does this caller rely on zero-on-claim?"*.  A freshly claimed payload
//! reads `0xDEADBEEF`, so a caller relying on zero-init fails loudly instead of inheriting
//! recycled bytes that happen to look like zeros (which is why `LOFT_NO_ZERO_CLAIM=1` was
//! never a real test of the question).
//!
//! This pins the census the owner's ruling is measured against: fix the callers that rely
//! on zeros rather than paying a memset on every claim.  The number only moves DOWN — a
//! new dependence is a regression, and the remaining ones are named in
//! `doc/claude/plans/157-native-4x-drawing/DESIGN.md` § Zero-on-claim.
use std::path::{Path, PathBuf};
use std::process::Command;

/// The scripts that still read un-initialised store words, by name.  Each is a PRODUCER
/// that does not initialise what it hands out; the fix is at that site, never a wider
/// memset.  Shrink this list; do not grow it.
const KNOWN_DEPENDENT: &[&str] = &[
    "40-par-ref-return",
    "75-native-stub",
    "945-stdlib-worked-examples",
    "987-par-empty-body-discard",
    "a-keyed-view-joins-a-nullable-element-vector",
    "json-walker-absent-field",
];

fn scripts() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts");
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("tests/scripts")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "loft"))
        .filter(|p| {
            let src = std::fs::read_to_string(p).unwrap_or_default();
            !src.contains("@EXPECT_ERROR") && !src.contains("@IGNORE")
        })
        .collect();
    out.sort();
    out
}

#[test]
fn the_zero_on_claim_census_only_shrinks() {
    let mut dependent: Vec<String> = Vec::new();
    for p in scripts() {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--interpret")
            .arg(&p)
            .env("LOFT_POISON_CLAIM", "1")
            .env("LOFT_TIMEOUT", "60")
            .output()
            .expect("spawn loft");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if text.contains("panicked")
            || text.contains("BUG (#")
            || text.contains("refused to walk")
            || text.contains("out of bounds")
        {
            dependent.push(p.file_stem().unwrap().to_string_lossy().to_string());
        }
    }
    let unexpected: Vec<&String> = dependent
        .iter()
        .filter(|d| !KNOWN_DEPENDENT.contains(&d.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "new dependence on zero-on-claim (a producer hands out un-initialised words — fix \
         it at that site, do not widen the memset): {unexpected:?}"
    );
    let fixed: Vec<&&str> = KNOWN_DEPENDENT
        .iter()
        .filter(|k| !dependent.iter().any(|d| d == *k))
        .collect();
    assert!(
        fixed.is_empty(),
        "these no longer depend on zero-on-claim — remove them from KNOWN_DEPENDENT: {fixed:?}"
    );
}
