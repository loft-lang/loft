// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-PushRec`'s heap clause and `@FR-R-Mint`'s rebound clause — the EMISSION pins.  A
//! heap-owning element's mint zeroes its slot (`push_record_hoisted_zero`), and a loop whose
//! body rebinds a mover to a fresh buffer or element slot hoists instead of declining.
//! `LOFT_NO_HEAP_RECORD_PUSH=1` and `LOFT_NO_REBOUND_MOVER=1` each restore one clause's
//! pre-form.  The rebound clause also reaches `(R-Scalar)`'s write-set walk: an emitter-owned
//! mint (a loop buffer's, a loop record's) and the element-first copy into a fresh element are
//! typed instead of declining every scalar of the loop.  The cell corpus
//! (`tests/scripts/157-group-push.loft`) says the VALUES hold on both backends, in every switch
//! state and under the falsifiers; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/157-group-push.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_HEAP_RECORD_PUSH")
        .env_remove("LOFT_NO_REBOUND_MOVER")
        .env_remove("LOFT_NO_RECORD_PUSH")
        .env_remove("LOFT_HOIST_VERIFY");
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
        out.status.success() && stdout.trim_end().ends_with("group push ok"),
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
        Some(("LOFT_NO_HEAP_RECORD_PUSH", "1")),
        Some(("LOFT_NO_REBOUND_MOVER", "1")),
        Some(("LOFT_NO_RECORD_PUSH", "1")),
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
