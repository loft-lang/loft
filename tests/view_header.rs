// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-n (loft#1426) — a `&`-bound vector VIEW (`d = &cv.data`) derives its header
//! ONCE, at the binding, for the rest of the block that indexes it, on the native backend.
//!
//! The cell corpus (`bytecode-comparisons/V-n-view-def-header-cells.loft`) can only say the
//! VALUES hold, because a header-served read and a runtime read of an unmoved vector answer
//! the same number.  This pins the EMISSION per helper — which bindings earn a header — and
//! the switch (`LOFT_NO_VIEW_HOIST=1`), which is what makes it red on the build before the
//! unit and on one that lost it.  Read off `--native-emit`.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-n-view-def-header-cells.loft";

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
fn the_switch_restores_the_runtime_reads() {
    let out = std::env::temp_dir().join("loft_view_header_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_VIEW_HOIST", "1")]);
    assert!(
        !rust.contains("V-n view header"),
        "LOFT_NO_VIEW_HOIST=1 must bind no view header"
    );
    let _ = fa::remove_file(&out);
}
