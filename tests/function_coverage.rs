// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `loft --tests` reports the functions a suite never entered.
//!
//! A test suite that never enters a function has not checked it, and before this the
//! two were indistinguishable in the output — the same silence-reads-as-coverage shape
//! as the backend-scope note next to it.
//!
//! The report is a LIST, never a percentage or a gate. A percentage becomes a target,
//! and a coverage target is what produces tests written to reach a line instead of
//! tests that check a behaviour. A gate would be worse still: a library is written
//! before its consumers exist, so a bar would fail exactly the case the package system
//! is meant to support.
//!
//! What is asserted here is mostly the QUIET direction, because a report that cannot
//! stay quiet gets ignored, and the ways this one could wrongly accuse are specific:
//! a `#native` declaration has no loft body to enter, a generator resumes instead of
//! being called, and a dependency's functions are not the package's to cover. Each got
//! it wrong during development, and each would have made the number meaningless.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Write a package (`loft.toml` + `src/<name>.loft` + `tests/t.loft`) into a temp dir,
/// run `loft --tests tests` in it, and return stdout.
///
/// A real package layout, not a loose script: the coverage report anchors on the
/// package root to tell the code under test from its dependencies, so a flat file
/// would not exercise the path that matters.
fn coverage_of(name: &str, lib: &str, test: &str) -> String {
    // The package DIRECTORY has to be named after the package: `use <name>;` resolves
    // by looking for a directory called `<name>`, so a uniquified dir name makes the
    // library unfindable and the whole fixture parse-fail.
    let holder = std::env::temp_dir().join(format!("loft_cov_{name}_{}", std::process::id()));
    let root = holder.join(name);
    let _ = fa::remove_dir_all(&holder);
    fa::create_dir_all(root.join("src")).expect("create src");
    fa::create_dir_all(root.join("tests")).expect("create tests");
    fa::write(
        root.join("loft.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\n[library]\nentry = \"src/{name}.loft\"\n"
        ),
    )
    .expect("write loft.toml");
    // The entry file is named after the package: `use <name>;` resolves to
    // `src/<name>.loft`, so a generic `lib.loft` would not be found at all.
    fa::write(root.join(format!("src/{name}.loft")), lib).expect("write lib");
    fa::write(root.join("tests/t.loft"), test).expect("write test");

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg("--tests")
        .arg("tests")
        .current_dir(&root)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "60")
        .output()
        .expect("failed to invoke loft");
    let _ = fa::remove_dir_all(&holder);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // A package that fails to parse produces no coverage at all, which would satisfy
    // every "must NOT be listed" assertion below without testing anything.  Fail loudly
    // instead — this fired for real when the entry file was named `lib.loft`.
    assert!(
        !stdout.contains("(parse errors)") && stdout.contains("test result: ok"),
        "the fixture package must build and pass before its coverage means anything: \
         {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// The positive case: a function the tests never call is named, with its file and line
/// so it can be opened directly.
#[test]
fn a_function_the_tests_never_call_is_named() {
    let out = coverage_of(
        "pos",
        "pub fn used(a: integer) -> integer { a + 1 }\n\
         pub fn never_called(a: integer) -> integer { a + 2 }\n",
        "use pos::*;\nfn test_one() { assert(used(1) == 2, \"used\"); }\n",
    );
    assert!(
        out.contains("never_called"),
        "the uncalled function must be named: {out}"
    );
    assert!(
        out.contains("src/pos.loft"),
        "it must carry a path to open, relative to the package: {out}"
    );
    assert!(
        !out.contains("  used"),
        "a function the tests DO call must not be listed: {out}"
    );
}

/// The quiet case, and the one that decides whether the report survives: a suite that
/// reaches everything says so explicitly. "All covered" and "nothing was measured" must
/// not look alike — that is the defect this whole feature exists to remove, so it would
/// be self-defeating to reproduce it in the report.
#[test]
fn a_fully_covered_package_says_so() {
    let out = coverage_of(
        "full",
        "pub fn a(x: integer) -> integer { x + 1 }\npub fn b(x: integer) -> integer { x + 2 }\n",
        "use full::*;\nfn test_all() { assert(a(1) == 2 && b(1) == 3, \"both\"); }\n",
    );
    assert!(
        out.contains("all") && out.contains("functions were entered"),
        "full coverage must be stated, not left as silence: {out}"
    );
    assert!(
        !out.contains("never entered"),
        "nothing may be reported uncovered here: {out}"
    );
}

/// A METHOD is covered like any other function. loft stores methods as
/// `t_<LEN><Type>_<method>`, so the first implementation — which matched a bare `n_`
/// prefix — counted only free functions and reported 4 of `arguments`' 33. Since a
/// library's API is mostly methods, that made the number close to meaningless.
#[test]
fn methods_are_counted_and_named_readably() {
    let out = coverage_of(
        "meth",
        "pub fn touched(self: integer) -> integer { self + 1 }\n\
         pub fn untouched(self: integer) -> integer { self + 2 }\n",
        "use meth::*;\nfn test_m() { x = 1; assert(x.touched() == 2, \"touched\"); }\n",
    );
    assert!(
        out.contains("untouched"),
        "an uncalled method must be reported: {out}"
    );
    assert!(
        out.contains("integer.untouched"),
        "it must be spelled the way it is written, not as its internal name: {out}"
    );
}

/// A GENERATOR is entered by resuming, not by `fn_call`, so hooking only the call path
/// reported every `iterator<T>` function as dead however hard it was iterated — `regex`
/// showed 6 of 8 uncovered while its tests drove all of them. Iterating one counts.
#[test]
fn an_iterated_generator_counts_as_entered() {
    let out = coverage_of(
        "gen",
        "pub fn counter(n: integer) -> iterator<integer> { for i in 0..n { yield i; } }\n",
        "use gen::*;\nfn test_g() { t = 0; for v in counter(3) { t = t + v; } assert(t == 3, \"sum\"); }\n",
    );
    assert!(
        !out.contains("counter"),
        "an iterated generator must not be reported as never entered: {out}"
    );
}

/// A generator that is CREATED but never iterated ran none of its body, so it stays on
/// the list. The distinction is the reason the hook sits on resume rather than on
/// creation: claiming coverage that did not happen is worse than missing some.
#[test]
fn a_generator_that_is_never_iterated_stays_uncovered() {
    let out = coverage_of(
        "genq",
        "pub fn made(n: integer) -> iterator<integer> { for i in 0..n { yield i; } }\n\
         pub fn other(x: integer) -> integer { x }\n",
        "use genq::*;\nfn test_q() { assert(other(1) == 1, \"other\"); }\n",
    );
    assert!(
        out.contains("made"),
        "a generator whose body never ran must be reported: {out}"
    );
}
