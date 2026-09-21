// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-GuardedChain` — the EMISSION pins.  A counted loop's admitted chains run plain
//! behind a guard evaluated once at the loop's entry (`//@FR-R-GuardedChain guard`): an
//! innermost loop is emitted twice (`plain chains` copy + checked `else` arm), a loop with
//! loops inside once with each admitted op branching on the guard (`(if __gc_N {`).
//! A loop with ONE admitted operator declines on PROFITABILITY: the guard is a fixed cost
//! per loop ENTRY against a saving of one null test per operator per ITERATION, and an
//! innermost loop is emitted twice, which in a small hot function costs the inline.
//! `LOFT_NO_GUARDED_CHAIN=1` restores the checked loops.  The cell corpus
//! (`tests/scripts/157-guarded-chain.loft`) says the VALUES hold on both backends, in every
//! switch state and under the falsifiers; this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/157-guarded-chain.loft";

/// Per function: `(guards, plain-copy markers, per-op branches; wrapping add, sub, mul, neg;
/// checked `op_add_int(`, `op_min_int(`, nullable adds)` — written beside the cells.
const EXPECTED: &[(&str, [usize; 10])] = &[
    // c1: the inner pixel loop (no loop inside it) is guarded and emitted twice: three chains
    // plain in the copy (`j*lw + i`, `x0 + i`, `y0 + j` — 3 adds, 1 mul), their checked forms
    // in the else arm beside `acc`'s self-stepping adds.  The outer loop has no chain of its
    // own (its only chains lie in the inner loop, which owns them).
    ("n_c1_run", [1, 2, 0, 3, 0, 1, 0, 9, 0, 0]),
    // c2: `base + i` is the nullable twin (the `??` operand) — admitted, plain in the copy,
    // its checked template in the else arm; the two `- base` and `-1` stay as they are.
    ("n_c2_run", [1, 2, 0, 1, 0, 0, 2, 2, 2, 1]),
    // c3: `y + 2 * i` over the null invariant — guarded, plain in the copy (one add, one
    // multiply), checked in the arm (the guard declines at RUN time, so the arm is what
    // runs).  Two operators on purpose: with one it would decline at COMPILE time on
    // profitability and stop testing the run-time decline.
    ("n_c3", [1, 2, 0, 1, 0, 1, 0, 1, 0, 0]),
    // c4: `-(a * i) - b` — negation, a product and a subtraction in one chain.
    ("n_c4", [1, 2, 0, 0, 1, 1, 3, 2, 1, 0]),
    // c5: `2 * i + 1` plain; `w + i` keeps its template (w is written in the loop).
    ("n_c5", [1, 2, 0, 1, 0, 1, 0, 5, 0, 0]),
    // c6: `base + 2 * i` with the parameter as the invariant leaf.
    ("n_c6_run", [1, 2, 0, 1, 0, 1, 0, 3, 0, 0]),
    // c7: c6 with the `2 *` removed — ONE admitted operator, so the profitability gate
    // declines it at compile time.  No guard, no copy, no plain operator, and BOTH its
    // additions (`base + i` and the accumulator's) keep their checked helper.
    ("n_c7_run", [0, 0, 0, 0, 0, 0, 0, 2, 0, 0]),
];

