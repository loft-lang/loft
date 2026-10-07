// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN77 T4/T5 — how `loft test --deps` turns a dependency into a directory.
//!
//! Four sources, in a fixed order: a path dep, an explicit `--lock` pin, a sibling
//! directory, then the project's own `loft.lock`.  The order is the contract, and two
//! cells here are the ones that make it one rather than an accident:
//!
//! - `the_working_copy_outranks_an_implicit_lock` — in a multi-package repo the sibling
//!   directory IS the dependency you mean.  Had the lock outranked it, adding lockfile
//!   support would have silently switched every such repo from testing its working copy
//!   to testing a published tarball out of the cache, while the run looked identical.
//! - `an_explicit_lock_outranks_the_working_copy` — the opposite way, because
//!   pre-flighting a candidate lock is the entire purpose of naming one.
//!
//! Each package answers with its own name, so a cell asserts WHICH copy ran rather than
//! that something ran: `sibling-WORKING-COPY` and `sibling-FROM-CACHE` are two packages
//! of the same name and version, and only the label separates them.
//!
//! `a_skipped_package_still_has_its_dependencies_walked` guards the other design choice.
//! `--skip` drops a package's own tests and keeps walking through it — the other reading
//! (skip the subtree) silently drops everything reachable only through it, and a skip is
//! asked for because a package is broken here, not because its dependencies are.

use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    fa::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fa::write(path, body).expect("write");
}

/// A package that answers `label` from `who()`, with a test asserting exactly that.
fn make_pkg(dir: &Path, name: &str, version: &str, label: &str, deps: &str) {
    write(
        &dir.join("loft.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nloft = \">=0.8\"\n\n\
             [library]\nentry = \"src/{name}.loft\"\n{deps}"
        ),
    );
    write(
        &dir.join(format!("src/{name}.loft")),
        &format!("pub fn who() -> text {{ \"{label}\" }}\n"),
    );
    write(
        &dir.join("tests/t.loft"),
        &format!(
            "use {name};\n\nfn test_identity() {{\n  \
             assert({name}::who() == \"{label}\", \"{label}\");\n}}\n"
        ),
    );
}

fn lock_entry(name: &str, version: &str) -> String {
    format!(
        "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n\
         url = \"https://example.invalid/{name}.tar.gz\"\n\
         sha256 = \"00\"\nsource = \"registry\"\n"
    )
}

/// Root depends on `sibling` (a working copy AND a cache copy exist), `pinned` (cache
/// only), `chain` (cache only, itself depending on `leaf`), and `unpinned` (nowhere).
struct Fixture {
    home: PathBuf,
    root: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let base = std::env::temp_dir().join(format!("loft_dep_walk_{tag}"));
    let _ = fa::remove_dir_all(&base);
    let home = base.join("home");
    let reg = home.join(".loft/registry");
    let proj = base.join("proj");

    make_pkg(
        &proj.join("sibling"),
        "sibling",
        "0.0.1",
        "sibling-WORKING-COPY",
        "",
    );
    make_pkg(
        &reg.join("sibling-0.0.1"),
        "sibling",
        "0.0.1",
        "sibling-FROM-CACHE",
        "",
    );
    make_pkg(
        &reg.join("pinned-1.0.0"),
        "pinned",
        "1.0.0",
        "pinned-v1",
        "",
    );
    make_pkg(
        &reg.join("pinned-2.0.0"),
        "pinned",
        "2.0.0",
        "pinned-v2",
        "",
    );
    make_pkg(&reg.join("leaf-1.0.0"), "leaf", "1.0.0", "leaf-v1", "");
    make_pkg(
        &reg.join("chain-1.0.0"),
        "chain",
        "1.0.0",
        "chain-v1",
        "\n[dependencies]\nleaf = \">=1.0\"\n",
    );

    let root = proj.join("root");
    make_pkg(
        &root,
        "root",
        "0.1.0",
        "root",
        "\n[dependencies]\nsibling = \">=0.0.1\"\npinned = \">=1.0\"\n\
         chain = \">=1.0\"\nunpinned = \">=1.0\"\n",
    );

    let base_lock = format!(
        "schema_version = 1\n{}{}{}{}",
        lock_entry("sibling", "0.0.1"),
        lock_entry("pinned", "1.0.0"),
        lock_entry("chain", "1.0.0"),
        lock_entry("leaf", "1.0.0"),
    );
    write(&root.join("loft.lock"), &base_lock);
    // The candidate differs from the project's lock in ONE pin, so a cell that sees
    // `pinned-v2` can only have read this file.
    write(
        &root.join("candidate.lock"),
        &base_lock.replace(
            "name = \"pinned\"\nversion = \"1.0.0\"",
            "name = \"pinned\"\nversion = \"2.0.0\"",
        ),
    );
    write(
        &root.join("badpin.lock"),
        &base_lock.replace(
            "name = \"pinned\"\nversion = \"1.0.0\"",
            "name = \"pinned\"\nversion = \"9.9.9\"",
        ),
    );
    Fixture { home, root }
}

fn run(fx: &Fixture, args: &[&str]) -> (String, i32) {
    let out = Command::new(loft_bin())
        .args(["test", "--deps"])
        .args(args)
        .current_dir(&fx.root)
        .env("LOFT_HOME", &fx.home)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_DENY_WARNINGS")
        .output()
        .expect("run loft test --deps");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (text, out.status.code().unwrap_or(-1))
}

