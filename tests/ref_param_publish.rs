// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#759 — the `&`-parameter publish guard.
//!
//! A callee can hand a heap value to its caller two ways: a `return`, and a write through
//! a `&` parameter. The work-ref buffer delivered by the second route was plain-freed at
//! scope exit, so the caller kept reading and writing a freed record.
//!
//! This needs its own test binary because the ordinary suite could not see the defect.
//! Freed bytes are usually still intact, so `tests/scripts/759-ref-param-publish.loft`
//! passes on value alone whether or not the bug is present — the suite was 3705/3705 with
//! it. Two gates are added on top, and they answer different questions:
//!
//!   - **Static** (`publishes_are_witness_freed`). The compiler must emit
//!     `OpFreeRefIfDistinct` for a published buffer, and `ref_param_publish_freed` must
//!     report nothing. This is deterministic in an ordinary release build — it does not
//!     depend on whether a freed store happened to be reused, which is what let the
//!     original defect through every gate the project runs.
//!   - **Behavioural** (`..._poison_...`). The same shapes under `LOFT_POISON=1`, which
//!     overwrites a freed store, on both backends. Setting the gate per-test costs
//!     nothing suite-wide and does not rely on anyone remembering a poison sweep.
//!
//! The positive control for the STATIC oracle is synthetic and lives with the walk
//! (`use_analysis::ref_param_publish_tests`): it feeds `scan_rpf` an injected publish +
//! plain free and requires a report, so a silent oracle cannot pass for a clean one. The
//! control here is for the harness — a script whose assertion is deliberately false must
//! fail, otherwise "the script printed OK" proves nothing.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/759-ref-param-publish.loft")
}

/// Run `file` on `backend` with extra env; return `(ok, stdout, stderr)`.
///
/// `LOFT_759_SUFFIX` names the scratch file the probe's `&File` cell writes, and it
/// has to differ per RUN. The probe is driven from two test binaries at once — the
/// five cells here, plus `wrap::loft_suite`, which sweeps every `tests/scripts/*.loft`
/// — and cargo runs those binaries in parallel. On one hardcoded name they raced:
/// one process `delete`d the file while another was reading its first record back, and
/// the cell reported `file: first record null, want 4242` as if the publish had failed.
/// A `Mutex` cannot reach it (the collision is cross-PROCESS), so the name carries the
/// distinction instead. The default is empty, so a bare `loft` run of the probe — and
/// `loft_suite`'s — keeps the original filename.
fn run(backend: &str, file: &PathBuf, env: &[(&str, &str)]) -> (bool, String, String) {
    let suffix = format!(
        "_{}_{}",
        backend.trim_start_matches('-'),
        env.iter().map(|(k, _)| *k).collect::<Vec<_>>().join("_")
    );
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg(file)
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_759_SUFFIX", suffix)
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

const OK: &str = "759 ref-param publish OK";

fn assert_cells_green(backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, &probe(), env);
    assert!(
        ok && stdout.contains(OK),
        "[{backend}/{tag}] every &-publish cell must be green\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

// ── Static: the emitted free is the conditional one, and the oracle is silent ─────────

/// The deterministic half. A plain `OpFreeRef` of a published buffer is the defect
/// itself, visible in the IR without needing the freed bytes to have been disturbed.
#[test]
fn publishes_are_witness_freed() {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("introspect")
        .arg("--show-ownership")
        .arg("--show-bytecode")
        .arg(probe())
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("failed to invoke loft introspect");
    assert!(out.status.success(), "introspect must exit 0");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    assert!(
        !text.contains("ref-param-publish-free"),
        "a work-ref buffer published through a `&` parameter is plain-freed \
         (loft#759 regressed)\n{text}"
    );
    assert!(
        text.contains("OpFreeRefIfDistinct"),
        "a publishing function must free its buffer by witness, not unconditionally\n{text}"
    );
}

// ── Value, both backends ─────────────────────────────────────────────────────────────

#[test]
fn publish_cells_interpret() {
    assert_cells_green("--interpret", &[], "value");
}

#[test]
fn publish_cells_native() {
    assert_cells_green("--native", &[], "value");
}

// ── The gate that sees a premature free at runtime ───────────────────────────────────

#[test]
fn publish_cells_poison_clean_interpret() {
    assert_cells_green("--interpret", &[("LOFT_POISON", "1")], "poison");
}

#[test]
fn publish_cells_poison_clean_native() {
    assert_cells_green("--native", &[("LOFT_POISON", "1")], "poison");
}

/// The buffer must still be freed on the path that did NOT publish — the neighbour a
/// blanket free-suppression would leak. `reb_cond(.., false)` is that cell.
#[test]
fn publish_cells_leak_clean_native() {
    let (_ok, _out, stderr) = run("--native", &probe(), &[("LOFT_NATIVE_LEAK_CHECK", "1")]);
    assert!(
        !stderr.to_lowercase().contains("not freed"),
        "[native] a store was not freed at exit — the un-published buffer leaked\n{stderr}"
    );
    assert_cells_green("--native", &[("LOFT_NATIVE_LEAK_CHECK", "1")], "leak");
}

// ── Control: the harness can fail ────────────────────────────────────────────────────
//
// Same shape as the probe, with the published record's expected value deliberately
// wrong. If this run reports success, then a green run above means only "the program
// executed", not "the record survived the publish".

const WRONG_EXPECTATION: &str = "struct C { v: integer }\n\
fn mk(n: integer) -> C { c = C { v: n }; c.v = c.v + 1; return c; }\n\
fn reb(b: &C, n: integer) -> integer { x = b.v; b = mk(n); return x; }\n\
fn main() {\n\
\x20 a = C { v: 11 };\n\
\x20 p = reb(a, 22);\n\
\x20 assert(p == 11 && a.v == 999, \"deliberately wrong: a.v is 23, not 999\");\n\
\x20 println(\"759 ref-param publish OK\");\n}\n";

#[test]
fn harness_can_fail() {
    let path = std::env::temp_dir().join(format!("loft_759_control_{}.loft", std::process::id()));
    fa::write(&path, WRONG_EXPECTATION).expect("write control probe");
    let (ok, stdout, _stderr) = run("--interpret", &path, &[]);
    let _ = fa::remove_file(&path);
    assert!(
        !(ok && stdout.contains(OK)),
        "a false assertion must fail the script — the OK line is not self-validating\n{stdout}"
    );
}
