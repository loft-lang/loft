// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1523 — a displaced-store free released the store its own container had just re-minted.
//!
//! The guard passes on the parent in a plain run: with slot reuse on, the allocator usually hands
//! the freed store straight back, so the premature free lands on the store it just released and
//! the write goes where it was going anyway. `LOFT_STRICT_STORES=1` never recycles a slot, which
//! makes `free == true` at an access unambiguous and turns that free into fourteen reported
//! use-after-frees. So the strict runs here ARE the guard's channel, per test, because the
//! scripts corpus does not run under it and nothing else would — the same reasoning as the poison
//! runs in `store_lifetime_1512_1513.rs` and the strict runs in `store_lifetime_1522.rs`.
//!
//! Only the INTERPRETER reports it. `--native` has been guarded here all along — its
//! `if _old.store_nr != new.store_nr` is exactly the runtime question this fix adds — so the
//! defect was an @FR-O-NoDiverge gap rather than a missing mechanism, and the control names its
//! backend rather than asking both.

use std::path::PathBuf;
use std::process::Command;

const GUARD: &str = "1523-a-displaced-free-asks-the-container-it-came-from.loft";
const OK: &str = "1523 a displaced free asks the container it came from OK";

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts")
        .join(GUARD)
}

fn run(backend: &str, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg(backend)
        .arg(script())
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

fn assert_no_violation(backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, env);
    assert!(
        ok && stdout.contains(OK),
        "[{backend}/{tag}] every cell of {GUARD} must be green\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("USE AFTER FREE"),
        "[{backend}/{tag}] {GUARD} must touch no freed store — the displaced free is releasing \
         the store its own container just re-minted (loft#1523)\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("NEVER FREED") && !stderr.contains("stores not freed"),
        "[{backend}/{tag}] {GUARD} must retain nothing — narrowing this free must not turn the \
         use-after-free into a leak, which is how three earlier cures failed\nstderr:\n{stderr}"
    );
}

#[test]
fn displaced_free_asks_the_container_interpret() {
    assert_no_violation("--interpret", &[], "value");
}

#[test]
fn displaced_free_asks_the_container_native() {
    assert_no_violation("--native", &[], "value");
}

#[test]
fn displaced_free_asks_the_container_strict_interpret() {
    assert_no_violation("--interpret", &[("LOFT_STRICT_STORES", "1")], "strict");
}

#[test]
fn displaced_free_asks_the_container_strict_native() {
    assert_no_violation("--native", &[("LOFT_STRICT_STORES", "1")], "strict");
}

/// The control: with the witness off the guard must FAIL on the interpreter.
///
/// Without it the tests above keep passing against an emission that has stopped asking anything —
/// a guard whose subject was optimised away reads exactly like a guard whose subject is fixed.
/// This is also the file's re-runnable falsification receipt.
#[test]
fn the_container_witness_is_load_bearing() {
    let (ok, stdout, stderr) = run(
        "--interpret",
        &[
            ("LOFT_NO_DISPLACED_WITNESS", "1"),
            ("LOFT_STRICT_STORES", "1"),
        ],
    );
    assert!(
        !ok && stderr.contains("USE AFTER FREE"),
        "LOFT_NO_DISPLACED_WITNESS=1 must reproduce the use-after-free on the interpreter — if it \
         no longer does, this guard has stopped measuring loft#1523\nstdout:\n{stdout}\n\
         stderr:\n{stderr}"
    );
}
