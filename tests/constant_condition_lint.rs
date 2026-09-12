// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `warning[constant-condition]` — and mostly the QUIET half, which is where the design lives.
//!
//! `LOFT.md` § Conversions is the language's rule: *"`false` and null are falsy; integer
//! `i32::MIN` is falsy; every other value is truthy"*.  A HEAP value is therefore falsy exactly
//! when it is absent, so a NON-optional one in a condition cannot fail the test.  The behaviour
//! is documented and is not changing; the silence at the call site was the defect.
//!
//! ⚠ **A SCALAR is not that case, and this file exists mostly to say so.**  A scalar's absent
//! value is IN-BAND and reachable from a non-optional declaration, so `if d` on a plain
//! `integer` is a genuine two-state test — an `integer` holding `i64::MIN` takes the ELSE
//! branch.  The first draft of this lint reported it, on the strength of `if 0` taking the THEN
//! branch, which is true and proves nothing because `0` is not the sentinel.  That draft would
//! have made a false claim about every integer condition in the language.
//!
//! The corpus guard `1003b-a-constant-condition-is-reported.loft` cannot score this half:
//! `@EXPECT_WARNING` pins are "at least these fired", so an OVER-WIDE lint still passes it —
//! measured, by admitting `Type::Integer` and watching it stay green.  Absence needs a channel
//! that reads the diagnostic stream, which is this file.
//!
//! Binary-invoked like `tests/group_apart_lint.rs`; `LOFT_NO_CACHE` because the warm program
//! cache skips the re-parse that produces these.

use std::path::PathBuf;
use std::process::Command;

const CODE: &str = "warning[constant-condition]";

fn diagnostics_of(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join("loft_constant_condition_lint");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{name}.loft"));
    std::fs::write(&path, src).expect("write probe");
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn it_fires_on_a_non_optional_heap_condition() {
    // The control for every silent case below: without one that fires, they are all vacuous.
    for (tag, decl) in [
        ("vector", "v: vector<integer> = [1, 2];"),
        ("empty_vector", "v: vector<integer> = [];"),
        ("text", "v: text = \"hi\";"),
    ] {
        let err = diagnostics_of(
            tag,
            &format!("fn main() {{ {decl} if v {{ print(\"x\"); }} }}\n"),
        );
        assert!(
            err.contains(CODE),
            "a non-optional heap value in a condition must report ({tag}); got: {err}"
        );
    }
}

#[test]
fn it_is_silent_on_a_scalar_whose_sentinel_is_in_band() {
    // The load-bearing cell.  `i64::MIN` is a reachable value of a plain `integer` and is
    // falsy, so this condition really can take its else branch.
    for (tag, decl) in [
        ("integer", "d: integer = 0;"),
        ("sentinel", "d: integer = 0 - 9223372036854775807 - 1;"),
        ("float", "d: float = 0.0;"),
        ("character", "d: character = 'a';"),
    ] {
        let err = diagnostics_of(
            tag,
            &format!("fn main() {{ {decl} if d {{ print(\"x\"); }} }}\n"),
        );
        assert!(
            !err.contains(CODE),
            "a scalar condition is a real two-state test and must stay quiet ({tag}); got: {err}"
        );
    }
}

#[test]
fn it_is_silent_on_a_nullable_subject() {
    // `(E-Truthy)` licenses exactly this: an absent value reads as false, so it is a test.
    let err = diagnostics_of(
        "nullable",
        "fn main() { v: vector<integer>? = null; if v { print(\"x\"); } }\n",
    );
    assert!(
        !err.contains(CODE),
        "a nullable subject is a presence test and must stay quiet; got: {err}"
    );
}

#[test]
fn it_is_silent_on_a_boolean_and_on_a_generic() {
    // A boolean has two states of its own.  A comparison inside a GENERIC types as the bound
    // type variable rather than `boolean` at parse time — the first draft reported 32 of those
    // from the stdlib on an EMPTY program, because a type variable is a `Reference` to its def
    // and there is no variant to exclude by name.  Hence the allow-list.
    let err = diagnostics_of(
        "boolean",
        "fn main() { b: boolean = true; if b { print(\"x\"); } }\n",
    );
    assert!(
        !err.contains(CODE),
        "a boolean condition is a test; got: {err}"
    );

    let err = diagnostics_of(
        "generic",
        "fn big<T: Comparable>(a: T, b: T) -> T { if a < b { return b; } a }\n\
         fn main() { print(\"{big(1, 2)}\"); }\n",
    );
    assert!(
        !err.contains(CODE),
        "a comparison in a generic is not a constant condition; got: {err}"
    );
}

#[test]
fn an_empty_program_is_quiet() {
    // The stdlib is parsed for every program, so a lint that fires there taxes every run.
    let err = diagnostics_of("empty", "fn main() { }\n");
    assert!(
        !err.contains(CODE),
        "parsing the stdlib must not report; got: {err}"
    );
}

#[test]
fn the_opt_out_silences_it() {
    let dir = std::env::temp_dir().join("loft_constant_condition_lint");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let path = dir.join("optout.loft");
    std::fs::write(
        &path,
        "fn main() { v: vector<integer> = [1]; if v { print(\"x\"); } }\n",
    )
    .expect("write probe");
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_CONSTANT_CONDITION", "1")
        .output()
        .expect("spawn loft");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.contains(CODE),
        "LOFT_NO_CONSTANT_CONDITION must silence it; got: {err}"
    );
}
