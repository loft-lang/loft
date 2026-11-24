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
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-q-hoisted-push-cells.loft";

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native")
        .arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        // A `vector<float>` local these cells push to is an inline buffer under
        // `@FR-R-Apart`'s instance 3, which takes the push header away: these pins are the
        // push hoist's own evidence, measured with that rule off.
        .env("LOFT_NO_APART_VECTOR", "1");
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
fn the_switch_hoists_no_push() {
    let out = std::env::temp_dir().join("loft_push_hoist_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_PUSH_HOIST", "1")]);
    assert!(
        !rust.contains("push_header(")
            && !rust.contains("push_hoisted::<")
            && !rust.contains("push_windowed::<")
            && !rust.contains("vector::push_window("),
        "LOFT_NO_PUSH_HOIST=1 must bind no push header, open no window on one, and route every push through its template"
    );
    let _ = fa::remove_file(&out);
}
