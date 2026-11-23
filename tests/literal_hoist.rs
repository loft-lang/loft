// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-x (`@FR-R-LitHoist`) — an invariant loop-body vector literal builds ONCE:
//! the local is pre-declared at function top and its declaration (the one wrapped `Set`,
//! or the flat `OpDatabase · Set · pushes` statement run) is guarded on the local being
//! unbound, so every later iteration and re-entry reuses the store.  The cell corpus
//! (`bytecode-comparisons/V-x-invariant-literal-cells.loft`) says the VALUES hold; this
//! pins the EMISSION per cell — which declarations earn the guard, and that every
//! declining shape (an append, a loop-var part, an element write, an escape, a heap
//! element, a non-const param read, a sanitized-name collision) keeps the per-iteration
//! build — and the switch (`LOFT_NO_LITERAL_HOIST=1`).  Read off `--native-emit`.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-x-invariant-literal-cells.loft";

const MARK: &str = "V-x invariant literal";

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native")
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
fn the_switch_restores_the_per_iteration_build() {
    let out = std::env::temp_dir().join("loft_literal_hoist_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_LITERAL_HOIST", "1")]);
    assert_eq!(
        rust.matches(MARK).count(),
        0,
        "under LOFT_NO_LITERAL_HOIST=1 every literal rebuilds per iteration"
    );
    let _ = fa::remove_file(&out);
}