const KEYS: [&str; 10] = [
    "GuardedChain guard",
    "plain chains",
    "(if __gc_",
    "wrapping_add",
    "wrapping_sub",
    "wrapping_mul",
    "wrapping_neg",
    "ops::op_add_int(",
    "ops::op_min_int(",
    "op_add_int_nullable",
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_RANGE_ARITH")
        .env_remove("LOFT_NO_GUARDED_CHAIN")
        .env_remove("LOFT_HOIST_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let out = std::env::temp_dir().join(format!(
        "loft_guarded_chain_{}_{tag}.rs",
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

fn counts(rust: &str) -> HashMap<String, [usize; 10]> {
    let mut map: HashMap<String, [usize; 10]> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        let row = map.entry(current.clone()).or_default();
        for (i, k) in KEYS.iter().enumerate() {
            row[i] += line.matches(k).count();
        }
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
        out.status.success() && stdout.trim_end().ends_with("guarded chain ok"),
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
        Some(("LOFT_NO_GUARDED_CHAIN", "1")),
        Some(("LOFT_NO_RANGE_ARITH", "1")),
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
    let rust = emit("on", &[]);
    let got = counts(&rust);
    for (name, expected) in EXPECTED {
        let row = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(row, *expected, "{name}: {KEYS:?}");
    }
    // The guard's shape: the range's ends tested, every chain bounded with checked operators.
    let c1 = &rust[rust.find("\nfn n_c1_run(").unwrap()..];
    assert!(
        c1.contains("if __lo == i64::MIN || __hi == i64::MIN { return None; }")
            && c1.contains(".checked_abs()?")
            && c1.contains(".checked_mul(")
            && c1.contains(".checked_add("),
        "c1's guard tests the ends and bounds its chains with checked operators:\n{}",
        &c1[..c1.len().min(3000)]
    );
}

#[test]
fn the_switch_restores_the_checked_loops() {
    let rust = emit("off", &[("LOFT_NO_GUARDED_CHAIN", "1")]);
    let got = counts(&rust);
    for (name, _) in EXPECTED {
        let row = got[*name];
        assert_eq!(
            (row[0], row[1], row[2]),
            (0, 0, 0),
            "{name}: LOFT_NO_GUARDED_CHAIN=1 guards no loop: {row:?}"
        );
    }
    // c1's chains are back on their templates (only R-Range's plain forms may remain).
    assert!(
        got["n_c1_run"][7] >= 6,
        "c1's chains are checked again: {:?}",
        got["n_c1_run"]
    );
}

/// `@FR-R-GuardedChain` / `@FR-R-BoundedNest` — loft#928's generator corpus, caught by CI's
/// native corpus and by nothing faster.  A coroutine's state-machine body carries NO
/// loop-entry guard, for two reasons: its persistent locals are spelled `self.var_…`, so a
/// guard naming one emits an identifier that does not exist (`E0425` on
/// `var_i__1__index`), and the machine RE-ENTERS its loop across a `next_*` call, so a fact
/// proved once at entry is not proved for the resumes after it.
///
/// Non-vacuous by construction: the same run asserts the ordinary cells file still emits
/// guards, so a detector that had gone blind fails here rather than passing quietly.
#[test]
fn a_coroutine_body_carries_no_loop_entry_guard_and_still_compiles() {
    let gen_src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/928-generator-duplicate-local-name.loft");
    let out = std::env::temp_dir().join(format!("loft_gc_coroutine_{}.rs", std::process::id()));
    let status = loft(
        &[
            "--native-emit",
            out.to_str().unwrap(),
            gen_src.to_str().unwrap(),
        ],
        &[],
    );
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    let _ = std::fs::remove_file(&out);
    for marker in ["GuardedChain guard", "BoundedNest guard"] {
        assert_eq!(
            rust.matches(marker).count(),
            0,
            "a generator's emission carries no `{marker}`"
        );
    }
    // The detector is alive: the ordinary cells still guard their loops.
    assert!(
        counts(&emit("alive", &[]))["n_c1_run"][0] > 0,
        "the guard still fires outside a coroutine"
    );
    // And the program itself compiles and runs on native, which is what E0425 denied.
    let ran = loft(&["--native", gen_src.to_str().unwrap()], &[]);
    let stdout = String::from_utf8_lossy(&ran.stdout).into_owned();
    assert!(
        ran.status.success() && stdout.trim_end().ends_with("ok"),
        "the generator corpus runs on native: exit {:?}\nstdout: {stdout}\nstderr: {}",
        ran.status,
        String::from_utf8_lossy(&ran.stderr)
    );
}

/// `@FR-R-GuardedChain` profitability — the c6/c7 PAIR, which is what makes this a claim
/// about the operator count and not about some other difference between two loops: the two
/// loops are `acc + (base + 2 * i)` and `acc + (base + i)`, identical but for one operator.
///
/// Measured on the drawing lane against the guard off: the one-operator loops in
/// `pil_hline` and `matches_at` cost `fill_circle` and `fill_star` ~50 %, `wide_line` 22 %
/// and `parse` 18 %, while six-operator `composite_layer` gains 30 %.  Thresholds 3 and 5
/// measured identical to 2 lane-wide, so 2 is the lowest that removes the loss.
#[test]
fn a_one_operator_loop_declines_on_profitability() {
    let got = counts(&emit("profit", &[]));
    let two = got["n_c6_run"];
    let one = got["n_c7_run"];
    assert!(
        two[0] == 1 && two[1] == 2,
        "c6's two-operator loop is guarded and copied: {two:?}"
    );
    assert_eq!(
        (one[0], one[1], one[2]),
        (0, 0, 0),
        "c7's one-operator loop takes no guard, no copy and no per-op branch: {one:?}"
    );
    assert!(
        one[3..7].iter().all(|&n| n == 0),
        "c7 emits no plain operator at all: {one:?}"
    );
    assert!(
        one[7] >= 2,
        "c7 keeps a checked helper for BOTH its additions: {one:?}"
    );
}
