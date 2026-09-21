// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RecPtr`'s mint clause — the EMISSION pins.  A minted element holds its slot's
//! address from the mint up to its own finish, and its fusable field writes go through it.
//! `LOFT_NO_MINT_WINDOW=1` restores the store write per field.  The cell corpus
//! (`tests/scripts/158-mint-window.loft`) says the VALUES hold on both backends, in every
//! switch state and under the falsifiers; this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/158-mint-window.loft";

/// Per function: `(element addresses bound, field writes through one)`.  A loop whose index
/// chains run behind a guard is emitted twice (`@FR-R-GuardedChain`), which doubles m1 and
/// m11.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_m1", 2, 4),
    // Four fusable fields through the address; the boolean keeps its store write.
    ("n_m2", 1, 4),
    // A text set re-allocates: the window declines.
    ("n_m3", 0, 0),
    // The outer element declines on its nested mint; the two nested elements hold one each.
    ("n_m4", 2, 4),
    // …and at a size that reallocates the store inside the window, nothing is held.
    ("n_m4b", 0, 0),
    // The call names the container, so it runs BEFORE the mint (loft#1548): a clean window.
    ("n_m5", 1, 2),
    // The call appends to an unrelated vector inside the window: declined.
    ("n_m5b", 0, 0),
    // Two literal groups, four elements.
    ("n_m6", 4, 8),
    // Keyed inserts: the claimed record takes the address, the finish files it by its key.
    ("n_m7", 3, 6),
    ("n_m10", 1, 2),
    ("n_m11", 2, 4),
    // The element is the callee's buffer: the caller has no write to serve.
    ("n_m12", 0, 0),
    ("n_m13", 0, 0),
    // A struct-enum element is a record too (`@FR-R-RecPtr`'s enum clause): two variants,
    // two windows, their three `integer` fields through the address; the tag keeps its
    // store write.
    ("n_m14", 2, 3),
];

const SWITCHES: [&str; 5] = [
    "LOFT_NO_MINT_WINDOW",
    "LOFT_NO_RECORD_PTR",
    "LOFT_NO_RECORD_PUSH",
    "LOFT_NO_GROUP_PUSH",
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
        std::env::temp_dir().join(format!("loft_mint_window_{}_{tag}.rs", std::process::id()));
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

type Row = (usize, usize);

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
        row.0 += line
            .matches(": *const u8 = vector::rec_ptr(&(var__elm_")
            .count();
        row.1 += line.matches("rec_set::<").count();
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
        out.status.success() && stdout.trim_end().ends_with("mint window ok"),
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
        Some(("LOFT_NO_MINT_WINDOW", "1")),
        Some(("LOFT_NO_RECORD_PTR", "1")),
        Some(("LOFT_NO_RECORD_PUSH", "1")),
        Some(("LOFT_NO_GROUP_PUSH", "1")),
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
    for (name, addresses, writes) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or_default(),
            (*addresses, *writes),
            "{name}: (element addresses, field writes through one)"
        );
    }
}

#[test]
fn the_switch_restores_the_store_write() {
    let got = counts(&emit("off", &[("LOFT_NO_MINT_WINDOW", "1")]));
    for (name, _, _) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or_default().0,
            0,
            "{name}: no element holds an address with the switch on"
        );
    }
}
