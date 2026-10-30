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

const SHAPE: &str = "tests/scripts/157-reset-capacity.loft";
/// The START room of `@FR-Col-Capacity` for the shape's 8-byte elements: 88 bytes, eleven
/// elements — what a fresh vector's first push takes (`vector::grown_elems(1, 8)`).
const MINIMUM: u32 = 11;

fn run(unit_off: bool) -> String {
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
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

// ── The empty clear (STORE_MODEL D19f) ──
//
// STORE_MODEL D19f — clearing a vector that holds nothing is nothing, a store-root vector's
// included: the interpreter does not reset the store, as the C runtime's
// `loft_vec_clear_release` does not (STORE_MODEL's rule: the C model is the standard;
// `doc/claude/plans/91-self-host/1b-reference.md` E-3).
// The values are the language's either way, so the observable is the reset itself:
// `LOFT_TRACE_CLEAR=1` prints `[clear] reset store=N cap=C` at every reset.  A buffer minted
// fresh for each call is cleared empty at the callee's entry; with the skip ON no reset of an
// empty vector (`cap=0`) is made, while the reset of a reused buffer that holds the previous
// call's elements still is.  The OFF half (`LOFT_NO_EMPTY_CLEAR_SKIP=1`, the behaviour
// before) is the positive control: the empty resets reappear.  Both halves print the same
// values.  The interpreter lane proves the contract for both Rust backends: the clear is one
// function both call.
const EMPTY_CLEAR_PROGRAM: &str = "struct Leg { a: integer, name: text }

fn legs(n: integer) -> vector<Leg> {
  out: vector<Leg> = [];
  for i in 0..n {
    out += [Leg { a: i, name: \"l{i}\" }];
  }
  out
}

fn main() {
  t = 0;
  for k in 1..4 {
    v = legs(k);
    t += len(v) * 10 + (v[k - 1].a ?? 0);
  }
  assert(t == 63, \"t = {t}\");
  println(\"empty-clear ok\");
}
";

fn run_empty_clear(skip_off: bool) -> String {
    let dir = std::env::temp_dir().join(format!("loft_empty_clear_{}", std::process::id()));
    loft::file_access::create_dir_all(&dir).expect("scratch directory");
    let file = dir.join("empty_clear.loft");
    loft::file_access::write(&file, EMPTY_CLEAR_PROGRAM).expect("write the program");
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("--interpret")
        .arg(&file)
        .env("LOFT_TRACE_CLEAR", "1")
        .env("LOFT_STRICT_STORES", "1")
        .env("LOFT_TIMEOUT", "120");
    if skip_off {
        cmd.env("LOFT_NO_EMPTY_CLEAR_SKIP", "1");
    }
    let out = cmd.output().expect("spawn loft --interpret");
    let _ = loft::file_access::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn an_empty_vector_clears_without_a_store_reset() {
    let on = run_empty_clear(false);
    assert!(
        on.contains("empty-clear ok"),
        "the program passes with the skip ON — got:\n{on}"
    );
    let caps = reset_caps(&on);
    assert!(
        !caps.is_empty() && caps.iter().all(|&c| c > 0),
        "a reused buffer that holds elements is still reset, and no empty vector is: \
         re-established capacities were {caps:?}"
    );

    // The falsifier: the switch restores the reset of an empty vector, and the trace sees it.
    let off = run_empty_clear(true);
    assert!(
        off.contains("empty-clear ok"),
        "the program passes with the skip OFF — got:\n{off}"
    );
    let off_caps = reset_caps(&off);
    assert!(
        off_caps.contains(&0) && off_caps.len() > caps.len(),
        "with the skip OFF an empty vector's clear resets its store (cap=0): {off_caps:?} \
         against {caps:?}"
    );
}
