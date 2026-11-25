// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-p (loft#1426) — a callee's INVARIANT INPUTS cross the call (`@FR-R-Inputs`):
//! a callee admitted by `@FR-R-Callee` gets a TWIN (`<fn>__inv`) that takes the record
//! parameter's invariant scalars and vector headers as extra parameters, and a caller loop
//! that hoisted every one of them for its argument variable calls the twin.
//!
//! The cell corpus (`bytecode-comparisons/V-p-callee-inputs-cells.loft`) can only say the
//! VALUES hold — a twin and the plain call answer the same number.  This pins the EMISSION:
//! which callees earn a twin, which calls take it, and the switch
//! (`LOFT_NO_CALLEE_INPUTS=1`), which is what makes it red on the build before the unit and
//! on one that lost it.  Read off `--native-emit`.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-p-callee-inputs-cells.loft";

/// @PLN157 § V-ac — the PATH-argument half: a callee indexing a plain VECTOR parameter, or a
/// record parameter's vector field, whose caller passes a pure path (`br.img`, `h.cv`); and a
/// callee answering a scalar record through its return buffer.  Cells and predictions:
/// `bytecode-comparisons/V-ac-vector-param-cells.loft`.
const CELLS_AC: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-ac-vector-param-cells.loft";

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native")
        .arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        // The predictions pin `(R-Inputs)`' twin selection.  `(R-RangedCall)` would add a
        // guarded copy of every loop whose callee's arithmetic repays it, doubling each
        // count for a reason this file does not test; its own guard pins that.
        .env("LOFT_NO_RANGED_CALLS", "1");
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
fn the_switch_emits_no_twin_and_no_twin_call_for_a_path_argument() {
    let out = std::env::temp_dir().join("loft_callee_inputs_ac_off.rs");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS_AC);
    let rust = emit(&src, &out, &[("LOFT_NO_CALLEE_INPUTS", "1")]);
    assert!(
        !rust.contains("__inv("),
        "LOFT_NO_CALLEE_INPUTS=1 must emit no twin and call none"
    );
    let _ = fa::remove_file(&out);
}

#[test]
fn the_switch_emits_no_twin_and_no_twin_call() {
    let out = std::env::temp_dir().join("loft_callee_inputs_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_CALLEE_INPUTS", "1")]);
    assert!(
        !rust.contains("__inv("),
        "LOFT_NO_CALLEE_INPUTS=1 must emit no twin and call none"
    );
    let _ = fa::remove_file(&out);
}
