// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#2010 — a package is identified by its manifest's `[package] name`, never by the
//! directory it sits in.
//!
//! Inside package `probepkg`, `use probepkg;` binds the package's own entry.  The defect:
//! the only route from a package's `tests/` to its own entry was the sibling probe
//! `<parent>/<name>/src/<name>.loft`, which finds the package itself only when its
//! directory is named after it.  In a checkout called anything else the name fell through
//! to the installed registry copy, or to a neighbouring directory carrying the name, and
//! the suite ran against that — green whenever the API had not changed.  Which spelling ran
//! the suite decided it too: the `--tests <dir>` flag put no package path on the import
//! path at all, while `loft test` and a single-file run did, and a manifest without
//! `[library] entry` put the package ROOT there instead of `src/`.
//!
//! Hermetic: `LOFT_HOME` holds a cached registry `probepkg` and the registry URL cannot be
//! fetched.  Each copy's `own_marker()` names itself — the package under test 2010, the
//! registry copy 1, a sibling directory 2 — so no cell passes by loading the wrong one.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

const OWN: &str = "2010";

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    fa::create_dir_all(path.parent().unwrap()).expect("mkdir");
    fa::write(path, body).expect("write");
}

/// A `probepkg` package at `dir` whose `own_marker()` answers `marker`.  With `entry` the
/// manifest declares `[library] entry`; without it the default `src/probepkg.loft` holds.
/// Its tests: `t.loft` binds the package by its own name, `via.loft` binds a second module
/// of its `src/`, which itself binds the package by name.
fn package(dir: &Path, marker: &str, entry: bool) {
    let library = if entry {
        "\n[library]\nentry = \"src/probepkg.loft\"\n"
    } else {
        ""
    };
    write(
        &dir.join("loft.toml"),
        &format!("[package]\nname = \"probepkg\"\nversion = \"0.9.0\"\n{library}"),
    );
    write(
        &dir.join("src/probepkg.loft"),
        &format!("pub fn own_marker() -> integer {{ {marker} }}\n"),
    );
    write(
        &dir.join("src/other.loft"),
        "use probepkg;\npub fn other_marker() -> integer { probepkg::own_marker() }\n",
    );
    write(
        &dir.join("tests/t.loft"),
        "use probepkg;\nfn main() {\n  m = probepkg::own_marker();\n  \
         assert(m == 2010, \"bound the copy marked {m}\");\n  println(\"OWN {m}\");\n}\n",
    );
    write(
        &dir.join("tests/via.loft"),
        "use other;\nfn main() {\n  m = other::other_marker();\n  \
         assert(m == 2010, \"the src module bound the copy marked {m}\");\n  \
         println(\"OWN {m}\");\n}\n",
    );
}

struct Fixture {
    base: PathBuf,
    home: PathBuf,
}

/// A fresh home holding a cached registry `probepkg` 0.1.0, marked 1.
fn fixture(tag: &str) -> Fixture {
    let base = std::env::temp_dir().join(format!("loft_2010_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&base);
    let home = base.join("home");
    package(&home.join(".loft/registry/probepkg-0.1.0"), "1", true);
    Fixture { base, home }
}

impl Fixture {
    fn run(&self, cwd: &Path, args: &[&str]) -> String {
        let out = loft::platform::process::harness_command(loft_bin())
            .args(args)
            .env("LOFT_HOME", &self.home)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("LOFT_REGISTRY_URL", "http://127.0.0.1:1/index.json")
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "300")
            .current_dir(cwd)
            .output()
            .expect("spawn loft");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    /// Every spelling of a test run binds the package at `pkg` by its name: the `--tests`
    /// flag from inside the package and from outside it, the `loft test` subcommand, and
    /// each test file run on its own on both backends.
    fn all_spellings_bind_their_own(&self, cell: &str, pkg: &Path) {
        let tests = pkg.join("tests");
        let tests_abs = tests.to_string_lossy().into_owned();
        for (spelling, cwd, args) in [
            (
                "--tests inside",
                pkg,
                vec!["--interpret", "--tests", "tests"],
            ),
            (
                "--tests outside",
                self.base.as_path(),
                vec!["--interpret", "--tests", tests_abs.as_str()],
            ),
            ("loft test", pkg, vec!["test"]),
        ] {
            let out = self.run(cwd, &args);
            assert!(
                out.contains("test result: ok") && out.contains("2 passed"),
                "[{cell}, {spelling}] both tests bind the package's own copy\n{out}"
            );
        }
        for file in ["tests/t.loft", "tests/via.loft"] {
            for backend in ["--interpret", "--native"] {
                let out = self.run(pkg, &[backend, file]);
                assert!(
                    out.contains(&format!("OWN {OWN}")),
                    "[{cell}, {backend} {file}] binds the package's own copy\n{out}"
                );
            }
        }
    }
}

/// A checkout not named after its package, with the registry carrying the same name: its
/// tests used to run against the registry copy under the `--tests` flag.
#[test]
fn a_package_directory_not_named_after_it_binds_its_own_entry() {
    let fx = fixture("renamed");
    let pkg = fx.base.join("checkout");
    package(&pkg, OWN, true);
    fx.all_spellings_bind_their_own("renamed", &pkg);
}

/// A neighbouring directory named after the package is a different package: it used to
/// outrank the package's own entry on every spelling.
#[test]
fn a_sibling_directory_carrying_the_name_does_not_outrank_the_package() {
    let fx = fixture("sibling");
    let pkg = fx.base.join("checkout");
    package(&pkg, OWN, true);
    package(&fx.base.join("probepkg"), "2", true);
    fx.all_spellings_bind_their_own("sibling", &pkg);
}

/// Without `[library] entry` the package's own `src/` is on the import path: the default
/// entry used to put the package ROOT there, so `use other` found nothing.
#[test]
fn a_package_without_a_declared_entry_finds_its_own_src_modules() {
    let fx = fixture("no_entry");
    let pkg = fx.base.join("checkout");
    package(&pkg, OWN, false);
    fx.all_spellings_bind_their_own("no entry", &pkg);
}

/// A directory named after its package: `use probepkg` bound its own entry on every
/// spelling already, through the sibling probe; `use other` under the `--tests` flag did
/// not, because that spelling put no source directory on the import path.
#[test]
fn a_package_directory_named_after_it_still_binds_its_own_entry() {
    let fx = fixture("named");
    let pkg = fx.base.join("probepkg");
    package(&pkg, OWN, true);
    fx.all_spellings_bind_their_own("named", &pkg);
}
