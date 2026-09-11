// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-q (loft#1426) — a loop that PUSHES to a vector path keeps a PUSH header for it
//! (`@FR-R-Push`): the header carries the capacity, the push writes through it and refreshes
//! it at a growth step, and every read of the path serves from it.
//!
//! The cell corpus (`bytecode-comparisons/V-q-hoisted-push-cells.loft`) can only say the
//! VALUES hold — a hoisted push and the template append answer the same vector.  This pins
//! the EMISSION per cell — which loops earn a push header, which decline — and the switch
//! (`LOFT_NO_PUSH_HOIST=1`), which is what makes it red on the build before the unit and on
//! one that lost it.  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-q-hoisted-push-cells.loft";

/// `(function, push headers bound, hoisted pushes emitted, plain headers bound)` — the
/// predictions written beside the cells.
const EXPECTED: &[(&str, usize, usize, usize)] = &[
    ("n_c1", 2, 2, 0),   // the two constant-fill comprehensions
    ("n_c2", 1, 1, 1),   // out pushed, xs read: two owners, both hoisted
    ("n_c3", 1, 1, 0), // the value READS the pushed vector: since § V-w the read rides a
    // pre-push TEMP instead of a per-iteration whole-vector copy, so the loop hoists —
    // the tail read serves from the push header and the accumulator is linear
    ("n_c4", 2, 2, 0), // two pushed locals
    ("n_c5", 1, 1, 0), // a view root, alone: admitted
    ("n_c6", 1, 1, 0), // v pushed; the view w is dropped from the hoist (a runtime read)
    ("n_c7", 1, 1, 1), // the outer loop holds out's push header and rows' header; the inner re-uses
    ("n_c8", 0, 0, 0), // the pushed root is rebound
    ("n_fill", 1, 1, 0), // c9: a parameter's field, alone: admitted
    ("n_c10", 1, 1, 0), // the growth ladder
    ("n_c11", 0, 0, 1), // boolean / character pushes are not fusable: that loop declines; the read-only loop over b keeps its header
    ("n_c12", 1, 1, 1), // a push beside an in-place write to another owned local
    ("n_c13", 1, 1, 0), // a callee reads the length through the runtime
    ("n_c14", 1, 1, 0), // len(v) after the push reads the header
    ("n_c15", 1, 1, 0), // the element just pushed, read back through the header
    ("n_c16", 1, 1, 0), // an empty vector: the first push takes the growth step
    ("n_c17", 1, 1, 1), // two owners
    ("n_c18", 0, 0, 0), // a record append beside the push blocks the loop
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

/// Per emitted function: `(push headers, hoisted pushes, plain headers)`.
fn counts(rust: &str) -> HashMap<String, (usize, usize, usize)> {
    let mut map: HashMap<String, (usize, usize, usize)> = HashMap::new();
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
        if line.contains("V-q push header") {
            e.0 += 1;
        }
        if line.contains("push_hoisted::<") {
            e.1 += 1;
        }
        if line.contains("vector::vec_header(") {
            e.2 += 1;
        }
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_loop_earns_exactly_the_push_header_predicted() {
    let out = std::env::temp_dir().join("loft_push_hoist_on.rs");
    let got = counts(&emit(&cells(), &out, &[]));
    for (name, push_headers, pushes, headers) in EXPECTED {
        let (ph, p, h) = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            (ph, p, h),
            (*push_headers, *pushes, *headers),
            "{name}: (push headers, hoisted pushes, plain headers)"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_hoists_no_push() {
    let out = std::env::temp_dir().join("loft_push_hoist_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_PUSH_HOIST", "1")]);
    assert!(
        !rust.contains("push_header(") && !rust.contains("push_hoisted::<"),
        "LOFT_NO_PUSH_HOIST=1 must bind no push header and route every push through its template"
    );
    let _ = std::fs::remove_file(&out);
}