#[test]
fn the_working_copy_outranks_an_implicit_lock() {
    let fx = fixture("implicit");
    let (out, _) = run(&fx, &[]);
    assert!(
        out.contains("testing sibling\n"),
        "the sibling working copy must win over the cache copy the lock pins:\n{out}"
    );
    assert!(
        !out.contains("testing sibling-0.0.1"),
        "an implicit lock must not silently redirect a repo to a published tarball:\n{out}"
    );
    // And the lock still fills the hole it is there for.
    assert!(out.contains("testing pinned-1.0.0"), "{out}");
}

#[test]
fn an_explicit_lock_outranks_the_working_copy() {
    let fx = fixture("explicit");
    let (out, _) = run(&fx, &["--lock=candidate.lock"]);
    assert!(
        out.contains("testing sibling-0.0.1"),
        "a lock that was ASKED for is the authority:\n{out}"
    );
    assert!(
        out.contains("testing pinned-2.0.0"),
        "the pin must come from the named file, not from newest-installed:\n{out}"
    );
}

#[test]
fn with_no_lockfile_only_path_and_sibling_deps_resolve() {
    let fx = fixture("nolock");
    fa::remove_file(fx.root.join("loft.lock")).expect("drop the lock");
    let (out, code) = run(&fx, &[]);
    assert!(out.contains("testing sibling"), "{out}");
    assert!(
        !out.contains("testing pinned"),
        "without a lock there is nothing to resolve a registry dep with:\n{out}"
    );
    assert_eq!(code, 0, "{out}");
}

#[test]
fn pinned_but_uninstalled_reads_differently_from_never_pinned() {
    let fx = fixture("badpin");
    let (out, _) = run(&fx, &["--lock=badpin.lock"]);
    assert!(
        out.contains("skipping pinned (locked, but not installed"),
        "a pin this box never installed is one `loft install` away, and must say so:\n{out}"
    );
    assert!(
        out.contains("skipping unpinned (no path-dep and no lockfile pin"),
        "a dep no lock names cannot be resolved by installing, and must read differently:\n{out}"
    );
}

#[test]
fn an_unreadable_lock_is_a_usage_error_not_a_test_failure() {
    let fx = fixture("badlock");
    let (out, code) = run(&fx, &["--lock=nope.lock"]);
    assert_eq!(
        code, 2,
        "a mistyped path is a usage error, not `tests failed`:\n{out}"
    );
    assert!(out.contains("--lock: no lockfile at"), "{out}");

    write(&fx.root.join("garbage.lock"), "not a lockfile\n[[bogus]]\n");
    let (out, code) = run(&fx, &["--lock=garbage.lock"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("--lock: cannot read"), "{out}");
    // It must fail BEFORE the project's own suite runs — otherwise a typo costs a
    // full test run before reporting itself.
    assert!(
        !out.contains("--deps:"),
        "the walk must not have started:\n{out}"
    );
}

#[test]
fn a_skipped_package_still_has_its_dependencies_walked() {
    let fx = fixture("skip");
    let (out, _) = run(&fx, &["--skip=chain"]);
    assert!(out.contains("skipping chain (--skip)"), "{out}");
    assert!(
        out.contains("testing leaf-1.0.0"),
        "`leaf` is reachable only through the skipped `chain`, and dropping it would be \
         a silent loss of coverage:\n{out}"
    );
}

#[test]
fn a_skip_that_matches_nothing_says_so() {
    let fx = fixture("typo");
    let (out, _) = run(&fx, &["--skip=chian"]);
    assert!(
        out.contains("--skip named chian which no dependency matched"),
        "a misspelled skip silently widens the run, so it has to be reported:\n{out}"
    );
}

#[test]
fn a_deps_lint_debt_fails_the_consumer_only_under_strict_deps() {
    let fx = fixture("strict");
    // A warning inside a dependency, with the test itself still passing.
    write(
        &fx.home.join(".loft/registry/leaf-1.0.0/tests/t.loft"),
        "use leaf;\n\nfn test_identity() {\n  unused_local = 42;\n  \
         assert(leaf::who() == \"leaf-v1\", \"leaf-v1\");\n}\n",
    );
    let deny = |args: &[&str]| -> i32 {
        Command::new(loft_bin())
            .args(["test", "--deps"])
            .args(args)
            .current_dir(&fx.root)
            .env("LOFT_HOME", &fx.home)
            .env("LOFT_TIMEOUT", "120")
            .env("LOFT_DENY_WARNINGS", "1")
            .output()
            .expect("run")
            .status
            .code()
            .unwrap_or(-1)
    };
    // `--no-warnings` silences the PRINTING only; whether a warning is fatal is read
    // from the environment the child inherits.  Without neutralising it, a consumer
    // who exports LOFT_DENY_WARNINGS=1 was failed by lint debt in a package it does
    // not own — which is the thing the default exists to prevent.
    assert_eq!(deny(&[]), 0, "a dep's warnings must not fail its consumer");
    assert_ne!(
        deny(&["--strict-deps"]),
        0,
        "--strict-deps is what opts back in, for the one who does own them"
    );
}
