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

use loft::file_access as fa;
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
    let _ = fa::remove_dir_all(&base);
    fa::create_dir_all(&dst).unwrap();
    for entry in fa::read_dir(root().join("default")).unwrap() {
        let path = entry.os_spelling();
        if fa::has_extension(&path, "loft") {
            fa::copy(&path, dst.join(fa::file_name(&path).unwrap())).unwrap();
        }
    }
    base
}

fn run_with(stdlib_parent: &Path) -> (bool, String, String) {
    let prog = stdlib_parent.join("hello.loft");
    fa::write(&prog, "fn main() {\n  println(\"ran {1 + 1}\");\n}\n").unwrap();
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

/// The `OpGotoWord` declaration and its `#rust` line, as a template for an injected pair, and
/// the index of the line the pair ENDS on.  Its `#hot` marker is left out: a hot operator is
/// numbered among the hot slots, which come first, so a hot probe would land inside that block
/// instead of where the test puts it.
fn goto_word_pair(code: &str) -> (usize, String) {
    let lines: Vec<&str> = code.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.starts_with("fn OpGotoWord("))
        .expect("OpGotoWord declared");
    let body = (at + 1..lines.len())
        .find(|&i| lines[i].starts_with("#rust"))
        .expect("OpGotoWord's #rust line");
    (body, format!("{}\n{}", lines[at], lines[body]))
}

#[test]
fn the_untouched_copy_runs_the_program() {
    let base = copy_stdlib("control");
    let (ok, out, err) = run_with(&base);
    assert!(ok, "control run failed:\n{err}");
    assert_eq!(out.trim(), "ran 2");
    let _ = fa::remove_dir_all(&base);
}

#[test]
fn an_operator_declared_mid_table_is_refused_by_slot_and_name() {
    let base = copy_stdlib("shifted");
    let file = base.join("default").join("01_code.loft");
    let code = fa::read_to_string(&file).unwrap();
    let (at, pair) = goto_word_pair(&code);
    let mut lines: Vec<String> = code.lines().map(str::to_string).collect();
    // Right after the OpGotoWord pair.  The probe is an ordinary (not `#hot`) operator, and
    // those are numbered in declaration order after the hot block, so it takes the slot after
    // the last ordinary operator declared above it — `OpGoto` — and displaces whatever the
    // binary's table carries there.  Both are read from that table, so a renumbered table
    // moves the expectation with it.
    lines.insert(at + 1, pair.replace("OpGotoWord", "OpSkewProbe"));
    fa::write(&file, lines.join("\n") + "\n").unwrap();
    let names = loft::fill::OPERATOR_NAMES;
    let slot = names
        .iter()
        .position(|n| *n == "OpGoto")
        .expect("OpGoto in the operator table")
        + 1;
    let (ok, out, err) = run_with(&base);
    assert!(!ok, "a shifted stdlib ran:\n{out}");
    assert!(err.contains("does not match this loft binary"), "{err}");
    let want = format!(
        "operator slot {slot} declares `OpSkewProbe` where the binary carries `{}`",
        names[slot]
    );
    assert!(err.contains(&want), "expected `{want}` in:\n{err}");
    assert!(
        !err.contains("was not found under the compiler path"),
        "{err}"
    );
    assert!(out.trim().is_empty(), "nothing may run: {out}");
    let _ = fa::remove_dir_all(&base);
}

#[test]
fn an_operator_past_the_table_names_make_fill() {
    let base = copy_stdlib("appended");
    // The LAST file the stdlib loads (they load in name order), so the probe is declared after
    // every operator the table holds — any earlier file is followed by later declarations.
    let mut files: Vec<PathBuf> = fa::read_dir(base.join("default"))
        .unwrap()
        .into_iter()
        .map(|e| e.os_spelling())
        .filter(|p| fa::has_extension(p, "loft"))
        .collect();
    files.sort();
    let file = files.pop().expect("a stdlib file");
    let mut code = fa::read_to_string(&file).unwrap();
    let (_, pair) =
        goto_word_pair(&fa::read_to_string(base.join("default").join("01_code.loft")).unwrap());
    code.push('\n');
    // `#cold`: operators are numbered hot, then ordinary, then cold, so only a cold one
    // declared last lies past the whole table — an ordinary one would take the first cold
    // slot and be read as a mismatch there instead.
    let (decl, body) = pair.split_once('\n').expect("declaration and body");
    code.push_str(&format!(
        "{}\n#cold\n{body}",
        decl.replace("OpGotoWord", "OpSkewTail")
    ));
    code.push('\n');
    fa::write(&file, code).unwrap();
    let (ok, out, err) = run_with(&base);
    assert!(!ok, "a stdlib with an extra operator ran:\n{out}");
    assert!(
        err.contains("declares `OpSkewTail`, and this binary has no body for it"),
        "{err}"
    );
    assert!(err.contains("make fill"), "{err}");
    let _ = fa::remove_dir_all(&base);
}
