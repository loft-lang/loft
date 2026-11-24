// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `(R-BoundedNest)`'s reduction clause — the EMISSION pins.  A counted loop that is one
//! integer accumulate of a scalar vector's elements, over a held header and base, runs the
//! plain block sum BEFORE the checked loop, which resumes where the first block declines;
//! `LOFT_NO_BOUNDED_SUM=1` restores the checked add on every element.  The cell corpus
//! (`tests/scripts/158-bounded-sum.loft`) says the VALUES hold on both backends, in every
//! switch state and under the falsifiers; this pins what is emitted, and for whom.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/158-bounded-sum.loft";

const SWITCHES: [&str; 5] = [
    "LOFT_NO_LOOP_KERNELS",
    "LOFT_NO_BOUNDED_SUM",
    "LOFT_NO_VECTOR_BASE",
    "LOFT_NO_VECTOR_HOIST",
    "LOFT_HOIST_VERIFY",
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args).env("LOFT_TIMEOUT", "300");
    for s in SWITCHES {
        cmd.env_remove(s);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    // One file per TEST: the tests run on threads of one process.
    let out =
        std::env::temp_dir().join(format!("loft_bounded_sum_{}_{tag}.rs", std::process::id()));
    let status = loft(
        &[
            "--native",
            "--native-emit",
            out.to_str().unwrap(),
            cells().to_str().unwrap(),
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

fn run(args: &[&str], env: &[(&str, &str)]) -> String {
    let mut a: Vec<&str> = args.to_vec();
    let s = cells().to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.trim_end().ends_with("bounded sum ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn the_cells_answer_the_interpreter_in_every_switch_state_under_the_falsifiers() {
    let oracle = run(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [
        None,
        Some(("LOFT_NO_BOUNDED_SUM", "1")),
        Some(("LOFT_NO_VECTOR_BASE", "1")),
        Some(("LOFT_NO_VECTOR_HOIST", "1")),
    ] {
        {
            // Every detector at once: one build per switch state, each detector still on
            // (falsifying a guard is a one-time receipt, not a cost of every run).
            let mut env = vec![
                ("LOFT_STRICT_STORES", "1"),
                ("LOFT_POISON", "1"),
                ("LOFT_NATIVE_LEAK_CHECK", "1"),
                ("LOFT_HOIST_VERIFY", "1"),
            ];
            env.extend(switch);
            let got = run(&["--native"], &env);
            assert_eq!(
                got, oracle,
                "native under {env:?} must print what the interpreter printed"
            );
        }
    }
}

#[test]
fn the_switch_restores_the_checked_loop_alone() {
    let rust = emit("off", &[("LOFT_NO_BOUNDED_SUM", "1")]);
    assert!(
        !rust.contains("sum_blocks_i64::<") && !rust.contains("FR-R-BoundedNest reduction"),
        "LOFT_NO_BOUNDED_SUM=1 emits no prelude anywhere"
    );
}
