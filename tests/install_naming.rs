// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#966 — what `loft install` means, and what it files a package under.
//!
//! Two halves. **Bare `loft install` resolves the manifest's `[dependencies]`** — the
//! npm/cargo reading, and the one `loft api` names when it reports a dependency
//! unresolved. It used to install the PROJECT, so the tool's only hint pointed at the one
//! command that does not fetch a dependency. `loft install .` keeps that behaviour; that
//! spelling always meant it.
//!
//! And an install **is named by the manifest**. It used the checkout DIRECTORY's name, so
//! a package whose directory differs from `[package] name` landed in
//! `~/.loft/lib/<directory>` — a name no `use` can reach. `loft api` went on reporting the
//! dependency unresolved, and the copy sat there shadowing nothing useful.
//!
//! ⚠ `~/.loft/lib/<name>` is searched BEFORE the registry cache, so a copy under the
//! RIGHT name is not harmless either — that is loft#667, where a locally installed `web`
//! lost its `wasm/` bridge and shadowed a good published one.  These tests therefore run
//! with `HOME` pointed at a temp directory: a suite that installed into the developer's
//! real `~/.loft/lib` would be re-arming that trap on every run.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    fa::create_dir_all(path.parent().unwrap()).expect("mkdir");
    fa::write(path, body).expect("write");
}

/// Build a package in a directory deliberately named something other than the manifest's
/// `[package] name`, run `loft install .` in it against a private `HOME`, and return
/// (stdout+stderr, that HOME).
///
/// `.`, not bare — bare `loft install` resolves dependencies now and installs nothing.
fn install_in_a_mismatched_directory(tag: &str) -> (String, PathBuf) {
    let base = std::env::temp_dir().join(format!("loft_966_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&base);
    let home = base.join("home");
    let pkg = base.join("checkout_dir_name");
    fa::create_dir_all(&home).expect("mkdir home");
    write(
        &pkg.join("loft.toml"),
        "[package]\nname    = \"instprobe\"\nversion = \"0.1.0\"\nloft    = \">=0.8\"\n\n\
         [library]\nentry = \"src/instprobe.loft\"\n",
    );
    write(
        &pkg.join("src/instprobe.loft"),
        "pub fn hi() -> integer { 1 }\n",
    );

    let out = loft::platform::process::harness_command(loft_bin())
        .args(["install", "."])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("LOFT_TIMEOUT", "90")
        .current_dir(&pkg)
        .output()
        .expect("spawn loft install .");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        home,
    )
}

/// The reported case: manifest name `instprobe`, directory `checkout_dir_name`.
#[test]
fn a_local_install_is_named_by_the_manifest_not_the_directory() {
    let (all, home) = install_in_a_mismatched_directory("name");
    let by_manifest = home.join(".loft/lib/instprobe");
    let by_directory = home.join(".loft/lib/checkout_dir_name");

    assert!(
        fa::is_dir(&by_manifest),
        "expected the install under the manifest's `[package] name`\n{all}"
    );
    // Both halves: a fix that installed under BOTH names would satisfy the first
    // assertion while still leaving the unreachable copy behind.
    assert!(
        !fa::exists(&by_directory),
        "the directory-named copy must not be left behind — nothing can `use` it\n{all}"
    );
    let _ = fa::remove_dir_all(home.parent().unwrap());
}

/// `loft api` must not send the reader to a command that does not resolve the dependency
/// it is reporting.  A `{ path = … }` dep needs no install at all — it resolves from the
/// path it names (loft#963) — so an unresolved one means the path is wrong.
#[test]
fn the_api_hint_for_a_broken_path_dep_names_the_path() {
    let base = std::env::temp_dir().join(format!("loft_966_hint_{}", std::process::id()));
    let _ = fa::remove_dir_all(&base);
    write(
        &base.join("loft.toml"),
        "[package]\nname    = \"instprobe\"\nversion = \"0.1.0\"\n\n[library]\n\
         entry = \"src/instprobe.loft\"\n\n[dependencies]\n\
         moros_map = { path = \"../nowhere/moros_map\" }\n",
    );
    write(
        &base.join("src/instprobe.loft"),
        "pub fn hi() -> integer { 1 }\n",
    );

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("api")
        .env("LOFT_TIMEOUT", "90")
        .current_dir(&base)
        .output()
        .expect("spawn loft api");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&base);

    assert!(
        all.contains("../nowhere/moros_map"),
        "the hint must name the path that did not resolve\n{all}"
    );
    assert!(
        !all.contains("NOT INSTALLED — run `loft install`\n"),
        "a path dep is not fixed by an install\n{all}"
    );
}

