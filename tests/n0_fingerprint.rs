// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 Arc N / N0 — the loft-build-fingerprint gate on native artifacts.
//!
//! A native package crate is valid only against the loft build whose rlib it
//! links.  `auto_build_native` stamps a `.loft-build-fp` sidecar with the current
//! `cache::loft_build_fingerprint()` after a build, and reuses a cached artifact
//! only when that sidecar matches.  This drives the package build directly
//! (bypassing the user-script codegen path) to prove: a *stale* sidecar forces a
//! rebuild that re-stamps the real fingerprint — the automatic replacement for
//! `make rebuild-native-cdylibs` after a loft change.

use loft::file_access as fa;
use std::path::Path;

#[test]
fn stale_fingerprint_forces_rebuild_and_restamp() {
    // Needs cargo + rustc + the in-tree native test package.  Skip cleanly when
    // the environment can't build it (matches the other native suites).
    let pkg = "tests/lib/native_pkg";
    if !fa::exists(Path::new(pkg).join("native").join("Cargo.toml")) {
        eprintln!("skip: {pkg}/native missing");
        return;
    }
    let sidecar = Path::new(pkg)
        .join("native")
        .join("target")
        .join("release")
        .join(".loft-build-fp");

    // First build: produces the rlib and stamps the fingerprint sidecar.
    let _ = loft::extensions::auto_build_native(pkg, "loft_native_test");
    let Ok(real) = fa::read_to_string(&sidecar) else {
        eprintln!("skip: package did not build (no sidecar) — cargo/rustc unavailable?");
        return;
    };
    let real = real.trim().to_string();
    assert!(
        real.parse::<u64>().is_ok() && real != "0",
        "first build must stamp a real (non-zero) fingerprint, got {real:?}"
    );

    // Simulate an old loft build: clobber the sidecar with a stale value.
    fa::write(&sidecar, "111").unwrap();

    // Second build: the gate must see the stale sidecar, rebuild, and re-stamp.
    let _ = loft::extensions::auto_build_native(pkg, "loft_native_test");
    let after = fa::read_to_string(&sidecar).unwrap_or_default();
    assert_eq!(
        after.trim(),
        real,
        "a stale fingerprint must trigger a rebuild that re-stamps the real fingerprint"
    );
}
