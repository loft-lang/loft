// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RecPtr` — the EMISSION pins.  A plain-record VIEW (`e = tbl[i]?`, `s = o.inner`)
//! carries its address (`__pa_N`) for the rest of its block: every scalar field read is
//! `vector::rec_get` through it, every in-place field write `vector::rec_set`, and a callee
//! twin's scalar inputs are read through it at the call.  A view the remainder rebinds, a
//! remainder that grows a store or frees a record, and a nullable view all decline.
//! `LOFT_NO_RECORD_PTR=1` (and `LOFT_NO_VECTOR_BASE=1`, one rule) restores the store reads.
//! The cell corpus (`tests/scripts/157-record-ptr.loft`) says the VALUES hold on both
//! backends, in every switch state and under the falsifiers; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/157-record-ptr.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    // These counts pin ONE copy of a loop; `@FR-R-FaultFlag` emits a statement it
    // admits twice (a fast copy and the exact one).
    cmd.env("LOFT_NO_FAULT_FLAG", "1");
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_RECORD_PTR")
        .env_remove("LOFT_NO_VECTOR_BASE")
        .env_remove("LOFT_HOIST_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn run_ok(args: &[&str], env: &[(&str, &str)]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut a: Vec<&str> = args.to_vec();
    let s = src.to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// @C122 — the contract is what a program computes, so a rewrite may be on or off: every cell
/// answers the same in every switch state, on both backends, under the falsifiers.
#[test]
fn the_cells_hold_on_both_backends_in_every_switch_state_under_the_falsifiers() {
    run_ok(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [
        None,
        Some(("LOFT_NO_RECORD_PTR", "1")),
        Some(("LOFT_NO_VECTOR_BASE", "1")),
    ] {
        {
            // Every detector at once: one build per switch state, each detector still on
            // (falsifying a guard is a one-time receipt, not a cost of every run).
            let mut env = vec![
                ("LOFT_STRICT_STORES", "1"),
                ("LOFT_POISON", "1"),
                ("LOFT_POISON_CLAIM", "1"),
                ("LOFT_NATIVE_LEAK_CHECK", "1"),
                ("LOFT_HOIST_VERIFY", "1"),
            ];
            env.extend(switch);
            run_ok(&["--native"], &env);
        }
    }
}

/// A view's VECTOR field read: the cells.
const NESTED: &str = "tests/scripts/a-vector-field-of-a-view-reads-through-its-address.loft";

/// The nested cells answer their hand-computed values on both backends, under the
/// falsifiers.
#[test]
fn the_nested_cells_hold_on_both_backends_and_under_the_falsifiers() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(NESTED);
    let src = src.to_str().unwrap();
    for (backend, env) in [
        ("--interpret", vec![]),
        ("--native", vec![("LOFT_HOIST_VERIFY", "1")]),
        ("--native", vec![("LOFT_POISON", "1")]),
    ] {
        let out = loft(&[backend, src], &env);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("n5 6 57"),
            "{backend} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