/// Run `loft install` (bare) in a fresh package directory against a private `HOME`.
/// Returns (stdout+stderr, exit code, that HOME).
fn bare_install(tag: &str, manifest: &str) -> (String, i32, PathBuf) {
    let base = std::env::temp_dir().join(format!("loft_966_bare_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&base);
    let home = base.join("home");
    let pkg = base.join("proj");
    fa::create_dir_all(&home).expect("mkdir home");
    write(&pkg.join("loft.toml"), manifest);
    write(&pkg.join("src/proj.loft"), "pub fn hi() -> integer { 1 }\n");

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("install")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        // Hermetic: a declared registry dep must fail from the index, not from the
        // network, so the assertion is about resolution rather than about connectivity.
        .env("LOFT_OFFLINE", "1")
        .env("LOFT_TIMEOUT", "90")
        .current_dir(&pkg)
        .output()
        .expect("spawn loft install");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (all, out.status.code().unwrap_or(-1), home)
}

/// The reported case, and the whole point of the change: bare `loft install` in a project
/// whose one dependency is unresolved must act on the DEPENDENCY.  It used to install the
/// project into `~/.loft/lib` and leave the dependency exactly as it found it, so `loft
/// api`'s "run `loft install`" was advice for a command that could not take it.
#[test]
fn a_bare_install_resolves_the_manifest_not_the_project() {
    let (all, code, home) = bare_install(
        "deps",
        "[package]\nname    = \"instprobe\"\nversion = \"0.1.0\"\n\n[library]\n\
         entry = \"src/proj.loft\"\n\n[dependencies]\n\
         moros_map = { path = \"../nowhere/moros_map\" }\n",
    );

    // Nothing lands in `~/.loft/lib` — under EITHER name.  A copy there shadows the
    // registry copy of the same name, which is loft#667, and bare install reached that
    // trap from a command whose name reads like "install my dependencies".
    let lib = home.join(".loft/lib");
    let installed: Vec<String> = fa::read_dir(&lib)
        .map(|d| {
            d.into_iter()
                .map(|e| e.file_name().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        installed.is_empty(),
        "bare `loft install` must not install the project: found {installed:?} in \
         {}\n{all}",
        lib.display()
    );
    assert!(
        all.contains("../nowhere/moros_map"),
        "it must name the dependency it could not resolve\n{all}"
    );
    // An unresolved dependency is a failure: the command was asked to resolve them.
    assert_eq!(
        code, 1,
        "an unresolved dependency must exit non-zero\n{all}"
    );
    let _ = fa::remove_dir_all(home.parent().unwrap());
}

/// A resolving path dependency needs no install at all — it is reached by the path it
/// names (loft#963).  So there is nothing to act on, and loft says nothing.
#[test]
fn a_bare_install_is_silent_when_every_path_dep_resolves() {
    let base = std::env::temp_dir().join(format!("loft_966_pathok_{}", std::process::id()));
    let _ = fa::remove_dir_all(&base);
    let home = base.join("home");
    fa::create_dir_all(&home).expect("mkdir home");
    write(
        &base.join("dep/loft.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/dep.loft\"\n",
    );
    write(
        &base.join("dep/src/dep.loft"),
        "pub fn d() -> integer { 7 }\n",
    );
    write(
        &base.join("app/loft.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\
         dep = { path = \"../dep\" }\n",
    );
    write(
        &base.join("app/src/app.loft"),
        "pub fn hi() -> integer { 1 }\n",
    );

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("install")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("LOFT_OFFLINE", "1")
        .env("LOFT_TIMEOUT", "90")
        .current_dir(base.join("app"))
        .output()
        .expect("spawn loft install");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0), "expected success\n{all}");
    assert!(
        all.trim().is_empty(),
        "nothing needed acting on, so nothing should be printed\n{all}"
    );
    let _ = fa::remove_dir_all(&base);
}

/// The one reader surprised by the new meaning is the one who typed it for the old one —
/// and they land here, with nothing installed and nothing to resolve.  So this is where
/// the other spelling is named, rather than on every run.
#[test]
fn a_bare_install_with_no_dependencies_names_the_other_spelling() {
    let (all, code, home) = bare_install(
        "nodeps",
        "[package]\nname = \"instprobe\"\nversion = \"0.1.0\"\n\n[library]\n\
         entry = \"src/proj.loft\"\n",
    );
    assert_eq!(code, 0, "declaring no dependencies is not an error\n{all}");
    assert!(
        all.contains("loft install ."),
        "name the spelling that installs this package\n{all}"
    );
    assert!(
        !fa::exists(home.join(".loft/lib/instprobe")),
        "it must still not install the project\n{all}"
    );
    let _ = fa::remove_dir_all(home.parent().unwrap());
}

/// With no manifest there is nothing to read, and both meanings are worth naming: the
/// reader either wanted a registry package or wanted this directory installed.
#[test]
fn a_bare_install_without_a_manifest_names_both_spellings() {
    let base = std::env::temp_dir().join(format!("loft_966_nomanifest_{}", std::process::id()));
    let _ = fa::remove_dir_all(&base);
    fa::create_dir_all(&base).expect("mkdir");
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("install")
        .env("LOFT_TIMEOUT", "90")
        .current_dir(&base)
        .output()
        .expect("spawn loft install");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&base);
    assert_eq!(out.status.code(), Some(1), "no manifest is an error\n{all}");
    assert!(
        all.contains("loft install <pkg>") && all.contains("loft install ."),
        "both spellings must be named\n{all}"
    );
}

/// A manifest's `[package] name` decides the install directory, and a manifest is data —
/// on a fetched package, data somebody else wrote.  A name that is a PATH rather than a
/// name therefore chose where the package landed: `name = "../../escaped"` wrote the whole
/// tree outside `~/.loft/lib` entirely.
///
/// The rule that refuses it is `libscan::is_valid_package_name`, the same one `loft new`
/// enforces when a package is created — so no name that could legitimately arrive here is
/// refused by it.
#[test]
fn an_install_refuses_a_package_name_that_is_a_path() {
    let base = std::env::temp_dir().join(format!("loft_instsec_{}", std::process::id()));
    let _ = fa::remove_dir_all(&base);
    let home = base.join("home");
    let pkg = base.join("evil");
    fa::create_dir_all(&home).expect("mkdir home");
    write(
        &pkg.join("loft.toml"),
        "[package]\nname    = \"../../escaped\"\nversion = \"0.1.0\"\n\n\
         [library]\nentry = \"src/pkg.loft\"\n",
    );
    write(&pkg.join("src/pkg.loft"), "pub fn hi() -> integer { 1 }\n");

    let out = loft::platform::process::harness_command(loft_bin())
        .args(["install", "."])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("LOFT_TIMEOUT", "90")
        .current_dir(&pkg)
        .output()
        .expect("spawn loft install .");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // The load-bearing assertion is the third one: a refusal that still copied the files
    // would satisfy the first two.  `escaped` is where `~/.loft/lib/../../escaped` lands.
    assert_ne!(out.status.code(), Some(0), "the install must fail\n{all}");
    assert!(
        all.contains("not a usable package name"),
        "the refusal must say what is wrong with the name\n{all}"
    );
    assert!(
        !fa::exists(home.join("escaped")),
        "the package escaped ~/.loft/lib and wrote to {}\n{all}",
        home.join("escaped").display()
    );
    let _ = fa::remove_dir_all(&base);
}
