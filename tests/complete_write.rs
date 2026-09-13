// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-y (`@FR-R-CompleteWrite`) — a record built by a COMPLETE literal group
//! skips the default prefill: the parser writes every field explicitly (declared
//! defaults, sentinels, the variant tag), so the emitter proves coverage of every
//! schema field position by the group's contiguous `OpSet*`s and calls the no-prefill
//! twin (`OpDatabaseNP` / `OpNewRecordNP`).  The cell corpus
//! (`bytecode-comparisons/V-y-complete-write-cells.loft`) says the VALUES hold; this
//! pins the EMISSION — which sites earn the twin, and that every declining shape (a
//! vector store, a nested struct arriving by copy, a `__nullable` element whose
//! discriminant the group never writes) keeps the prefill — and the switch
//! (`LOFT_NO_COMPLETE_WRITE=1`).  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-y-complete-write-cells.loft";

/// `(function, no-prefill sites, prefilled sites)` — predictions written beside the cells.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_c1", 1, 0), // every field named
    ("n_c2", 1, 0), // omitted fields: the parser writes them explicitly
    ("n_c3", 1, 0), // a variant literal writes its own tag
    ("n_c4", 2, 0), // the S mint AND the vector store (its group's own OpSetInt4 is
    // exactly the one-u32 prefill, width-equal) both skip
    ("n_c5", 1, 0), // the FALLBACK literal skips; the cast path is not an emitter site
    ("n_c6", 1, 1), // the inner N literal skips; O (nested by OpCopyRecord) declines
    ("n_c7", 1, 0), // a conditional part still writes its field
    ("n_c8", 2, 3), // the vector store and the discharge fallback skip; the __nullable
                    // mints keep the prefill (the discriminant is not in the group's write set)
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

/// Per emitted function: `(no-prefill sites, prefilled sites)`.
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
        let row = map.entry(current.clone()).or_default();
        row.0 += line.matches("OpDatabaseNP(").count() + line.matches("OpNewRecordNP(").count();
        row.1 += line.matches("OpDatabase(").count() + line.matches("OpNewRecord(").count();
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_elides_exactly_the_prefills_predicted() {
    let out = std::env::temp_dir().join("loft_complete_write_on.rs");
    let rust = emit(&cells(), &out, &[]);
    let got = counts(&rust);
    for (name, np, plain) in EXPECTED {
        let (n, p) = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            (n, p),
            (*np, *plain),
            "{name}: (no-prefill, prefilled) sites"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_every_prefill() {
    let out = std::env::temp_dir().join("loft_complete_write_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_COMPLETE_WRITE", "1")]);
    assert_eq!(
        rust.matches("OpDatabaseNP(").count() + rust.matches("OpNewRecordNP(").count(),
        0,
        "under LOFT_NO_COMPLETE_WRITE=1 every record keeps its default prefill"
    );
    let _ = std::fs::remove_file(&out);
}
