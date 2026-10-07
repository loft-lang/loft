// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#987 — a `par` loop with an EMPTY body must run its workers on BOTH backends.
//!
//! The discard route drops every result, so nothing a worker returns can witness it.
//! That is what let `--native` ship a lowering that did not exist: the emitter fell
//! through to the declaration-driven default, which wrote `n_parallel_discard`'s loft
//! declaration out with an EMPTY body.  A six-argument call site against that
//! five-parameter declaration is the only reason it was ever seen — rustc refused it.
//! Add the missing parameter and the program compiles and does nothing, silently, on
//! one backend only.
//!
//! So the oracle here is a SIDE EFFECT, not a value: each worker prints its row.  That
//! is the one thing a discarded result cannot hide, and it answers both questions at
//! once — did the workers run, and did each get its own row.
//!
//! [`tests/scripts/987-par-empty-body-discard.loft`] carries the shape matrix (text,
//! struct, vector and text-input workers, each asserting the row it was handed).  This
//! file carries the two things a script cannot state: that stdout shows every row, and
//! that the generated Rust CALLS the runtime helper rather than an empty body.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// An empty par body over three distinctive rows, each worker announcing its own.
/// The rows are 10/20/30 rather than 1/2/3 so a row delivered as something else —
/// the element's `DbRef` bits, or row 0 three times — cannot look like a pass.
const PROBE: &str = "fn i987_w(n: integer) -> integer {\n\
\x20 println(\"row {n}\");\n\
\x20 n * n\n\
}\n\
fn main() {\n\
\x20 rows = [10, 20, 30];\n\
\x20 for a in rows par(b = i987_w(a), 2) { }\n\
\x20 println(\"done\");\n\
}\n";

fn write_probe(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("loft_987_{tag}_{}.loft", std::process::id()));
    fa::write(&path, PROBE).expect("write probe");
    path
}

fn run(backend: &str, file: &PathBuf) -> (bool, String, String) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn assert_every_row_ran(backend: &str) {
    let probe = write_probe(backend.trim_start_matches('-'));
    let (ok, stdout, stderr) = run(backend, &probe);
    let _ = fa::remove_file(&probe);
    assert!(
        ok,
        "[{backend}] an empty par body must compile and run\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    for row in ["row 10", "row 20", "row 30"] {
        assert!(
            stdout.contains(row),
            "[{backend}] the discard route must run every worker — `{row}` missing\n\
             stdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
    assert!(
        stdout.contains("done"),
        "[{backend}] the program must reach the statement after the loop\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn an_empty_par_body_runs_its_workers_on_the_interpreter() {
    assert_every_row_ran("--interpret");
}

#[test]
fn an_empty_par_body_runs_its_workers_on_native() {
    assert_every_row_ran("--native");
}

/// The mechanism, not the effect: the CALL must go to the runtime helper.
///
/// Worth its own cell because "compiles and does nothing" is what an arity-only patch
/// would have produced, and the declaration it would have called is still in the
/// output — the generator emits a body-less stub for any VOID native it has no
/// implementation for, where a value-returning one gets a loud `todo!()`.  That
/// asymmetry is what hid loft#987, and it is not this route's to fix (ten stubs are
/// emitted, each dead only because its own call site is rewritten elsewhere).  So the
/// assertion is on the call site: nothing may reach that stub.
#[test]
fn the_discard_route_lowers_to_the_runtime_helper() {
    let probe = write_probe("emit");
    let out_rs = std::env::temp_dir().join(format!("loft_987_emit_{}.rs", std::process::id()));
    let status = loft::platform::process::harness_command(loft_bin())
        .args(["--native-emit", out_rs.to_str().unwrap()])
        .arg(&probe)
        .env("LOFT_TIMEOUT", "300")
        .status()
        .expect("failed to invoke loft binary");
    assert!(status.success(), "--native-emit must succeed");
    let src = fa::read_to_string(&out_rs).expect("read emitted Rust");
    let _ = fa::remove_file(&probe);
    let _ = fa::remove_file(&out_rs);
    assert!(
        src.contains("n_parallel_discard_native(cell,"),
        "the discard route must lower to `n_parallel_discard_native`, not to the \
         declaration-driven default"
    );
    assert!(
        !src.contains("  n_parallel_discard(cell,"),
        "no call may reach the loft declaration `n_parallel_discard` — its emitted body \
         is empty, which is the silent no-op loft#987 is about"
    );
}
