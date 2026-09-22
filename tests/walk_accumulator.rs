// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-only
//! `@FR-R-Range`'s accumulator clause — the EMISSION pins.  A local seeded once by a ranged
//! value and stepped only by literals inside a character walk over an unwritten text emits
//! its steps as the processor's `wrapping_add` / `wrapping_sub`; a step under a second
//! loop, under a counted loop, by a non-literal, after a second seed, or from a parameter
//! seed keeps the checked template; `LOFT_NO_RANGE_ARITH=1` restores every template and
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

/// `(plain steps, checked steps)` of the accumulator `n` in one function.
fn steps(b: &str) -> (usize, usize) {
    let plain = b.matches("var_n = ((var_n).wrapping_add(").count()
        + b.matches("var_n = ((var_n).wrapping_sub(").count();
    let checked = b.matches("var_n = ops::op_add_int((var_n)").count()
        + b.matches("var_n = ops::op_min_int((var_n)").count();
    (plain, checked)
}

/// (function, plain, checked) — what the clause admits, and what it declines.
const CELLS_FORMS: [(&str, usize, usize); 11] = [
    ("n_count_marks", 2, 0), // a1: the bench's two steps under one walk
    ("n_a2", 1, 0),          // a negative step
    ("n_a3", 2, 0),          // two walks stepping one accumulator
    ("n_a4", 2, 0),          // a step in each arm of an `if`
    ("n_a5", 1, 0),          // the seed's block re-run by an outer loop
    ("n_a6", 0, 1),          // DECLINES: the seed outside the loop holding the walk
    ("n_a7", 0, 1),          // DECLINES: a non-literal step
    ("n_a8", 0, 0),          // DECLINES: a walk over an appended text (the parser's nullable add)
    ("n_a9", 0, 2),          // DECLINES: a second seed
    ("n_a10", 0, 1),         // DECLINES: a counted loop
    ("n_from", 0, 1),        // DECLINES: a parameter seed
];

#[test]
fn a_walks_accumulator_steps_plain_and_every_other_shape_keeps_the_template() {
    let rust = emit("on", &[]);
    for (name, plain, checked) in CELLS_FORMS {
        assert_eq!(
            steps(body(&rust, name)),
            (plain, checked),
            "{name}: (plain steps, checked steps)"
        );
    }
}

#[test]
fn the_switch_restores_every_template_and_the_verify_form_checks_each_plain_step() {
    let off = emit("off", &[("LOFT_NO_RANGE_ARITH", "1")]);
    for (name, plain, checked) in CELLS_FORMS {
        assert_eq!(
            steps(body(&off, name)),
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
