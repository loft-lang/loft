// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN38 phase 01b — end-to-end exercise of the loft-callable
//! durable-store binding via the compiled `loft` binary.
//!
//! Tests the full check → seal → check loop and the corrupt-detection
//! path.  Mirrors the in-Rust tests at `tests/store_durable_tier1.rs`
//! but driven through `target/release/loft --interpret`, so the
//! `n_store_durable_check` / `n_store_durable_seal` native dispatch
//! and the loft-side `pub fn` declarations both get exercised.

#![cfg(feature = "mmap")]

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn smoke_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_durable_smoke.loft")
}

/// Create a fresh per-test scratch dir under TMPDIR (or `/tmp`).
fn scratch(test_name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = base.join("loft-store-durable-loft").join(test_name);
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run the smoke script with `LOFT_DURABLE_TEST_PATH=<main>` and
/// `LOFT_DURABLE_TEST_MODE=<mode>` set, returning stdout + the exit
/// code as `(stdout, code)`.
fn run_smoke(main: &Path, mode: &str) -> (String, i32) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(smoke_script())
        .env("LOFT_DURABLE_TEST_PATH", main)
        .env("LOFT_DURABLE_TEST_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let code = out.status.code().unwrap_or(-1);
    // Surface stderr on failure so a regression is easy to diagnose.
    if !out.status.success() {
        eprintln!("smoke stderr:\n{stderr}");
    }
    (stdout, code)
}

fn dmeta_path(main: &Path) -> PathBuf {
    let mut p = main.as_os_str().to_owned();
    p.push(".dmeta");
    PathBuf::from(p)
}

/// Write a deterministic main file (so CRCs are stable across runs).
fn write_main_file(path: &Path) {
    fa::write(path, b"some durable store payload content of length 48").unwrap();
}

// ── full lifecycle ──────────────────────────────────────────────────────────

#[test]
fn check_then_seal_then_recheck_loop() {
    let dir = scratch("check_then_seal_then_recheck_loop");
    let main = dir.join("data.store");
    write_main_file(&main);

    let (stdout, code) = run_smoke(&main, "initial");
    assert_eq!(code, 0, "expected exit 0; stdout={stdout:?}");
    assert!(
        stdout.contains("OK clean"),
        "expected 'OK clean', got: {stdout:?}"
    );

    // Sidecar must exist after the seal call.
    assert!(
        fa::exists(dmeta_path(&main)),
        "sidecar should exist after store_durable_seal"
    );
}

// ── corruption detection ────────────────────────────────────────────────────

#[test]
fn corrupted_sidecar_detected_via_loft_binding() {
    let dir = scratch("corrupted_sidecar_detected_via_loft_binding");
    let main = dir.join("data.store");
    write_main_file(&main);

    // First, run the initial flow to produce a valid sidecar.
    let (init_out, init_code) = run_smoke(&main, "initial");
    assert_eq!(init_code, 0, "initial run failed: {init_out:?}");
    assert!(init_out.contains("OK clean"));

    // XOR a byte in the sidecar's payload_crc field (offset 32-35).
    let meta = dmeta_path(&main);
    let mut bytes = fa::read(&meta).expect("read sidecar");
    bytes[32] ^= 0xFF;
    fa::write(&meta, bytes).expect("write corrupted sidecar");

    // Now the smoke script's corrupt path must observe check==false.
    let (stdout, code) = run_smoke(&main, "corrupt");
    assert_eq!(code, 0, "corrupt run exit code: {stdout:?}");
    assert!(
        stdout.contains("OK corrupt-detected"),
        "expected 'OK corrupt-detected', got: {stdout:?}"
    );
}
