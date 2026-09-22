// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `(R-BoundedNest)`'s reduction clause — the EMISSION pins.  A counted loop that is one
//! integer accumulate of a scalar vector's elements, over a held header and base, runs the
//! plain block sum BEFORE the checked loop, which resumes where the first block declines;
//! `LOFT_NO_BOUNDED_SUM=1` restores the checked add on every element.  The cell corpus
//! (`tests/scripts/158-bounded-sum.loft`) says the VALUES hold on both backends, in every
//! switch state and under the falsifiers; this pins what is emitted, and for whom.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/158-bounded-sum.loft";

/// Per function: the reduction preludes emitted.
const EXPECTED: &[(&str, usize)] = &[
    // The stdlib `sum`'s integer instance, and the callee twin the hoisting caller reaches.
    ("i_7integer_n_sum", 1),
    ("i_7integer_n_sum__inv", 1),
    // The same loop written by hand, and with the operands the other way round.
    ("n_s_user", 1),
    ("n_s_acc", 1),
    // Not the shape: a float accumulate, a `?`-discharged read (a join), a second statement
    // in the body, a computed index, a `for x in v` walk.
    ("n_s_float", 0),
    ("n_s_join", 0),
    ("n_s_two", 0),
    ("n_s_step", 0),
    ("n_s_walk", 0),
];

const SWITCHES: [&str; 4] = [
    "LOFT_NO_BOUNDED_SUM",
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
    let out =
        std::env::temp_dir().join(format!("loft_bounded_sum_{}_{tag}.rs", std::process::id()));
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

fn counts(rust: &str) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        *map.entry(current.clone()).or_default() +=
            line.matches("//@FR-R-BoundedNest reduction").count();
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
fn each_cell_emits_exactly_the_preludes_predicted() {
    let got = counts(&emit("on", &[]));
    for (name, preludes) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or_default(),
            *preludes,
            "{name}: reduction preludes"
        );
    }
}

#[test]
fn the_prelude_stands_before_the_loop_and_the_loop_is_unchanged() {
    // The plain part advances `#index`; the checked loop that follows is emitted exactly as
    // it was, guards and all, so it resumes where the first block declined.
    let rust = emit("shape", &[]);
    let start = rust.find("\nfn n_s_user(").expect("n_s_user was emitted");
    let rest = &rust[start + 1..];
    let f = &rest[..rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3)];
    let prelude = f
        .find("//@FR-R-BoundedNest reduction")
        .expect("the prelude");
    let lp = f.find("'l3: loop {").expect("the loop");
    assert!(prelude < lp, "the prelude stands before the loop:\n{f}");
    assert!(
        f.contains("vector::sum_blocks_i64::<false>(__vb_")
            && f.contains("ops::op_add_int(")
            && f.contains("get_elem_at::<i64, false>("),
        "the plain part reads through the held base, and the loop keeps its checked add and fused read:\n{f}"
    );
}

#[test]
fn the_switch_restores_the_checked_loop_alone() {
    let rust = emit("off", &[("LOFT_NO_BOUNDED_SUM", "1")]);
    assert!(
        !rust.contains("sum_blocks_i64::<") && !rust.contains("FR-R-BoundedNest reduction"),
        "LOFT_NO_BOUNDED_SUM=1 emits no prelude anywhere"
    );
}

#[test]
fn the_verify_form_reruns_every_admitted_block_through_the_checked_add() {
    let rust = emit("verify", &[("LOFT_HOIST_VERIFY", "1")]);
    assert!(
        rust.contains("vector::sum_blocks_i64::<true>("),
        "LOFT_HOIST_VERIFY=1 emits the checking form"
    );
}
