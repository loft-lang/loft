// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The 64 KiB stack frame (`loft::limits::frame_too_large`): a function whose parameters,
//! variables and pending values fit in 64 KiB runs, and one past it is refused with one
//! `error:` naming the function — on both backends, never an internal compiler error and
//! never a wrong answer.  The refusal is met after the parser (the slot layout, the code
//! generator), so it is pinned here through the binary rather than as an `@EXPECT_ERROR`
//! script, which the in-process runners compile without the CLI's panic hook.
//!
//! A 300-element integer tuple takes 2400 bytes: 26 of them fit in a frame, 27 (64 800
//! bytes, plus the return slot and the running total) do not.
use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Output;

const REFUSAL: &str = "`main` needs more than 64 KiB of stack";

fn tuple_type() -> String {
    format!("({})", vec!["integer"; 300].join(", "))
}

fn tuple_literal() -> String {
    let items: Vec<String> = (1..=300).map(|i| i.to_string()).collect();
    format!("({})", items.join(", "))
}

/// `n` locals, each a 300-element tuple `(1, 2, …, 300)`; prints `Σ v_k.(k)` = `Σ (k + 1)`.
fn locals(n: usize) -> String {
    let t = tuple_literal();
    let mut s = String::from("fn main() {\n");
    for k in 0..n {
        s += &format!("  v{k} = {t};\n");
    }
    s += "  t = 0;\n";
    for k in 0..n {
        s += &format!("  t += v{k}.{k};\n");
    }
    s + "  println(\"{t}\");\n}\n"
}

/// `f(t, f(t, … f(t, 0)))` nested `n` deep, each call adding `t.1` = 2: prints `2 n`.  Every
/// outer call holds its tuple argument on the stack while the inner one runs.
fn nested_calls(n: usize) -> String {
    let mut e = String::from("0");
    for _ in 0..n {
        e = format!("f(t, {e})");
    }
    format!(
        "fn f(a: {}, b: integer) -> integer {{ a.1 + b }}\nfn main() {{\n  t = {};\n  println(\"{{{e}}}\");\n}}\n",
        tuple_type(),
        tuple_literal()
    )
}

fn run(tag: &str, src: &str, backend: &str) -> Output {
    let path = std::env::temp_dir().join(format!("loft_limits_{}_{tag}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .args([backend, path.to_str().unwrap()])
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_ERRORS", "compact")
        .output()
        .expect("spawn loft");
    let _ = fa::remove_file(&path);
    out
}

fn assert_prints(tag: &str, src: &str, backend: &str, expected: &str) {
    let out = run(tag, src, backend);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim() == expected,
        "{tag} {backend}: expected `{expected}`, exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn assert_refused(tag: &str, src: &str, backend: &str) {
    let out = run(tag, src, backend);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{tag} {backend}: exit\nstderr: {stderr}"
    );
    assert_eq!(
        stderr.matches(REFUSAL).count(),
        1,
        "{tag} {backend}: one refusal\n{stderr}"
    );
    assert!(
        !stderr.contains("internal compiler error"),
        "{tag} {backend}: {stderr}"
    );
    assert!(out.stdout.is_empty(), "{tag} {backend}: ran anyway");
}

#[test]
fn variables_up_to_64_kib_run_and_past_it_are_refused() {
    // Σ_{k<26} (k + 1) = 351.
    assert_prints("locals26", &locals(26), "--interpret", "351");
    for backend in ["--interpret", "--native"] {
        assert_refused("locals27", &locals(27), backend);
    }
}

#[test]
fn values_pending_in_nested_calls_up_to_64_kib_run_and_past_it_are_refused() {
    assert_prints("calls26", &nested_calls(26), "--interpret", "52");
    for backend in ["--interpret", "--native"] {
        assert_refused("calls27", &nested_calls(27), backend);
        // 40 deep is where the position wrapped and the interpreter printed 54.
        assert_refused("calls40", &nested_calls(40), backend);
    }
}
