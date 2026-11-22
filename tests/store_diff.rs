// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@PLN206 P4c` — `scripts/store_diff.py`, the byte comparison of the stores the C backend and
//! the interpreter leave: it reports a byte planted in the C's dump (its positive control), and
//! a program whose stores agree reads `same`.
//!
//! @falsified-at: none — the receipt is a SABOTAGE (2026-11-22): with `loft_database_wrapper`'s
//!   root type tag struck again (`src/c/loft_rt.h`, STORE_MODEL D47), the vector program reads
//!   `bytes 4..5 1400 in the interpreter, ffff in the C`.
//!   CHANNEL: the script's verdict.  WITNESS: "1 difference(s)".
use loft::platform::process::harness_command;

const VECTORS: &str = "tests/scripts/124-vector-amortised-growth.loft";

fn have_tools() -> bool {
    let ok = |p: &str| {
        harness_command(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    ok("cc") && ok("python3")
}

/// The script's verdict; `None` when the op-census interpreter it runs is not built.
fn store_diff(args: &[&str]) -> Option<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if !loft::file_access::exists(root.join("target/op-census/release/loft")) {
        return None;
    }
    let out = harness_command("python3")
        .arg("scripts/store_diff.py")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn the_store_differential_reports_its_plant_and_passes_agreeing_stores() {
    if !have_tools() {
        eprintln!("skipped: no cc or python3 on PATH");
        return;
    }
    let Some(same) = store_diff(&[VECTORS]) else {
        eprintln!(
            "skipped: no op-census build (cargo build --release --features op-census --target-dir target/op-census)"
        );
        return;
    };
    assert!(
        same.contains(": same ("),
        "the vector program's stores agree: {same}"
    );
    let plant = store_diff(&["--plant", VECTORS]).unwrap_or_default();
    assert!(
        plant.contains("the planted byte is reported"),
        "the differential sees a planted byte: {plant}"
    );
}
