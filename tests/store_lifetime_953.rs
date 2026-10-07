// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#953 — a copy may not claim the return buffer its CALLER owns.
//!
//! A heap-returning callee is handed a hidden destination argument: the caller mints a `__ref_N`
//! work-ref, the callee fills it, and the value it hands back IS that buffer. That work-ref is a
//! caller FUNCTION-scope variable with its own scope-exit `OpFreeRef`, so the store is already
//! spoken for. `OpCopyRecord`'s `0x8000` free-source bit claimed it anyway
//! (`Parser::answers_caller_buffer` is the cut), releasing the buffer once per call while the
//! caller still held it.
//!
//! **Why this file exists beside the script.** `tests/scripts/953-…loft` asserts values, and
//! values catch only the cells where something RECYCLED the freed store number. Hold the loop and
//! the append fixed and vary only the callee, and every variant commits the same use-after-free —
//! but a callee with no second vector local, or with the second one declared first, or with a
//! `text` second local, reads back perfectly because nothing took the number. Those three were
//! filed as the boundary of the bug and were not: they were the same defect, silent. The oracle
//! that separates "correct" from "got away with it" is `LOFT_STRICT_STORES=1`, so this file runs
//! the script under it.
//!
//! The A/B is on ONE binary: `LOFT_NO_RETBUF_CLAIM_GUARD=1` restores the claim, which is both the
//! before-half here and the first bisect step for a leak in a heap-returning call chain.

use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test binary path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("loft")
}

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/953-call-result-into-nested-vector.loft")
}

/// Run the script under the strict-store oracle, with the guard on or off.
fn run_strict(guard: bool) -> String {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("--interpret")
        .arg(script())
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_STRICT_STORES", "1");
    if !guard {
        cmd.env("LOFT_NO_RETBUF_CLAIM_GUARD", "1");
    }
    let out = cmd.output().expect("invoke loft");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Every cell of the matrix, values AND store lifetime. `NEVER FREED` is checked too: strict mode
/// asks that every store be freed exactly ONCE, so declining to claim the buffer has to leave it
/// freed by the caller's scope — a fix that traded the use-after-free for a leak would pass a
/// value-only gate.
#[test]
fn call_result_into_a_nested_vector_leaves_the_buffer_alone() {
    let all = run_strict(true);
    assert!(all.contains("953 ok"), "values are wrong:\n{all}");
    assert!(
        !all.contains("USE AFTER FREE"),
        "the copy still claims the caller's return buffer\n{all}"
    );
    assert!(
        !all.contains("NEVER FREED"),
        "the buffer is no longer claimed but nothing frees it either\n{all}"
    );
    assert!(
        !all.contains("[strict-store]"),
        "strict-store reported a violation\n{all}"
    );
}

/// The before-half, on the same binary. Without it a green run above is not evidence that the
/// oracle can fail — the whole point of this file is the three cells whose VALUES never moved.
#[test]
fn without_the_guard_the_oracle_still_fires() {
    let all = run_strict(false);
    assert!(
        all.contains("USE AFTER FREE"),
        "LOFT_NO_RETBUF_CLAIM_GUARD no longer restores the claim — this test can no longer \
         fail, so the green one above proves nothing:\n{all}"
    );
}
