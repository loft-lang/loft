// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Base`'s join clause — the EMISSION pins.  A scalar field of a `?`-discharged
//! element, `v[i]?.f`, in a loop that holds the vector's header and element base is one range
//! test and one load, and the join the `?` lowers to is written ONCE, in the fallback arm.
//! `LOFT_NO_JOIN_READ=1` restores the join on every pass.  The cell corpus
//! (`tests/scripts/158-join-read.loft`) says the VALUES hold on both backends, in every switch
//! state and under the falsifiers; this pins what is emitted — and one thing no value can
//! show: that the pre-eval collector and the emitter agree.  The collector lifts every `Block`
//! argument into a `let _pre_N`, so an emitter arm it did not know about would compile, pass
//! every value test and never fire.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/158-join-read.loft";

/// Per function: `(join reads, joins lifted into a `let _pre_N`)`.
const EXPECTED: &[(&str, usize, usize)] = &[
    // One load in the loop; the two lifted joins are `v[99]?.x` AFTER it, where no header
    // is held.
    ("n_j1", 1, 2),
    ("n_j2", 3, 0),
    ("n_j3", 3, 0),
    // A negative index: the load is emitted, its range test refuses, the join answers.
    ("n_j4", 1, 0),
    // Three joins in one short-circuit condition, over a FIELD path.
    ("n_j6_count", 3, 0),
    // The loop pushes, so it holds no element base: the join is kept.
    ("n_j7", 0, 1),
    // No loop, no header.
    ("n_j8", 0, 4),
    ("n_j9", 1, 0),
    ("n_j10", 1, 0),
    // A computed index would be evaluated twice on the fallback path: the join is kept.
    ("n_j11", 0, 1),
    // A callee of a loop: its join's absent arm mints a discharge buffer, which the callee
    // admission reads as a store write — no twin, no header, both joins general.  Pinned at
    // zero ON PURPOSE: when that admission widens this row moves, and the twin path is then
    // owed the verification it has never had.
    ("n_j12_at", 0, 2),
    ("n_j12_scan", 0, 0),
];

const SWITCHES: [&str; 5] = [
    "LOFT_NO_JOIN_READ",
    "LOFT_NO_ELEM_FUSE",
    "LOFT_NO_VECTOR_BASE",
    "LOFT_NO_VECTOR_HOIST",
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
    let out = std::env::temp_dir().join(format!("loft_join_read_{}_{tag}.rs", std::process::id()));
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

/// The joins the pre-eval collector LIFTED in `text`: exactly `let _pre_<digits> = { //ncc_`.
/// Exact on purpose — the fallback arm this rewrite emits is `let __jr: DbRef = { //ncc_…`,
/// which a looser ` = { //ncc_` would count as lifted.
fn lifted_joins(text: &str) -> usize {
    text.match_indices("let _pre_")
        .filter(|(at, m)| {
            let rest = &text[at + m.len()..];
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            digits > 0 && rest[digits..].starts_with(" = { //ncc_")
        })
        .count()
}

fn counts(rust: &str) -> HashMap<String, (usize, usize)> {
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        let row = map.entry(current.clone()).or_default();
        row.0 += line.matches("/*@FR-R-Base join read*/").count();
        row.1 += lifted_joins(line);
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
        out.status.success() && stdout.trim_end().ends_with("join read ok"),
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
        Some(("LOFT_NO_JOIN_READ", "1")),
        Some(("LOFT_NO_ELEM_FUSE", "1")),
        Some(("LOFT_NO_VECTOR_BASE", "1")),
        Some(("LOFT_NO_VECTOR_HOIST", "1")),
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
    for (name, reads, lifted) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or_default(),
            (*reads, *lifted),
            "{name}: (join reads, joins lifted into a `let _pre_N`)"
        );
    }
}

#[test]
fn a_folded_join_is_written_once_and_only_in_the_fallback_arm() {
    // The collector and the emitter ask ONE recogniser.  Were they to disagree, the join
    // would be lifted AND folded — run on every pass and then run again off the fast path.
    let rust = emit("once", &[]);
    let start = rust.find("\nfn n_j3(").expect("n_j3 was emitted");
    let rest = &rust[start + 1..];
    let j3 = &rest[..rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3)];
    assert_eq!(
        j3.matches("vector::elem_field_at::<").count(),
        3,
        "three fields, three loads:\n{j3}"
    );
    assert_eq!(
        j3.matches("None => { let __jr: DbRef = { //ncc_").count(),
        3,
        "each join stands in its own fallback arm:\n{j3}"
    );
    assert_eq!(
        lifted_joins(j3),
        0,
        "no join of j3 is lifted out of its read:\n{j3}"
    );
}

#[test]
fn the_switch_restores_the_join_on_every_pass() {
    let rust = emit("off", &[("LOFT_NO_JOIN_READ", "1")]);
    assert!(
        !rust.contains("elem_field_at::<") && !rust.contains("FR-R-Base join read"),
        "LOFT_NO_JOIN_READ=1 folds no join anywhere"
    );
    let got = counts(&rust);
    assert_eq!(
        got.get("n_j3").copied().unwrap_or_default(),
        (0, 3),
        "n_j3's three joins are lifted again"
    );
}

#[test]
fn the_verify_form_checks_the_header_and_the_base_at_every_join_read() {
    let rust = emit("verify", &[("LOFT_HOIST_VERIFY", "1")]);
    assert!(
        rust.contains("vector::elem_field_at::<i64, true>(")
            && rust.contains("vector::elem_field_at::<f64, true>(")
            && rust.contains("vector::elem_field_at::<f32, true>("),
        "LOFT_HOIST_VERIFY=1 emits the checking form for each scalar kind"
    );
}
