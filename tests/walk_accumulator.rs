// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-only
//! `@FR-R-Range`'s accumulator clause — the EMISSION pins.  A local seeded once by a ranged
//! value and stepped only by literals inside a character walk over an unwritten text emits
//! its steps as the processor's `wrapping_add` / `wrapping_sub`, and so does one under
//! counted loops whose trip bounds multiply to a range that fits; a step under a loop
//! nothing bounds (a `while`, a counted loop with an unranged end), by a non-literal, after
//! a second seed, from a parameter seed, or whose bound passes i64 keeps the checked
//! template; `LOFT_NO_RANGE_ARITH=1` restores every template and
//! `LOFT_HOIST_VERIFY=1` compares each plain answer with its template's.  The guard
//! (`tests/scripts/158-walk-accumulator.loft`) says the VALUES hold on both backends; this
//! pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/158-walk-accumulator.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!("loft_walk_acc_{}_{tag}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_RANGE_ARITH")
        .env_remove("LOFT_NO_CHAR_WALK")
        .env_remove("LOFT_HOIST_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
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

/// The emitted body of one function, up to the next top-level `fn`.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

/// `(plain steps, checked steps)` of the accumulator `var` in one function.
fn steps(b: &str, var: &str) -> (usize, usize) {
    let v = format!("var_{var}");
    let plain = b.matches(&format!("{v} = (({v}).wrapping_add(")).count()
        + b.matches(&format!("{v} = (({v}).wrapping_sub(")).count();
    let checked = b.matches(&format!("{v} = ops::op_add_int(({v})")).count()
        + b.matches(&format!("{v} = ops::op_min_int(({v})")).count();
    (plain, checked)
}

/// (function, accumulator, plain, checked) — what the clause admits, and what it declines.
const CELLS_FORMS: [(&str, &str, usize, usize); 18] = [
    ("n_count_marks", "n", 2, 0), // a1: the bench's two steps under one walk
    ("n_a2", "n", 1, 0),          // a negative step
    ("n_a3", "n", 2, 0),          // two walks stepping one accumulator
    ("n_a4", "n", 2, 0),          // a step in each arm of an `if`
    ("n_a5", "n", 1, 0),          // the seed's block re-run by an outer loop
    ("n_a6", "n", 1, 0),          // the seed outside a counted loop holding the walk
    ("n_a7", "n", 0, 1),          // DECLINES: a non-literal step
    ("n_a8", "n", 0, 0), // DECLINES: a walk over an appended text (the parser's nullable add)
    ("n_a9", "n", 0, 2), // DECLINES: a second seed
    ("n_a10", "n", 1, 0), // a counted loop over a literal range
    ("n_from", "n", 0, 1), // DECLINES: a parameter seed
    ("n_a12", "j", 1, 0), // the insertion sort's cursor under `for _ in 0..i`
    ("n_upto", "k", 0, 1), // DECLINES: a counted loop whose end is a parameter
    ("n_a14", "w", 0, 1), // DECLINES: a step under a `while`
    ("n_a15", "q", 1, 0), // two nested counted loops, the bounds multiplying
    ("n_a16", "g", 1, 0), // the boundary pair: the bound fits i64
    ("n_a17", "h", 0, 1), // DECLINES: the same step from 10^18 passes i64::MAX
    ("n_a18", "e", 0, 1), // DECLINES: the steps really overflow
];

#[test]
fn a_walks_accumulator_steps_plain_and_every_other_shape_keeps_the_template() {
    let rust = emit("on", &[]);
    for (name, var, plain, checked) in CELLS_FORMS {
        assert_eq!(
            steps(body(&rust, name), var),
            (plain, checked),
            "{name}: (plain steps, checked steps)"
        );
    }
}

#[test]
fn the_switch_restores_every_template_and_the_verify_form_checks_each_plain_step() {
    let off = emit("off", &[("LOFT_NO_RANGE_ARITH", "1")]);
    for (name, var, plain, checked) in CELLS_FORMS {
        assert_eq!(
            steps(body(&off, name), var),
            (0, plain + checked),
            "LOFT_NO_RANGE_ARITH=1: {name} keeps every checked template"
        );
    }
    let verify = emit("verify", &[("LOFT_HOIST_VERIFY", "1")]);
    let b = body(&verify, "n_count_marks");
    assert!(
        b.matches("range_verify").count() >= 2,
        "LOFT_HOIST_VERIFY=1: each plain step is compared with its template:\n{b}"
    );
}
