// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 / `@FR-H-ClearRelease` — a REUSED vector's entry clear releases what its
//! elements own.  `vector::clear_vector` is a length reset, sound where the vector's
//! store dies right after and unsound for the shape-A return buffer the ABI reuses
//! across calls: each clear stranded the previous elements' owned heap in a store that
//! never dies (~357 KB per `fronds` call, unbounded — measured on both architectures).
//!
//! Self-falsifying: the SAME shape under a small store-heap ceiling PASSES with the
//! release on and TRIPS the ceiling with `LOFT_NO_CLEAR_RELEASE=1` — so the test proves
//! both that the leak is closed and that the guard can still see it.
use std::path::PathBuf;
use std::process::Command;

const SHAPE: &str = "tests/scripts/157-clear-release.loft";

fn run(release_off: bool) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    // The interpreter lane: its store-heap ceiling is what SEES this leak (the native
    // `--tests` ceiling does not track the generated program's stores), and the fix is
    // one function both backends call, so the interpreter proves the contract for both.
    cmd.arg("--interpret")
        .arg("--tests")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SHAPE))
        .env("LOFT_MEMORY_LIMIT", "64M")
        .env("LOFT_TIMEOUT", "300");
    if release_off {
        cmd.env("LOFT_NO_CLEAR_RELEASE", "1");
    }
    cmd.output().expect("spawn loft --tests")
}

#[test]
fn a_reused_buffer_releases_its_heap_owning_elements() {
    // Release ON: the heap is freed each call, the ceiling is never approached.
    let on = run(false);
    let on_out = format!(
        "{}{}",
        String::from_utf8_lossy(&on.stdout),
        String::from_utf8_lossy(&on.stderr)
    );
    assert!(
        on_out.contains("1 passed") && !on_out.contains("store memory limit"),
        "release ON must pass under the 64M ceiling — got:\n{on_out}"
    );
    // Release OFF (the pre-fix reset): the same shape strands its elements' heap and
    // crosses the ceiling — the leak the fix closes, still visible to the guard.
    let off = run(true);
    let off_out = format!(
        "{}{}",
        String::from_utf8_lossy(&off.stdout),
        String::from_utf8_lossy(&off.stderr)
    );
    assert!(
        off_out.contains("store memory limit"),
        "release OFF must strand the heap and trip the ceiling (the falsifier) — got:\n{off_out}"
    );
}
