// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RecPtr`'s order clause — the EMISSION pins.  A record free on a path that leaves
//! the function is not a free before a use of the view, so the loop variable of a
//! find-and-answer helper (`for c in m.chunks { if … { return c.hexes[i]?.h } }`) holds its
//! address.  The order walk itself is unit-tested where it lives
//! (`hoist::free_order_tests`); the cell corpus (`tests/scripts/158-leaving-free.loft`)
//! says the VALUES hold on both backends, in every switch state and under the falsifiers;
//! this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/158-leaving-free.loft";

/// Per function: the record addresses bound.
const EXPECTED: &[(&str, usize)] = &[
    ("n_q1_get", 1),
    // A record literal in the extent is a store growth: declined before the order rule.
    ("n_q2", 0),
    ("n_q3", 0),
    ("n_q4_get", 1),
    ("n_q5_get", 1),
    // Both loop variables — and again in the twin a hoisting caller reaches it through.
    ("n_q6_find", 2),
    ("n_q6_find__inv", 2),
    ("n_q7", 0),
    ("n_q8_pick", 1),
    ("n_q9_get", 1),
];

const SWITCHES: [&str; 3] = [
    "LOFT_NO_RECORD_PTR",
    "LOFT_NO_BASE_RECPTR",
    "LOFT_HOIST_VERIFY",
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
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
        std::env::temp_dir().join(format!("loft_leaving_free_{}_{tag}.rs", std::process::id()));
    let status = loft(
        &[
            "--native-emit",
            out.to_str().unwrap(),
            cells().to_str().unwrap(),
        ],
        env,
    );
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    let _ = std::fs::remove_file(&out);
    rust
}

type Row = usize;

fn counts(rust: &str) -> HashMap<String, Row> {
    let mut map: HashMap<String, Row> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        *map.entry(current.clone()).or_default() += line.matches("let __pa_").count();
    }
    map
}

fn run(args: &[&str], env: &[(&str, &str)]) -> String {
    let mut a: Vec<&str> = args.to_vec();
    let s = cells().to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.trim_end().ends_with("leaving free ok"),
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
        Some(("LOFT_NO_RECORD_PTR", "1")),
        Some(("LOFT_NO_BASE_RECPTR", "1")),
    ] {
        for falsifier in [
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ("LOFT_HOIST_VERIFY", "1"),
        ] {
            let mut env = vec![falsifier];
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
fn each_cell_binds_exactly_the_addresses_predicted() {
    let got = counts(&emit("on", &[]));
    for (name, addresses) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or_default(),
            *addresses,
            "{name}: record addresses bound"
        );
    }
}
