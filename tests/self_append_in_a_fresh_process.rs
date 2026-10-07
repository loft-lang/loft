// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The self-append fixture, run the way a USER runs it: the `loft` binary, a fresh process.
//!
//! `tests/scripts/1062-self-append-reallocation.loft` is the guard for loft#1062, and the
//! corpus runner executes it in-process — it builds a `State` inside the `wrap` test binary
//! and interprets there. That is enough for every assertion the file makes, and not enough
//! for the fault it exists to catch.
//!
//! The fault needs the allocator to actually MOVE the destination's block while a handle on
//! the old one is still live. Whether it moves is a property of the process heap at that
//! instant, not of the loft program: in `wrap` the heap already holds the parser, the corpus
//! and fifty-odd other tests, and the growth is served in place; in a fresh process it is
//! served by a move. So the same source answered `1062 ok` in the suite and SIGSEGV'd through
//! the binary, on the same commit — measured across three of them, with the DEBUG build of
//! each answering correctly and only `--release` faulting (loft#1216).
//!
//! A guard that cannot fail is not a guard, so this cell exists to run the file where the
//! condition can arise. It is deliberately a separate process rather than another `wrap` case:
//! the isolation IS the instrument, and moving this into the in-process runner would restore
//! exactly the blindness it was written for.
//!
//! It scores the EXIT STATUS, not the output. A use-after-free that happens not to fault still
//! leaves the wrong bytes behind, and the fixture's own assertions — which compare CONTENT, not
//! length — are what catch that; this cell adds the channel they cannot reach from inside.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

#[test]
fn self_append_past_the_reallocation_threshold_survives_a_fresh_process() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/1062-self-append-reallocation.loft");
    assert!(
        fa::is_file(&fixture),
        "fixture missing: {}",
        fixture.display()
    );

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(&fixture)
        .output()
        .expect("could not run the loft binary");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Name the signal when there is one: a bare "exit code 101" here reads as an ordinary
    // assertion failure, and this cell's whole point is telling a crash from a wrong answer.
    assert!(
        !stderr.contains("SIGSEGV") && !stdout.contains("SIGSEGV"),
        "self-append crashed in a fresh process — the loft#1062 use-after-free is back.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        out.status.success(),
        "self-append fixture failed in a fresh process ({}).\nstdout: {stdout}\nstderr: {stderr}",
        out.status
    );
}
