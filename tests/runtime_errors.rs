// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-07 phase 4 — typed runtime error binary tests.
//!
//! End-to-end check that the loft `panic("msg")` and failed
//! `assert(test, "msg")` builtins surface as a `RuntimeError`-rendered
//! pretty error (rustc-style `error:` header, `--> file:line:col`,
//! source line, caret) and exit non-zero — replacing the legacy Rust
//! panic that ate the call stack and printed an obscure `panicked at`
//! frame.
//!
//! Phase 4 step 4.1 + 4.2 + 4.11 + 4.13 are the foundation; later
//! steps (4.3-4.10, 4.12, 4.14, 4.15) add more kinds + backtrace
//! capture + the renderer's frame list.  Each new kind lands here as
//! a `kind_<name>` test.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run a loft snippet under `--interpret` and return (stdout, stderr,
/// exit-status-code).  Status is `Some(c)` when the process exited
/// normally; on signal, returns `None`.
fn run_loft_snippet(name: &str, source: &str) -> (String, String, Option<i32>) {
    let script_path = std::env::temp_dir().join(format!("loft_{name}.loft"));
    std::fs::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// @P356 — run a snippet under `--interpret` with `LOFT_DEV_SOFT_HALT=1`,
/// the opt-in fail-fast mode that surfaces RECOVERABLE faults (OOB /
/// negative index) loudly (stderr `soft-halt:` + non-zero exit) for
/// debugging.  Verifies the structured-fault rendering still works for the
/// index kinds, which now log-and-continue by default.
fn run_loft_snippet_soft_halt(name: &str, source: &str) -> (String, String, Option<i32>) {
    let script_path = std::env::temp_dir().join(format!("loft_{name}.loft"));
    std::fs::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .env("LOFT_DEV_SOFT_HALT", "1")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Phase 4 step 4.11 — `panic("msg")` builtin produces a typed runtime
/// error rendered through the phase-2 pretty renderer.  The rendered
/// stderr includes the kind label (`panic:`), the user message, the
/// source location, and a caret line.
#[test]
fn kind_user_panic_prints_pretty_error() {
    let source = "\
fn main() {
  panic(\"boom!\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_user_panic", source);
    assert_eq!(
        code,
        Some(1),
        "panic builtin should exit 1; stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stderr.contains("error:") && stderr.contains("panic: boom!"),
        "stderr missing 'error:' header or message; got: {stderr:?}"
    );
    assert!(
        stderr.contains("--> ") && stderr.contains(":2:"),
        "stderr missing source location pointing at line 2 (the panic call); got: {stderr:?}"
    );
    // Caret line ends the rendering — single `^` after the source line.
    assert!(
        stderr.contains("^"),
        "stderr missing caret marker; got: {stderr:?}"
    );
    // Ensure the LEGACY Rust panic message is GONE — the conversion's
    // whole point.  A `panicked at` line in stderr means the typed
    // error was bypassed.
    assert!(
        !stderr.contains("panicked at"),
        "stderr still contains a Rust panic — typed RuntimeError conversion bypassed; got: {stderr:?}"
    );
}

/// loft#1147 — a failed `assert_eq` reports BOTH sides, HALTS the run, and names the CALL
/// SITE.  All three are invisible in a passing cell, and all three are the properties that
/// make it usable as a test assertion at all:
///
/// * both sides, because the whole reason it exists is that `assert(got == want, …)` says
///   only what was got;
/// * halting with a non-zero exit, because a version that printed would turn every converted
///   assertion into a silent pass;
/// * the caller's position, because `assert_eq` is a stdlib function that forwards to
///   `assert` — without the injection every failure would name `01_code.loft`, the forwarder
///   rather than the test that broke.
#[test]
fn assert_eq_reports_both_sides_and_halts_at_the_call_site() {
    let source = "\
fn main() {
  print(\"before\\n\");
  assert_eq(2 + 2, 5, \"math is broken\");
  print(\"after\\n\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_assert_eq", source);
    assert_eq!(
        code,
        Some(1),
        "a failed assert_eq must exit 1, not pass quietly"
    );
    assert!(
        stdout.contains("before") && !stdout.contains("after"),
        "execution must halt AT the failing assert_eq; got stdout: {stdout:?}"
    );
    assert!(
        stderr.contains("assertion failed: math is broken: got 4, want 5"),
        "stderr must name BOTH sides; got: {stderr:?}"
    );
    // Line 3 of the snippet — NOT a position inside `default/01_code.loft`.
    assert!(
        stderr.contains("--> ") && stderr.contains(":3:") && !stderr.contains("01_code.loft"),
        "the position must be the CALL SITE, not the stdlib forwarder; got: {stderr:?}"
    );
}

/// loft#1147 — the label is optional, and dropping it leaves the two values as the whole
/// message.  A source position already qualifies them, so the bare form is the one worth
/// having; this pins that it does not degrade to an empty or colon-prefixed message.
#[test]
fn assert_eq_without_a_label_reports_the_two_values_alone() {
    let source = "fn main() {\n  assert_eq(7, 9);\n}\n";
    let (_stdout, stderr, code) = run_loft_snippet("rt_assert_eq_bare", source);
    assert_eq!(code, Some(1), "a failed assert_eq must exit 1");
    assert!(
        stderr.contains("assertion failed: got 7, want 9"),
        "the bare form must read as the two values alone; got: {stderr:?}"
    );
}

/// loft#1147 — `assert_ne` is the mirror, and names the value the two sides SHARE (naming
/// both would print it twice).
#[test]
fn assert_ne_names_the_shared_value() {
    let source = "fn main() {\n  assert_ne(5, 5, \"must differ\");\n}\n";
    let (_stdout, stderr, code) = run_loft_snippet("rt_assert_ne", source);
    assert_eq!(code, Some(1), "a failed assert_ne must exit 1");
    assert!(
        stderr.contains("assertion failed: must differ: both sides are 5"),
        "assert_ne must name the shared value; got: {stderr:?}"
    );
}

/// Phase 4 step 4.13 — failed `assert(test, "msg")` produces an
/// `AssertionFailed` typed error rendered through the same renderer.
/// Successful prints from earlier in the program still reach stdout
/// (the assert halts execution AT the failing call, not earlier).
#[test]
fn kind_assertion_failed_prints_pretty_error_after_partial_stdout() {
    let source = "\
fn main() {
  print(\"before\\n\");
  assert(2 + 2 == 5, \"math is broken\");
  print(\"after\\n\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_assert", source);
    assert_eq!(code, Some(1), "failed assert should exit 1");
    assert!(
        stdout.contains("before"),
        "pre-assert stdout should still flush; got: {stdout:?}"
    );
    assert!(
        !stdout.contains("after"),
        "post-assert stdout should NOT print — execution must halt; got: {stdout:?}"
    );
    assert!(
        stderr.contains("error:") && stderr.contains("assertion failed: math is broken"),
        "stderr missing assertion-failed entry; got: {stderr:?}"
    );
    assert!(
        stderr.contains("--> ") && stderr.contains(":3:"),
        "stderr missing source location at line 3; got: {stderr:?}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "stderr still contains a Rust panic; got: {stderr:?}"
    );
}

/// C80 / E-Uncomp — integer `/` by zero is a recoverable calculation fault:
/// null sentinel + continue (exit 0), not a halt.  (Was: a `DivideByZero`
/// dev-halt pretty error — reversed by C80.)
#[test]
fn kind_divide_by_zero_int_returns_null_and_continues() {
    let source = "\
fn main() {
  a = 10;
  b = 0;
  c = a / b;
  print(\"{c}\\n\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_div0_int", source);
    // C80 / E-Uncomp: integer `/` by zero is a RECOVERABLE calculation fault — it
    // yields the null sentinel and CONTINUES (exit 0), like OOB below.  (Formerly
    // a dev-halt pretty error; reversed by C80.)
    assert_eq!(
        code,
        Some(0),
        "div-by-zero is null-and-continue (exit 0); stderr={stderr:?}"
    );
    assert!(
        stdout.contains("null"),
        "post-fault stdout should print the null result; got: {stdout:?}"
    );
    assert!(
        !stderr.contains("error:"),
        "div-by-zero must NOT raise a typed error; got: {stderr:?}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "stderr still contains a Rust panic — bypassed the null-and-continue path; got: {stderr:?}"
    );
}

/// C80 / E-Uncomp — integer `%` by zero is the same recoverable fault as `/`:
/// null sentinel + continue (exit 0), not a halt.
#[test]
fn kind_mod_by_zero_int_returns_null_and_continues() {
    let source = "\
fn main() {
  z = 0;
  c = 7 % z;
  print(\"{c}\\n\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_mod0_int", source);
    assert_eq!(
        code,
        Some(0),
        "mod-by-zero is null-and-continue (exit 0); stderr={stderr:?}"
    );
    assert!(
        stdout.contains("null"),
        "mod-by-zero should yield null and continue; got: {stdout:?}"
    );
    assert!(
        !stderr.contains("error:"),
        "mod-by-zero must NOT raise a typed error; got: {stderr:?}"
    );
}

/// The nullable `??`-rescued shape STILL produces the sentinel +
/// fallback; conversion to typed errors only changes the
/// non-nullable variant per the C54.G-hybrid design.  Regression
/// guard so a future "raise on every div" change doesn't break
/// `?? 0` programs.
#[test]
fn divide_by_zero_with_nullable_rescue_does_not_raise() {
    let source = "\
fn main() {
  a = 10;
  b = 0;
  c = a / b ?? 42;
  print(\"{c}\\n\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_div0_nullable", source);
    assert_eq!(code, Some(0), "?? rescue should exit 0; stderr={stderr:?}");
    assert!(
        stdout.contains("42"),
        "expected `?? 42` to surface; got: {stdout:?}"
    );
    assert!(
        !stderr.contains("divide by zero"),
        "?? rescue path must NOT raise typed error; got: {stderr:?}"
    );
}

/// @P356 — vector positive-index OOB is a RECOVERABLE fault: by default
/// `v[5]` on a length-3 vector returns `null` and execution CONTINUES
/// (exit 0).  Runtime aborts for reversible faults belong only in opt-in
/// debugging; the value is the type's null sentinel and the compile-time
/// warning already nudged toward `v[i] ?? <fallback>`.  Both backends
/// behave identically.
#[test]
fn kind_index_out_of_bounds_vector_returns_null_and_continues() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  x = v[5];
  print(\"x={x}\\n\");
}
";
    let (stdout, _stderr, code) = run_loft_snippet("rt_oob_vec", source);
    assert_eq!(code, Some(0), "recoverable OOB should exit 0");
    assert!(
        stdout.contains("x=null"),
        "OOB index should yield null and continue; got: {stdout:?}"
    );
}

/// @P356 — `LOFT_DEV_SOFT_HALT` fail-fast still surfaces the structured OOB
/// fault (idx / len rendered inline) for debugging, so the diagnostic path
/// is preserved even though the default is now log-and-continue.
#[test]
fn kind_index_out_of_bounds_vector_soft_halt_surfaces() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  x = v[5];
  print(\"x={x}\\n\");
}
";
    let (_stdout, stderr, code) = run_loft_snippet_soft_halt("rt_oob_vec_sh", source);
    assert_eq!(code, Some(1), "soft-halt OOB should exit non-zero");
    assert!(
        stderr.contains("index 5 out of bounds for length 3"),
        "soft-halt stderr missing structured OOB message; got: {stderr:?}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "stderr still contains a Rust panic; got: {stderr:?}"
    );
}

/// @P356 — vector negative index out of range after Python-style addressing
/// is RECOVERABLE: `v[-1]` (one before the end) still works; `v[-N]` with
/// `N > len` returns `null` and continues (exit 0).  Soft-halt surfaces the
/// `NegativeIndex` fault.
#[test]
fn kind_negative_index_vector_returns_null_and_continues() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  print(\"v[-1]={v[-1]}\\n\");
  x = v[-10];
  print(\"x={x}\\n\");
}
";
    let (stdout, _stderr, code) = run_loft_snippet("rt_neg_idx_vec", source);
    assert_eq!(code, Some(0), "recoverable negative index should exit 0");
    assert!(
        stdout.contains("v[-1]=30"),
        "negative-index Python-style addressing should still work; got: {stdout:?}"
    );
    assert!(
        stdout.contains("x=null"),
        "out-of-range negative index should yield null and continue; got: {stdout:?}"
    );

    let (_so, stderr, sh) = run_loft_snippet_soft_halt("rt_neg_idx_vec_sh", source);
    assert_eq!(sh, Some(1), "soft-halt negative index should exit non-zero");
    assert!(
        stderr.contains("negative index -10"),
        "soft-halt stderr missing structured NegativeIndex message; got: {stderr:?}"
    );
}

/// @P356 — vector-of-struct-ref OOB goes through `OpVectorRef` (separate
/// dispatch from primitive `OpGetVector`); regression guard that the
/// struct-ref path is ALSO recoverable + surfaced under soft-halt.
#[test]
fn kind_index_out_of_bounds_vector_of_struct_soft_halt_surfaces() {
    let source = "\
struct P { v: integer }
fn main() {
  v = [P{v:1}, P{v:2}, P{v:3}];
  x = v[5];
}
";
    let (_stdout, _stderr, code) = run_loft_snippet("rt_oob_struct_vec", source);
    assert_eq!(code, Some(0), "recoverable struct-ref OOB should exit 0");

    let (_so, stderr, sh) = run_loft_snippet_soft_halt("rt_oob_struct_vec_sh", source);
    assert_eq!(sh, Some(1), "soft-halt struct-ref OOB should exit non-zero");
    assert!(
        stderr.contains("index 5 out of bounds for length 3"),
        "soft-halt stderr missing struct-ref OOB; got: {stderr:?}"
    );
}

/// @P356 — text positive-index OOB is RECOVERABLE: `s[100]` on a length-5
/// string returns the null char and continues (exit 0); soft-halt surfaces
/// the structured `IndexOutOfBounds`.
#[test]
fn kind_index_out_of_bounds_text_returns_null_and_continues() {
    let source = "\
fn main() {
  s = \"hello\";
  c = s[100];
  print(\"after={s}\\n\");
}
";
    let (stdout, _stderr, code) = run_loft_snippet("rt_oob_text", source);
    assert_eq!(code, Some(0), "recoverable text OOB should exit 0");
    assert!(
        stdout.contains("after=hello"),
        "text OOB should continue past the fault; got: {stdout:?}"
    );

    let (_so, stderr, sh) = run_loft_snippet_soft_halt("rt_oob_text_sh", source);
    assert_eq!(sh, Some(1), "soft-halt text OOB should exit non-zero");
    assert!(
        stderr.contains("index 100 out of bounds for length 5"),
        "soft-halt stderr missing text-OOB; got: {stderr:?}"
    );
}

/// A program without a panic / failed assert exits 0 and writes
/// nothing to stderr from the runtime-error path — guards against the
/// renderer firing on an empty `runtime_error` slot.
#[test]
fn no_runtime_error_exits_clean() {
    let source = "\
fn main() {
  print(\"hi\\n\");
  assert(true, \"trivially true\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_none", source);
    assert_eq!(code, Some(0), "clean run should exit 0; stderr={stderr:?}");
    assert!(
        stdout.contains("hi"),
        "expected stdout 'hi'; got: {stdout:?}"
    );
    // Stderr might carry warnings/leak reports; only assert the
    // runtime-error renderer didn't fire.
    assert!(
        !stderr.contains("error:") || !stderr.contains("--> "),
        "renderer fired without a typed error; got: {stderr:?}"
    );
}

// ── loft#1053 — a fault inside a `par` worker stops the WHOLE program ─────────────────
//
// `assert` and `panic` are not exceptions; they are the program's own request to stop, and
// the decided semantics is that the stop is total — the whole program, not one arm of it.
// A worker raises against its own `Stores` CLONE, which is dropped at join, so every halt
// raised inside a worker used to die with it: the arms printed nothing and the program
// exited 0, having computed a result from workers that had all failed their assertions
// (`par_fold` cheerfully returned 190).
//
// One case per par FAMILY, because the fix was first written for the block form alone and
// the other three stayed silent — the families do not share a spawn path, and a guard on
// one proves nothing about the others.

/// The shape each case shares: a worker that always fails, and a program that would print
/// `survived` if the halt were swallowed.
fn assert_par_family_halts(name: &str, source: &str, expected: &str) {
    let (stdout, stderr, code) = run_loft_snippet(name, source);
    assert_eq!(
        code,
        Some(1),
        "{name}: a failing worker must stop the whole program\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(expected),
        "{name}: expected {expected:?} in stderr, got: {stderr}"
    );
    assert!(
        !stdout.contains("survived"),
        "{name}: the program continued past the failing par construct\nstdout: {stdout}"
    );
}

#[test]
fn i1053_parallel_block_arm_assert_halts_the_program() {
    assert_par_family_halts(
        "i1053_block",
        "fn w(n: integer) -> integer { assert(false, \"BLOCK ARM\"); n }\n\
         fn main() { o = 1; parallel { w(o); w(o); } println(\"survived\"); }\n",
        "assertion failed: BLOCK ARM",
    );
}

#[test]
fn i1053_par_queue_worker_assert_halts_the_program() {
    assert_par_family_halts(
        "i1053_queue",
        "fn w(n: integer) -> integer { assert(false, \"QUEUE WORKER\"); n * 2 }\n\
         fn main() { v: vector<integer> = []; for i in 0..6 { v += [i]; } t = 0;\n\
         \x20 for a in v par(b = w(a), 2) { t += b; } println(\"survived t={t}\"); }\n",
        "assertion failed: QUEUE WORKER",
    );
}

#[test]
fn i1053_par_discard_worker_assert_halts_the_program() {
    assert_par_family_halts(
        "i1053_discard",
        "fn w(n: integer) -> integer { assert(false, \"DISCARD WORKER\"); n }\n\
         fn main() { v: vector<integer> = []; for i in 0..6 { v += [i]; }\n\
         \x20 for a in v par(b = w(a), 2) { } println(\"survived\"); }\n",
        "assertion failed: DISCARD WORKER",
    );
}

#[test]
fn i1053_par_fold_worker_assert_halts_the_program() {
    assert_par_family_halts(
        "i1053_fold",
        "fn add(a: integer, b: integer) -> integer { assert(false, \"FOLD WORKER\"); a + b }\n\
         fn main() { v: vector<integer> = []; for i in 0..20 { v += [i]; }\n\
         \x20 println(\"survived {par_fold(v, 0, add, 2)}\"); }\n",
        "assertion failed: FOLD WORKER",
    );
}

/// `panic` too, not just `assert` — they share the halt path, and a fix that covered only
/// the assert would leave the louder of the two silent.
#[test]
fn i1053_par_worker_panic_halts_the_program() {
    assert_par_family_halts(
        "i1053_panic",
        "fn w(n: integer) -> integer { panic(\"QUEUE PANIC\"); n }\n\
         fn main() { v: vector<integer> = []; for i in 0..6 { v += [i]; } t = 0;\n\
         \x20 for a in v par(b = w(a), 2) { t += b; } println(\"survived\"); }\n",
        "panic: QUEUE PANIC",
    );
}

/// The other half: a clean par program must be untouched.  A halt-on-worker-fault change
/// that also stopped healthy runs would pass every test above.
#[test]
fn i1053_clean_par_programs_are_unaffected() {
    let (stdout, stderr, code) = run_loft_snippet(
        "i1053_clean",
        "fn w(n: integer) -> integer { n * 2 }\n\
         fn add(a: integer, b: integer) -> integer { a + b }\n\
         fn main() { v: vector<integer> = []; for i in 0..10 { v += [i]; } t = 0;\n\
         \x20 for a in v par(b = w(a), 2) { t += b; }\n\
         \x20 println(\"t={t} s={par_fold(v, 0, add, 2)}\"); }\n",
    );
    assert_eq!(code, Some(0), "a clean par program must exit 0: {stderr}");
    assert!(
        stdout.contains("t=90") && stdout.contains("s=45"),
        "clean par results must be unchanged, got: {stdout}"
    );
}

// ── loft#1056 — one fault, one rendering, whatever backend ran it ─────────────────────
//
// `assert` and `panic` are the two explicit halt statements, so they are one event with
// one rendering: the loft diagnostic (message + the program's own `file:line:col` +
// source line + caret), then the loft frames the fault fired under.  `assert` used to
// break every part of that on `--native`, the DEFAULT backend — it reached the user as
// `thread '<unnamed>' panicked at /tmp/loft_native_2466316.rs:966`, a Rust location in a
// generated file the author has never seen.  And neither backend printed the frames.
//
// The cases below are differential on purpose: each asserts that the two backends emit
// the SAME bytes, then what those bytes must say.  Equality alone would pass on two
// binaries that are wrong in the same way, which is why every case also pins content.

/// Run a snippet on `backend`, returning (stdout, stderr, exit code).
///
/// The script path is shared by both backends of a case so their renderings are
/// comparable byte-for-byte — the diagnostic names the source file.
fn run_loft_snippet_on(backend: &str, name: &str, source: &str) -> (String, String, Option<i32>) {
    let script_path = std::env::temp_dir().join(format!("loft_{name}.loft"));
    std::fs::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(&script_path)
        .env("LOFT_TIMEOUT", "120")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Run a snippet on both backends, assert they rendered the fault identically, and hand
/// back the one rendering for the per-case content checks.
///
/// `--native` is bounded by `LOFT_TIMEOUT` because rustc can hang.
fn rendering_shared_by_both_backends(name: &str, source: &str) -> (String, String) {
    let (i_out, i_err, i_code) = run_loft_snippet_on("--interpret", name, source);
    let (n_out, n_err, n_code) = run_loft_snippet_on("--native", name, source);
    let _ = std::fs::remove_file(std::env::temp_dir().join(format!("loft_{name}.loft")));
    assert_eq!(
        i_err, n_err,
        "{name}: the same fault must read the same way on both backends\n\
         --interpret:\n{i_err}\n--native:\n{n_err}"
    );
    assert_eq!(i_code, n_code, "{name}: exit codes diverged");
    assert_eq!(i_out, n_out, "{name}: stdout diverged");
    (i_out, i_err)
}

/// A failed `assert` is a loft diagnostic naming the program's own source, not a Rust
/// panic naming the generated temp file.
#[test]
fn i1056_assert_renders_as_a_loft_diagnostic_on_both_backends() {
    let (_out, err) = rendering_shared_by_both_backends(
        "i1056_plain",
        "fn main() { assert(1 == 2, \"PLAIN ASSERT\"); }\n",
    );
    assert!(
        err.contains("error: assertion failed: PLAIN ASSERT"),
        "expected the loft diagnostic; got: {err}"
    );
    assert!(
        err.contains("loft_i1056_plain.loft:1:1"),
        "the diagnostic must name the loft source and position; got: {err}"
    );
    assert!(
        !err.contains("panicked at") && !err.contains("loft_native_"),
        "a Rust panic naming the generated file is what this closes; got: {err}"
    );
}

/// The frames a fault fired under are part of the rendering — on BOTH backends.
///
/// `assert` and `panic` are native fns that never saw a `State`, so both left the chain
/// empty and a reader was told the line but never the call that reached it.
#[test]
fn i1056_a_fault_names_the_calls_that_reached_it() {
    let (_out, err) = rendering_shared_by_both_backends(
        "i1056_frames",
        "fn inner1056(n: integer) -> integer { assert(n < 5, \"NESTED\"); n }\n\
         fn middle1056(n: integer) -> integer { inner1056(n + 1) }\n\
         fn main() { x = middle1056(9); println(\"survived {x}\"); }\n",
    );
    assert!(
        err.contains("in fn inner1056() ← called from")
            && err.contains("fn middle1056()")
            && err.contains("fn main()"),
        "expected the call chain innermost-first; got: {err}"
    );
}

/// A `panic` inside a `par` worker names the WORKER's frames, not the parent's.
///
/// The parent re-raises a worker's halt as its own (loft#1053), so whatever frames were
/// attached at that moment belonged to the parent — `main`, which is not where the fault
/// happened.  Naming the wrong function is worse than naming none.
#[test]
fn i1056_a_par_worker_fault_names_the_workers_own_frames() {
    let (out, err) = rendering_shared_by_both_backends(
        "i1056_worker",
        "fn deep1056(n: integer) -> integer { assert(n < 0, \"IN WORKER\"); n }\n\
         fn worker1056(n: integer) -> integer { deep1056(n) * 2 }\n\
         fn main() {\n\
         \x20 v: vector<integer> = []; for i in 0..6 { v += [i]; } t = 0;\n\
         \x20 for a in v par(b = worker1056(a), 2) { t += b; }\n\
         \x20 println(\"survived t={t}\");\n\
         }\n",
    );
    assert!(
        err.contains("in fn deep1056() ← called from") && err.contains("fn worker1056()"),
        "expected the worker's own chain; got: {err}"
    );
    assert!(
        !err.contains("fn main()"),
        "`main` is the parent's frame, not a frame the worker fault fired under; got: {err}"
    );
    assert!(
        !out.contains("survived"),
        "the program continued past the failing worker; got: {out}"
    );
}

/// A halting fault is reported ONCE, however many workers reach it together.
///
/// Six rows over two workers printed the diagnostic twice on `--native` and once on
/// `--interpret`: the same halt, told a different number of times per backend.
#[test]
fn i1056_a_halting_fault_is_reported_once() {
    let (_out, err) = rendering_shared_by_both_backends(
        "i1056_once",
        "fn worker1056(n: integer) -> integer { panic(\"ONE REPORT\"); n * 2 }\n\
         fn main() {\n\
         \x20 v: vector<integer> = []; for i in 0..6 { v += [i]; } t = 0;\n\
         \x20 for a in v par(b = worker1056(a), 2) { t += b; }\n\
         \x20 println(\"survived t={t}\");\n\
         }\n",
    );
    assert_eq!(
        err.matches("error: panic: ONE REPORT").count(),
        1,
        "one halt, one report; got: {err}"
    );
}

/// The control: a program that faults nowhere renders nothing and exits 0.
///
/// Without it every case above would also pass against a build that printed a
/// diagnostic on every run.
#[test]
fn i1056_a_clean_program_renders_no_fault() {
    let (out, err) = rendering_shared_by_both_backends(
        "i1056_clean",
        "fn inner1056(n: integer) -> integer { assert(n < 5, \"NOT HIT\"); n }\n\
         fn main() { x = inner1056(1); println(\"clean1056 {x}\"); }\n",
    );
    assert!(
        out.contains("clean1056 1"),
        "the program must run; got: {out}"
    );
    assert!(
        !err.contains("assertion failed") && !err.contains("called from"),
        "a passing assert must say nothing; got: {err}"
    );
}

/// A fault inside a lazy-store DRIVER is contained on both backends, not fatal.
///
/// @PLN133 S8 decided that a buggy driver makes the lookup answer null and reports itself
/// through `store_lazy_error` — it is not the program's to die of. The generated driver
/// call runs under `catch_unwind`, so the native path has to UNWIND rather than exit, and
/// `report_and_exit` exits. `panic` had been exiting the process from inside a driver
/// since it started using that path, and loft#1056 moved `assert` onto it too; both now
/// take the same `in_lazy_driver` bypass `cr_stack_overflow` already had.
///
/// Both statements are checked because they reach the halt by different routes, and the
/// interpreter contained both all along — so a regression here shows up as a backend
/// disagreement, which is what `rendering_shared_by_both_backends` reports.
#[test]
fn i1056_a_fault_inside_a_lazy_driver_is_contained_on_both_backends() {
    for (name, halt, marker) in [
        (
            "i1056_drv_assert",
            "assert(false, \"DRIVER FAULT\")",
            "assertion_failed: assertion failed: DRIVER FAULT",
        ),
        (
            "i1056_drv_panic",
            "panic(\"DRIVER FAULT\")",
            "user_panic: panic: DRIVER FAULT",
        ),
    ] {
        let source = format!(
            "struct LzP {{ const id: integer, v: integer }}\n\
             fn lazy_fetch(coll: hash<LzP[id]>, source: text, key_int: integer, key_text: text) -> integer {{\n\
             \x20 {halt};\n\
             \x20 return 0;\n\
             }}\n\
             fn main() {{\n\
             \x20 people: hash<LzP[id]> = [];\n\
             \x20 if !store_bind_lazy(people, \"postgres://127.0.0.1:1/nope\") {{ println(\"bind failed\"); return }}\n\
             \x20 r = people[7];\n\
             \x20 println(\"null={{r == null}}\");\n\
             \x20 println(\"why={{store_lazy_error(people)}}\");\n\
             \x20 println(\"survived1056\");\n\
             }}\n"
        );
        let (out, _err) = rendering_shared_by_both_backends(name, &source);
        assert!(
            out.contains("survived1056"),
            "{name}: a driver's fault must not halt the program\nstdout: {out}"
        );
        assert!(
            out.contains("null=true") && out.contains(&format!("why={marker}")),
            "{name}: the lookup answers null and the reason reaches `store_lazy_error`\nstdout: {out}"
        );
    }
}

/// A call-stack overflow reports the function that is RUNNING — not the one it refused
/// to enter, and not an arbitrary one of ten thousand identical call sites.
///
/// This is the divergence loft#1058 filed: `--interpret` printed the loft diagnostic
/// (`-->`, source line, caret) and `--native` a hand-rolled line with no position block
/// and its own frame format.  Converging them turned up the reason they could not simply
/// be folded — the two backends describe the same stack from opposite ends.  The
/// interpreter refuses a call before the frame exists; native pushed the frame and
/// tripped after, so it named the callee while the interpreter named its caller.  A
/// refused call never runs, so it is not on the stack the diagnostic reports, and both
/// guards now say so.
///
/// `runaway1058(helper1058(n))` is what makes the difference visible: the call being
/// refused is `helper1058`, the function running is `runaway1058`, and they are
/// different names.  Plain self-recursion cannot tell the two readings apart.
#[test]
fn i1058_an_overflow_reports_the_running_function_on_both_backends() {
    let (out, err) = rendering_shared_by_both_backends(
        "i1058_running",
        "fn helper1058(n: integer) -> integer { n + 1 }\n\
         \n\
         fn runaway1058(n: integer) -> integer { return runaway1058(helper1058(n)); }\n\
         fn main() { println(\"go1058\"); x = runaway1058(1); println(\"{x}\"); }\n",
    );
    assert!(
        out.contains("go1058"),
        "the program must run up to the overflow; got stdout: {out}"
    );
    assert!(
        err.contains("error: call stack overflow"),
        "expected the typed diagnostic; got: {err}"
    );
    assert!(
        err.contains("loft_i1058_running.loft:3:1")
            && err.contains("fn runaway1058(n: integer)")
            && err.contains('^'),
        "the diagnostic must carry the position block, the declaration's source line and \
         a caret — none of which `--native` printed; got: {err}"
    );
    assert!(
        err.contains("in fn runaway1058() ← called from"),
        "the running function heads the chain; got: {err}"
    );
    assert!(
        !err.contains("in fn helper1058()"),
        "the call that was REFUSED never ran, so it is not on the reported stack; got: {err}"
    );
    assert!(
        !err.contains("in runaway1058 ("),
        "the native-only frame spelling is what this closes; got: {err}"
    );
}

/// Both backends admit exactly the same number of frames.
///
/// One cap, two guards: `State::fn_call` reads `call_stack.len()`, the generated binary
/// reads its shadow stack in `cr_call_push`.  The interpreter used to test a separate
/// `call_depth` counter that never counted `main` and was left untouched when a
/// coroutine truncated the stack — so the cap meant two different things and
/// `rec1058(9999)` printed an answer on `--interpret` and halted on `--native`.
///
/// The boundary is pinned from BOTH sides in one program, because a budget that moves
/// by one is invisible to a test that only checks the side it moved away from:
/// `rec1058(9998)` is `main` plus 9 999 frames — exactly `MAX_CALL_DEPTH` — and must
/// answer, while one more frame must halt.  Both numbers follow from
/// `State::MAX_CALL_DEPTH`; change it and this test is the place that says so.
#[test]
fn i1058_the_call_stack_budget_is_the_same_on_both_backends() {
    let (out, err) = rendering_shared_by_both_backends(
        "i1058_budget",
        "fn rec1058(n: integer) -> integer { if n <= 0 { return 0; } return rec1058(n - 1) + 1; }\n\
         fn main() {\n\
         \x20 println(\"at_limit={rec1058(9998)}\");\n\
         \x20 println(\"past_limit={rec1058(9999)}\");\n\
         }\n",
    );
    assert!(
        out.contains("at_limit=9998"),
        "a call stack filled to exactly MAX_CALL_DEPTH must answer; got stdout: {out}"
    );
    assert!(
        !out.contains("past_limit="),
        "one frame past the cap must halt, not answer; got stdout: {out}"
    );
    assert!(
        err.contains("error: call stack overflow — exceeded 10000 stack frames"),
        "the cap is part of the message, on both backends; got: {err}"
    );
}

/// An overflow inside a lazy-store DRIVER is contained, like every other fault there.
///
/// `cr_stack_overflow` used to carry its own `in_lazy_driver` bypass (@PLN133 S8); it now
/// reaches the one on `report_and_exit`, shared with `panic` and `assert`.  That is a
/// removal, so it needs a guard: the lookup must answer null, `store_lazy_error` must
/// carry the reason in the `<kind label>: <message>` spelling both backends use, and the
/// program must survive.
#[test]
fn i1058_an_overflow_inside_a_lazy_driver_is_contained_on_both_backends() {
    let (out, _err) = rendering_shared_by_both_backends(
        "i1058_driver",
        "struct LzQ { const id: integer, v: integer }\n\
         fn spin1058(n: integer) -> integer { return spin1058(n + 1); }\n\
         fn lazy_fetch(coll: hash<LzQ[id]>, source: text, key_int: integer, key_text: text) -> integer {\n\
         \x20 return spin1058(0);\n\
         }\n\
         fn main() {\n\
         \x20 people: hash<LzQ[id]> = [];\n\
         \x20 if !store_bind_lazy(people, \"postgres://127.0.0.1:1/nope\") { println(\"bind failed\"); return }\n\
         \x20 r = people[7];\n\
         \x20 println(\"null={r == null}\");\n\
         \x20 println(\"why={store_lazy_error(people)}\");\n\
         \x20 println(\"survived1058\");\n\
         }\n",
    );
    assert!(
        out.contains("survived1058"),
        "a driver's overflow must not halt the program; got stdout: {out}"
    );
    assert!(
        out.contains("null=true")
            && out
                .contains("why=stack_overflow: call stack overflow — exceeded 10000 stack frames"),
        "the lookup answers null and the reason reaches `store_lazy_error`; got stdout: {out}"
    );
}

/// After a contained fault unwinds ~10 000 frames, the call stack is BALANCED again.
///
/// The frame count is bookkeeping separate from the frames' storage (on native a
/// `Cell` depth beside the frame array — @PLN157 P1), so an unwind that skipped the
/// per-frame drops would leave the depth at ~10 000 while the program runs on at
/// depth 1: `stack_trace()` is the reader that would see it, and the next real
/// overflow would fire thousands of calls early.  `< 10` rather than an exact count
/// so the guard pins balance, not each backend's incidental frame shape; the red
/// case sits four orders of magnitude away.
#[test]
fn i1058_after_a_contained_overflow_the_call_stack_is_balanced() {
    let (out, _err) = rendering_shared_by_both_backends(
        "i1058_balance",
        "struct LzB { const id: integer, v: integer }\n\
         fn spin1058b(n: integer) -> integer { return spin1058b(n + 1); }\n\
         fn lazy_fetch(coll: hash<LzB[id]>, source: text, key_int: integer, key_text: text) -> integer {\n\
         \x20 return spin1058b(0);\n\
         }\n\
         fn main() {\n\
         \x20 people: hash<LzB[id]> = [];\n\
         \x20 if !store_bind_lazy(people, \"postgres://127.0.0.1:1/nope\") { println(\"bind failed\"); return }\n\
         \x20 r = people[7];\n\
         \x20 t = stack_trace();\n\
         \x20 println(\"null={r == null} balanced={len(t) < 10}\");\n\
         }\n",
    );
    assert!(
        out.contains("null=true balanced=true"),
        "the unwound driver fault must leave the call stack at its pre-lookup depth; got stdout: {out}"
    );
}
