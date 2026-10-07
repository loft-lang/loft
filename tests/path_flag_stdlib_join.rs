// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1112 — `--path <dir>` means the same directory with or without a trailing slash.
//!
//! The test runner built the stdlib path by CONCATENATION (`default_dir + "default"`), which
//! is only correct for a value that already ends in a separator. `project_dir()` hands back
//! one, so an ordinary run was fine; `--path <dir>` arrives verbatim from the command line,
//! and `<dir>` + `default` is `<dir>default` — a directory that does not exist. The run then
//! reported *"cannot load default library"* and exited 1 against a file that passes with no
//! flag at all, and every caller that met it compensated with a trailing slash, which hides a
//! contract defect behind one caller's habit.
//!
//! `main.rs` had already been converted to `Path::join` for the ordinary run (@P363); these
//! two sites in the test runner were missed, so the fault survived on the `--tests` path only.
//!
//! The second half of the filed report — that `--path` also seeds test DISCOVERY, so the two
//! spellings walk different trees — is measured here and does not hold: `tests_dir` comes from
//! the named target (`resolve_test_target`), never from `--path`. [`the_path_flag_does_not_move_test_discovery`]
//! pins that, so a future change that DOES entangle the two fails with a name attached.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// The repository root — the directory that CONTAINS `default/`, which is what `--path` names.
fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn write_probe(tag: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_1112_{tag}_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{tag}.loft"));
    fa::write(&path, body).expect("write probe");
    path
}

/// Run `--tests <target>`, optionally with `--path <dir>`; return `(output, exit code)`.
fn run_tests(path_flag: Option<&str>, target: &PathBuf) -> (String, Option<i32>) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("--interpret");
    if let Some(p) = path_flag {
        cmd.arg("--path").arg(p);
    }
    cmd.arg("--tests")
        .arg(target)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1");
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code(),
    )
}

const ONE_PASSING_TEST: &str = "fn test_p1112_one() { assert(1 == 1, \"one\"); }\n";

#[test]
fn a_path_without_a_trailing_slash_finds_the_default_library() {
    let target = write_probe("noslash", ONE_PASSING_TEST);
    let root = project_root();
    let no_slash = root.to_string_lossy().trim_end_matches('/').to_string();

    let (out, code) = run_tests(Some(&no_slash), &target);
    assert!(
        !out.contains("cannot load default library"),
        "`--path {no_slash}` (no trailing slash) must locate `default/`; got:\n{out}"
    );
    assert_eq!(code, Some(0), "run should succeed; got:\n{out}");
    assert!(
        out.contains("1 passed"),
        "the probe's one test should run; got:\n{out}"
    );
}

#[test]
fn both_spellings_of_the_same_directory_agree() {
    let target = write_probe("bothspellings", ONE_PASSING_TEST);
    let root = project_root();
    let no_slash = root.to_string_lossy().trim_end_matches('/').to_string();
    let with_slash = format!("{no_slash}/");

    let (out_bare, code_bare) = run_tests(Some(&no_slash), &target);
    let (out_slash, code_slash) = run_tests(Some(&with_slash), &target);
    let (out_none, code_none) = run_tests(None, &target);

    assert_eq!(
        code_bare, code_slash,
        "the two spellings of one directory must agree.\nbare:\n{out_bare}\nslash:\n{out_slash}"
    );
    assert_eq!(
        code_bare, code_none,
        "and both must agree with the no-flag run.\nbare:\n{out_bare}\nnone:\n{out_none}"
    );
    for (label, out) in [
        ("bare", &out_bare),
        ("slash", &out_slash),
        ("none", &out_none),
    ] {
        assert!(
            out.contains("1 passed"),
            "the {label} run should report the same one test; got:\n{out}"
        );
    }
}

/// `--path` names the directory holding `default/`; it does not choose what to discover.
///
/// The filed report read a 3251-file walk as the trailing slash seeding discovery. What
/// selects the tree is the TARGET: a directory argument walks it recursively, a file argument
/// runs exactly that file — and `--path` moves neither.
#[test]
fn the_path_flag_does_not_move_test_discovery() {
    let target = write_probe("discovery", ONE_PASSING_TEST);
    let dir = target.parent().expect("probe parent").to_path_buf();
    // A second file in a SUBDIRECTORY, so a recursive walk is distinguishable from a flat one.
    let sub = dir.join("nested");
    fa::create_dir_all(&sub).expect("nested dir");
    fa::write(
        sub.join("t2.loft"),
        "fn test_p1112_two() { assert(2 == 2, \"two\"); }\n",
    )
    .expect("write nested probe");

    let root = project_root();
    let no_slash = root.to_string_lossy().trim_end_matches('/').to_string();
    let with_slash = format!("{no_slash}/");

    // Directory target: both spellings walk the same tree.
    let (dir_bare, _) = run_tests(Some(&no_slash), &dir);
    let (dir_slash, _) = run_tests(Some(&with_slash), &dir);
    assert!(
        dir_bare.contains("2 files") && dir_slash.contains("2 files"),
        "a directory target discovers its tree either way.\nbare:\n{dir_bare}\nslash:\n{dir_slash}"
    );

    // File target: both spellings run exactly the named file, and nothing else.
    let (file_bare, _) = run_tests(Some(&no_slash), &target);
    let (file_slash, _) = run_tests(Some(&with_slash), &target);
    assert!(
        file_bare.contains("1 file") && file_slash.contains("1 file"),
        "a named file is authoritative either way.\nbare:\n{file_bare}\nslash:\n{file_slash}"
    );
}
