// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#861 — `loft cache status` / `loft cache prune` over a synthetic `~/.loft`.
//!
//! The unit tests in `src/cache_gc.rs` cover which entries are chosen. These cover
//! the part only the binary can answer: that `status` touches nothing, that `prune`
//! refuses when it is not the installed loft, and — the one that matters most — that
//! neither ever reaches a package's SOURCES. A cleanup tool that deletes the thing it
//! was pointed near is worse than no cleanup tool.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// A `~/.loft` with one build tree stamped with a key no loft will ever have, one
/// registry package carrying sources, and an auto-native artifact beside them.
fn fake_home(name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("loft_cache_cli_{name}"));
    let _ = fa::remove_dir_all(&home);
    let bc = home.join(".loft/build-cache/demo-1.0.0/release");
    fa::create_dir_all(&bc).expect("build-cache");
    fa::write(bc.join("libdemo.rlib"), vec![0u8; 64 * 1024]).unwrap();
    // A key that cannot be any loft's: the real one is `.max(1)`-ed, never 0.
    fa::write(bc.join(".loft-build-fp"), "0").unwrap();

    let pkg = home.join(".loft/registry/demo-1.0.0");
    fa::create_dir_all(pkg.join("src")).expect("registry");
    fa::write(pkg.join("src/lib.loft"), "fn demo() -> integer { 1 }\n").unwrap();
    fa::write(pkg.join("loft.toml"), "[package]\nname=\"demo\"\n").unwrap();
    let auto = pkg.join("native-auto");
    fa::create_dir_all(&auto).unwrap();
    let ext = std::env::consts::DLL_EXTENSION;
    fa::write(
        auto.join(format!("libloft_auto_demo_1_0_0_{:016x}.{ext}", 1u64)),
        vec![0u8; 8192],
    )
    .unwrap();
    home
}

/// `PATH` with the test binary's own directory removed.
///
/// The refusal under test asks "is the running loft the one on `PATH`", and this suite's
/// premise is that it is not.  On Unix that held by accident of environment; on Windows
/// **cargo puts the target directory on `PATH`** so a test can resolve its DLLs, which
/// makes the dev build genuinely the loft on `PATH` — so the guard answered `Some(true)`,
/// prune proceeded, and the test failed on the nightly's Windows leg.
///
/// Filtered rather than emptied: a Windows child still needs `System32` on `PATH` to
/// start at all, so clearing it would trade one platform artefact for another.
fn path_without_the_test_binary() -> std::ffi::OsString {
    let own = loft_bin().parent().map(Path::to_path_buf);
    let kept: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|d| own.as_deref().is_none_or(|o| d != o))
                .collect()
        })
        .unwrap_or_default();
    std::env::join_paths(kept).expect("rejoin PATH")
}

fn run(home: &Path, args: &[&str]) -> (String, bool) {
    let out = loft::platform::process::harness_command(loft_bin())
        .args(args)
        // `loft_home()` appends `.loft` itself, so this is the level ABOVE it.
        .env("LOFT_HOME", home)
        .env("PATH", path_without_the_test_binary())
        .output()
        .expect("spawn loft");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.success(),
    )
}

fn sources_intact(home: &Path) -> bool {
    fa::exists(home.join(".loft/registry/demo-1.0.0/src/lib.loft"))
        && fa::exists(home.join(".loft/registry/demo-1.0.0/loft.toml"))
}

/// `status` reports and changes nothing — which is what makes it the dry run for
/// `prune`, so there is no way to learn the figure only by deleting.
#[test]
fn status_reports_without_touching_anything() {
    let home = fake_home("status");
    let (out, ok) = run(&home, &["cache", "status"]);
    assert!(ok, "status must succeed:\n{out}");
    assert!(
        out.contains("build-cache") && out.contains("reclaimable"),
        "the footprint and the reclaimable part are the whole point:\n{out}"
    );
    assert!(
        fa::exists(home.join(".loft/build-cache/demo-1.0.0/release/libdemo.rlib")),
        "status must not delete:\n{out}"
    );
    assert!(sources_intact(&home));
    let _ = fa::remove_dir_all(&home);
}

