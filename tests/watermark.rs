// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Plan-57 Phase 5 — watermark regression guard (@P393).
//!
//! Last-use freeing (reclaim) is ON by default since Phase 5: a store-backed local
//! is freed at its data's last use, not held to scope exit.  These tests pin the
//! store high-water (`Stores::peak`) for the two shapes reclaim targets — sequential
//! reassignment of one local, and many distinct sequential locals — at the small
//! constant bound reclaim achieves.  Without reclaim each shape pins O(N) stores to
//! scope exit (peak grows with the number of bindings); with reclaim the stores
//! interleave through ~2 slots.  A regression that re-pins stores (peak back to O(N))
//! fails here loudly.
//!
//! Measured on the interpreter: 11× reassign peaks at 4 (was 13); 10 distinct locals
//! peak at 3 (was 12).  The bound is set with slack but well below O(N) so the guard
//! catches a re-pinning regression without being brittle.

use loft::compile::byte_code;
use loft::file_access as fa;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;

mod common;
use common::cached_default;

/// Parse + execute `fn test()` and return the store high-water mark (`Stores::peak`).
fn peak_for(code: &str) -> u16 {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(code, "watermark_test", false);
    // Errors only: these programs overwrite a local on purpose (the reclaim they measure),
    // which the dead-assignment lint rightly reports.
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("test", &p.data);
    state.database.peak
}

/// One local reassigned 11× (cluster III straight-line).  Each overwrite's store is
/// dead at the next assignment; reclaim frees it before the next allocates, so the
/// 11 stores cycle through ~2 slots instead of stacking to scope exit.
#[test]
fn reassign_watermark_bounded_by_reclaim() {
    let peak = peak_for(
        r#"
pub fn test() {
  v = [0, 1];
  v = [1, 2];
  v = [2, 3];
  v = [3, 4];
  v = [4, 5];
  v = [5, 6];
  v = [6, 7];
  v = [7, 8];
  v = [8, 9];
  v = [9, 10];
  v = [10, 11];
  total = v[0];
  assert(total == 10, "total={total}");
}
"#,
    );
    assert!(
        peak <= 6,
        "11x reassign store peak {peak} > 6 — reclaim regression (overwritten stores re-pinned to scope exit; un-reclaimed peak is ~13)"
    );
}

/// Many distinct sequential locals, each read once then dead (cluster I-b).  Reclaim
/// frees each before the next allocates, so the watermark stays constant instead of
/// climbing with the number of locals.
#[test]
fn distinct_locals_watermark_bounded_by_reclaim() {
    let peak = peak_for(
        r#"
pub fn test() {
  total = 0;
  a0: vector<integer> = [0, 1]; total += a0[0];
  a1: vector<integer> = [1, 2]; total += a1[0];
  a2: vector<integer> = [2, 3]; total += a2[0];
  a3: vector<integer> = [3, 4]; total += a3[0];
  a4: vector<integer> = [4, 5]; total += a4[0];
  a5: vector<integer> = [5, 6]; total += a5[0];
  a6: vector<integer> = [6, 7]; total += a6[0];
  a7: vector<integer> = [7, 8]; total += a7[0];
  a8: vector<integer> = [8, 9]; total += a8[0];
  a9: vector<integer> = [9, 10]; total += a9[0];
  assert(total == 45, "total={total}");
}
"#,
    );
    assert!(
        peak <= 6,
        "10 distinct-locals store peak {peak} > 6 — reclaim regression (locals pinned to scope exit; un-reclaimed peak is ~12)"
    );
}

/// Positive control for the Plan-57 **Phase-4 Goal-E guard** — `scopes::check`'s
/// `reclaim_unfreed_eligible == 0` assertion.  This is the *live* store guard (the
/// `[store-guard]` eprintln is superseded by it, scopes.rs:312), and it is what
/// Goal E's "`LOFT_STORE_GUARD` silent corpus-wide" check and the N5 mixed-path
/// store assertion (`tests/n3_parity.rs`) both lean on.
///
/// A detector whose *silence* is treated as evidence MUST be shown to fire, or its
/// silence is evidence of nothing (engineering-rigor: silence-is-evidence-only-
/// after-a-positive-control).  `LOFT_STORE_GUARD_INJECT` is a test-only fault that
/// skips the early-free while keeping the guard armed — a simulated reclaim
/// regression — so a reclaim-active program (11× reassign: every overwrite's store
/// is dead at the next assignment) MUST trip the assertion.  Without the fault the
/// same program is silent (reclaim frees the dead stores): the negative control.
/// Spawned as a subprocess so the env fault can never leak into sibling tests.
#[test]
fn phase4_goal_e_guard_is_falsifiable() {
    let prog = "fn main() {\n\
        \x20 v = [0,1]; v = [1,2]; v = [2,3]; v = [3,4]; v = [4,5]; v = [5,6];\n\
        \x20 v = [6,7]; v = [7,8]; v = [8,9]; v = [9,10]; v = [10,11];\n\
        \x20 println(\"{v[0]}\");\n\
        }\n";
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("loft_phase4_pc_{pid}.loft"));
    fa::write(&path, prog).unwrap();

    let run = |inject: bool| {
        let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
        cmd.arg("--interpret")
            .arg(&path)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_STORE_GUARD", "1"); // arm the guard in a release binary too
        if inject {
            cmd.env("LOFT_STORE_GUARD_INJECT", "1");
        }
        cmd.output().expect("spawn loft binary")
    };

    // Negative control: reclaim works → guard silent → clean exit.
    let base = run(false);
    assert!(
        base.status.success(),
        "baseline (no inject) must be clean, but the guard fired:\n{}",
        String::from_utf8_lossy(&base.stderr)
    );

    // Positive control: the fault leaves eligible stores live-but-dead → guard fires.
    let inj = run(true);
    let err = String::from_utf8_lossy(&inj.stderr);
    assert!(
        !inj.status.success(),
        "INJECT must trip the Phase-4 Goal-E guard, but the program exited cleanly \
         — the guard is unfalsifiable / dead"
    );
    assert!(
        err.contains("plan-57 Phase 4") && err.contains("reclaim-eligible"),
        "INJECT must panic with the Phase-4 Goal-E message, got:\n{err}"
    );

    let _ = fa::remove_file(&path);
}
