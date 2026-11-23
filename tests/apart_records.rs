// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Apart` instance 4 — the EMISSION pins.  A record of record vectors built in a
//! frame and handed only to the functions that copy it into a collection is an `Apr<n>`
//! value: its owners declare `__ap_<v>`, its callees are emitted as `__apr` twins, and the
//! store meets it at `apart_write_fresh` or `rotate_from_parts`.  A driver or a callee that
//! reads an element keeps the store form; `LOFT_NO_APART_RECORDS=1` emits none.  The plan is
//! made only where no live tier ships (`--ship`), so every test runs that tier.  The
//! cell corpus (`tests/scripts/158-apart-records.loft`) says the VALUES hold; this pins what is
//! emitted.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/158-apart-records.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_APART_RECORDS")
        .env_remove("LOFT_NO_APART")
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
    let out = std::env::temp_dir().join(format!(
        "loft_apart_records_{}_{tag}.rs",
        std::process::id()
    ));
    let out_s = out.to_str().unwrap().to_string();
    let status = loft(
        &["--ship", "--native", "--native-emit", &out_s, &cells()],
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
    let c = cells();
    a.push(&c);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.trim_end().ends_with("apart records ok"),
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
        Some(("LOFT_NO_APART_RECORDS", "1")),
        Some(("LOFT_NO_HAND_OVER", "1")),
    ] {
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
            let got = run(&["--ship"], &env);
            assert_eq!(
                got, oracle,
                "native under {env:?} must print what the interpreter printed"
            );
        }
    }
}

#[test]
fn the_switch_emits_no_apart_record() {
    let rust = emit("off", &[("LOFT_NO_APART_RECORDS", "1")]);
    assert_eq!(rust.matches("__apr(").count(), 0);
    assert_eq!(rust.matches("let mut __ap_e: ").count(), 0);
}
