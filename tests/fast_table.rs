// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-FastTable` — the interpreter's operators are compiled twice, with the stack access
//! mode fixed (`fill::OPERATORS` checks it per access, `fill::OPERATORS_FAST` takes the direct
//! path), and a run dispatches the fast table only when `State::fast_stack` holds.  What that
//! can break is a run that needs the checked path — every stack instrument — getting the
//! direct one: the instrument then sees nothing, and says nothing.  So a census of stack
//! writes must still see its accessor at work, and a program must answer the same on the
//! direct path as on the checked one (`LOFT_STRICT_STORES=1` forces the checked path).
use loft::file_access as fa;
use std::path::PathBuf;

const PROGRAM: &str = "struct P { x: float, id: integer }\n\
fn walk(v: const vector<P>) -> float {\n  t = 0.0;\n  for p in v { t += p.x * 2.0 + p.id as float; }\n  t\n}\n\
fn main() {\n  v: vector<P> = [];\n  for i in 0..500 { v += [P { x: i as float * 0.5, id: i % 7 }]; }\n\
\x20 s = \"\";\n  for i in 0..50 { s += \"{i % 10}\"; }\n\
\x20 println(\"{walk(v)} {len(s)} {s[3..9]}\");\n}\n";

fn run(env: &[(&str, &str)]) -> (String, String) {
    let file = std::env::temp_dir().join(format!("loft_fast_table_{}.loft", std::process::id()));
    fa::write(&file, PROGRAM).expect("write program");
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg(&file)
        .env("LOFT_TIMEOUT", "60")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_STACK_CENSUS")
        .env_remove("LOFT_STRICT_STORES");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let _ = fa::remove_file(&file);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_stack_instrument_still_sees_the_checked_accessor() {
    let (_, err) = run(&[("LOFT_STACK_CENSUS", "1")]);
    let line = err
        .lines()
        .find(|l| l.trim_start().starts_with("via put_stack"))
        .unwrap_or_else(|| panic!("no census report:\n{err}"));
    // The share, not a byte count: `State` methods push through `put_stack` too, so a census
    // given the direct path in every operator still counts thousands of bytes there — 6.6 %
    // of the writes, against 92.6 % when the operators take the checked path (2026-10-01).
    let share: f64 = line
        .split('(')
        .nth(1)
        .and_then(|r| r.split('%').next())
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("unreadable census line: {line}"));
    assert!(
        share > 50.0,
        "under the census the operators must write through `put_stack` (the checked \
         table); a census that sees them write elsewhere was given the direct path: {line}"
    );
}

#[test]
fn the_direct_path_answers_what_the_checked_path_answers() {
    let (fast, _) = run(&[]);
    let (checked, err) = run(&[("LOFT_STRICT_STORES", "1")]);
    assert_eq!(fast.trim(), "126244 50 345678", "the direct path's answer");
    assert_eq!(fast, checked, "the checked path's answer:\n{err}");
}

#[test]
fn the_two_tables_hold_the_same_operators() {
    assert_eq!(
        loft::fill::OPERATORS.len(),
        loft::fill::OPERATORS_FAST.len()
    );
    assert_eq!(
        loft::fill::OPERATORS.len(),
        loft::fill::OPERATOR_NAMES.len()
    );
}
