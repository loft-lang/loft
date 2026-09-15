// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-ai — the store reset of `@FR-H-ClearRelease`'s release arm re-establishes
//! the root vector at the CAPACITY the previous fill reached, not at the fresh-vector
//! minimum.  The buffer is reused across calls (`@FR-R-Reuse`) and the store already
//! holds the extent (`@FR-H-RootExtent`), so the growth ladder runs once per buffer
//! instead of once per call; each rung of that ladder freed a block into the store, and
//! one freed block took every later claim off `bump_tail` and into the free tree.
//!
//! The values are the language's either way, so the observable is the capacity itself:
//! `LOFT_TRACE_CLEAR=1` prints `[clear] reset store=N cap=C` at every reset.  The ON half
//! must show the ladder's rungs above the minimum; the OFF half (`LOFT_NO_RESET_CAPACITY=1`,
//! the pre-unit behaviour) must show the minimum at every reset — the positive control
//! that proves the trace can see the difference — and both halves must print the same
//! values.  The interpreter lane proves the contract for both backends: the arm is one
//! function both call, and `tests/scripts/157-reset-capacity.loft` runs on both through
//! the corpus.
use std::path::PathBuf;
use std::process::Command;

const SHAPE: &str = "tests/scripts/157-reset-capacity.loft";
const MINIMUM: u32 = 11;

fn run(unit_off: bool) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("--interpret")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SHAPE))
        .env("LOFT_TRACE_CLEAR", "1")
        .env("LOFT_STRICT_STORES", "1")
        .env("LOFT_TIMEOUT", "300");
    if unit_off {
        cmd.env("LOFT_NO_RESET_CAPACITY", "1");
    }
    let out = cmd.output().expect("spawn loft --interpret");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Every `cap=` a reset re-established, in order.
fn reset_caps(out: &str) -> Vec<u32> {
    out.lines()
        .filter(|l| l.starts_with("[clear] reset "))
        .filter_map(|l| l.split("cap=").nth(1))
        .filter_map(|c| c.trim().parse().ok())
        .collect()
}

#[test]
fn a_reset_buffer_keeps_the_capacity_it_reached() {
    let on = run(false);
    assert!(
        on.contains("reset-capacity ok"),
        "the cells must pass with the unit ON — got:\n{on}"
    );
    let caps = reset_caps(&on);
    assert!(
        !caps.is_empty(),
        "the trace must show at least one reset, or the shape never reaches the arm — got:\n{on}"
    );
    for rung in [24, 50, 102] {
        assert!(
            caps.contains(&rung),
            "a reset must re-establish the ladder's rung {rung} (a fill that reached it) — \
             re-established capacities were {caps:?}"
        );
    }

    // The falsifier: the switch restores the fresh minimum, and the trace sees it.
    let off = run(true);
    assert!(
        off.contains("reset-capacity ok"),
        "the cells must pass with the unit OFF too (the values are the language's) — got:\n{off}"
    );
    let off_caps = reset_caps(&off);
    assert_eq!(
        off_caps.len(),
        caps.len(),
        "both halves must reset the same number of times"
    );
    assert!(
        off_caps.iter().all(|&c| c == MINIMUM),
        "with LOFT_NO_RESET_CAPACITY=1 every reset must re-establish the minimum {MINIMUM} — \
         got {off_caps:?}"
    );
}
