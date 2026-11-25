// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@C173` — `loft develop`: a library the project pins can always be debugged and fixed.
//!
//! One project declares and pins `probepkg 0.1.0`, which a private registry cache holds.
//! `loft develop probepkg` must put an editable copy in `lib/probepkg` that outranks the pin
//! (an edit to it is what runs), list it, and `--done` must restore the manifest byte for byte
//! and leave the copy.  `--path` must take a checkout of the same package and refuse another.
//! Hermetic: a private `~/.loft`, an unreachable registry, `LOFT_OFFLINE`, `LOFT_NO_CACHE`.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fa::create_dir_all(parent).expect("mkdir");
    }
    fa::write(path, body).expect("write");
}

const MANIFEST: &str =
    "[package]\nname = \"proj\"\nversion = \"0.1.0\"\n\n[dependencies]\nprobepkg = \"0.1.0\"\n";

/// A private `~/.loft` holding `probepkg 0.1.0`, and a project that declares and pins it.
fn setup(tag: &str) -> (PathBuf, PathBuf) {
    let home = std::env::temp_dir().join(format!("loft_develop_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&home);
    let pkg = home.join(".loft/registry/probepkg-0.1.0");
    write(
        &pkg.join("loft.toml"),
        "[package]\nname = \"probepkg\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/probepkg.loft\"\n",
    );
    write(
        &pkg.join("src/probepkg.loft"),
        "pub fn probe_id() -> text { \"probepkg-0.1.0\" }\n",
    );
    let proj = home.join("proj");
    write(&proj.join("loft.toml"), MANIFEST);
    write(
        &proj.join("loft.lock"),
        "schema_version = 1\n\n[[package]]\nname = \"probepkg\"\nversion = \"0.1.0\"\n\
         url = \"http://127.0.0.1:1/probepkg-0.1.0.tar.gz\"\n\
         sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
         source = \"registry\"\n",
    );
    write(
        &proj.join("s.loft"),
        "use probepkg::*;\nfn main() { println(probe_id()); }\n",
    );
    (home, proj)
}

/// Run `args` in `proj` against `home`: (exit code, stdout, stderr).
fn loft(home: &Path, proj: &Path, args: &[&str]) -> (i32, String, String) {
    let out = loft::platform::process::harness_command(loft_bin())
        .args(args)
        .env("LOFT_HOME", home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("LOFT_REGISTRY_URL", "http://127.0.0.1:1/index.json")
        .env("LOFT_OFFLINE", "1")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "120")
        .current_dir(proj)
        .output()
        .expect("spawn loft");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_pinned_library_is_developed_in_a_copy_and_returned_to_its_pin() {
    let (home, proj) = setup("copy");
    let (code, said, err) = loft(&home, &proj, &["develop", "probepkg"]);
    assert_eq!(code, 0, "{said}\n{err}");
    let manifest = fa::read_to_string(proj.join("loft.toml")).expect("manifest");
    assert!(
        manifest.contains(
            "probepkg = { path = \"lib/probepkg\", version = \"0.1.0\" }  # loft develop: was \"0.1.0\""
        ),
        "{manifest}"
    );
    // The copy is the pinned version, and an edit to it is what runs: it outranks the pin.
    let copy = proj.join("lib/probepkg/src/probepkg.loft");
    assert!(
        fa::read_to_string(&copy)
            .expect("the copy")
            .contains("probepkg-0.1.0")
    );
    write(&copy, "pub fn probe_id() -> text { \"edited copy\" }\n");
    let (_, out, err) = loft(&home, &proj, &["--interpret", "s.loft"]);
    assert_eq!(out, "edited copy", "{err}");
    let (_, listed, _) = loft(&home, &proj, &["develop"]);
    assert_eq!(listed, "probepkg  lib/probepkg  (was \"0.1.0\")");
    // Done: the manifest is what it was, the pin runs again, the copy stays.
    let (code, said, err) = loft(&home, &proj, &["develop", "--done", "probepkg"]);
    assert_eq!(code, 0, "{said}\n{err}");
    assert_eq!(
        fa::read_to_string(proj.join("loft.toml")).expect("manifest"),
        MANIFEST
    );
    let (_, out, err) = loft(&home, &proj, &["--interpret", "s.loft"]);
    assert_eq!(out, "probepkg-0.1.0", "{err}");
    assert!(fa::exists(&copy), "the copy is left as it is");
    let (_, listed, _) = loft(&home, &proj, &["develop"]);
    assert_eq!(listed, "", "nothing is developed any more");
}

#[test]
fn a_checkout_of_the_library_is_developed_and_another_package_refused() {
    let (home, proj) = setup("path");
    let checkout = home.join("checkout");
    write(
        &checkout.join("loft.toml"),
        "[package]\nname = \"probepkg\"\nversion = \"0.1.0\"\n",
    );
    write(
        &checkout.join("src/probepkg.loft"),
        "pub fn probe_id() -> text { \"my checkout\" }\n",
    );
    let other = home.join("other");
    write(
        &other.join("loft.toml"),
        "[package]\nname = \"something\"\nversion = \"1.0.0\"\n",
    );
    let other_s = other.to_string_lossy().into_owned();
    let (code, _, err) = loft(&home, &proj, &["develop", "probepkg", "--path", &other_s]);
    assert_ne!(code, 0, "another package is refused");
    assert!(err.contains("is the package `something`"), "{err}");
    assert_eq!(
        fa::read_to_string(proj.join("loft.toml")).expect("manifest"),
        MANIFEST,
        "and the manifest is untouched"
    );
    let checkout_s = checkout.to_string_lossy().into_owned();
    let (code, said, err) = loft(
        &home,
        &proj,
        &["develop", "probepkg", "--path", &checkout_s],
    );
    assert_eq!(code, 0, "{said}\n{err}");
    let (_, out, err) = loft(&home, &proj, &["--interpret", "s.loft"]);
    assert_eq!(out, "my checkout", "{err}");
    let (code, _, err) = loft(&home, &proj, &["develop", "probepkg"]);
    assert_ne!(code, 0, "developing it twice is refused");
    assert!(err.contains("already being developed"), "{err}");
}
