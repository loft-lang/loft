// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN85 fuzz-proof gate — the STANDING JOB (done-criterion 1 of
//! `doc/claude/plans/85-store-lifetime-retirement/fuzz-proof-gate.md`).
//!
//! The generative harness (`fuzz/ownership_fuzz.py` + `fuzz/grammar_gen.py`)
//! proves the store-lifetime class closed BY CONSTRUCTION: 54 generated
//! programs over the ownership-composition grammar (9 shapes × 2 values ×
//! 3 churn), judged by the four-way oracle (crash / leak / cross-backend
//! divergence / clean exit).  This file wires it into `cargo test` so every
//! CI run re-proves it — the difference between *quiet* and *sealed*.
//!
//! Budget (recorded, no silent cap):
//! - every `cargo test` run: the self-test CONTROL PAIRS (2 cells × 2 configs,
//!   both backends — proves the detectors fire on the preserved gate-OFF bug
//!   and the default-gate fix holds) + the 54-cell interp+poison fast loop
//!   (crash + leak + poison-UAF channels; native runs only on a flagged cell).
//! - the release-gate sweep (`--ignored`): the full 54-cell map with
//!   `--poison --native-replay` — all four oracle channels on BOTH backends
//!   (rustc per cell on a cold cache):
//!   `cargo test --release --test ownership_fuzz_gate -- --ignored`
//!
//! Coverage is non-vacuous (done-criterion 2): the grammar's 9 shapes contain
//! every historical cluster shape — match_return (P14 / cluster C),
//! elem_accumulate (P10), local_source (the #462 conditional-local-view root),
//! the field-view family (clusters II/III), if_return (cluster V), index_read
//! (#426B), nested_field (P13) — and the self-test proves the detectors CAN
//! fire on exactly those shapes via the preserved `LOFT_NO_JOIN_OWN=1` path.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fuzz_dir() -> PathBuf {
    workspace_root().join("doc/claude/plans/85-store-lifetime-retirement/fuzz")
}

/// Run the harness with the given args; return (exit_ok, combined output).
fn run_harness(args: &[&str]) -> (bool, String) {
    let out = Command::new("python3")
        .arg(fuzz_dir().join("ownership_fuzz.py"))
        .args(args)
        .current_dir(workspace_root())
        .env("LOFT", loft_bin())
        .output()
        .expect("python3 must be runnable (the fuzz gate is a python harness)");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// The positive-control pairs — NOT ignored, per the differential-oracle
/// convention: a green sweep is only evidence once the detector is known to
/// fire.  Fails if (a) a detector channel is silent on the preserved gate-OFF
/// bug (vacuous harness), or (b) the default gate flags a control cell (the
/// join_own flip regressed).
// @speed 2.2
#[test]
fn fuzz_gate_positive_control_pairs() {
    let (ok, text) = run_harness(&["--self-test"]);
    assert!(
        ok,
        "fuzz-gate self-test failed — see the harness verdict:\n{text}"
    );
    assert!(
        text.contains("SELF-TEST PASS"),
        "expected an explicit SELF-TEST PASS:\n{text}"
    );
}

/// The 54-cell map, interp + poison fast loop — crash / leak / poison-UAF
/// channels on every `cargo test` run (native runs only when a cell flags, so
/// the clean path costs no rustc).  Zero findings expected.
// @speed 4.0
#[test]
fn fuzz_gate_map54_fast_loop() {
    let cells = tempfile_dir();
    generate_cells(&cells);
    let (ok, text) = run_harness(&["--corpus", cells.to_str().unwrap(), "--poison"]);
    assert!(
        ok && text.contains(" 0/54 flagged"),
        "54-cell interp+poison map is not clean:\n{text}"
    );
}

/// The release-gate sweep: the full map with native replay — all four oracle
/// channels on BOTH backends.  rustc per cell on a cold cache, so ignored by
/// default; run explicitly:
/// `cargo test --release --test ownership_fuzz_gate -- --ignored`
#[test]
#[ignore = "full both-backends replay (rustc per cell) — runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --test ownership_fuzz_gate fuzz_gate_map54_full_replay -- --ignored`"]
fn fuzz_gate_map54_full_replay() {
    let cells = tempfile_dir();
    generate_cells(&cells);
    let (ok, text) = run_harness(&[
        "--corpus",
        cells.to_str().unwrap(),
        "--poison",
        "--native-replay",
    ]);
    assert!(
        ok && text.contains(" 0/54 flagged"),
        "54-cell full-replay map is not clean:\n{text}"
    );
}

fn tempfile_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("loft_fuzz_gate_{}", std::process::id()));
    let _ = fa::remove_dir_all(&d);
    fa::create_dir_all(&d).expect("create cell dir");
    d
}

fn generate_cells(dir: &PathBuf) {
    let out = Command::new("python3")
        .arg(fuzz_dir().join("grammar_gen.py"))
        .arg("--out")
        .arg(dir)
        .current_dir(workspace_root())
        .output()
        .expect("python3 must be runnable");
    assert!(
        out.status.success(),
        "grammar_gen failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
