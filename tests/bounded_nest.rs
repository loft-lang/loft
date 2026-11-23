// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-BoundedNest` — the EMISSION pins.  An innermost counted loop that accumulates
//! `?`-discharged element products gets a guard (`__nb_N`) and a plain-operator arm, with
//! the checked loop as its `else`; the element bounds (`__bd_N`) are derived at the
//! outermost loop whose body leaves the vector alone, never at the nest's own prelude; a
//! loop whose body writes the vector holds no bound and the nest under it declines;
//! `LOFT_NO_BOUNDED_NEST=1` restores the checked form everywhere.  Step 2: where every read's
//! index chain is affine in the counter the guard also proves both range ends in `[0, len)`
//! and the arm reads RAW through the held base with no null select; `LOFT_NO_NEST_RAW_READS=1`
//! keeps step 1's bounds-tested arm.  The cell corpus
//! (`tests/scripts/157-bounded-nest.loft`) says the VALUES hold on both backends, in both
//! switch states and under `LOFT_HOIST_VERIFY=1`; this pins what is emitted and runs it.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/157-bounded-nest.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    // These counts pin ONE copy of a loop; `@FR-R-FaultFlag` emits a statement it
    // admits twice (a fast copy and the exact one).
    cmd.env("LOFT_NO_FAULT_FLAG", "1");
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_BOUNDED_NEST")
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
        std::env::temp_dir().join(format!("loft_bounded_nest_{}_{tag}.rs", std::process::id()));
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
        out.status.success() && stdout.trim_end().ends_with("ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_switch_restores_the_checked_form_everywhere() {
    let rust = emit("off", &[("LOFT_NO_BOUNDED_NEST", "1")]);
    assert!(!rust.contains("__nb_"), "a guard survived the switch");
    assert!(
        !rust.contains("abs_bound_i64"),
        "a bound survived the switch"
    );
    assert!(
        !rust.contains("plain nest"),
        "a plain arm survived the switch"
    );
}

#[test]
fn the_values_hold_on_both_backends_in_both_switch_states_and_under_verify() {
    run_ok(&["--interpret"], &[]);
    run_ok(&["--ship"], &[]);
    run_ok(&["--native"], &[("LOFT_NO_BOUNDED_NEST", "1")]);
    run_ok(&["--native"], &[("LOFT_NO_NEST_RAW_READS", "1")]);
    run_ok(
        &["--native"],
        &[
            ("LOFT_HOIST_VERIFY", "1"),
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            // A raw read one past the end lands in the claim's zeroed slack and would answer
            // the checked read's 0 by luck; poisoned, n19 can only pass through the checked loop.
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
        ],
    );
}
