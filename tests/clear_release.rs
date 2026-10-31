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

const SHAPE: &str = "tests/scripts/157-clear-release.loft";

fn run(release_off: bool) -> std::process::Output {
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
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

/// STORE_MODEL.md D19f (@PLN206 P1) — clearing a vector that is already empty does nothing: no
/// store reset, no record made again, as the C's `loft_vec_clear_release` answers it.  A function
/// that keeps answering an empty vector clears its pooled return buffer (`@FR-R-Reuse`) while it
/// is still empty, and the interpreter reset that buffer's store on every call — measured on
/// 2 000 000 calls of `make(0)`: 8.96e9 instructions before, 5.17e9 with the early exit.
/// `LOFT_TRACE_CLEAR` names every clear that reaches the release; none may.  Falsified: without
/// the early exit the trace prints `[clear] reset store=… cap=0` per call.
#[test]
fn clearing_an_empty_vector_resets_no_store() {
    let dir = std::env::temp_dir().join(format!("loft-clear-empty-{}", std::process::id()));
    loft::file_access::create_dir_all(&dir).expect("scratch");
    let file = dir.join("buf.loft");
    loft::file_access::write(
        &file,
        "struct Heavy { hv: vector<float> }\nfn make(k: integer) -> vector<Heavy> {\n  \
         mk_out: vector<Heavy> = [];\n  for i in 0..k { mk_out += [Heavy { hv: [i as float] }]; }\n  \
         mk_out\n}\nfn main() {\n  n = 0;\n  for _ in 0..3 { r = make(0); n += len(r); }\n  \
         r = make(2);\n  println(\"{n} {len(r)} {r[1].hv[0]}\");\n}\n",
    )
    .expect("write program");
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(&file)
        .env("LOFT_TRACE_CLEAR", "1")
        .env("LOFT_TIMEOUT", "60")
        .output()
        .expect("spawn loft");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "0 2 1\n",
        "the answer:\n{err}"
    );
    assert!(
        !err.contains("[clear] reset store="),
        "an empty buffer's clear reset its store:\n{err}"
    );
    let _ = loft::file_access::remove_dir_all(&dir);
}
