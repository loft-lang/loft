// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1954 — a sibling scope's split copy of a local bound twice had its call buffers
//! pooled, and the second rebind released the first call's pooled store.
//!
//! The guard script passes its VALUE assertions on the parent build in a plain run: with slot
//! reuse on, the allocator hands the freed store straight back and the next call refills it.
//! `LOFT_STRICT_STORES=1` never recycles a slot, which turns that free into reported
//! use-after-frees on both backends — so the strict runs are the guard's channel, per test,
//! because the scripts corpus does not run under it.  The leak channel is scored in both
//! modes: not pooling a nullable local that keeps what its rebind displaces (the loft#1522
//! veto) retained one store per pass, which is the direction a too-wide decline would take.

use std::path::PathBuf;

const GUARD: &str = "1954-a-sibling-rebind-keeps-the-pooled-buffer.loft";
const OK: &str = "1954 sibling rebind keeps the pooled buffer OK";

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts")
        .join(GUARD)
}

/// Run the guard on `backend` with `env`; return `(exit ok, stdout, stderr)`.
fn run(backend: &str, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
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
        "[{backend}/{tag}] {GUARD} must touch no freed store — a rebind is releasing a pooled \
         call buffer (loft#1954)\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("NEVER FREED") && !stderr.contains("stores not freed"),
        "[{backend}/{tag}] {GUARD} must retain nothing — a buffer left unpooled for a local \
         whose rebind keeps what it displaces leaks the call's store\nstderr:\n{stderr}"
    );
}

#[test]
fn sibling_rebind_keeps_the_pooled_buffer_interpret() {
    assert_no_violation("--interpret", &[], "value");
}

#[test]
fn sibling_rebind_keeps_the_pooled_buffer_native() {
    assert_no_violation("--native", &[], "value");
}

#[test]
fn sibling_rebind_keeps_the_pooled_buffer_strict_interpret() {
    assert_no_violation("--interpret", &[("LOFT_STRICT_STORES", "1")], "strict");
}

#[test]
fn sibling_rebind_keeps_the_pooled_buffer_strict_native() {
    assert_no_violation("--native", &[("LOFT_STRICT_STORES", "1")], "strict");
}
