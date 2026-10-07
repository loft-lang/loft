// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#921 — `upper-case-local` speaks about a LOCAL, so a constant must never trip it.
//!
//! A constant used ABOVE its own `const NAME = …` cannot resolve in pass 1, which parks the
//! name as a placeholder variable; pass 2 has the declaration and pastes the value, leaving
//! the placeholder unread in the table the lint walks.  The advice then said the constant was
//! a local variable — the one message whose job is to say a name is *not* a constant — and it
//! fired only when the declaration sat BELOW the use, so the same constant advised or stayed
//! silent depending on where in the file it was declared.
//!
//! The matrix is the subject and its control (declaration order, nothing else) plus the two
//! shapes the advice exists for, which must keep firing: a real UPPER_CASE local, and a name
//! that resolves to no declaration at all.
//!
//! Binary-level because these are compile-time diagnostics on stderr — same approach as
//! `tests/runtime_warnings.rs`.  `LOFT_NO_CACHE` forces a cold parse, without which a warm
//! run skips the pass that emits them.

use loft::file_access as fa;

const ADVICE: &str = "upper-case-local";

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Compile+run `source` under `--interpret`, returning `(stdout, stderr)`.
fn run(name: &str, source: &str) -> (String, String) {
    let script = std::env::temp_dir().join(format!("loft_921_{name}_{}.loft", std::process::id()));
    fa::write(&script, source).expect("write temp script");
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(&script)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "60")
        .output()
        .expect("failed to invoke loft binary");
    let _ = fa::remove_file(&script);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The subject and its control in ONE file: the only difference between `AFTER` and `BEFORE`
/// is which side of its use the declaration sits on, and neither is a local variable.
#[test]
fn a_constant_is_never_a_local_whichever_side_it_is_declared_on() {
    let (stdout, stderr) = run(
        "decl_order",
        "fn after_decl() -> integer { AFTER }\n\
         const AFTER = 13;\n\
         const BEFORE = 13;\n\
         fn before_decl() -> integer { BEFORE }\n\
         fn main() { print(\"{after_decl()}/{before_decl()}\"); }\n",
    );
    assert!(
        !stderr.contains(ADVICE),
        "a constant must not be advised as a local:\n{stderr}"
    );
    // The values are the reason the report is `sev:low` — pin them, so a future fix that
    // silences the advice by breaking the resolution cannot pass this test.
    assert_eq!(stdout.trim(), "13/13", "stderr was:\n{stderr}");
}

/// The shape the lint is written for: a genuine UPPER_CASE local, no `const` keyword.
#[test]
fn a_real_upper_case_local_is_still_advised() {
    let (_, stderr) = run("real_local", "fn main() { FOO = 1; print(\"{FOO}\"); }\n");
    assert!(
        stderr.contains(ADVICE),
        "the lint's own shape stopped firing:\n{stderr}"
    );
}

/// A misspelled constant — no declaration anywhere — is why pass 1 parks the name as a
/// variable at all.  It reports the unknown name AND keeps the advice.
#[test]
fn a_name_that_resolves_to_no_declaration_is_still_advised() {
    let (_, stderr) = run(
        "misspelled",
        "fn use_it() -> integer { MISSPELED }\n\
         fn main() { print(\"{use_it()}\"); }\n",
    );
    assert!(
        stderr.contains("Unknown variable 'MISSPELED'"),
        "the unknown-name error stopped firing:\n{stderr}"
    );
    assert!(
        stderr.contains(ADVICE),
        "the advice on an unresolvable UPPER_CASE name stopped firing:\n{stderr}"
    );
}
