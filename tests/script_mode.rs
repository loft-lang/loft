// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN13 — beginner scripts, end to end. A script (loose top-level statements, no
//! `fn main`) runs on BOTH backends — under `--script` (step 2) and AUTO-DETECTED with no
//! flag (step 3) — with the statements executed once in order sharing state and a
//! top-level def hoisted. `;` is optional between the top-level statements (step 4). The
//! desugar's unit-level shape + the 0-corpus-classification invariant are covered in
//! `loft::script::tests`.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run `loft <args> <file>` and return (stdout, success).
fn run(args: &[&str], file: &Path) -> (String, bool) {
    let out = loft::platform::process::harness_command(loft_bin())
        .args(args)
        .arg(file)
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

#[test]
fn script_runs_without_fn_main_under_flag() {
    let fixture = workspace_root().join("tests/data/script_hello.loft");
    // double(1)+double(2)+double(3) = 2+4+6 = 12 — proves ordering + shared state + a
    // hoisted top-level def, all inside the synthesised run-once `main`.
    let (out, ok) = run(&["--script", "--interpret"], &fixture);
    assert!(ok && out.contains("count=12"), "interp --script: {out:?}");
    // cross-backend: native must produce the byte-identical result.
    let (nout, nok) = run(&["--script", "--native"], &fixture);
    assert!(
        nok && nout.contains("count=12"),
        "native --script: {nout:?}"
    );
}

#[test]
fn script_auto_detected_without_flag() {
    let fixture = workspace_root().join("tests/data/script_hello.loft");
    // Step 3 — no flag needed: a loose-top-level-statement source (which loft rejected
    // before) is auto-detected as a script and desugared, producing the same result as
    // the explicit `--script` run above.
    let (out, ok) = run(&["--interpret"], &fixture);
    assert!(
        ok && out.contains("count=12"),
        "auto-detect (no flag): {out:?}"
    );
}

#[test]
fn semicolon_less_script_runs_on_both_backends() {
    let fixture = workspace_root().join("tests/data/script_semicolon_less.loft");
    // Step 4 — the top-level statements omit their `;`: 0 + triple(2) + triple(3)
    // = 0 + 6 + 9 = 15, proving the newline split + terminator insertion + shared state.
    let (out, ok) = run(&["--interpret"], &fixture);
    assert!(ok && out.contains("total=15"), "interp ;-less: {out:?}");
    let (nout, nok) = run(&["--native"], &fixture);
    assert!(nok && nout.contains("total=15"), "native ;-less: {nout:?}");
}

#[test]
fn if_else_is_one_statement_at_script_scope() {
    let fixture = workspace_root().join("tests/data/script_if_else.loft");
    // loft#736 — the splitter ended a top-level item at the `}` that closed it back to
    // depth 0, so `else { … }` became its own item and parsed as a bare `else`.  The
    // fixture spells out one expected letter per layout, so a boundary that splits (or
    // one that over-merges two adjacent `if`s) changes the string rather than the exit
    // code: A=same-line, b=own-line else, C=else after a comment, d=`else if` chain,
    // 7=the expression form, f=nested, HI=two `if`s that must stay separate, small=a
    // hoisted def called from an `elsewhere` identifier (which must not read as `else`).
    let (out, ok) = run(&["--interpret"], &fixture);
    assert!(
        ok && out.contains("seen=AbCd7fHIsmall"),
        "interp if/else script: {out:?}"
    );
    let (nout, nok) = run(&["--native"], &fixture);
    assert!(
        nok && nout.contains("seen=AbCd7fHIsmall"),
        "native if/else script: {nout:?}"
    );
}

// ── T0.2 — a script's diagnostics use the USER's line numbers ────────────────
//
// The desugar hoists defs and inserts lines (the `fn main() {` prologue, a
// fresh-line `;` after each `;`-less statement), so a diagnostic carried
// GENERATED coordinates: a 2-line script reported its second statement on line
// 4.  That also silently dropped the source snippet, because the renderer looked
// up a line the user's file does not have — one cause, two symptoms, so the
// snippet returns with the line fix.

/// Run `loft <args> <file>` and return stderr (where diagnostics go).
fn run_stderr(args: &[&str], file: &Path) -> String {
    let out = loft::platform::process::harness_command(loft_bin())
        .args(args)
        .arg(file)
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn t02_script_diagnostic_uses_source_line_and_shows_snippet() {
    let dir = std::env::temp_dir();
    let file = dir.join(format!("loft_t02_{}.loft", std::process::id()));
    // The review's repro verbatim: the misspelled call is on line 2 of 2.
    fa::write(&file, "name = \"world\"\nprintt(\"Hello, {name}!\\n\")\n").expect("write");
    for backend in ["--interpret", "--native"] {
        let err = run_stderr(&[backend], &file);
        assert!(
            err.contains("Unknown function printt"),
            "{backend}: expected the unknown-function error; got {err:?}"
        );
        assert!(
            err.contains(".loft:2:"),
            "{backend}: the error must be reported on SOURCE line 2, not the \
             generated line; got {err:?}"
        );
        // The snippet is the second symptom of the same cause: it renders only
        // when the reported line actually exists in the user's file.
        assert!(
            err.contains("printt(\"Hello, {name}!"),
            "{backend}: the source snippet must render; got {err:?}"
        );
    }
    let _ = fa::remove_file(&file);
}

/// A one-line script reports line 1 — the prologue must not shift it.
#[test]
fn t02_one_line_script_reports_line_one() {
    let dir = std::env::temp_dir();
    let file = dir.join(format!("loft_t02_one_{}.loft", std::process::id()));
    fa::write(&file, "printt(\"hi\")\n").expect("write");
    let err = run_stderr(&["--interpret"], &file);
    assert!(
        err.contains(".loft:1:"),
        "a one-line script must report line 1; got {err:?}"
    );
    let _ = fa::remove_file(&file);
}

// ── First-use: a mistyped path is one of the commonest first actions ─────────

/// A typo'd filename suggests the nearest sibling instead of dead-ending.
/// The old text was `Unknown file:<path>` — no space, no suggestion — which made
/// a one-character slip read like a broken install.
#[test]
fn mistyped_file_path_suggests_the_neighbour() {
    let dir = std::env::temp_dir().join(format!("loft_fu_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("mkdir");
    fa::write(dir.join("hello.loft"), "println(\"hi\")\n").expect("write");

    let out = loft::platform::process::harness_command(loft_bin())
        .arg(dir.join("helo.loft"))
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("no such file:"),
        "expected the plain no-such-file wording; got {err:?}"
    );
    assert!(
        err.contains("did you mean 'hello.loft'?"),
        "expected a sibling suggestion; got {err:?}"
    );

    // A name with no near neighbour must NOT invent a suggestion.
    let out2 = loft::platform::process::harness_command(loft_bin())
        .arg(dir.join("zzzzzzzz.loft"))
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft");
    let err2 = String::from_utf8_lossy(&out2.stderr);
    assert!(err2.contains("no such file:"), "got {err2:?}");
    assert!(
        !err2.contains("did you mean"),
        "an unrelated name must not get a suggestion; got {err2:?}"
    );
    let _ = fa::remove_dir_all(&dir);
}

/// A program that is NOT a script must not be desugared as one — and an escaped quote
/// inside a format hole used to make it look like one (loft#1271).
///
/// `"got: {shout("a\"b")}"` is one literal: `{` opens a hole, the `"` inside it opens a
/// NESTED string, and only the `"` at hole depth 0 closes it. `split_top_level` read it
/// flat, ended the item after `shout(`, and the `fn main` that FOLLOWED no longer STARTED
/// an item — so `is_script` stopped seeing it and desugared an ordinary program. The
/// symptom was a LEXER error, `fatal: String not correctly terminated`, on a line the
/// compiler accepts one function up.
///
/// This is the end-to-end half, and it needs a PLAIN run: `loft --tests` does not consult
/// the classifier at all, so the same source passes under the test harness on a build where
/// this fails. The unit-level half is `loft::script::tests`.
///
/// Order is the whole trick and is why the fixture puts the escaped quote in the function
/// IMMEDIATELY before `fn main`: with another def between them the mis-split resynchronises
/// and the bug hides, and with `fn main` first it never fires.
#[test]
fn an_escaped_quote_in_a_hole_does_not_make_a_program_a_script() {
    let dir = std::env::temp_dir().join(format!("loft_1271_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("mkdir");
    let file = dir.join("p.loft");
    fa::write(
        &file,
        "fn shout(v: text) -> text { \"{v}!\" }\n\
         fn probe() -> text { \"got: {shout(\"a\\\"b\")}\" }\n\
         fn main() { print(\"{probe()}\\n\"); }\n",
    )
    .expect("write");

    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(loft_bin())
            .args([backend, file.to_str().unwrap()])
            .env("LOFT_TIMEOUT", "120")
            .current_dir(workspace_root())
            .output()
            .expect("invoke loft");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success(),
            "[{backend}] a program with an escaped quote in a hole must run:\n{stderr}"
        );
        assert!(
            stdout.contains("got: a\"b!"),
            "[{backend}] and the escape must survive as one quote: {stdout:?}"
        );
    }
    let _ = fa::remove_dir_all(&dir);
}
