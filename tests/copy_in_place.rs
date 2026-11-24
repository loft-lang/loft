// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-InPlace`'s copy clause — the EMISSION pins.  A flag-free copy of a record that
//! owns no heap is an in-place write, so a loop that holds one keeps its headers, its bases
//! and its record addresses — the write-back idiom `e = v[i]?; …; v[i] = e`.
//! `LOFT_NO_COPY_IN_PLACE=1` makes the copy a store writer again.  The cell corpus
//! (`tests/scripts/158-copy-in-place.loft`) says the VALUES hold on both backends, in every
//! switch state and under the falsifiers; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/158-copy-in-place.loft";

const SWITCHES: [&str; 4] = [
    "LOFT_NO_COPY_IN_PLACE",
    "LOFT_NO_RECORD_PTR",
    "LOFT_NO_SCALAR_HOIST",
    "LOFT_HOIST_VERIFY",
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    // These counts pin ONE copy of a loop; `@FR-R-FaultFlag` emits a statement it
    // admits twice (a fast copy and the exact one).
    cmd.env("LOFT_NO_FAULT_FLAG", "1");
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

fn run(args: &[&str], env: &[(&str, &str)]) -> String {
    let mut a: Vec<&str> = args.to_vec();
    let s = cells().to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.trim_end().ends_with("copy in place ok"),
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
        Some(("LOFT_NO_COPY_IN_PLACE", "1")),
        Some(("LOFT_NO_RECORD_PTR", "1")),
        Some(("LOFT_NO_SCALAR_HOIST", "1")),
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
            let got = run(&["--native"], &env);
            assert_eq!(
                got, oracle,
                "native under {env:?} must print what the interpreter printed"
            );
        }
    }
}
