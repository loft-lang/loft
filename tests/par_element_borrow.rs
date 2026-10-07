// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1044 — a `par` element is a VIEW into the caller's vector, and binding it as a
//! whole value must not free the caller's record.
//!
//! This needs its own binary for `keyed_element_borrow`'s reason, and the reason is the
//! whole point of the issue: **the freed bytes read back intact**. Without
//! `LOFT_POISON=1` the program answers correctly, the leak ledger balances, and
//! `tests/scripts/1044-…loft` is green whether or not the defect is present — so the
//! script alone is not a guard, it is a program the guard runs.
//!
//! The shape, measured when the issue was filed: a `par`, a STRUCT element, and a
//! WHOLE-VALUE bind of the element in the body (`_ = e;` — the idiom that silences the
//! unused-variable warning). A field read never tripped it and a non-`par` loop never
//! tripped it; both ride along in the script as in-file controls.
//!
//! ⚠ The gate that caught this class was green **by absence**, not by health:
//! `tests/scripts/1040-…loft` was the first script in the suite to run a struct-element
//! par at all. A poison run over a corpus that never writes the shape proves nothing,
//! which is why this file pins the shape rather than trusting the sweep.
//!
//! [`harness_can_fail`] is the control for the harness itself: if a deliberately false
//! assertion in the same shape does not fail the run, then "the script printed OK"
//! measures nothing.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/1044-par-element-view-borrows-its-vector.loft")
}

/// Run `file` under `--tests` with extra env; return `(ok, stdout, stderr)`.
///
/// ⚠ `backend` is `None` for the interpreter and `Some("--native")` for the compiled
/// one — `--tests --interpret <file>` is NOT the interpreter form: it makes the runner
/// walk its whole corpus instead of the file named, which is 178 s of somebody else's
/// probes and a core dump from one of them.
fn run(backend: Option<&str>, file: &PathBuf, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--tests");
    if let Some(b) = backend {
        cmd.arg(b);
    }
    cmd.arg(file)
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

/// Every cell of the probe, under the arena poison that makes a freed read loud.
fn assert_cells_green_under_poison(backend: Option<&str>) {
    let tag = backend.unwrap_or("--interpret");
    let (ok, stdout, stderr) = run(backend, &probe(), &[("LOFT_POISON", "1")]);
    assert!(
        ok,
        "[{tag}] a par element must not free the caller's record — under \
         LOFT_POISON a freed read is a 0xDEADBEEF store_nr, not stale-but-correct \
         bytes\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn par_element_cells_interpret_under_poison() {
    assert_cells_green_under_poison(None);
}

#[test]
fn par_element_cells_native_under_poison() {
    assert_cells_green_under_poison(Some("--native"));
}

/// ⚠ THE CONTROL FOR THE HARNESS. A `par` in the same shape whose assertion is
/// deliberately false must FAIL the run. Without this, every assertion above could be
/// satisfied by a harness that never reached them — which is exactly how the class this
/// issue belongs to stayed hidden: a corpus that does not write the shape reports green.
#[test]
fn harness_can_fail() {
    let src = "struct Cell { v: integer }\n\
               fn one_c(x: Cell) -> integer { 1 }\n\
               fn test_deliberately_false() {\n\
               \x20 s: vector<Cell> = [Cell { v: 41 }];\n\
               \x20 t = 0;\n\
               \x20 for e in s par(r = one_c(e), 1) { t += r; _ = e; }\n\
               \x20 assert((s[0].v ?? -1) == 999, \"CONTROL: this must fail\");\n\
               }\n";
    let path = std::env::temp_dir().join(format!("loft_1044_control_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write control probe");
    let (ok, stdout, stderr) = run(None, &path, &[("LOFT_POISON", "1")]);
    let _ = fa::remove_file(&path);
    assert!(
        !ok,
        "the harness must fail a false assertion in this exact shape, or a green run \
         above proves nothing\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
