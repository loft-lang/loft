// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN53 F3.1 — differential `--interpret` ≡ `--native` on GENERATED keyed
//! programs.
//!
//! Each spec becomes a printing program (`generate_keyed_summary`: builds the
//! collection, prints the canonical population + ordered `key=value;` summary —
//! the F3 deterministic-output subset), run on both backends via the shared
//! `run_cross_mode` helper, which asserts both succeed AND their normalised
//! stdout is byte-identical. A divergence is a cross-backend codegen finding.
//!
//! Feature-gated (needs `loft::fuzz_keyed`) and `#[ignore]` (native shells out
//! to rustc per program — heavy). Run with:
//!   cargo test --features fuzzing --test fuzz_differential -- --ignored --nocapture
//!
//! Design: `doc/claude/plans/53-program-level-fuzzing/F3-DESIGN.md`.

#![cfg(feature = "fuzzing")]

mod common;

use loft::fuzz_keyed::{KeyedSpec, Kind, generate_keyed_summary};

/// F3.1 + F3.2 — the differential over generated keyed programs. A programmatic
/// corpus (all three types × several sizes × remove patterns × closures on/off,
/// including an emptied collection) run on `--interpret` and `--native`; each
/// must succeed and print byte-identical normalised stdout. Bounded because
/// native shells out to rustc per program (the coverage cap F3-DESIGN.md §
/// failure path 3 names). The closures axis puts the slot-allocator path in the
/// cross-backend diff (F3.2).
#[test]
#[ignore = "F3 differential (rustc per program) — runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --features fuzzing --test fuzz_differential -- --ignored`"]
fn keyed_generated_agree_across_backends() {
    let mut corpus: Vec<KeyedSpec> = Vec::new();
    for closures in [false, true] {
        for kind in [Kind::Hash, Kind::Sorted, Kind::Index] {
            corpus.push(spec(kind, 5, &[], closures)); // no removes
            corpus.push(spec(kind, 6, &[2], closures)); // one remove
            corpus.push(spec(kind, 8, &[0, 3, 7], closures)); // several removes
            corpus.push(spec(kind, 3, &[0, 1, 2], closures)); // emptied
        }
    }

    for (i, s) in corpus.iter().enumerate() {
        let body = generate_keyed_summary(s);
        // run_cross_mode appends `fn main() { test(); }`, runs both backends,
        // and panics on failure or a normalised-stdout divergence.
        common::cross_mode::run_cross_mode(&format!("f3_keyed_{i}"), &body);
    }
    eprintln!(
        "F3: {} generated keyed programs agree interp==native",
        corpus.len()
    );
}

fn spec(kind: Kind, n_keys: u32, remove: &[u32], closures: bool) -> KeyedSpec {
    KeyedSpec {
        kind,
        n_keys,
        remove: remove.to_vec(),
        closures,
    }
}
