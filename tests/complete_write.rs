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
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-y-complete-write-cells.loft";

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native")
        .arg("--lean") // the CLOSED program: an open build takes no value records (NATIVE.md § Optimisation tiers)
        .arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        fa::exists(out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    fa::read_to_string(out).expect("read the emitted Rust")
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
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
    let _ = fa::remove_file(&out);
}
