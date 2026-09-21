// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-InPlace`'s copy clause — the EMISSION pins.  A flag-free copy of a record that
//! owns no heap is an in-place write, so a loop that holds one keeps its headers, its bases
//! and its record addresses — the write-back idiom `e = v[i]?; …; v[i] = e`.
//! `LOFT_NO_COPY_IN_PLACE=1` makes the copy a store writer again.  The cell corpus
//! (`tests/scripts/158-copy-in-place.loft`) says the VALUES hold on both backends, in every
//! switch state and under the falsifiers; this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/158-copy-in-place.loft";

/// Per function, with the clause ON and OFF: `(hoist blocks opened, record addresses
/// bound, record scalars hoisted)`.
const EXPECTED: &[(&str, Row, Row)] = &[
    ("n_c1_tick", (1, 1, 0), (0, 0, 0)),
    ("n_c2", (1, 1, 0), (0, 0, 0)),
    ("n_c3", (1, 1, 0), (0, 0, 0)),
    // The copy is the loop's only whole-type write, and it evicts `first`'s scalars: the
    // header hoists, no scalar does.
    ("n_c3b", (1, 0, 0), (0, 0, 0)),
    // A heap-owning record and a call's freed result: still store writers.
    ("n_c4", (0, 0, 0), (0, 0, 0)),
    ("n_c5", (0, 0, 0), (0, 0, 0)),
    // Two vectors: both headers held across the copy (the read walk hoisted before too).
    ("n_c6", (2, 3, 0), (1, 1, 0)),
    ("n_c7", (1, 1, 0), (0, 0, 0)),
    ("n_c8", (2, 3, 0), (2, 2, 0)),
    // A record holding a vector, and a struct-enum value: unchanged by THIS clause (the
    // enum cell's addresses are `@FR-R-RecPtr`'s enum clause at work).
    ("n_c9", (1, 1, 0), (1, 1, 0)),
    ("n_c11", (2, 3, 0), (2, 3, 0)),
];

const SWITCHES: [&str; 4] = [
    "LOFT_NO_COPY_IN_PLACE",
    "LOFT_NO_RECORD_PTR",
    "LOFT_NO_SCALAR_HOIST",
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
    let out = std::env::temp_dir().join(format!(
        "loft_copy_in_place_{}_{tag}.rs",
        std::process::id()
    ));
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

type Row = (usize, usize, usize);

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
        let row = map.entry(current.clone()).or_default();
        row.0 += usize::from(line.contains("loft#885 loop-invariant vector headers"));
        row.1 += line.matches("let __pa_").count();
        row.2 += line.matches("let __vs_").count();
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
        for falsifier in [
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
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
fn each_cell_emits_exactly_the_forms_predicted_with_the_clause_on_and_off() {
    let on = counts(&emit("on", &[]));
    let off = counts(&emit("off", &[("LOFT_NO_COPY_IN_PLACE", "1")]));
    for (name, want_on, want_off) in EXPECTED {
        assert_eq!(
            on.get(*name).copied().unwrap_or_default(),
            *want_on,
            "{name} ON: (hoist blocks, record addresses, hoisted scalars)"
        );
        assert_eq!(
            off.get(*name).copied().unwrap_or_default(),
            *want_off,
            "{name} OFF: (hoist blocks, record addresses, hoisted scalars)"
        );
    }
}
