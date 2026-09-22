// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1026 — a generic whose return is a discharged `T?` was lowered by a different route
//! than its non-generic twin, and the route had two faults. Both need a store-backed `T`,
//! so `T = text` is the subject and `T = integer` the control.
//!
//! Three oracles, because the two faults leave different traces and one of them leaves
//! none at all on an ordinary run:
//!
//!   - **Static** ([`a_discharged_generic_return_delivers_through_the_caller_buffer`]).
//!     The monomorph's SIGNATURE must carry the hidden `&text` buffer its non-generic twin
//!     gets (`-> text["___acc_1"]`), on BOTH backends. Deterministic, and it is the fact
//!     the other two only observe the consequences of.
//!   - **Arena poison** ([`a_poisoned_arena_survives_the_discharge`]). `LOFT_POISON=1`
//!     fills each freshly reserved frame with `0xDEADBEEF`, so the `OpAppendText` that read
//!     a value nobody pushed becomes a SIGSEGV instead of a plausible stale `Str`. This is
//!     the fault the issue was filed on, and it is INVISIBLE without the flag.
//!   - **Text timeline** ([`the_discharge_orphans_no_text_buffer`]). `LOFT_TEXT_TIMELINE=1`
//!     ledgers every stack-frame `String` buffer, so the orphaned owned-text return
//!     (loft#568 class) is counted. The suite's own leak gate is store-only and cannot see
//!     a `String`, which is why this one is spelled out here rather than left to the corpus.
//!     It drives the standalone probe rather than the script: the summary hangs off
//!     `check_store_leaks`, which the `--tests` runner does not call. So the probe carries
//!     the LOOP — nine calls orphaned nine buffers, and the COUNT is what fails, not the
//!     eight bytes.
//!
//! [`the_harness_can_fail`] is the control for the harness itself: a program that really is
//! broken must be reported as broken, so a green run above means something.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/1026-generic-discharged-null-return.loft")
}

/// The smallest program with both faults: an earlier `T`-typed parameter, a `T?` parameter,
/// and the discharge as the return. Deliberately ONE generic — the script corpus has many
/// shapes, and a static assertion satisfied by a neighbouring monomorph would mask a
/// regression in the one under test.
const MINIMAL: &str = "pub fn g1026m<T>(x: T, a: T?) -> T { _ = x; a? }\n\
fn main() {\n\
\x20 n = 0;\n\
\x20 for i in 0..8 { _ = g1026m(\"q\", \"zz\"); n += i; }\n\
\x20 r = g1026m(\"q\", \"zz\");\n\
\x20 print(\"len={len(r)} r={r} n={n}\\n\");\n\
}\n";

fn write_minimal(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft1026-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("minimal.loft");
    std::fs::write(&file, MINIMAL).expect("write probe");
    file
}

/// Run `loft` with `args` plus extra env; return `(exit-ok, stdout, stderr)`.
fn run(args: &[&str], env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = Command::new(loft_bin());
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The static oracle. Before the fix both lines read a bare `-> text` and the callee handed
/// out an owned copy; after it, the monomorph takes the caller's buffer exactly as the
/// hand-written `fn g(x: text, a: text?) -> text { a? }` does.
#[test]
fn a_discharged_generic_return_delivers_through_the_caller_buffer() {
    let probe = write_minimal("sig");
    let path = probe.to_string_lossy().into_owned();
    let (_ok, stdout, stderr) = run(
        &[
            "introspect",
            "--all-fns",
            "--fn",
            "i_4text_n_g1026m",
            "--bytecode",
            &path,
        ],
        &[],
    );
    let sigs: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains("i_4text_n_g1026m("))
        .collect();
    assert!(
        sigs.len() >= 2,
        "introspect must print the monomorph's IR signature AND its generated Rust\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        sigs.iter().any(|s| s.contains("___acc_1:&text")),
        "the interpreter monomorph must take the hidden `&text` caller buffer, got:\n{sigs:#?}"
    );
    assert!(
        sigs.iter().any(|s| s.contains("var____acc_1: &mut String")),
        "the native monomorph must take the same buffer — a fix on one backend only is not \
         a fix, got:\n{sigs:#?}"
    );
    assert!(
        sigs.iter().all(|s| !s.trim_end().ends_with("-> text {")),
        "no monomorph may keep the owned-by-value `-> text` return, got:\n{sigs:#?}"
    );
}

/// The fault the issue was filed on. `--interpret` only: `LOFT_POISON` instruments the
/// interpreter's own frames, and the native run never enters them.
#[test]
fn a_poisoned_arena_survives_the_discharge() {
    let probe = write_minimal("poison");
    let path = probe.to_string_lossy().into_owned();
    let (ok, stdout, stderr) = run(&["--interpret", &path], &[("LOFT_POISON", "1")]);
    assert!(
        ok && stdout.contains("len=2 r=zz n=28"),
        "a discharged `T?` return must read its own text under a poisoned arena\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let (ok, stdout, stderr) = run(
        &["--interpret", "--tests", &script().to_string_lossy()],
        &[("LOFT_POISON", "1")],
    );
    assert!(
        ok && stdout.contains("test result: ok"),
        "the whole 1026 corpus must run under a poisoned arena\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// The leak half. The buffer is 8 bytes here and unbounded in a loop, so the count is what
/// this asserts, not the size.
#[test]
fn the_discharge_orphans_no_text_buffer() {
    let probe = write_minimal("leak");
    let path = probe.to_string_lossy().into_owned();
    let (ok, stdout, stderr) = run(&["--interpret", &path], &[("LOFT_TEXT_TIMELINE", "1")]);
    assert!(
        ok && stdout.contains("n=28"),
        "the probe must run all nine of its calls: {stdout}"
    );
    assert!(
        stderr.contains("NO text leak"),
        "a monomorph's text return must be delivered through the caller buffer, not \
         orphaned — nine calls orphaned nine buffers\nstderr:\n{stderr}"
    );
}

/// Both backends answer the same values. The static oracle above says they take the same
/// ABI; this says they compute the same thing through it.
#[test]
fn both_backends_answer_the_same() {
    for backend in ["--interpret", "--native"] {
        let (ok, stdout, stderr) = run(&[backend, "--tests", &script().to_string_lossy()], &[]);
        assert!(
            ok && stdout.contains("test result: ok"),
            "[{backend}] the 1026 corpus must be green\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

/// The control for the harness: a deliberately false assertion must be REPORTED. Without
/// it, "the corpus is green" could equally mean "the runner never reached it".
#[test]
fn the_harness_can_fail() {
    let dir = std::env::temp_dir().join(format!("loft1026-ctl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("control.loft");
    std::fs::write(
        &file,
        "pub fn gctl<T>(x: T, a: T?) -> T { _ = x; a? }\n\
         fn test_this_must_fail() {\n\
         \x20 assert(gctl(\"q\", \"z\") == \"NOT-z\", \"deliberately false\");\n\
         }\n",
    )
    .expect("write control");
    let (ok, stdout, stderr) = run(&["--interpret", "--tests", &file.to_string_lossy()], &[]);
    assert!(
        !ok || !stdout.contains("test result: ok"),
        "a false assertion on this exact shape must be reported as a failure\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
