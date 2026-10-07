// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The store-heap ceiling that test runs carry, and the report a refused growth prints.
//!
//! A corrupted length does not always end in a bad dereference — often it ends in an
//! allocation, and no time bound catches that: chasing loft#796 one run reached 59.6 GiB
//! in seconds and the kernel's OOM killer answered by killing two unrelated processes.
//! So the ceiling lives inside loft, where the thing being allocated still has a name.
//!
//! What is asserted here is the part a person acts on: that the run STOPS, that the
//! message names the TYPE that filled the heap, and that a well-behaved test in the same
//! file still passes. The exact byte figures are deliberately not asserted — they move
//! with the store's growth factor, and pinning them would make this a change-detector.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Write `body` to a temp `.loft` file and run it under `--tests`, returning stdout+stderr.
fn run_tests(name: &str, body: &str, limit: Option<&str>) -> String {
    let path = std::env::temp_dir().join(format!("loft_memlimit_{name}.loft"));
    fa::write(&path, body).expect("write probe");
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("--interpret").arg("--tests").arg(&path);
    // Bound the run: a ceiling that fails to fire must end the test, not the machine.
    cmd.env("LOFT_TIMEOUT", "120");
    if let Some(l) = limit {
        cmd.env("LOFT_MEMORY_LIMIT", l);
    }
    let out = cmd.output().expect("spawn loft");
    let _ = fa::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A vector that grows without bound — the shape a corrupted length produces.
const RUNAWAY: &str = r#"
struct Cell { const q: integer, m: integer }
fn test_ok() {
  v: vector<Cell> = [];
  for i in 0..10 { v += [Cell { q: i, m: 1 }] }
  assert(len(v) == 10, "ok test still runs");
}
fn test_runaway() {
  big: vector<Cell> = [];
  for i in 0..100000000 { big += [Cell { q: i, m: 2 }] }
  assert(len(big) > 0, "never reached");
}
"#;

#[test]
fn a_runaway_allocation_stops_at_the_ceiling_and_names_the_type() {
    let out = run_tests("runaway", RUNAWAY, Some("64M"));
    assert!(
        out.contains("store memory limit reached"),
        "the run must stop at the ceiling instead of eating the machine.\n{out}"
    );
    // The whole point of the report: WHICH type filled the heap.  Without this the
    // message is just a nicer OOM.
    assert!(
        out.contains("vector<Cell>"),
        "the report must name the type that filled the heap.\n{out}"
    );
    assert!(
        out.contains("where the memory is"),
        "the report must show the heap by type — one store vs many is what separates \
         a runaway length from a leak.\n{out}"
    );
    // A ceiling that fails the whole file would be a blunt instrument: the sibling
    // test is untouched and must still pass.
    assert!(
        out.contains("1 failed, 1 passed") || out.contains("1 passed"),
        "the well-behaved test in the same file must still pass.\n{out}"
    );
}

/// A modest allocation — far under the default ceiling, far over a 1 MiB one.
/// Both cells below use it, so "trips" and "does not trip" are the SAME program and
/// the only difference is the setting under test.
const MODEST: &str = r#"
struct Cell { const q: integer, m: integer }
fn test_modest() {
  v: vector<Cell> = [];
  for i in 0..200000 { v += [Cell { q: i, m: 1 }] }
  assert(len(v) == 200000, "modest allocation is fine");
}
"#;

#[test]
fn the_ceiling_can_be_turned_off() {
    // Paired against the same program at a ceiling it DOES cross, so this proves the
    // setting is what silenced it rather than the program being too small to trip.
    let tight = run_tests("off_tight", MODEST, Some("1M"));
    assert!(
        tight.contains("store memory limit reached"),
        "control: at 1M this program must cross the ceiling.\n{tight}"
    );
    let off = run_tests("off", MODEST, Some("0"));
    assert!(
        !off.contains("store memory limit reached"),
        "LOFT_MEMORY_LIMIT=0 must remove the ceiling.\n{off}"
    );
    assert!(off.contains("1 passed"), "and the test must pass.\n{off}");
}

#[test]
fn an_unreadable_limit_keeps_the_default_and_says_so() {
    // A typo must not silently REMOVE the limit — the one failure mode a memory
    // ceiling cannot have.  Asserted through the message rather than by allocating up
    // to the default, which would cost the suite tens of seconds to learn the same fact.
    let out = run_tests("typo", MODEST, Some("plenty"));
    assert!(
        out.contains("is not a size"),
        "an unparseable limit must be reported.\n{out}"
    );
    assert!(
        out.contains("keeping the default 2.0 GiB"),
        "and must fall back to the DEFAULT ceiling, not to none.\n{out}"
    );
}

#[test]
fn an_ordinary_test_run_is_not_disturbed_by_the_ceiling() {
    let out = run_tests("modest", MODEST, None);
    assert!(
        !out.contains("store memory limit reached"),
        "the default ceiling must not trip on an ordinary test.\n{out}"
    );
    assert!(out.contains("1 passed"), "the test must pass.\n{out}");
}
