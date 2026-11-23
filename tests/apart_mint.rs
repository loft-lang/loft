// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-PushRec`'s apart-call clause — the pins: which loops of the guard
//! `tests/scripts/a-vector-built-across-a-writing-call-keeps-its-elements.loft` hold a
//! record-push header across their writing call, and which the clause declines.  The guard
//! checks the values on both backends; this checks the FORM, so a clause that stops firing
//! (or fires where a value can reach the vector) is named by function.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-vector-built-across-a-writing-call-keeps-its-elements.loft";

fn emit(out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--ship")
        .arg("--native")
        .arg("--native-emit")
        .arg(out)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
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

#[test]
fn the_switch_takes_every_apart_header_away() {
    let out = std::env::temp_dir().join("loft_apart_mint_off.rs");
    let rust = emit(&out, &[("LOFT_NO_APART_MINT", "1")]);
    assert_eq!(
        rust.matches("§ V-t record push header").count(),
        0,
        "LOFT_NO_APART_MINT=1 left a loop header in a loop that calls a writer"
    );
    let _ = fa::remove_file(&out);
}
