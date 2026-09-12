// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1522 — a rebind released the store its construction work-ref still named.
//!
//! The guard script passes on the parent build in a plain run: with slot reuse ON the
//! allocator usually hands the buffer its own freed number straight back, so the premature
//! free lands on the store it just released and nothing is observable.  `LOFT_STRICT_STORES=1`
//! never recycles a slot, which makes `free == true` at an access unambiguous and turns that
//! free into four reported use-after-frees.  So the strict runs here ARE the guard's channel,
//! per test, because the scripts corpus does not run under it and nothing else would — the
//! same reasoning as the poison runs in `store_lifetime_1512_1513.rs`.
//!
//! Both directions are scored.  The UAF cells need `--native` as much as the interpreter: the
//! native build answered a WRONG `o.opt` rather than reporting anything, so a run that only
//! checked the interpreter would have called the emission fixed on one backend.
//!
//! The leak channel is scored in a PLAIN run, and it is not a formality: the first cure here
//! was a static veto that traded the use-after-free for a retained store, and the plain-run
//! leak warning is what caught it.

use std::path::PathBuf;
use std::process::Command;

const GUARD: &str = "1522-a-rebind-releases-the-buffer-it-adopted.loft";
const OK: &str = "1522 rebind releases the buffer it adopted OK";

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts")
        .join(GUARD)
}

/// Run the guard on `backend` with `env`; return `(exit ok, stdout, stderr)`.
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
        "[{backend}/{tag}] {GUARD} must touch no freed store — a rebind is releasing a store \
         its construction work-ref still names (loft#1522)\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("NEVER FREED") && !stderr.contains("stores not freed"),
        "[{backend}/{tag}] {GUARD} must retain nothing — declining the rebind free must not \
         trade the use-after-free for a leak\nstderr:\n{stderr}"
    );
}

#[test]
fn rebind_keeps_the_buffers_store_interpret() {
    assert_no_violation("--interpret", &[], "value");
}

#[test]
fn rebind_keeps_the_buffers_store_native() {
    assert_no_violation("--native", &[], "value");
}

#[test]
fn rebind_keeps_the_buffers_store_strict_interpret() {
    assert_no_violation("--interpret", &[("LOFT_STRICT_STORES", "1")], "strict");
}

#[test]
fn rebind_keeps_the_buffers_store_strict_native() {
    assert_no_violation("--native", &[("LOFT_STRICT_STORES", "1")], "strict");
}

/// The control: with the pairing off the guard must FAIL, on both backends.
///
/// Without this the three tests above would keep passing against an emission that had stopped
/// declining anything — a guard whose subject has been optimised away reads exactly like a
/// guard whose subject is fixed.  `LOFT_NO_BUFFER_VETO=1` is also the file's own falsification
/// receipt, which is why it is re-runnable rather than a recorded sha.
#[test]
fn the_pairing_is_load_bearing() {
    for backend in ["--interpret", "--native"] {
        let (ok, stdout, stderr) = run(
            backend,
            &[("LOFT_NO_BUFFER_VETO", "1"), ("LOFT_STRICT_STORES", "1")],
        );
        assert!(
            !ok && stderr.contains("USE AFTER FREE"),
            "[{backend}] LOFT_NO_BUFFER_VETO=1 must reproduce the use-after-free — if it no \
             longer does, this guard has stopped measuring loft#1522\nstdout:\n{stdout}\n\
             stderr:\n{stderr}"
        );
    }
}
