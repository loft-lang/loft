// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-HandOver`'s rotate clause — the EMISSION pins.  An append of a copy to a vector,
//! then the drop of its oldest element past a depth K by a suffix-keep callee, is emitted as
//! ONE `stores.rotate_into` guarding S and the callee's `__nk` twin, with the statements as
//! written in its `else`; `LOFT_NO_HAND_OVER=1` emits no site.  The cell corpus
//! (`tests/scripts/158-rotate.loft`) says the VALUES hold on both backends, in both switch
//! states and under the falsifiers; this pins what is emitted.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/158-rotate.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_HAND_OVER");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn cells() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CELLS)
        .to_str()
        .unwrap()
        .to_string()
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    // One file per TEST: the tests run on threads of one process.
    let out = std::env::temp_dir().join(format!("loft_rotate_{}_{tag}.rs", std::process::id()));
    let out_s = out.to_str().unwrap().to_string();
    let status = loft(&["--native", "--native-emit", &out_s, &cells()], env);
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
    let c = cells();
    a.push(&c);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.trim_end().ends_with("rotate ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn the_cells_answer_the_interpreter_in_both_switch_states_under_the_falsifiers() {
    let oracle = run(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [None, Some(("LOFT_NO_HAND_OVER", "1"))] {
        {
            // Every detector at once: one build per switch state, each detector still on
            // (falsifying a guard is a one-time receipt, not a cost of every run).
            let mut env = vec![
                ("LOFT_STRICT_STORES", "1"),
                ("LOFT_POISON", "1"),
                ("LOFT_POISON_CLAIM", "1"),
                ("LOFT_NATIVE_LEAK_CHECK", "1"),
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
fn the_switch_emits_no_site() {
    let rust = emit("off", &[("LOFT_NO_HAND_OVER", "1")]);
    assert_eq!(rust.matches("//@FR-R-HandOver rotate").count(), 0);
    assert_eq!(rust.matches("__nk(").count(), 0);
}
