// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1512 and loft#1513 — two release-machinery defects around a construction's
//! work-ref, plus the poison arming for their neighbour guards.
//!
//! loft#1512 leaked whole (a tuple-literal ARGUMENT's call-minted member had no release
//! site at all), so its guard fails on VALUE in a plain run and the leak channel pins the
//! cheap non-fix.  loft#1513 was the opposite kind of invisible: every count and value was
//! right in a plain run — the scope-end hook read a store the work-ref's bare free had
//! already released, which only `LOFT_POISON=1` turns into a wrong answer.  So the poison
//! runs here ARE that guard's own channel, per test, because the scripts corpus does not
//! run under poison and nothing else would.
//!
//! The 1506 and 1511 guards are the fixed neighbours in the same family (D-heap-3/D-heap-4,
//! `formal/heap.md`); their plain runs live in the scripts corpus, and the poison runs here
//! keep the whole family's memory channel armed — the adopt-base attempt (D-heap-3
//! exclusion 3) is the measured case of a cure that passes every count cell while reading
//! freed memory.
//!
//! Every run gets a UNIQUE cwd with `LOFT_PATHS=cwd`: the guards trace through a relative
//! `loft_*_trace.tmp`, which is program-relative by default — two concurrent runs of one
//! script (this harness beside the corpus, value beside poison) would interleave one file.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn script(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts")
        .join(name)
}

const I1512: &str = "1512-a-tuple-argument-call-member-releases-once.loft";
const I1512_OK: &str = "1512 tuple-argument call member OK";
const I1513: &str = "1513-a-rebound-adoption-hooks-live-memory.loft";
const I1513_OK: &str = "1513 rebound adoption hooks live memory OK";
const I1506: &str = "1506-a-call-result-projection-releases-once.loft";
const I1506_OK: &str = "1506 OK";
const I1511: &str = "1511-a-call-result-in-a-tuple-member-releases-once.loft";
// The 1511 guard prints nothing on success — its verdict is the exit code alone.
const I1511_OK: &str = "";

/// Run `file` on `backend` with extra env in a unique cwd; return `(ok, stdout, stderr)`.
fn run(backend: &str, file: &PathBuf, env: &[(&str, &str)], tag: &str) -> (bool, String, String) {
    let cwd = std::env::temp_dir().join(format!(
        "loft_1512_1513_{}_{}_{}",
        file.file_stem().and_then(|s| s.to_str()).unwrap_or("g"),
        backend.trim_start_matches('-'),
        tag
    ));
    std::fs::create_dir_all(&cwd).expect("create per-run cwd");
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(file)
        .current_dir(&cwd)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_PATHS", "cwd");
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

fn assert_green(name: &str, ok_line: &str, backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, &script(name), env, tag);
    assert!(
        ok && stdout.contains(ok_line),
        "[{backend}/{tag}] every cell of {name} must be green\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// The leak channel: "never release it" passes every hook-count cell, so the count cells
/// alone cannot exclude it.  `LOFT_NATIVE_LEAK_CHECK` reports retained stores on exit.
fn assert_leak_free(name: &str, ok_line: &str) {
    let (ok, stdout, stderr) = run(
        "--native",
        &script(name),
        &[("LOFT_NATIVE_LEAK_CHECK", "1")],
        "leak",
    );
    assert!(
        ok && stdout.contains(ok_line),
        "[--native/leak] {name} must run green\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("stores not freed"),
        "[--native/leak] {name} must exit with no retained stores\nstderr:\n{stderr}"
    );
}

// ── loft#1512 — a tuple-literal argument's call-minted member ─────────────────────────

#[test]
fn tuple_argument_cells_interpret() {
    assert_green(I1512, I1512_OK, "--interpret", &[], "value");
}

#[test]
fn tuple_argument_cells_native() {
    assert_green(I1512, I1512_OK, "--native", &[], "value");
}

#[test]
fn tuple_argument_cells_poison_interpret() {
    assert_green(
        I1512,
        I1512_OK,
        "--interpret",
        &[("LOFT_POISON", "1")],
        "poison",
    );
}

#[test]
fn tuple_argument_cells_poison_native() {
    assert_green(
        I1512,
        I1512_OK,
        "--native",
        &[("LOFT_POISON", "1")],
        "poison",
    );
}

/// The filed channel: the record leaked WHOLE, so a fix that only silences the hook count
/// (or a revert that trades the double claim back for the leak) fails here.
#[test]
fn tuple_argument_stays_leak_free() {
    assert_leak_free(I1512, I1512_OK);
}

// ── loft#1513 — the rebound adoption's scope-end hook ─────────────────────────────────

#[test]
fn rebound_adoption_cells_interpret() {
    assert_green(I1513, I1513_OK, "--interpret", &[], "value");
}

#[test]
fn rebound_adoption_cells_native() {
    assert_green(I1513, I1513_OK, "--native", &[], "value");
}

/// The guard's OWN channel: the plain run passed on the broken build (counts and values
/// right off stale bytes); under poison the scope-end hook noted the poison pattern and the
/// sequence assertions failed, on both backends.
#[test]
fn rebound_adoption_cells_poison_interpret() {
    assert_green(
        I1513,
        I1513_OK,
        "--interpret",
        &[("LOFT_POISON", "1")],
        "poison",
    );
}

#[test]
fn rebound_adoption_cells_poison_native() {
    assert_green(
        I1513,
        I1513_OK,
        "--native",
        &[("LOFT_POISON", "1")],
        "poison",
    );
}

/// The overshoot direction: disarming the work-ref where nothing adopted its store would
/// read as a fix on the poison channel while retaining every record.
#[test]
fn rebound_adoption_stays_leak_free() {
    assert_leak_free(I1513, I1513_OK);
}

// ── the family's neighbour guards, poison-armed ───────────────────────────────────────

#[test]
fn projection_guard_poison_interpret() {
    assert_green(
        I1506,
        I1506_OK,
        "--interpret",
        &[("LOFT_POISON", "1")],
        "poison",
    );
}

#[test]
fn projection_guard_poison_native() {
    assert_green(
        I1506,
        I1506_OK,
        "--native",
        &[("LOFT_POISON", "1")],
        "poison",
    );
}

#[test]
fn tuple_member_guard_poison_interpret() {
    assert_green(
        I1511,
        I1511_OK,
        "--interpret",
        &[("LOFT_POISON", "1")],
        "poison",
    );
}

#[test]
fn tuple_member_guard_poison_native() {
    assert_green(
        I1511,
        I1511_OK,
        "--native",
        &[("LOFT_POISON", "1")],
        "poison",
    );
}
