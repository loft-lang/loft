// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Which functions `--tests` runs, and how many it says it ran.
//!
//! Two cases, one `if` in `src/test_runner.rs` (mirrored in `src/main.rs`'s native
//! entry-point generator, deliberately in step):
//!
//! * a file that names at least one `test_*` has said which functions are tests, so
//!   those are the whole set and every helper beside them is a helper;
//! * a file that names NONE keeps arity — every zero-parameter function runs, `main`
//!   included, and a parameter is the only way to say "not an entry point" (loft#1010).
//!
//! The counted total is the half a loft file cannot check about itself:
//! `tests/scripts/1010-test-runner-discovery.loft` says so in its own header, and
//! predicted that a move to a name-based rule would leave it passing — which is what
//! happened, and why the count is asserted from out here instead. The `(N fns: …)` line
//! is the cheapest reading of the rule there is.
//!
//! The underscore matters and nothing reports it: `testify`, or a function called exactly
//! `test`, beside a `test_*` is not a test — the run says `ok` having never called it. A
//! camel-case `testDouble` cannot reach that trap, because loft refuses the NAME.

use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Write one source into a directory of its own and run `--tests` over it.
fn run_tests(case: &str, source: &str, extra: &[&str]) -> String {
    run_tests_env(case, source, extra, &[])
}

