// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A crate with no `main` is compile-CHECKED, not linked (loft#1171).
//!
//! `--native` synthesises a `main` for a test-only file by calling its zero-parameter
//! functions. A LIBRARY has none — every `pub fn` takes arguments — so nothing was emitted
//! and rustc refused the crate with `E0601`, naming a temp file in `~/.cache` the user never
//! wrote and suggesting they add `main` to it. loft printed "codegen bug" over the top, which
//! points at the compiler for what is a correct library.
//!
//! The two backends also disagreed: `--interpret` on the same file runs nothing and exits 0.
//!
//! The rule: a crate with no entry point has no program to link, so it is compiled as a
//! library and reported as compiling cleanly. That makes `loft build` / `loft check` on a
//! library-only package a pass, which is what those commands mean for a library.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn tmp_dir(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("loft_1171_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    fa::create_dir_all(&root).expect("mkdir");
    root
}

/// The whole defect in one file: a library compiles, and says so, instead of failing.
#[test]
fn a_file_with_no_main_compiles_and_reports_nothing_to_run() {
    let dir = tmp_dir("nomain");
    let file = dir.join("greeter.loft");
    fa::write(
        &file,
        "pub fn greet(who: text) -> text { \"hello, {who}!\" }\n",
    )
    .expect("write");

    let out = Command::new(loft_bin())
        .args(["--native", file.to_str().unwrap()])
        .output()
        .expect("run loft --native");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        out.status.success(),
        "a library must compile cleanly, not fail:\n{combined}"
    );
    assert!(
        combined.contains("defines no `main`"),
        "the message must name the reason there is nothing to run:\n{combined}"
    );
    // The failure this replaces, in the words a user saw. Asserted by ABSENCE because both
    // were rustc's and loft's, and either one reappearing is the regression.
    assert!(
        !combined.contains("E0601"),
        "a raw rustc error must not reach the user:\n{combined}"
    );
    assert!(
        !combined.contains("codegen bug"),
        "a correct library must not be reported as a compiler bug:\n{combined}"
    );
    let _ = fa::remove_dir_all(&dir);
}

/// The control: a program WITH `main` still links and runs. A fix that stopped linking
/// everything would pass the test above and break every real program.
#[test]
fn a_file_with_main_still_links_and_runs() {
    let dir = tmp_dir("withmain");
    let file = dir.join("hello.loft");
    fa::write(&file, "fn main() {\n  println(\"ran\");\n}\n").expect("write");

    let out = Command::new(loft_bin())
        .args(["--native", file.to_str().unwrap()])
        .output()
        .expect("run loft --native");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "a program must still run:\n{combined}"
    );
    assert!(
        combined.contains("ran"),
        "the program's own output must appear:\n{combined}"
    );
    let _ = fa::remove_dir_all(&dir);
}
