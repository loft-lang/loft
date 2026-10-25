// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#2153 — a native run rebuilds loft's own `libloft.rlib` only for the binary built
//! from the working directory.  `cargo build --lib` there refreshes that checkout's
//! `target/`, which an installed loft (it links `share/loft/libloft.rlib`) and a dev binary
//! of another checkout never link: for them the build changes nothing they run, and on a
//! fresh checkout it outlasts the run's own timeout.
//!
//! The cell runs this checkout's `target/debug/loft` in a directory that LOOKS like a loft
//! checkout to a working-directory test — a `Cargo.toml` naming the `loft` package and a
//! `src/` newer than any rlib — but is not where the binary was built.

use loft::file_access as fa;
use std::path::PathBuf;

#[test]
fn a_native_run_outside_its_own_checkout_builds_no_runtime() {
    let root = std::env::temp_dir().join(format!("loft_rlib_refresh_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    fa::create_dir_all(root.join("src")).expect("root");
    fa::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"loft\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .expect("manifest");
    // Written now, so it is newer than the rlib the binary links.
    fa::write(root.join("src/lib.rs"), "// newer than any rlib\n").expect("source");
    fa::write(
        root.join("t.loft"),
        "fn test_two() { assert(1 + 1 == 2, \"two\"); }\n",
    )
    .expect("script");

    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .args(["--tests", "--native", "t.loft"])
        .current_dir(&root)
        .env("LOFT_TIMEOUT", "240")
        .output()
        .expect("spawn loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let built = fa::exists(root.join("target"));
    let _ = fa::remove_dir_all(&root);

    assert!(
        !stderr.contains("rebuilding libloft.rlib"),
        "a binary built elsewhere tried to rebuild its runtime in this directory:\n{stderr}"
    );
    assert!(
        !built,
        "a cargo build ran in the working directory (a target/ appeared)"
    );
    assert!(
        out.status.success() && format!("{stdout}{stderr}").contains("1 passed"),
        "the run itself did not pass:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