/// The guard that stops a development build from wiping the installed loft's live
/// generation. The test binary is never the loft on PATH, so the refusal is the
/// default path here — which is exactly the situation it is written for.
#[test]
fn prune_refuses_from_a_binary_that_is_not_the_installed_loft() {
    let home = fake_home("guard");
    let (out, ok) = run(&home, &["cache", "prune"]);
    assert!(!ok, "it must refuse, not proceed:\n{out}");
    assert!(
        out.contains("not the installed loft"),
        "and say why — a bare non-zero exit teaches nothing:\n{out}"
    );
    assert!(
        fa::exists(home.join(".loft/build-cache/demo-1.0.0/release/libdemo.rlib")),
        "a refused prune deletes nothing:\n{out}"
    );
    let _ = fa::remove_dir_all(&home);
}

/// With `--force`, the dead generation goes — and the package's SOURCES stay. The
/// build cache sits beside a registry tree that holds the only copy of the library's
/// `.loft` files, and reaching those is the one failure this must never have.
#[test]
fn forced_prune_takes_the_dead_generation_and_spares_the_sources() {
    let home = fake_home("force");
    let (out, ok) = run(&home, &["cache", "prune", "--force"]);
    assert!(ok, "prune --force must succeed:\n{out}");
    assert!(
        !fa::exists(home.join(".loft/build-cache/demo-1.0.0")),
        "the tree stamped with a key no loft has must go:\n{out}"
    );
    assert!(
        sources_intact(&home),
        "the package sources are NOT cache and must survive:\n{out}"
    );
    assert!(
        out.contains("freed"),
        "and it reports what it actually gave back:\n{out}"
    );
    let _ = fa::remove_dir_all(&home);
}

/// `--all` is the implemented form of the `rm -rf` the docs used to prescribe. It
/// takes the live generation too — and still must not reach the sources.
#[test]
fn prune_all_clears_the_caches_but_not_the_package() {
    let home = fake_home("all");
    let (out, ok) = run(&home, &["cache", "prune", "--all", "--force"]);
    assert!(ok, "prune --all must succeed:\n{out}");
    let auto = home.join(".loft/registry/demo-1.0.0/native-auto");
    let left = fa::read_dir(&auto)
        .map(|d| {
            d.iter()
                .filter(|e| e.file_name().is_some_and(|n| n.contains("loft_auto_demo")))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(left, 0, "--all takes the artifacts too:\n{out}");
    assert!(
        sources_intact(&home),
        "…and never the sources beside them:\n{out}"
    );
    let _ = fa::remove_dir_all(&home);
}

/// An unknown subcommand fails loudly with the usage line rather than defaulting to
/// something destructive.
#[test]
fn an_unknown_cache_subcommand_is_refused() {
    let home = fake_home("bad");
    let (out, ok) = run(&home, &["cache", "obliterate"]);
    assert!(!ok, "an unknown verb must not succeed:\n{out}");
    assert!(out.contains("usage: loft cache"), "{out}");
    assert!(sources_intact(&home));
    let _ = fa::remove_dir_all(&home);
}

/// A machine that never installed a package says so, instead of printing an empty
/// table or a zero-byte total dressed up as a report.
#[test]
fn an_empty_cache_says_so() {
    let home = std::env::temp_dir().join("loft_cache_cli_empty");
    let _ = fa::remove_dir_all(&home);
    fa::create_dir_all(home.join(".loft")).unwrap();
    let (out, ok) = run(&home, &["cache", "status"]);
    assert!(ok, "{out}");
    assert!(out.contains("nothing cached yet"), "{out}");
    let _ = fa::remove_dir_all(&home);
}
