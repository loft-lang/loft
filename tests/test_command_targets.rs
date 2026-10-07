// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `loft test` never reports a green for a file it did not run (loft#916).
//!
//! Everything after the first target used to be dropped in silence:
//! `loft test good.loft alsogood.loft` ran the first, printed `ok … 1 file`, and
//! exited 0 — even though `alsogood.loft` contains a failing test.  The file count
//! was the only place it showed, and that reads as correct unless you already knew
//! how many you asked for.  Naming two files is the natural move when a change
//! touches two suites and the whole run is slow, which is exactly when nobody
//! re-reads the count; it cost a sabotage sweep whose second half never executed and
//! was reported green.
//!
//! One target per run, and a second one is now an ERROR rather than a drop.  The
//! rows below pin that, and pin that the spellings which DO work still do — a check
//! that refused an argument after any flag would break `--lib <dir>`, whose value is
//! also a bare token.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// A package with one passing and one failing test file.
fn fixture(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("loft_916_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    fa::create_dir_all(root.join("tests")).expect("mkdir tests");
    fa::write(
        root.join("loft.toml"),
        "[package]\nname = \"t916\"\nversion = \"0.1.0\"\n",
    )
    .expect("manifest");
    fa::write(
        root.join("tests/good.loft"),
        "fn test_one() { assert(1 + 2 == 3, \"add\"); }\n",
    )
    .expect("good");
    fa::write(
        root.join("tests/alsogood.loft"),
        "fn test_two() { assert(2 + 2 == 5, \"THIS TEST FAILS ON PURPOSE\"); }\n",
    )
    .expect("alsogood");
    root
}

fn run(root: &PathBuf, args: &[&str]) -> (i32, String) {
    let out = Command::new(loft_bin())
        .current_dir(root)
        .args(args)
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run loft test");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// The defect: a second file must never be dropped into a green.  Asserted on the
/// EXIT CODE as well as the text, because the exit code is what a CI job reads and
/// it was the half that made this dangerous rather than merely confusing.
#[test]
fn a_second_target_is_refused_not_dropped() {
    let root = fixture("two");
    let (code, out) = run(&root, &["test", "good.loft", "alsogood.loft"]);
    assert_ne!(code, 0, "naming two files must not exit 0:\n{out}");
    assert!(
        out.contains("one target per run"),
        "the refusal must say what is wrong:\n{out}"
    );
    assert!(
        out.contains("good.loft") && out.contains("alsogood.loft"),
        "both targets must be named, so it is obvious which was dropped:\n{out}"
    );
    assert!(
        !out.contains("test result: ok."),
        "no green may be printed for a run that did not happen:\n{out}"
    );
    let _ = fa::remove_dir_all(&root);
}

/// The order-swapped form from the report: it is the SECOND POSITION that was
/// dropped, not a particular file, so the refusal must not depend on which file
/// happens to fail.
#[test]
fn the_refusal_does_not_depend_on_which_file_fails() {
    let root = fixture("swapped");
    let (code, out) = run(&root, &["test", "alsogood.loft", "good.loft"]);
    assert_ne!(code, 0, "the swapped order must be refused too:\n{out}");
    assert!(
        out.contains("one target per run"),
        "the swapped order must be refused for the same reason:\n{out}"
    );
    let _ = fa::remove_dir_all(&root);
}

/// The control that makes the refusal meaningful: with NO target both files run, and
/// the failing one is reported.  Without this row, a `loft test` that refused
/// everything would pass the tests above.
#[test]
fn no_target_still_runs_every_file() {
    let root = fixture("all");
    let (code, out) = run(&root, &["test"]);
    assert_ne!(code, 0, "the failing file must fail the run:\n{out}");
    assert!(
        out.contains("2 files"),
        "both files must run when none is named:\n{out}"
    );
    assert!(
        out.contains("THIS TEST FAILS ON PURPOSE"),
        "the failing assertion must be reported:\n{out}"
    );
    let _ = fa::remove_dir_all(&root);
}

/// One target still works, in each spelling — a bare file, a file with a `::`
/// selector, and a file followed by a FLAG.  The last is the row that matters for
/// the shape of the check: a flag's value is a bare token too, so a rule written as
/// "no bare token anywhere after the target" would break `--lib <dir>`.
#[test]
fn one_target_still_works_in_every_spelling() {
    let root = fixture("single");

    let (code, out) = run(&root, &["test", "good.loft"]);
    assert_eq!(code, 0, "a single passing file must pass:\n{out}");
    assert!(out.contains("1 file"), "and report one file:\n{out}");

    let (code, out) = run(&root, &["test", "alsogood.loft"]);
    assert_ne!(code, 0, "a single failing file must fail:\n{out}");

    let (code, out) = run(&root, &["test", "good.loft::test_one"]);
    assert_eq!(code, 0, "a `::selector` must still resolve:\n{out}");

    let (code, out) = run(&root, &["test", "good.loft", "--no-warnings"]);
    assert_eq!(code, 0, "a trailing flag is not a second target:\n{out}");

    let _ = fa::remove_dir_all(&root);
}

/// loft#925 — a target written AFTER a flag is the target, not a dropped argument.
///
/// `loft test` reads only a LEADING positional, so `loft test --lib src good.loft`
/// left the target at its `tests/` default and the path fell through to the
/// script-file slot, which the `--tests` dispatch never reads. Both files ran and
/// the run reported on both — a green (or a red) over a scope nobody asked for,
/// which is loft#916's failure mode surviving in the ordering its fix did not
/// reach. It is also what stopped loft#925's reporter cutting a standalone repro:
/// every invocation they tried ran the whole suite.
///
/// The failing file is the oracle. If the named target were still being dropped,
/// `--lib` + `good.loft` would run `alsogood.loft` too and exit non-zero — so this
/// row cannot pass by accident.
#[test]
fn a_target_after_a_flag_is_the_target() {
    let root = fixture("afterflag");

    let (code, out) = run(&root, &["test", "--no-warnings", "good.loft"]);
    assert_eq!(
        code, 0,
        "a target after a flag must run only that file:\n{out}"
    );
    assert!(
        out.contains("1 file"),
        "and the run must be scoped to it, not to the directory:\n{out}"
    );
    assert!(
        !out.contains("THIS TEST FAILS ON PURPOSE"),
        "the file that was NOT named must not run:\n{out}"
    );

    // The same ordering with a flag that TAKES A VALUE: `--lib`'s value is a bare
    // token, so a rule that adopted any trailing positional would swallow it and
    // treat the directory as the test target.
    let src = root.join("src");
    fa::create_dir_all(&src).expect("mkdir src");
    let (code, out) = run(
        &root,
        &["test", "--lib", src.to_str().unwrap(), "good.loft"],
    );
    assert_eq!(
        code, 0,
        "a flag with a value must not eat the target:\n{out}"
    );
    assert!(
        out.contains("1 file"),
        "and the target after it still scopes the run:\n{out}"
    );

    let _ = fa::remove_dir_all(&root);
}

/// Two targets split by a flag are two targets. The leading-positional check could
/// not see this pair at all, so it is the ordering where a dropped file was silent
/// even after loft#916 — and the refusal must name BOTH, exactly as the adjacent
/// spelling does.
#[test]
fn two_targets_separated_by_a_flag_are_refused() {
    let root = fixture("split");
    let (code, out) = run(
        &root,
        &["test", "good.loft", "--no-warnings", "alsogood.loft"],
    );
    assert_ne!(code, 0, "two targets must not exit 0:\n{out}");
    assert!(
        out.contains("one target per run"),
        "the refusal must say what is wrong:\n{out}"
    );
    assert!(
        out.contains("good.loft") && out.contains("alsogood.loft"),
        "both targets must be named:\n{out}"
    );
    assert!(
        !out.contains("test result: ok."),
        "no green may be printed for a run that did not happen:\n{out}"
    );
    let _ = fa::remove_dir_all(&root);
}

/// The `--tests` FLAG spelling of loft#925: it skips only the flags it knows
/// (`--native`, `--no-warnings`, `--deny-warnings`) before its path, so
/// `loft --tests --interpret tests/good.loft` left the path to the script-file slot
/// and swept every `.loft` under the working directory.  From a repo root that walk
/// reached `target/` and died at the watchdog, which reads as the named guard
/// crashing.  The failing sibling is the oracle again: had the path been dropped,
/// `alsogood.loft` would run and the exit would be non-zero.
#[test]
fn the_tests_flag_keeps_its_target_after_any_flag() {
    let root = fixture("flag");
    for args in [
        vec!["--tests", "--interpret", "tests/good.loft"],
        vec!["--interpret", "--tests", "tests/good.loft"],
        vec!["--tests", "tests/good.loft", "--interpret"],
    ] {
        let (code, out) = run(&root, &args);
        assert_eq!(code, 0, "{args:?} must run only the named file:\n{out}");
        assert!(
            out.contains("1 file"),
            "{args:?} must be scoped to it:\n{out}"
        );
        assert!(
            !out.contains("THIS TEST FAILS ON PURPOSE"),
            "{args:?} ran a file that was not named:\n{out}"
        );
    }
    // A flag that takes a VALUE between them: the value must not be taken as the target.
    let src = root.join("src");
    fa::create_dir_all(&src).expect("mkdir src");
    let (code, out) = run(
        &root,
        &["--tests", "--lib", src.to_str().unwrap(), "tests/good.loft"],
    );
    assert_eq!(
        code, 0,
        "a flag with a value must not eat the target:\n{out}"
    );
    assert!(
        out.contains("1 file"),
        "and the target after it scopes the run:\n{out}"
    );
    let _ = fa::remove_dir_all(&root);
}
