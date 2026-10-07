// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 transparent-link widening — the BASELINE guard (build step 1).
//!
//! Pins the copy-semantics answer for the safety + observability matrices
//! (`tests/scripts/link-widen-baseline.loft`) on BOTH backends, value + leak. This is the reference
//! the widening (link-where-safe-AND-unobservable) must reproduce byte-value-identically: the
//! observability cells (O2/O3/O4) fail loudly if a wrong link lets a write cross the copy boundary,
//! and the safety cells (S2/S4) are run under the poison / native-leak gates so a wrong link surfaces
//! as a UAF. See doc/claude/plans/102-stability-contract/alias-where-correct-build.md.
//!
//! Step 1 makes NO product change — this only captures + locks the baseline, and proves (via
//! `harness_can_fail`) that the probe is not vacuous. Steps 4–5 re-run it under `LOFT_LINK_WIDEN`.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/link-widen-baseline.loft")
}

/// Run `file` on `backend` with extra env; return `(ok, stdout, stderr)`.  A failure's
/// stderr carries the exit status first — a code says the program or `rustc` ended it,
/// a signal says something killed it — because an empty stdout+stderr on its own once
/// left a red gate with nothing to read.
fn run(backend: &str, file: &PathBuf, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg(file)
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let mut stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        stderr = format!("[exit status: {}]\n{stderr}", out.status);
    }
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr,
    )
}

const OK: &str = "link-widen baseline OK";

// ── The baseline holds: value-identical on both backends ─────────────────────────────────────────

fn assert_baseline(backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, &probe(), env);
    assert!(
        ok && stdout.contains(OK),
        "[{backend}/{tag}] baseline must pass with all cells green\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn baseline_interpret() {
    assert_baseline("--interpret", &[], "value");
}

#[test]
fn baseline_native() {
    assert_baseline("--native", &[], "value");
}

// ── The safety cells are leak/UAF-clean under the poison + native-leak gates ─────────────────────
// (S2 source-dead, S4 escaping copy — a wrong link would dangle; today they are clean.)

#[test]
fn baseline_poison_clean_interpret() {
    let (_ok, _out, stderr) = run("--interpret", &probe(), &[("LOFT_POISON", "1")]);
    assert!(
        !stderr.to_lowercase().contains("poison"),
        "[interpret] poison gate tripped — a store was read after free\n{stderr}"
    );
    assert_baseline("--interpret", &[("LOFT_POISON", "1")], "poison");
}

#[test]
fn baseline_leak_clean_native() {
    let (_ok, _out, stderr) = run("--native", &probe(), &[("LOFT_NATIVE_LEAK_CHECK", "1")]);
    assert!(
        !stderr.to_lowercase().contains("not freed") && !stderr.to_lowercase().contains("leak"),
        "[native] leak gate tripped — a store was not freed at exit\n{stderr}"
    );
    assert_baseline("--native", &[("LOFT_NATIVE_LEAK_CHECK", "1")], "leak");
}

// ── Positive control: the probe is NOT vacuous — a simulated wrong link (a write crossing the
//    copy boundary) MUST fail. Injects O2's wrong-link value (source reads the copy's 99). ────────

const WRONG_LINK: &str = "struct S { v: vector<integer> }\n\
fn main() {\n  s = S { v: [10, 20, 30] };\n  a = s.v;\n  a[1] = 99;\n\
  assert(s.v[1] == 99, \"if this PASSES, copy is broken or the probe is vacuous\");\n\
  print(\"unexpectedly passed\");\n}\n";

fn assert_wrong_link_fails(backend: &str) {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("loft_linkfail_{}.loft", std::process::id()));
    fa::write(&path, WRONG_LINK).expect("write");
    let (ok, stdout, _e) = run(backend, &path, &[]);
    let _ = fa::remove_file(&path);
    assert!(
        !ok && !stdout.contains("unexpectedly passed"),
        "[{backend}] the copy-boundary probe is VACUOUS — a wrong-link value did not fail"
    );
}

#[test]
fn harness_can_fail_interpret() {
    assert_wrong_link_fails("--interpret");
}

#[test]
fn harness_can_fail_native() {
    assert_wrong_link_fails("--native");
}
