// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Callee` / `@FR-R-InPlace`, the hidden-buffer allowance's SET half — a write into a
//! `??` fallback's private discharge buffer (a text field's `OpSetText` included) keeps a
//! caller's headers, in the loop body and through an admitted callee alike.
//!
//! The cell corpus (`tests/scripts/a-discharge-buffers-text-set-keeps-the-callers-headers.loft`)
//! can only say the VALUES hold.  This pins the EMISSION per cell: the loop calling the
//! discharging callee keeps its push header and headers (c1), the fallback in the body
//! itself keeps them (c2), a callee that sets a text into an ELEMENT of the walked vector
//! keeps nothing (c3), and the switch (`LOFT_NO_NULL_BUFFER_HOIST=1`, which covers the mint
//! and the set) empties c1.  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-discharge-buffers-text-set-keeps-the-callers-headers.loft";

/// `(function, push headers, vector headers)` — predictions written beside the cells.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_c1", 1, 2), // out's push header; st.nodes and st.seqs read through headers
    ("n_c2", 0, 2), // no push; the two headers, the fallback discharged in the body
    // rename grows an element's text: the walk reads through the runtime.  The BUILD loop's
    // two pushes and its text-field mint hold (`@FR-R-Mint`'s text clause).
    ("n_c3", 3, 0),
    ("n_c4", 0, 1), // the callee misses on every call: the loop keeps its one header
];

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        // The pins count ONE copy of each loop: `@FR-R-Alias`'s versioned clause emits a
        // second, holding a parameter's headers, which is that rule's to pin.
        .env("LOFT_NO_DISTINCT_VERSION", "1");
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

/// Per emitted function: `(push headers, vector headers)`.
fn counts(rust: &str) -> HashMap<String, (usize, usize)> {
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
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
        e.0 += line.matches("push_header(").count();
        e.1 += line.matches("vec_header(").count();
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_loop_keeps_exactly_the_headers_predicted() {
    let out = std::env::temp_dir().join("loft_discharge_set_on.rs");
    let got = counts(&emit(&cells(), &out, &[]));
    for (name, pushes, headers) in EXPECTED {
        let (p, h) = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            (p, h),
            (*pushes, *headers),
            "{name}: (push headers, vector headers)"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_declines_the_discharging_callees_loop() {
    let out = std::env::temp_dir().join("loft_discharge_set_off.rs");
    let got = counts(&emit(&cells(), &out, &[("LOFT_NO_NULL_BUFFER_HOIST", "1")]));
    assert_eq!(
        got["n_c1"].0, 0,
        "with the allowance off, c1's loop holds no push header"
    );
    let _ = std::fs::remove_file(&out);
}
