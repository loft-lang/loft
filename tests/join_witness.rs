// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-af (loft#1426) — a value BRANCH of buffer-delivering calls witnesses every
//! arm's buffer (`@FR-O-Complete`), so the buffers are allocated once per function activation
//! and the joined local's scope-exit free is the multi-buffer ladder.
//!
//! The cell corpus (`bytecode-comparisons/V-af-join-witness-cells.loft`) answers the same
//! numbers with and without the pairing, because the unsound form frees a store and writes it
//! again while the bytes stay readable.  This pins the IR, read off `loft introspect`: each
//! joined cell's preamble allocates its arms' buffers, its scope exit frees the local through
//! the `OpDistinctStore` ladder, and the switch (`LOFT_NO_JOIN_BUFFER_WITNESS=1`) restores the
//! plain free and no preamble allocation — the build before the unit.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-af-join-witness-cells.loft";

/// `(cell, buffers allocated in the preamble, ladder frees)` with the pairing on.
const PAIRED: &[(&str, usize, usize)] = &[
    ("n_j1", 2, 1),  // two arms
    ("n_j2", 2, 1),  // the call arm, and the literal arm's buffer — a literal already paired
    ("n_j3", 3, 1),  // three arms
    ("n_j4", 3, 1),  // nested branches: all three tails
    ("n_j5", 0, 0),  // a view arm: the join is lowered through the lift-join path instead
    ("n_j6", 0, 1),  // reassigned: no reuse; the one guard is the reassignment path's own
    ("n_j7", 0, 0),  // outside a loop: paired the other way round, the BUFFER's free guarded
    ("n_j8", 3, 1),  // escaping into a vector: the two arms and the append's own buffer
    ("n_j9", 3, 1),  // into a field: the two arms and the field store's buffer
    ("n_j10", 4, 2), // two joined locals, two buffers each
    ("n_j11", 2, 1), // nested loops
    ("n_j12", 2, 1), // a forwarding callee fills the buffer it is handed
];

fn introspect(env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect").arg(&src).env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Per function in the IR dump: `(preamble OpDatabase on a __ref_N, OpDistinctStore ladders)`.
fn shape(ir: &str) -> HashMap<String, (usize, usize)> {
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in ir.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
            continue;
        }
        let e = map.entry(current.clone()).or_default();
        if line.trim_start().starts_with("OpDatabase(__ref_") {
            e.0 += 1;
        }
        if line.contains("if OpDistinctStore(") {
            e.1 += 1;
        }
    }
    map
}

#[test]
fn every_arm_of_a_joined_local_is_a_witnessed_buffer() {
    let fns = shape(&introspect(&[]));
    for (name, allocs, ladders) in PAIRED {
        let got = fns
            .get(*name)
            .unwrap_or_else(|| panic!("{name} not in the IR dump"));
        assert_eq!(
            *got,
            (*allocs, *ladders),
            "{name}: (preamble allocations, ladders)"
        );
    }
}

#[test]
fn the_switch_pairs_nothing() {
    let fns = shape(&introspect(&[("LOFT_NO_JOIN_BUFFER_WITNESS", "1")]));
    for (name, _, _) in PAIRED {
        let got = fns
            .get(*name)
            .unwrap_or_else(|| panic!("{name} not in the IR dump"));
        assert_eq!(
            got.1, 0,
            "{name} under LOFT_NO_JOIN_BUFFER_WITNESS=1 must free the local plainly"
        );
    }
}
