// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-LazySplit` — the EMISSION pins.  A `for piece in text.split(c)` loop iterates
//! `codegen_runtime::lazy_split` and never calls `t_4text_split`; a parameter nothing
//! writes is borrowed and every other source is iterated as a copy; the buffer the call
//! would have filled is never minted; a variable separator, a `rev` and a generator keep
//! the vector; `LOFT_NO_LAZY_SPLIT=1` restores the vector everywhere.  The guard
//! (`tests/scripts/157-lazy-split.loft`) says the VALUES hold on both backends; this pins
//! what is emitted.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/157-lazy-split.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!("loft_lazy_split_{}_{tag}.rs", std::process::id()));
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native")
        .arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_LAZY_SPLIT");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        fa::exists(&out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = fa::read_to_string(&out).expect("read the emitted Rust");
    let _ = fa::remove_file(&out);
    rust
}

#[test]
fn the_switch_restores_the_vector_everywhere() {
    let rust = emit("switch", &[("LOFT_NO_LAZY_SPLIT", "1")]);
    // The iterator itself stays in use: `(R-SplitTable)` collects it for s14's table, under
    // its own switch.  What this switch removes is the lazy LOOP form.
    assert!(
        !rust.contains("let mut __ls_") && !rust.contains("__ls_done_"),
        "LOFT_NO_LAZY_SPLIT=1 must emit no lazy split loop"
    );
}
