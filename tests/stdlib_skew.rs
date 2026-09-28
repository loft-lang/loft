// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A `default/` that does not match the binary is refused before anything runs.
//!
//! The dispatch table is positional (`fill::OPERATORS`), so a stdlib with one operator
//! declaration more, fewer or elsewhere runs every later body under the wrong opcode and
//! ends in a corrupt reference — measured 2026-09-28 when a binary from one branch ran with
//! the other branch's `default/01_code.loft`.  `stdlib_ops::verify` refuses that at load;
//! this drives the real binary through `--path` at a COPY of the stdlib, so the wiring in
//! `Parser::parse_dir` and the message `main` prints are what is tested, not the comparison
//! alone (that has its own unit tests).  The control cell — the untouched copy runs the
//! program — is what proves the refusal is about the injected declaration and not about
//! `--path` at a copy.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A private copy of `default/` under `target/tmp`, fresh per call.
fn copy_stdlib(tag: &str) -> PathBuf {
    let base = root()
        .join("target")
        .join("tmp")
        .join(format!("stdlib-skew-{tag}-{}", std::process::id()));
    let dst = base.join("default");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&dst).unwrap();
    for entry in fs::read_dir(root().join("default")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "loft") {
            fs::copy(&path, dst.join(path.file_name().unwrap())).unwrap();
        }
    }
    base
}

fn run_with(stdlib_parent: &Path) -> (bool, String, String) {
    let prog = stdlib_parent.join("hello.loft");
    fs::write(&prog, "fn main() {\n  println(\"ran {1 + 1}\");\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--path", stdlib_parent.to_str().unwrap(), "--interpret"])
        .arg(&prog)
        // The stdlib cache is keyed on the path and would serve a bundle saved
        // before the injection below; the parse is what this test drives.
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run loft");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// The `OpGotoWord` declaration and its `#rust` line, as a template for an injected pair.
fn goto_word_pair(code: &str) -> (usize, String) {
    let lines: Vec<&str> = code.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.starts_with("fn OpGotoWord("))
        .expect("OpGotoWord declared");
    (at, format!("{}\n{}", lines[at], lines[at + 1]))
}

#[test]
fn the_untouched_copy_runs_the_program() {
    let base = copy_stdlib("control");
    let (ok, out, err) = run_with(&base);
    assert!(ok, "control run failed:\n{err}");
    assert_eq!(out.trim(), "ran 2");
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn an_operator_declared_mid_table_is_refused_by_slot_and_name() {
    let base = copy_stdlib("shifted");
    let file = base.join("default").join("01_code.loft");
    let code = fs::read_to_string(&file).unwrap();
    let (at, pair) = goto_word_pair(&code);
    let mut lines: Vec<String> = code.lines().map(str::to_string).collect();
    // Right after the OpGotoWord pair: slot 2, where the binary carries OpGotoFalse.
    lines.insert(at + 2, pair.replace("OpGotoWord", "OpSkewProbe"));
    fs::write(&file, lines.join("\n") + "\n").unwrap();
    let (ok, out, err) = run_with(&base);
    assert!(!ok, "a shifted stdlib ran:\n{out}");
    assert!(err.contains("does not match this loft binary"), "{err}");
    assert!(
        err.contains(
            "operator slot 2 declares `OpSkewProbe` where the binary carries `OpGotoFalse`"
        ),
        "{err}"
    );
    assert!(
        !err.contains("was not found under the compiler path"),
        "{err}"
    );
    assert!(out.trim().is_empty(), "nothing may run: {out}");
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn an_operator_past_the_table_names_make_fill() {
    let base = copy_stdlib("appended");
    let file = base.join("default").join("02_files.loft");
    let mut code = fs::read_to_string(&file).unwrap();
    let (_, pair) =
        goto_word_pair(&fs::read_to_string(base.join("default").join("01_code.loft")).unwrap());
    code.push('\n');
    code.push_str(&pair.replace("OpGotoWord", "OpSkewTail"));
    code.push('\n');
    fs::write(&file, code).unwrap();
    let (ok, out, err) = run_with(&base);
    assert!(!ok, "a stdlib with an extra operator ran:\n{out}");
    assert!(
        err.contains("declares `OpSkewTail`, and this binary has no body for it"),
        "{err}"
    );
    assert!(err.contains("make fill"), "{err}");
    let _ = fs::remove_dir_all(&base);
}
