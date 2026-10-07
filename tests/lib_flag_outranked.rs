// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#1352 — `--lib <dir>` is silently dropped when the working directory has a `lib/`
//! that provides the same name: resolution is first-wins and the project-local `lib/` is
//! probed before the flag.  The precedence is reported, not moved.
//!
//! The control is a `--lib` copy that CANNOT run — a line of non-loft appended — so a clean
//! run is positive evidence that the flag was dropped, not absence of evidence.  From a
//! directory without a `lib/` the same flag is honoured and the copy's parse error shows.
//!
//! Hermetic: every cell builds its own tree under the temp dir; `LOFT_NO_CACHE` keeps the
//! whole-program cache from answering for a previous cell.

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

/// A project directory with `lib/who.loft`, a bare directory with the same `main.loft`, and
/// an override directory whose `who.loft` cannot parse.
fn tree(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("loft_1352_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&base);
    let proj = base.join("proj");
    let bare = base.join("bare");
    let over = base.join("override");
    write(
        &proj.join("lib/who.loft"),
        "pub fn who() -> text { \"project lib\" }\n",
    );
    write(
        &over.join("who.loft"),
        "pub fn who() -> text { \"override\" }\nthis line is not loft\n",
    );
    let main = "use who::*;\nfn main() { println(\"{who()}\"); }\n";
    write(&proj.join("main.loft"), main);
    write(&bare.join("main.loft"), main);
    (proj, bare, over)
}

fn run(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .current_dir(cwd);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// From the project directory the flag loses to `lib/` — the corrupted copy never parses,
/// the project's answer is printed — and the advice says so, naming both files.
#[test]
fn a_dropped_lib_flag_is_reported() {
    let (proj, _, over) = tree("reported");
    let over_s = over.to_string_lossy().into_owned();
    let (code, stdout, stderr) = run(&proj, &["--interpret", "--lib", &over_s, "main.loft"], &[]);
    assert_eq!(code, 0, "the project's own lib must run\nstderr:\n{stderr}");
    assert!(
        stdout.contains("project lib"),
        "the cwd lib/ answered: {stdout}"
    );
    assert!(
        stderr.contains("advice[lib-flag-outranked]") && stderr.contains("who.loft"),
        "the dropped flag must be reported, naming the file it provides\nstderr:\n{stderr}"
    );
}

/// From a directory without a `lib/` the flag is honoured: the corrupted copy's parse error
/// is the proof, and nothing is reported.
#[test]
fn an_honoured_lib_flag_is_silent_and_its_copy_runs() {
    let (_, bare, over) = tree("honoured");
    let over_s = over.to_string_lossy().into_owned();
    let (code, _, stderr) = run(&bare, &["--interpret", "--lib", &over_s, "main.loft"], &[]);
    assert_ne!(
        code, 0,
        "the corrupted copy must refuse to parse\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("this line is not loft") || stderr.contains("Expect token"),
        "the copy behind the flag is what parsed\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("lib-flag-outranked"),
        "nothing was outranked\nstderr:\n{stderr}"
    );
}

/// No flag, no report; and the off switch silences the report without changing the run.
#[test]
fn no_flag_and_the_off_switch_are_quiet() {
    let (proj, _, over) = tree("quiet");
    let over_s = over.to_string_lossy().into_owned();
    let (code, stdout, stderr) = run(&proj, &["--interpret", "main.loft"], &[]);
    assert!(
        code == 0 && stdout.contains("project lib"),
        "stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("lib-flag-outranked"),
        "no flag was given\nstderr:\n{stderr}"
    );
    let (code, stdout, stderr) = run(
        &proj,
        &["--interpret", "--lib", &over_s, "main.loft"],
        &[("LOFT_NO_LIB_OUTRANKED", "1")],
    );
    assert!(
        code == 0 && stdout.contains("project lib"),
        "stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("lib-flag-outranked"),
        "the off switch must silence it\nstderr:\n{stderr}"
    );
}