/// [`run_tests`] with environment variables set for the child.
fn run_tests_env(case: &str, source: &str, extra: &[&str], env: &[(&str, &str)]) -> String {
    let dir = std::env::temp_dir().join(format!("loft_discovery_{case}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("case dir");
    let file = dir.join("subject.loft");
    fa::write(&file, source).expect("source");
    let out = Command::new(loft_bin())
        .arg("--tests")
        .args(extra)
        .arg("subject.loft")
        .current_dir(&dir)
        .env("LOFT_TIMEOUT", "300")
        .envs(env.iter().copied())
        .output()
        .expect("loft --tests");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&dir);
    combined
}

/// The `(N fns: a, b)` fragment of the per-file line, which names the set that ran.
fn ran_set(report: &str) -> String {
    report
        .lines()
        .find_map(|l| {
            let open = l.find(" (")?;
            let close = l.rfind(')')?;
            l.get(open + 2..close).map(str::to_string)
        })
        .unwrap_or_else(|| panic!("no per-file line in:\n{report}"))
}

const NAMED: &str = "\
fn helper_that_is_not_a_test() { println(\"HELPER RAN\"); }
fn testify() { println(\"NO_UNDERSCORE RAN\"); }
fn test() { println(\"BARE_TEST RAN\"); }
fn test_the_only_one() { assert(1 + 1 == 2, \"the named test runs\"); }
";

const UNNAMED: &str = "\
fn first_entry() { assert(1 + 1 == 2, \"runs\"); }
fn second_entry() { println(\"SECOND RAN\"); }
fn opted_out(unused: integer) { println(\"PARAM RAN\"); assert(unused < 0, \"must not run\"); }
";

#[test]
fn one_test_underscore_makes_the_rest_helpers() {
    let report = run_tests("named", NAMED, &[]);
    assert_eq!(
        ran_set(&report),
        "1 fn: test_the_only_one",
        "a file naming a `test_*` runs only those:\n{report}"
    );
    assert!(
        !report.contains("HELPER RAN"),
        "the helper beside a `test_*` must not run:\n{report}"
    );
    // The silent half: without the underscore it is not a test, and nothing says so.
    // (A camel-case spelling like `testDouble` cannot reach this — loft refuses it as a
    // function name, so lower-case near-misses are the whole hazard.)
    assert!(
        !report.contains("NO_UNDERSCORE RAN"),
        "`testify` has no underscore and is not a test:\n{report}"
    );
    assert!(
        !report.contains("BARE_TEST RAN"),
        "a function called exactly `test` is not a `test_*` either:\n{report}"
    );
    assert!(
        report.contains("test result: ok."),
        "and the run still reports ok, which is why this is worth a guard:\n{report}"
    );
}

#[test]
fn a_file_with_no_test_underscore_keeps_arity() {
    let report = run_tests("unnamed", UNNAMED, &[]);
    assert_eq!(
        ran_set(&report),
        "2 fns: first_entry, second_entry",
        "with no `test_*` every zero-parameter function is an entry point:\n{report}"
    );
    assert!(
        report.contains("SECOND RAN"),
        "including one that only prints:\n{report}"
    );
    assert!(
        !report.contains("PARAM RAN"),
        "a parameter is what opts out (loft#1010):\n{report}"
    );
}

/// `src/main.rs`'s generated entry point applies the same filter on purpose — a native
/// run that executes a different SET than the interpreter is a backend divergence the
/// suite would read as a wrong answer.
#[test]
fn the_native_entry_point_runs_the_same_set() {
    if !rustc_available() {
        eprintln!("SKIP the_native_entry_point_runs_the_same_set: no rustc");
        return;
    }
    for (case, source, want) in [
        ("named_native", NAMED, "1 fn: test_the_only_one"),
        (
            "unnamed_native",
            UNNAMED,
            "2 fns: first_entry, second_entry",
        ),
    ] {
        let report = run_tests(case, source, &["--native"]);
        assert_eq!(
            ran_set(&report),
            want,
            "native ran a different set than the interpreter:\n{report}"
        );
    }
}

fn rustc_available() -> bool {
    Command::new("rustc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The worked example for @F89 lives in the corpus and cannot count its own run, so the
/// count it predicts is checked here against the file itself.
#[test]
fn the_corpus_witness_reports_the_name_based_count() {
    let witness = Path::new("tests/scripts/1010-test-runner-discovery.loft");
    let source = fa::read_to_string(witness).expect("the discovery witness");
    let report = run_tests("witness", &source, &[]);
    assert_eq!(
        ran_set(&report),
        "1 fn: test_the_ordinary_shape",
        "the witness names a `test_*`, so that is the whole set:\n{report}"
    );
}

const ONE_FAILS: &str = "\
fn test_ok() { assert(1 + 1 == 2, \"ok\"); }
fn test_bad() { assert(1 + 1 == 3, \"the bad one\"); }
fn test_also_ok() { assert(2 * 2 == 4, \"also ok\"); }
";

/// A native run attributes a failure to the test that failed (loft#1649).  The file is
/// ONE binary, and its exit status used to be read once: one failing test marked every
/// test in the file failed, each with the same message and none named.  Each test is now
/// its own run of that binary, so the report matches the interpreter's.
#[test]
fn a_native_failure_names_the_test_that_failed() {
    for (case, extra) in [("attr_interp", &[][..]), ("attr_native", &["--native"][..])] {
        let report = run_tests(case, ONE_FAILS, extra);
        assert!(
            report.contains("1 failed, 2 passed"),
            "{case}: one of three tests fails:\n{report}"
        );
        assert!(
            report.contains("::test_bad  —  assertion failed: the bad one"),
            "{case}: the failing test is named with its assertion:\n{report}"
        );
        assert!(
            !report.contains("::test_ok  —") && !report.contains("::test_also_ok  —"),
            "{case}: the passing tests are not reported as failed:\n{report}"
        );
    }
}

/// loft#1865 — a file is compiled ONCE however many tests it has, and each test still runs on
/// a heap of its own.  The runner cloned the checked program and recompiled every function per
/// test function, so a file's test time grew with tests × size (300 small tests: 4.5 s, against
/// 0.18 s compiled once).  `LOFT_TIMING` reports one `byte_code_from` line per compile, which
/// makes the count observable; `LOFT_TEST_RECOMPILE=1` keeps the per-test path, and is the
/// control that the count can move.
#[test]
fn a_file_is_compiled_once_however_many_tests_it_has() {
    let src = "\
fn test_a() { v: vector<integer> = [1, 2]; v += [3]; assert(len(v) == 3, \"a\"); }
fn test_b() { v: vector<integer> = [7]; assert(len(v) == 1, \"b sees its own heap\"); }
fn test_c() { panic(\"c fails alone\"); }
";
    let compiles = |report: &str| {
        report
            .lines()
            .filter(|l| l.contains("byte_code_from"))
            .count()
    };
    let once = run_tests("compile_once", src, &[]);
    let per_test = run_tests_env(
        "compile_each",
        src,
        &[],
        &[("LOFT_TIMING", "1"), ("LOFT_TEST_RECOMPILE", "1")],
    );
    let timed = run_tests_env("compile_once_t", src, &[], &[("LOFT_TIMING", "1")]);
    assert_eq!(
        compiles(&per_test),
        3,
        "the control compiles per test:\n{per_test}"
    );
    assert_eq!(compiles(&timed), 1, "one compile for three tests:\n{timed}");
    // The verdicts are the per-test ones: two pass, the panic fails only its own test.
    for report in [&once, &timed, &per_test] {
        assert!(report.contains("FAIL  subject.loft::test_c"), "{report}");
        assert!(report.contains("(1 failed, 2 passed)"), "{report}");
    }
}
