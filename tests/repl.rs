// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 04 — interactive REPL shell, end-to-end.
//!
//! Drives the `loft` binary with piped stdin and asserts on stdout (where
//! evaluated results print; the prompt + errors go to stderr, discarded here).

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `loft <args>` feeding `input` on stdin, return captured stdout.
fn repl(args: &[&str], input: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn loft");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Acceptance #1 — `loft repl` evaluates input and prints results; a binding
/// persists so a later read shows it.
#[test]
fn basic_session_prints_result() {
    let out = repl(&["repl"], "x = 40 + 2\nx\n:quit\n");
    assert!(out.contains("42"), "expected 42 in stdout, got {out:?}");
}

/// A bare `loft` (no args) drops into the REPL.
#[test]
fn bare_loft_starts_repl() {
    let out = repl(&[], "1 + 2\n:quit\n");
    assert!(out.contains('3'), "expected 3 in stdout, got {out:?}");
}

/// Acceptance #2 — multi-line input (a function spanning lines) works, and the
/// function is callable afterward.
#[test]
fn multi_line_fn_then_call() {
    let out = repl(
        &["repl"],
        "fn dbl(n: integer) -> integer {\nn + n\n}\ndbl(21)\n:quit\n",
    );
    assert!(out.contains("42"), "expected 42 in stdout, got {out:?}");
}

/// A session may hold MORE THAN ONE lambda.
///
/// Every input minted its lambdas from zero while the previous input's definitions were still
/// in the session, so the second input carrying one died on *"Cannot redefine
/// `__lambda_0`"* — one lambda per session, on the released build too.  The counter continues
/// across inputs; only the two PASSES of one input share a start, which is what makes both
/// mint the same names.
#[test]
fn a_session_holds_more_than_one_lambda() {
    let out = repl(
        &["repl"],
        "[1, 2].map(|n| { n * 3 })\n[3, 1].filter(|n| { n > 1 })\n[1, 2, 3].reduce(0, |a, b| { a + b })\n:quit\n",
    );
    assert!(
        out.contains("[3,6]") && out.contains("[3]") && out.contains('6'),
        "expected all three lambda inputs to answer, got {out:?}"
    );
    assert!(
        !out.contains("__lambda_"),
        "no lambda name should reach the reader, got {out:?}"
    );
}

/// Acceptance #3 — a parse error doesn't crash; the session keeps working.
#[test]
fn recovers_from_parse_error() {
    let out = repl(&["repl"], "x = 1 2 3\nx = 7\nx\n:quit\n");
    assert!(
        out.contains('7'),
        "session should continue after error: {out:?}"
    );
}

/// Struct results print in loft's native rendering.
#[test]
fn struct_result_echo() {
    let out = repl(
        &["repl"],
        "struct P { a: integer, b: integer }\nP { a: 1, b: 2 }\n:quit\n",
    );
    assert!(
        out.contains("a:1") && out.contains("b:2"),
        "struct echo: {out:?}"
    );
}

/// @PLN16 G1 — interactive breakpoint at the REPL, end to end through the binary:
/// `:break` a function, the call suspends into the paused sub-mode, edit a local,
/// `:continue`, and the resumed call uses the edited value.  `calc(5)` is normally
/// 50; editing `n` to 99 makes the resumed call print 990.
#[test]
fn interactive_breakpoint_edit_continue() {
    let out = repl(
        &["repl"],
        "fn calc(n: integer) -> integer {\n  n * 10\n}\n:break calc\ncalc(5)\nn = 99\n:continue\n:quit\n",
    );
    assert!(
        out.contains("990"),
        "edited n=99 → calc prints 990: {out:?}"
    );
    assert!(
        !out.contains("50"),
        "the un-edited result 50 must not print: {out:?}"
    );
}

/// @PLN16 G1 — REPL-at-frame through the binary: at a pause, type an expression
/// and it is evaluated against the live frame.  Paused in `calc` with `n == 5`,
/// `n * 3` prints `15`; `:continue` then prints the call's own result, `50`.
#[test]
fn interactive_breakpoint_eval_expression() {
    let out = repl(
        &["repl"],
        "fn calc(n: integer) -> integer {\n  n * 10\n}\n:break calc\ncalc(5)\nn * 3\n:continue\n:quit\n",
    );
    assert!(out.contains("15"), "n * 3 evaluated at the frame: {out:?}");
    assert!(
        out.contains("50"),
        ":continue prints the call result: {out:?}"
    );
}

/// @PLN16 G1 — a **non-integer** edit through the binary, on a pure `if`/constant
/// body (no arithmetic — exercises the dense `line_numbers` breakpoint resolution).
/// `pick(true)` is normally 111; flipping `b` to false at the pause makes the
/// resumed call print 222 — proving both edit-and-continue beyond integer and that
/// a body with no fault-prone operator is breakable.
#[test]
fn interactive_breakpoint_edit_boolean() {
    let out = repl(
        &["repl"],
        "fn pick(b: boolean) -> integer {\n  if b { 111 } else { 222 }\n}\n:break pick\npick(true)\nb = false\n:continue\n:quit\n",
    );
    assert!(out.contains("222"), "flipped b → 222: {out:?}");
    assert!(
        !out.contains("111"),
        "the un-edited 111 must not print: {out:?}"
    );
}

/// `:quit` exits cleanly even with no input after it.
#[test]
fn quit_exits() {
    let out = repl(&["repl"], ":quit\n");
    assert!(
        out.is_empty() || !out.contains("error"),
        "clean quit: {out:?}"
    );
}

// ── phase 05: introspection commands ─────────────────────────────────────────

const DEF: &str = "fn dbl(n: integer) -> integer { n + n }\n";

#[test]
fn bytecode_command() {
    let out = repl(&["repl"], &format!("{DEF}:bytecode dbl\n:quit\n"));
    assert!(
        out.contains("n_dbl") && out.contains("=== bytecode ==="),
        "bytecode: {out:?}"
    );
}

#[test]
fn rust_command() {
    let out = repl(&["repl"], &format!("{DEF}:rust dbl\n:quit\n"));
    assert!(out.contains("fn n_dbl"), "rust: {out:?}");
}

#[test]
fn slots_command() {
    let out = repl(&["repl"], &format!("{DEF}:slots dbl\n:quit\n"));
    assert!(
        out.contains("n_dbl") && out.contains("arg"),
        "slots: {out:?}"
    );
}

#[test]
fn fns_command_lists_user_fns() {
    let out = repl(
        &["repl"],
        &format!("{DEF}fn inc(x: integer) -> integer {{ x + 1 }}\n:fns\n:quit\n"),
    );
    assert!(out.contains("dbl -> integer"), "fns: {out:?}");
    assert!(out.contains("inc -> integer"), "fns: {out:?}");
}

/// An unknown `:command` doesn't crash the session.
#[test]
fn unknown_command_is_safe() {
    let out = repl(&["repl"], ":nonsense\n1 + 1\n:quit\n");
    assert!(
        out.contains('2'),
        "session continues after unknown cmd: {out:?}"
    );
}

// ── result echo / :type / runtime-error reporting ────────────────────────────

/// Run `loft <args>` feeding `input`, return (stdout, stderr).
fn repl_full(args: &[&str], input: &str) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn loft");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A bare string-literal expression echoes its value (the bind-then-print path,
/// which earlier failed on the nested quotes).
#[test]
fn text_literal_echoes() {
    let out = repl(&["repl"], "\"hi there\"\n:quit\n");
    assert!(out.contains("hi there"), "text echo: {out:?}");
}

/// `:type <expr>` reports the inferred type without running the expression.
#[test]
fn type_command_infers() {
    let out = repl(&["repl"], "x = 5\n:type x + 1\n:type \"hi\"\n:quit\n");
    assert!(out.contains("integer"), "type of int expr: {out:?}");
    assert!(out.contains("text"), "type of text expr: {out:?}");
}

/// A failed `assert` is reported (not silently swallowed) and the session keeps
/// going — and the report is clean (no raw Rust panic backtrace).
#[test]
fn runtime_error_reported_and_recovers() {
    let (out, err) = repl_full(&["repl"], "assert(false, \"boom\")\n7\n:quit\n");
    assert!(err.contains("assertion failed"), "error reported: {err:?}");
    assert!(
        !err.contains("panicked at"),
        "no raw panic backtrace: {err:?}"
    );
    assert!(
        out.contains('7'),
        "session continues after error: out={out:?}"
    );
}

// ── :vars — value-bearing variable listing (REPL.T) ──────────────────────────

/// `:vars` prints each bound variable with its current value, in loft's native
/// rendering (text without quotes), to stdout.
#[test]
fn vars_command_shows_values() {
    let out = repl(&["repl"], "x = 5\nname = \"Alice\"\n:vars\n:quit\n");
    assert!(out.contains("x = 5"), "int var value: {out:?}");
    assert!(out.contains("name = Alice"), "text var value: {out:?}");
}

/// `:vars` reflects the *latest* value after a rebind, not the original.
#[test]
fn vars_command_reflects_latest_value() {
    let out = repl(&["repl"], "n = 5\nn = n + 100\n:vars\n:quit\n");
    assert!(out.contains("n = 105"), "rebound value: {out:?}");
}

/// `:vars` with nothing bound reports that (to stderr), and the session
/// continues.
#[test]
fn vars_command_empty_message() {
    let (_out, err) = repl_full(&["repl"], ":vars\n1 + 1\n:quit\n");
    assert!(err.contains("no variables"), "empty-vars message: {err:?}");
}

// ── session semantics: side-effecting bindings (REPL.X) ──────────────────────

/// REPL.X value-snapshot: a binding's RHS runs **once**, at binding time — its
/// value is captured and stored as a literal, so later observations re-run a
/// side-effect-free body.  `a = noisy()` prints "ran" exactly once however many
/// times `a` is observed afterward (two observations here).  (Before REPL.X the
/// body re-ran the RHS per observation, printing "ran" twice.)
#[test]
fn side_effecting_binding_runs_once() {
    let out = repl(
        &["repl"],
        "fn noisy() -> integer {\n  println(\"ran\");\n  42\n}\na = noisy()\na + 0\na + 0\n:quit\n",
    );
    let runs = out.matches("ran").count();
    assert_eq!(
        runs, 1,
        "side effect runs once at binding time under REPL.X; got {runs} in {out:?}"
    );
    assert!(
        out.contains("42"),
        "captured value still observable: {out:?}"
    );
}

/// The same value-snapshot for a **text** RHS: the effect runs once, the captured
/// text is observable afterward.
#[test]
fn side_effecting_text_binding_runs_once() {
    let out = repl(
        &["repl"],
        "fn greet() -> text {\n  println(\"called\");\n  \"hi\"\n}\ng = greet()\ng\ng\n:quit\n",
    );
    assert_eq!(
        out.matches("called").count(),
        1,
        "text RHS runs once at binding time: {out:?}"
    );
    assert!(out.contains("hi"), "captured text observable: {out:?}");
}

/// Heap values — **struct** and **vector** — are captured via `show_loft` on the
/// `DbRef` return: the RHS runs **once** and the value is preserved on later
/// observes (no repeat of the side effect).
#[test]
fn capture_heap_types_run_once() {
    let out = repl(
        &["repl"],
        "struct P { a: integer, b: integer }\n\
         fn mk() -> P {\n  println(\"mk\");\n  P { a: 7, b: 9 }\n}\n\
         p = mk()\np.a + p.b\np.a + p.b\n\
         fn vec() -> vector<integer> {\n  println(\"v!\");\n  [10, 20, 30]\n}\n\
         w = vec()\nw[2]\nw[2]\n:quit\n",
    );
    assert_eq!(
        out.matches("mk").count(),
        1,
        "struct RHS runs once: {out:?}"
    );
    assert!(out.contains("16"), "struct value preserved (7+9): {out:?}");
    assert_eq!(
        out.matches("v!").count(),
        1,
        "vector RHS runs once: {out:?}"
    );
    assert!(out.contains("30"), "vector value preserved (w[2]): {out:?}");
}

// ── CORRECTNESS: the capture-error wart (REPL.X) ─────────────────────────────

/// A binding whose RHS **faults at capture time** must run its side effect
/// **once** and report the error **at the binding line**, leaving the session
/// usable.  Today capture *swallows* the fault and falls back to storing the RHS
/// as source — so the effect repeats on every later observe and the session is
/// poisoned (each observe re-runs the faulting RHS).  FIXED via the tri-state
/// `Capture`: a faulting RHS surfaces its error at the binding (`Capture::Failed`)
/// and stores nothing, so the effect runs once and no source binding re-faults.
#[test]
fn erroring_binding_runs_once_and_does_not_poison_session() {
    let (out, err) = repl_full(
        &["repl"],
        "fn bad() -> integer { println(\"fired\"); assert(false, \"boom\"); 42 }\n\
a = bad()\n9 + 9\n:quit\n",
    );
    // The side effect runs exactly once (at the binding), not once per observe.
    assert_eq!(
        out.matches("fired").count(),
        1,
        "side effect must run once, not repeat: out={out:?} err={err:?}"
    );
    // The session stays usable — a later observe still evaluates.
    assert!(
        out.contains("18"),
        "session must stay usable after an erroring binding: out={out:?} err={err:?}"
    );
}

/// Typed value-snapshot covers every inline scalar.  The **large integer**
/// (> 32-bit) is the key case — it round-trips only if capture reads the return
/// as 64-bit (`integer` is always 64-bit), so this guards against the earlier
/// `i32` read that truncated big values.
#[test]
fn capture_typed_scalars() {
    let out = repl(
        &["repl"],
        "big = 5000000000\nbig\nf = 2.5\nf\nb = true\nb\nc = 'x'\nc\ns = 1.5f\ns\n:quit\n",
    );
    assert!(out.contains("5000000000"), "64-bit integer: {out:?}");
    assert!(out.contains("2.5"), "float: {out:?}");
    assert!(out.contains("true"), "boolean: {out:?}");
    assert!(out.contains('x'), "character: {out:?}");
    assert!(out.contains("1.5"), "single: {out:?}");
}

/// A simple **enum** value is captured via its inline discriminant byte (read
/// at width 1, mapped to `Enum.Variant`): the RHS runs **once** and the variant
/// is preserved.  (Struct/vector capture — `DbRef` returns — is a separate stage;
/// those still fall back to source, covered by `struct_binding_falls_back_to_source`.)
#[test]
fn capture_enum_runs_once() {
    let out = repl(
        &["repl"],
        "enum Color { Red, Green, Blue }\n\
         fn pick() -> Color {\n  println(\"picked\");\n  Color.Blue\n}\n\
         c = pick()\nc == Color.Blue\nc == Color.Blue\n:quit\n",
    );
    assert_eq!(
        out.matches("picked").count(),
        1,
        "enum RHS runs once (captured): {out:?}"
    );
    assert!(
        out.contains("true"),
        "enum variant preserved (Blue): {out:?}"
    );
}
