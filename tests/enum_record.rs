// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-PushRec`'s and `@FR-R-RecPtr`'s enum clauses — the EMISSION pins.  A USER
//! struct-enum value is a record to the hoist family: a vector of them appends through a
//! push header (the slot zeroed, the literal writing its own tag), and a view of one holds
//! its address, the tag read through it.  `LOFT_NO_ENUM_RECORD=1` restores the templates.
//! The cell corpus (`tests/scripts/158-enum-record.loft`) says the VALUES hold on both
//! backends, in every switch state and under the falsifiers; this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/158-enum-record.loft";

/// Per function, with the clause ON: `(zeroed slots through a header, record addresses
/// bound, tag reads through an address)`.  With it OFF every column is 0, except e8's
/// three addresses: its `Holder` elements are plain records and held them before.
const EXPECTED: &[(&str, usize, usize, usize)] = &[
    // The match: one address, the tag read once per arm through it.
    ("n_score", 0, 1, 3),
    ("n_score__inv", 0, 1, 3),
    // Three variants, three mint windows.
    ("n_e1_build", 3, 3, 0),
    ("n_e2_fill", 2, 2, 0),
    // The text-carrying arm keeps its template; the arm beside it takes a group header
    // (the loop's index chain runs behind a guard, so it is emitted twice).
    ("n_e3_fill", 2, 2, 0),
    ("n_e4", 1, 1, 0),
    // A unit variant's literal writes the tag alone: a zeroed slot, no address.
    ("n_e5", 2, 3, 3),
    ("n_e6", 0, 1, 2),
    // The tag of an INLINE enum field, through a nested path.
    ("n_e8", 0, 3, 2),
    ("n_e9", 0, 1, 3),
    // A plain enumerate and a nullable element are not struct-enum records.
    ("n_e11", 0, 0, 0),
    ("n_e12", 0, 0, 0),
];

const SWITCHES: [&str; 5] = [
    "LOFT_NO_ENUM_RECORD",
    "LOFT_NO_RECORD_PUSH",
    "LOFT_NO_MINT_WINDOW",
    "LOFT_NO_RECORD_PTR",
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
        std::env::temp_dir().join(format!("loft_enum_record_{}_{tag}.rs", std::process::id()));
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
        row.0 += line.matches("push_record_hoisted_zero").count();
        row.1 += line.matches("let __pa_").count();
        row.2 += line.matches("rec_get::<u8>").count();
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
        out.status.success() && stdout.trim_end().ends_with("enum record ok"),
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
        Some(("LOFT_NO_ENUM_RECORD", "1")),
        Some(("LOFT_NO_RECORD_PUSH", "1")),
        Some(("LOFT_NO_MINT_WINDOW", "1")),
        Some(("LOFT_NO_RECORD_PTR", "1")),
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
fn each_cell_emits_exactly_the_forms_predicted() {
    let got = counts(&emit("on", &[]));
    for (name, zeroed, addresses, tags) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or_default(),
            (*zeroed, *addresses, *tags),
            "{name}: (zeroed slots, record addresses, tag reads through one)"
        );
    }
}

#[test]
fn the_switch_restores_the_templates() {
    let got = counts(&emit("off", &[("LOFT_NO_ENUM_RECORD", "1")]));
    for (name, _, _, _) in EXPECTED {
        let row = got.get(*name).copied().unwrap_or_default();
        let plain_addresses = if *name == "n_e8" { 3 } else { 0 };
        assert_eq!(
            row,
            (0, plain_addresses, 0),
            "{name}: no struct-enum value is a record with the switch on"
        );
    }
}
