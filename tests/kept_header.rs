// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Header`'s KEPT clause — the EMISSION pins.  A vector field of a record parameter
//! that a chain of callees walks (`m.cs`) has one holder: bound at entry by the body that
//! reads it twice or more, handed down by `&mut` to each callee's `__kh` twin, and derived
//! again after an append to it and after a plain call that may change it.  A body that
//! appends inside a loop, or calls an appender inside an expression, keeps nothing.
//! `LOFT_NO_KEPT_HEADER=1` restores the per-walk derivation.  The cells
//! (`tests/scripts/a-kept-vector-field-follows-its-appends-across-calls.loft`) say the VALUES
//! hold on both backends, in both switch states and under the falsifiers; this pins what is
//! emitted.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/a-kept-vector-field-follows-its-appends-across-calls.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_KEPT_HEADER")
        .env_remove("LOFT_NO_VECTOR_HOIST")
        .env_remove("LOFT_HOIST_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // One file per TEST: the tests run on threads of one process.
    let out =
        std::env::temp_dir().join(format!("loft_kept_header_{}_{tag}.rs", std::process::id()));
    let status = loft(
        &[
            "--native",
            "--native-emit",
            out.to_str().unwrap(),
            src.to_str().unwrap(),
        ],
        env,
    );
    assert!(
        fa::exists(&out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = fa::read_to_string(&out).expect("read the emitted Rust");
    let _ = fa::remove_file(&out);
    rust
}

fn run_ok(args: &[&str], env: &[(&str, &str)]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut a: Vec<&str> = args.to_vec();
    let s = src.to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("g10 24"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// @C122 — the contract is what a program computes, so the clause may be on or off: every
/// cell answers the same in both switch states, on both backends, under the falsifiers.
#[test]
fn the_cells_hold_on_both_backends_in_both_switch_states_under_the_falsifiers() {
    run_ok(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [None, Some(("LOFT_NO_KEPT_HEADER", "1"))] {
        {
            // Every detector at once: one build per switch state, each detector still on
            // (falsifying a guard is a one-time receipt, not a cost of every run).
            let mut env = vec![
                ("LOFT_HOIST_VERIFY", "1"),
                ("LOFT_POISON", "1"),
                ("LOFT_STRICT_STORES", "1"),
                ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ];
            env.extend(switch);
            run_ok(&["--native"], &env);
        }
    }
}

#[test]
fn the_switch_restores_the_per_walk_derivation() {
    let rust = emit("off", &[("LOFT_NO_KEPT_HEADER", "1")]);
    assert!(
        !rust.contains("__kh"),
        "LOFT_NO_KEPT_HEADER=1 emits no holder and no kept twin"
    );
}
