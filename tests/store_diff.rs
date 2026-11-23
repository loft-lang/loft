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

/// With every C-only rewrite off the two runtimes leave the same stores, which the comparison
/// sees only if it pairs each store with its twin: `an-append-twin` needs each C store named by
/// the line that made it (`LOFT_C_TRACE_SITES`), `a-result-read` two stores of one line paired
/// by what they hold and `store_memory()` counting the program's stores alone (STORE_MODEL D26),
/// `2177` a sidecar's `slot=` read as the slot of one store.  And a difference a rewrite makes
/// is attributed to it, padding apart: `37-stress`'s presized vector is `R-PushFill`'s.
///
/// @falsified-at: none — a SABOTAGE (2026-11-22): with the comparison's emit run without
///   `LOFT_C_TRACE_SITES`, `an-append-twin` reads `4 difference(s)` (two vector stores of lines
///   98 and 99 paired crosswise).  CHANNEL: the script's verdict.  WITNESS: no `: same (` line
///   for the program.
#[test]
fn the_store_differential_pairs_each_store_with_its_twin() {
    if !have_tools() {
        eprintln!("skipped: no cc or python3 on PATH");
        return;
    }
    let programs = [
        "tests/scripts/an-append-twin-writes-through-its-callers-window.loft",
        "tests/scripts/a-result-read-in-place-is-a-caller-buffer.loft",
        "tests/scripts/2177-a-malformed-image-answers-every-paged-load.loft",
    ];
    let mut args = vec!["--every-rewrite-off"];
    args.extend(programs);
    let Some(out) = store_diff(&args) else {
        eprintln!("skipped: no op-census build");
        return;
    };
    for p in programs {
        assert!(
            out.contains(&format!("{p}: same (")),
            "{p}: the runtimes' stores agree with every rewrite off:\n{out}"
        );
    }
    let attr = store_diff(&["--attribute", "tests/scripts/37-stress.loft"]).unwrap_or_default();
    assert!(
        attr.contains("the rewrite's: R-PushFill"),
        "37-stress's difference is R-PushFill's: {attr}"
    );
}
