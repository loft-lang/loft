// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#1684 — a program whose library has dependencies, or builds native, warms like any
//! other.
//!
//! Resolving a cached registry package appends the registry root to `p.lib_dirs`, and the
//! program cache was keyed on `p.lib_dirs` AFTER the parse, while the warm load looks it up
//! BEFORE: every such program saved its bundle under a name its next run never computed,
//! and cold-parsed on every launch.  Independently, a program whose libraries compiled to an
//! auto-native cdylib — every registry library does by default — never saved a bundle at
//! all.  Together: `use graphics;` alone cost ~1.1 s and 80 MB per run.
//!
//! Every cell asserts the program's ANSWER beside the cache verdict, which it reads off
//! `LOFT_TRACE_WARM=1` (a named hit or a named miss), never off timing.
//!
//! @falsified-at: hand-measured (2026-09-26), two sabotages, one per half of the fix:
//!   - `save_program` keyed on `p.lib_dirs` again (the post-parse search path): c1's second
//!     run names `[warm] manifest: no manifest at …` where this file says `[warm] hit`;
//!   - the save gated on `!has_auto_native` again: no bundle is written, and c1 fails the
//!     same way.
//!
//!   The answer stays `probepkg-0.1.0 45` under both.  CHANNEL: the cache verdict (the trace
//!   line), not the value.

use std::path::{Path, PathBuf};
use std::process::Command;

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

/// A private tree with a library `probepkg` under `libs/` (the search path the run is given)
/// that DEPENDS on `probedep` by a path OUTSIDE it (`ext/`).  The dependency is what makes the
/// case: registering a package's `[dependencies]` appends the dependency's parent to the
/// search path mid-parse — for a registry package that is the registry root, which is how
/// `use graphics;` (→ mesh3d, glb) reached it.  A path dependency takes the same step without
/// a registry, so the cell needs no index and no network.  `probe_id()` answers the
/// library's name and version, and `probe_sum` crosses into the dependency.
fn home_with_package(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("loft_1684_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    write(
        &home.join("ext/probedep/loft.toml"),
        "[package]\nname = \"probedep\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\n\
         [library]\nentry = \"src/probedep.loft\"\n",
    );
    write(
        &home.join("ext/probedep/src/probedep.loft"),
        "pub fn dep_sum(n: integer) -> integer { s = 0; for i in 0..n { s += i; } s }\n",
    );
    write(
        &home.join("libs/probepkg/loft.toml"),
        "[package]\nname = \"probepkg\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\n\
         [library]\nentry = \"src/probepkg.loft\"\n\n\
         [dependencies]\nprobedep = { path = \"../../ext/probedep\" }\n",
    );
    write(
        &home.join("libs/probepkg/src/probepkg.loft"),
        "use probedep;\n\
         pub fn probe_id() -> text { return \"probepkg-0.1.0\"; }\n\
         pub fn probe_sum(n: integer) -> integer { dep_sum(n) }\n",
    );
    write(
        &home.join("proj/s.loft"),
        "use probepkg;\nfn main() { println(\"{probe_id()} {probe_sum(10)}\"); }\n",
    );
    home
}

/// One `--interpret` run of the project's script with the program cache forced on (a
/// `target/` build disables it by default) in a private cache dir, offline, traced.
fn run(home: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(["--interpret", "--lib"])
        .arg(home.join("libs"))
        .arg("s.loft")
        .env("LOFT_HOME", home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("LOFT_PROGRAM_CACHE", "1")
        .env("LOFT_TRACE_WARM", "1")
        .env("LOFT_OFFLINE", "1")
        .env_remove("LOFT_NO_CACHE")
        .env_remove("LOFT_NO_NATIVE_LIBS")
        .env("LOFT_TIMEOUT", "240")
        .current_dir(home.join("proj"));
    // The cell's own switches last, so the defaults above cannot remove them.
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const ANSWER: &str = "probepkg-0.1.0 45";

/// The auto-native cdylibs the saved manifest records (`alib` lines).
fn recorded_artifacts(home: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(home.join("cache/loft"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "manifest")
            && let Ok(text) = std::fs::read_to_string(&p)
        {
            out.extend(
                text.lines()
                    .filter_map(|l| l.strip_prefix("alib "))
                    .map(String::from),
            );
        }
    }
    out
}

/// c1 — the second run of an unchanged program that uses a registry package is a warm HIT,
/// and answers the same; the manifest records the library's auto-native cdylib.
/// c2 — that cdylib removed: a named miss, the same answer, and the run after warms again.
/// c3 — `LOFT_NO_NATIVE_LIBS=1` over a bundle marked native: a named miss by context and the
/// same answer, never a replay of the native marks.
#[test]
fn a_program_whose_library_has_dependencies_or_builds_native_warms() {
    let home = home_with_package("warm");

    let cold = run(&home, &[]);
    assert!(cold.contains(ANSWER), "c1 cold answer:\n{cold}");
    let warm = run(&home, &[]);
    assert!(warm.contains(ANSWER), "c1 warm answer:\n{warm}");
    assert!(
        warm.contains("[warm] hit"),
        "c1: the second run of an unchanged program must be a warm hit (loft#1684):\n{warm}"
    );
    let arts = recorded_artifacts(&home);
    assert!(
        !arts.is_empty(),
        "c1: the manifest must record the library's auto-native cdylib (alib)"
    );

    for a in &arts {
        std::fs::remove_file(a).expect("remove the recorded cdylib");
    }
    let gone = run(&home, &[]);
    assert!(
        gone.contains(ANSWER),
        "c2 answer with the cdylib gone:\n{gone}"
    );
    assert!(
        gone.contains("[warm] miss: auto-native artifact"),
        "c2: a missing recorded cdylib must be a named miss:\n{gone}"
    );
    let again = run(&home, &[]);
    assert!(
        again.contains(ANSWER) && again.contains("[warm] hit"),
        "c2 re-warm:\n{again}"
    );

    let nolibs = run(&home, &[("LOFT_NO_NATIVE_LIBS", "1")]);
    assert!(nolibs.contains(ANSWER), "c3 answer:\n{nolibs}");
    assert!(
        nolibs.contains("[warm] miss: the native-library context differs"),
        "c3: a bundle marked native must not serve a LOFT_NO_NATIVE_LIBS run:\n{nolibs}"
    );

    let _ = std::fs::remove_dir_all(&home);
}
