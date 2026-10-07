// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! PKG.STUB — `loft api`, the agent-facing library discovery surface.
//!
//! Out-of-project libraries are invisible to coding agents exploring a
//! consumer project; `loft api` lists what is reachable and prints a
//! library's public surface (pub signatures + doc comments, bodies
//! stripped).  All offline: path-form resolution against the in-repo
//! fixture packages, and name-form resolution against a fake `LOFT_HOME`.

use loft::file_access as fa;
use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `loft api <path>` prints the package's pub signatures with doc comments
/// and without function bodies.
#[test]
fn api_path_form_prints_public_surface() {
    let out = Command::new(loft_bin())
        .args(["api", "tests/fixtures/libs/imaging"])
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft api");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "loft api failed: {text}");
    assert!(
        text.contains("public API surface") && text.contains("use imaging::*;"),
        "missing stub header: {text}"
    );
    assert!(text.contains("pub fn "), "no pub signatures: {text}");
    assert!(
        !text.contains("\n    "),
        "function bodies must be stripped (found indented body lines): {text}"
    );
}

/// Name-form resolution finds the newest installed registry copy under
/// `LOFT_HOME`, and the bare `loft api` listing enumerates it.
#[test]
fn api_name_form_resolves_installed_registry_copy() {
    let pid = std::process::id();
    let home = std::env::temp_dir().join(format!("loft_api_home_{pid}"));
    let _ = fa::remove_dir_all(&home);
    // Two installed versions; resolution must pick the newest (0.10.0 > 0.9.0
    // numerically — lexicographic order would pick wrong).
    for ver in ["0.9.0", "0.10.0"] {
        let pkg = home.join(".loft/registry").join(format!("fakelib-{ver}"));
        fa::create_dir_all(pkg.join("src")).expect("mkdir pkg");
        fa::write(
            pkg.join("loft.toml"),
            format!("[package]\nname = \"fakelib\"\nversion = \"{ver}\"\n"),
        )
        .expect("write manifest");
        fa::write(
            pkg.join("src/fakelib.loft"),
            format!(
                "// Greets v{ver}.\npub fn greet_{}() {{\n    println(\"hi\");\n}}\n",
                ver.replace('.', "_")
            ),
        )
        .expect("write src");
    }

    let run = |args: &[&str]| {
        let out = Command::new(loft_bin())
            .args(args)
            .current_dir(workspace_root())
            .env("LOFT_HOME", &home)
            .output()
            .expect("invoke loft api");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };

    let (ok, text) = run(&["api", "fakelib"]);
    assert!(ok, "api fakelib failed: {text}");
    assert!(
        text.contains("fakelib 0.10.0") && text.contains("pub fn greet_0_10_0()"),
        "must resolve the numerically newest version: {text}"
    );

    let (ok, listing) = run(&["api"]);
    assert!(ok, "api listing failed: {listing}");
    assert!(
        listing.contains("fakelib 0.9.0") && listing.contains("fakelib 0.10.0"),
        "listing must enumerate installed copies: {listing}"
    );

    let _ = fa::remove_dir_all(&home);
}
