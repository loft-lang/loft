// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#952 — what the execution watchdog says when it stops a run.
//!
//! The watchdog is the guaranteed-termination layer and it always worked; what it could
//! not do was say WHERE. An interpreted run checkpointed once, with the literal
//! `"<entry>"`, so every report read `phase=run-interpret fn=<entry> file=?:0` — three
//! placeholders and no location. A slow test therefore became an undebuggable `SIGABRT`,
//! and the reporter recovered the culprit by grepping raw output for a repeating error
//! line. `--native` had named its function per call all along; this is the interpreter
//! catching up.
//!
//! Asserted here is the part a person acts on: that the report names the loft FUNCTION
//! the run was in, the FILE it lives in, and — under `--tests`, where discovery sweeps in
//! every sibling it can reach — the TEST it was reached from. The timings are not
//! asserted; they move with the machine and pinning them would make this a
//! change-detector.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Write `body` to a temp `.loft` file, run it with a short deadline, return stdout+stderr.
///
/// `extra` carries `--tests` where a cell needs it. The deadline is short on purpose: the
/// hard kill lands at `3 + 2` seconds, so a cell that fails to fire ends the test rather
/// than the run.
fn run_until_deadline(name: &str, body: &str, extra: &[&str]) -> String {
    let path = std::env::temp_dir().join(format!("loft_timeout_{name}.loft"));
    fa::write(&path, body).expect("write probe");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret");
    for a in extra {
        cmd.arg(a);
    }
    cmd.arg(&path).env("LOFT_TIMEOUT", "3");
    let out = cmd.output().expect("spawn loft");
    let _ = fa::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A hang with NO further loft call in it.  It stops GRACEFULLY: at the deadline the
/// watchdog sets the dispatch loop's stop flag, which every op tests (`@FR-R-DispatchStop`),
/// so a loop needs no call to reach a checkpoint.
const SPINS_INSIDE_ONE_FN: &str = r#"
fn spin952(n: integer) -> integer {
  t = 0;
  for i in 0..n { t += i; }
  t
}
fn main() {
  x = spin952(100000000000);
  println("never {x}");
}
"#;

/// The same hang reached from a test function, which is what `--tests` runs.
const STUCK_TEST: &str = r#"
fn helper952(n: integer) -> integer {
  t = 0;
  for i in 0..n { t += i; }
  t
}
fn test_the_stuck_one() {
  assert(helper952(100000000000) > 0, "never");
}
"#;

/// A hang that keeps CALLING.  `work952` is a scalar leaf, inlined into `main`
/// (`@FR-R-InlineLeaf`), so the frame that runs — and the one the report names — is
/// `main`'s, as it is under `--native`, where LLVM inlines it.
const CALLS_IN_A_LOOP: &str = r#"
fn work952(i: integer) -> integer { i * 2 }
fn main() {
  t = 0;
  for i in 0..100000000000 { t += work952(i); }
  println("never {t}");
}
"#;

/// A loop that calls nothing stops gracefully and names the function it was in, not a
/// placeholder.
#[test]
fn a_loop_without_calls_stops_gracefully_and_names_its_function() {
    let out = run_until_deadline("hardkill", SPINS_INSIDE_ONE_FN, &[]);
    assert!(
        out.contains("(graceful)") && !out.contains("hard-kill"),
        "a loop reaches the stop flag after every op, so it stops gracefully\n{out}"
    );
    assert!(
        out.contains("fn=spin952"),
        "the report must name the loft function the run was in\n{out}"
    );
    assert!(
        out.contains("loft_timeout_hardkill.loft"),
        "and the file that function lives in\n{out}"
    );
    // The control: the placeholders this issue was filed about must be gone. Without
    // this the two assertions above could pass on a report that still said nothing,
    // because `fn=<entry>` also contains no contradiction.
    assert!(
        !out.contains("fn=<entry>") && !out.contains("file=?:0"),
        "the pre-952 placeholders must not survive\n{out}"
    );
}

/// Under `--tests` the report also names the test the stuck function was reached from —
/// the field that replaces grepping raw output for which swept-in file was responsible.
#[test]
fn hard_kill_names_the_test_it_came_from() {
    let out = run_until_deadline("stucktest", STUCK_TEST, &["--tests"]);
    assert!(
        out.contains("fn=helper952"),
        "the innermost loft function\n{out}"
    );
    assert!(
        out.contains("entry=test_the_stuck_one"),
        "and the test it was reached from\n{out}"
    );
}

/// A run that keeps calling exits cooperatively at the deadline — cleanly, with the same
/// attribution, rather than being aborted `grace` seconds later.
#[test]
fn a_calling_loop_exits_gracefully_and_is_attributed() {
    let out = run_until_deadline("graceful", CALLS_IN_A_LOOP, &[]);
    assert!(
        out.contains("(graceful)"),
        "a loop that keeps calling must reach a cooperative checkpoint\n{out}"
    );
    assert!(
        out.contains("fn=main"),
        "and name the frame it was in — `main`, which runs the inlined `work952`\n{out}"
    );
}

/// The harness can fail: a program that finishes well inside the deadline says nothing
/// about timeouts at all. Without this cell, a build where every run printed a report
/// would still pass the three above.
///
/// The distinctive marker is checked at the START of a line, not merely present: a parse
/// error echoes the offending source line back, so a probe that never RAN still contains
/// everything it was going to print. That is not a hypothetical — it is how the first
/// version of this cell passed against three probes loft had refused outright.
#[test]
fn a_fast_program_reports_nothing() {
    let out = run_until_deadline("fast", "fn main() { println(\"m952_ran\"); }\n", &[]);
    assert!(
        out.lines().any(|l| l.trim() == "m952_ran"),
        "the program must actually run, not merely be quoted back by an error\n{out}"
    );
    assert!(
        !out.contains("[timeout]"),
        "a run inside its deadline must print no timeout report\n{out}"
    );
}

/// The hard kill remains for code that never returns to the dispatch loop: here a native
/// waiting on a stdin that never closes.  The report still names the function.
#[test]
fn a_hang_inside_a_native_is_hard_killed_and_named() {
    let path = std::env::temp_dir().join("loft_timeout_native952.loft");
    fa::write(
        &path,
        "fn wait952() -> text {\n  host_input()\n}\nfn main() {\n  t = wait952();\n  println(\"never {t}\");\n}\n",
    )
    .expect("write probe");
    let mut child = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_TIMEOUT", "3")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn loft");
    // Holding the child's stdin open keeps `host_input` waiting: `wait_with_output` would
    // close it first.
    let _stdin = child.stdin.take();
    let out = child.wait_with_output().expect("wait for loft");
    let _ = fa::remove_file(&path);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("hard-kill"),
        "the watchdog ends a native hang\n{text}"
    );
    assert!(
        text.contains("fn=wait952"),
        "and names the function\n{text}"
    );
}
