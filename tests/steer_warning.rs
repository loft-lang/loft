// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 arc C step 3 — the recommended-idiom steer, at the CLI.
//!
//! A call FROM OWNED source to a `#superseded "Y"` symbol emits a
//! `warning: … is superseded — use `Y` …`.  These binary-level tests assert:
//!   - the steer fires on the interpreter AND on `--native` (the steer lives in
//!     the shared parser, but native surfaces diagnostics through its own run
//!     path — a native regression here is exactly the divergence class we guard);
//!   - `LOFT_NO_STEER=1` silences it.
//!
//! `LOFT_NO_CACHE=1` forces a cold parse so the steer re-fires (a whole-program
//! warm-cache hit skips the parse, like every inline parse diagnostic).  The
//! owned-vs-dependency provenance gate is the parser unit test
//! `steer_fires_from_owned_source_silent_from_dependency` in `src/ir_read.rs`.

use loft::file_access as fa;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// `old_add` is FOLDED onto `new_add` (a shim over the successor) so the arc-C
// step-4 fold lint stays clean; the steer still fires on a *call* to `old_add`.
const PROG: &str = "\
fn old_add(a: integer, b: integer) -> integer { new_add(a, b) }
#superseded \"new_add\"
fn new_add(a: integer, b: integer) -> integer { a + b }
fn main() { println(\"{old_add(2, 3)} {new_add(4, 5)}\") }
";

/// Run PROG on `backend` (cold parse), optionally with `LOFT_NO_STEER=1`, and
/// return the compiler's stderr.
fn stderr_of(name: &str, backend: &str, no_steer: bool) -> String {
    let path = std::env::temp_dir().join(format!("loft_steer_{name}.loft"));
    fa::write(&path, PROG).expect("write temp script");
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg(&path)
        .current_dir(workspace_root())
        .env("LOFT_NO_CACHE", "1") // cold parse so the steer re-fires
        .env("LOFT_TIMEOUT", "120");
    if no_steer {
        cmd.env("LOFT_NO_STEER", "1");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = fa::remove_file(&path);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn steer_fires_on_interpret() {
    let err = stderr_of("interp", "--interpret", false);
    assert!(
        err.contains("`old_add` is superseded") && err.contains("use `new_add`"),
        "the steer must fire on the interpreter for an owned #superseded call; got: {err}"
    );
}

#[test]
fn steer_fires_on_native() {
    let err = stderr_of("native", "--native", false);
    assert!(
        err.contains("`old_add` is superseded") && err.contains("use `new_add`"),
        "the steer must fire on --native for an owned #superseded call; got: {err}"
    );
}

#[test]
fn loft_no_steer_silences_the_steer() {
    let err = stderr_of("optout", "--interpret", true);
    assert!(
        !err.contains("is superseded"),
        "LOFT_NO_STEER=1 must silence the steer; got: {err}"
    );
}
