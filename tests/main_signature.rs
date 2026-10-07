// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `main` takes nothing, or the invocation arguments as one `vector<text>` (loft#1172).
//!
//! `State::execute_argv` fills exactly one shape: it pushes a TEXT vector before the return
//! address when `main` declares a single vector parameter, and pushes nothing otherwise.
//! Every other signature was still accepted and never filled — `main(who: text)` read `""`,
//! two integers read whatever the frame happened to hold, and a `text` among two crashed on a
//! corrupt store reference. A `vector` of any other element type is the same fault one step
//! on: a text vector pushed into a slot typed for something else.
//!
//! They are REFUSED rather than filled, because none of them does anything today — there is no
//! argument to lose. The supported shape is the cure the message names.
//!
//! The supported shape also leaked: the argv store is nobody's to free from loft code, so it
//! is released with the entry frame.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn run(src: &str, args: &[&str], strict: bool) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!(
        "loft_1172_{}_{}",
        std::process::id(),
        fastrand_ish(src)
    ));
    fa::create_dir_all(&dir).expect("mkdir");
    let file = dir.join("m.loft");
    fa::write(&file, src).expect("write");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret").arg(&file).args(args);
    if strict {
        cmd.env("LOFT_STRICT_STORES", "1");
    }
    let out = cmd.output().expect("run loft");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&dir);
    (out.status.success(), combined)
}

/// A stable per-source suffix so concurrent cases cannot share a directory.
fn fastrand_ish(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// The supported shape carries the arguments, and does not leak the vector it was given.
#[test]
fn one_vector_of_text_receives_the_arguments_and_frees_them() {
    let src = "fn main(args: vector<text>) {\n  println(\"n={len(args)}\");\n  for a in args { println(\"<{a}>\"); }\n}\n";
    let (ok, out) = run(src, &["Ada", "Grace"], true);
    assert!(ok, "the supported shape must run:\n{out}");
    assert!(out.contains("n=2"), "both arguments arrive:\n{out}");
    assert!(
        out.contains("<Ada>") && out.contains("<Grace>"),
        "in order:\n{out}"
    );
    // The leak this closes. `LOFT_STRICT_STORES=1` fails the run on a live store at exit,
    // so its ABSENCE is the assertion — asserted on the text as well, because a future
    // change to the gate's exit code must not quietly turn this cell into a no-op.
    assert!(
        !out.contains("NEVER FREED"),
        "the argv vector must be freed with the entry frame:\n{out}"
    );
}

/// No parameters is still the ordinary program, and still clean.
#[test]
fn no_parameters_is_unaffected() {
    let (ok, out) = run("fn main() {\n  println(\"ran\");\n}\n", &["ignored"], true);
    assert!(ok, "a plain main must run:\n{out}");
    assert!(out.contains("ran"), "{out}");
    assert!(!out.contains("NEVER FREED"), "and must not leak:\n{out}");
}

/// Every shape that was never filled is refused, and the message names the one that works.
#[test]
fn an_unfillable_signature_is_refused_and_names_the_cure() {
    for sig in [
        "who: text",
        "n: integer",
        "a: integer, b: integer",
        "a: text, b: text",
        "v: vector<integer>",
    ] {
        let src = format!("fn main({sig}) {{\n  println(\"x\");\n}}\n");
        let (ok, out) = run(&src, &["Ada", "3"], false);
        assert!(!ok, "`fn main({sig})` must be refused, not run:\n{out}");
        assert!(
            out.contains("`main` takes no parameters, or one `vector<text>`"),
            "the refusal must name the shape that works, for `{sig}`:\n{out}"
        );
        // The failures this replaces, in the words a user saw.
        assert!(
            !out.contains("Store access out of bounds"),
            "`fn main({sig})` must not reach a corrupt read:\n{out}"
        );
    }
}
