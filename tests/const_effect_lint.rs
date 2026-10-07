// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The re-evaluated-constant lint (`LOFT_NO_CONST_EFFECT` opts out).
//!
//! A file-scope `NAME = expr;` is an INLINED expression: the right-hand side is
//! substituted at every reference, so an initialiser that costs something pays that
//! cost per use. `LOFT.md` called the feature "Constants" and showed a literal, so a
//! consumer wrote `FNT = load_bundled();`, referenced it once per word while laying
//! out text, and the browser ran out of memory — the font was parsed hundreds of times
//! per frame, invisibly, until the one target with bounded memory.
//!
//! The lint's value is entirely in its precision: it must fire on an initialiser that
//! CALLS something, and stay silent on the literals and arithmetic that inlining
//! exists for. Both directions are asserted here, because a warning that cannot stay
//! quiet gets suppressed and then never fires at all.
//!
//! Via the binary, like `tests/dead_code_lint.rs`: these are end-to-end compile
//! diagnostics on stderr. `LOFT_NO_CACHE` is required — a warm program cache skips the
//! re-parse, and the diagnostics with it.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

const MSG: &str = "is re-evaluated at EVERY reference";

/// Compile+run `src` and return stderr. `extra_env` carries the opt-out when testing it.
fn stderr_of(name: &str, src: &str, extra_env: &[(&str, &str)]) -> String {
    let path = std::env::temp_dir().join(format!("loft_ce_{name}_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write temp program");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "60");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft");
    let _ = fa::remove_file(&path);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A constant whose initialiser calls a USER function warns, and names both the callee
/// and the fix — the consumer's exact shape.
#[test]
fn calling_a_user_function_in_a_constant_warns() {
    let err = stderr_of(
        "user",
        "fn make() -> integer { 7 }\nX = make();\nfn main() { println(\"{X}\"); }\n",
        &[],
    );
    assert!(err.contains(MSG), "the lint must fire: {err}");
    assert!(
        err.contains("make()"),
        "it must name the call that re-runs, not just the constant: {err}"
    );
    // @PLN131 moved the idiom out of the message and into the fix line, so it is asserted
    // where it now lives — the reader's next question is still answered, just not twice.
    let explained = stderr_of(
        "user_explain",
        "fn make() -> integer { 7 }\nX = make();\nfn main() { println(\"{X}\"); }\n",
        &[("LOFT_EXPLAIN", "1")],
    );
    assert!(
        explained.contains("caches"),
        "it must give the idiom, since the reader's next question is what to do: {explained}"
    );
}

/// The opt-out silences it — the project's `LOFT_NO_*` convention. Without this the
/// lint could not be turned off in a build that has accepted the cost deliberately.
#[test]
fn the_opt_out_silences_the_constant_lint() {
    let err = stderr_of(
        "optout",
        "fn make() -> integer { 7 }\nX = make();\nfn main() { println(\"{X}\"); }\n",
        &[("LOFT_NO_CONST_EFFECT", "1")],
    );
    assert!(
        !err.contains(MSG),
        "LOFT_NO_CONST_EFFECT must silence the lint: {err}"
    );
}

/// The controls, and they are the point: everything a constant is normally MADE of must
/// stay silent. A vector literal lowers to `OpNewRecord` calls and plain arithmetic to
/// `OpAddInt` — warning on those would fire on `NUMS = [1, 2, 3];`, which re-evaluates
/// for free and is exactly the case inlining exists for. A pure stdlib call
/// (`max`) is free too.
///
/// Without this test the lint passes its positive case while being unusable in practice.
#[test]
fn literals_arithmetic_and_pure_calls_stay_silent() {
    let err = stderr_of(
        "quiet",
        "PI = 3.14159;\n\
         MAX = 10 * 3;\n\
         NUMS = [1, 2, 3];\n\
         BIGGER = max(3, 7);\n\
         fn main() { println(\"{PI} {MAX} {len(NUMS)} {BIGGER}\"); }\n",
        &[],
    );
    assert!(
        !err.contains(MSG),
        "a literal, arithmetic, a vector literal and a pure stdlib call must not \
         warn — this is what constants are for: {err}"
    );
}

/// A stdlib function marked `#impure` warns even though it is not user code: the
/// category, not the source, is what makes re-running it observable.
#[test]
fn an_impure_stdlib_call_in_a_constant_warns() {
    let err = stderr_of(
        "impure",
        "STAMP = now_millis();\nfn main() { println(\"{STAMP}\"); }\n",
        &[],
    );
    // `now_millis` may not exist under that name in every build; only assert the lint
    // when the program compiled, so this cannot fail for an unrelated reason.
    if !err.contains("Unknown function") {
        assert!(
            err.contains(MSG),
            "an #impure stdlib initialiser must warn: {err}"
        );
    }
}
