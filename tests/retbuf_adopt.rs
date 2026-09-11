// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-u (loft#1426) — a vector-returning function's result local ADOPTS the
//! hidden return buffer (`@FR-R-RetAdopt`): the local builds in the caller's buffer from
//! its declaration (`hoist::ret_adopt`), the `one_buffer_vec_copy` delivery pair emits as
//! nothing while the block's scope-exit frees stay, and the buffer's backing is reused
//! across calls.  Only the SHAPE-A ABI (a separate `__retbuf` attr) adopts; the
//! witness-promoted shape already delivers copy-free through the aliasing-safe
//! `OpReplaceVector` and declines.
//!
//! The cell corpus (`bytecode-comparisons/V-u-retbuf-adopt-cells.loft`) can only say the
//! VALUES hold; this pins the EMISSION — which functions adopt — and the switch
//! (`LOFT_NO_RETBUF_ADOPT=1`).  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-u-retbuf-adopt-cells.loft";

/// `(function, adoption inits)` — 1 where the fn's result local aliases the buffer.
const EXPECTED: &[(&str, usize)] = &[
    ("n_mid", 0),       // u1: the witness-promoted shape — already copy-free, declines
    ("n_twolocal", 0),  // u2: two delivered locals — declines
    ("n_rebound", 0),   // u3: the result local rebound — declines
    ("n_mkr", 0),       // u4: witness-promoted — declines
    ("n_deep", 0),      // u4: witness-promoted — declines
    ("n_mixed", 1),     // u6: the shape-A adopter
    ("n_twolocal2", 0), // u7: shape A with two delivered locals — declines
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

/// Per emitted function: adoption inits (the buffer-aliasing declaration).
fn counts(rust: &str) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        if line.contains("if var___retbuf.store_nr == u16::MAX || var___retbuf.rec == 0") {
            *map.entry(current.clone()).or_default() += 1;
        }
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_adopts_exactly_as_predicted() {
    let out = std::env::temp_dir().join("loft_retbuf_adopt_on.rs");
    let rust = emit(&cells(), &out, &[]);
    let got = counts(&rust);
    for (name, adopts) in EXPECTED {
        let a = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(a, *adopts, "{name}: adoption inits");
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_the_delivery_copies() {
    let out = std::env::temp_dir().join("loft_retbuf_adopt_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_RETBUF_ADOPT", "1")]);
    let got = counts(&rust);
    for (name, _) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or(0),
            0,
            "{name}: under LOFT_NO_RETBUF_ADOPT=1 no result local adopts"
        );
    }
    let _ = std::fs::remove_file(&out);
}
