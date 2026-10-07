// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-ab (loft#1426) — a counted `for` whose start is not a literal runs a second
//! counter seeded AT the start (`@FR-I-Range` lowered to one compare and one step), instead of a
//! null-encoded "not started yet" counter that paid a null test and a select on every iteration.
//!
//! The value guard (`tests/scripts/157-next-counter.loft`) can only say the VALUES hold — both
//! forms compute the same sequence by construction.  This pins the EMISSION per cell — which
//! loops bind a `next` counter, and that no iterator block still pays a null test — and the
//! switch (`LOFT_NO_NEXT_COUNTER=1`, `@FR-R-Switch`), which is what makes it red on the build
//! before the unit and on one that lost it.  Read off `--native-emit`.
use loft::file_access as fa;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/loop-start-cells.loft";

/// `(function, next counters bound, null tests in the function)` with the unit on.  The
/// predictions are written beside the cells: a literal start binds none (P3b's single
/// counter), a computed start binds one, a reverse exclusive range binds none (the one
/// counter is seeded at `till`), a reverse inclusive range binds one.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_c1_range", 0, 0),
    ("n_c2_incl", 0, 0),
    ("n_c3_from", 1, 0),
    ("n_c4_empty", 1, 0),
    ("n_c5_one", 1, 0),
    ("n_c6_nested", 0, 0),
    ("n_c7_break", 0, 0),
    ("n_c8_cont", 0, 0),
    ("n_c9_vec", 0, 0),
    ("n_c10_neg", 1, 0),
    ("n_d1_litnz", 0, 0),
    ("n_d2_var0", 1, 0),
    ("n_d3_expr", 1, 0),
    ("n_r1_rev", 0, 0),
    ("n_r2_revincl", 1, 0),
];

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        fa::exists(out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    fa::read_to_string(out).expect("read the emitted Rust")
}

/// Per emitted function: `(next counters bound, null tests)`.
///
/// A null test inside the range's `Range null guard` block is not counted: that guard
/// (`(I-RangeNull)`) runs ONCE, before the loop, to turn a null bound into an empty range —
/// the claim here is about the ITERATOR, which must pay none per round.
fn counts(rust: &str) -> HashMap<String, (usize, usize)> {
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut current = String::new();
    let mut in_guard = false;
    for line in rust.lines() {
        if line.contains("//Range null guard_") {
            in_guard = true;
        }
        if line.contains("/*Range null guard_") {
            in_guard = false;
            continue;
        }
        if in_guard {
            continue;
        }
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
            continue;
        }
        let e = map.entry(current.clone()).or_default();
        if line.contains("let mut var__next_") {
            e.0 += 1;
        }
        e.1 += line.matches("op_conv_bool_from_int(").count();
    }
    map
}

fn cells() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    root.join(CELLS)
}

#[test]
fn a_computed_start_binds_one_next_counter_and_no_iterator_pays_a_null_test() {
    let out = std::env::temp_dir().join("loft_next_counter_on.rs");
    let _ = fa::remove_file(&out);
    let rust = emit(&cells(), &out, &[]);
    let got = counts(&rust);
    let mut wrong = Vec::new();
    for (name, next, nulls) in EXPECTED {
        match got.get(*name) {
            Some(&(n, t)) if n == *next && t == *nulls => {}
            Some(&(n, t)) => wrong.push(format!(
                "{name}: expected ({next} next, {nulls} null tests), emitted ({n}, {t})"
            )),
            None => wrong.push(format!("{name}: not emitted")),
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_switch_restores_the_null_encoded_counter() {
    let out = std::env::temp_dir().join("loft_next_counter_off.rs");
    let _ = fa::remove_file(&out);
    let rust = emit(&cells(), &out, &[("LOFT_NO_NEXT_COUNTER", "1")]);
    let got = counts(&rust);
    let mut wrong = Vec::new();
    for (name, next, _) in EXPECTED {
        // Under the switch every loop that would bind a `next` counter — and the reverse
        // exclusive range, whose plain seed is the switch's other half — pays one null test
        // per iterator block instead; a literal start keeps P3b's form either way.
        let nulls = if *next == 1 || *name == "n_r1_rev" {
            1
        } else {
            0
        };
        match got.get(*name) {
            Some(&(0, t)) if t == nulls => {}
            Some(&(n, t)) => wrong.push(format!(
                "{name}: expected (0 next, {nulls} null tests) under the switch, emitted ({n}, {t})"
            )),
            None => wrong.push(format!("{name}: not emitted")),
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
