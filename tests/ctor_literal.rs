// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-CtorLiteral` and `@FR-R-LoopRecord`'s refill clause — PINNED on the cells of
//! `tests/scripts/157-a-constructor-call-builds-its-literal-in-place.loft` and
//! `tests/scripts/157-a-loop-record-keeps-its-vectors.loft`.  The cells check values (and a
//! live-record count), which pass whether or not a rule fires; this pins WHERE each fires, read
//! off `LOFT_TRACE_CTOR_LITERAL` and `LOFT_TRACE_LOOP_RECORD`, so a rule that stops firing — or
//! starts firing where a cell says it must not — is a red test rather than a quiet slowdown.
//! It also runs the cells under every switch and the store falsifiers, since the default run
//! never reaches the paths a switch restores.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CTOR: &str = "tests/scripts/157-a-constructor-call-builds-its-literal-in-place.loft";
const REFILL: &str = "tests/scripts/157-a-loop-record-keeps-its-vectors.loft";

const SWITCHES: [&str; 4] = [
    "LOFT_NO_CTOR_LITERAL",
    "LOFT_NO_LOOP_RECORD_REFILL",
    "LOFT_NO_LOOP_RECORD",
    "LOFT_NO_VADD_FITS",
];

fn loft(args: &[&str], file: &str, env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(file))
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_TRACE_CTOR_LITERAL")
        .env_remove("LOFT_TRACE_LOOP_RECORD");
    for s in SWITCHES {
        cmd.env_remove(s);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn run_ok(args: &[&str], file: &str, env: &[(&str, &str)]) {
    let out = loft(args, file, env);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("ok"),
        "{file} {args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The distinct trace lines starting with `prefix`, each with how often it was printed.
fn trace(out: &Output, prefix: &str) -> Vec<(String, usize)> {
    let err = String::from_utf8_lossy(&out.stderr);
    let mut lines: Vec<(String, usize)> = Vec::new();
    for l in err.lines().filter(|l| l.starts_with(prefix)) {
        match lines.iter_mut().find(|(k, _)| k == l) {
            Some((_, n)) => *n += 1,
            None => lines.push((l.to_string(), 1)),
        }
    }
    lines.sort();
    lines
}

#[test]
fn a_constructor_call_is_its_literal_where_the_cells_say_and_nowhere_else() {
    let out = loft(&["--interpret"], CTOR, &[("LOFT_TRACE_CTOR_LITERAL", "1")]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        trace(&out, "ctor-literal:"),
        vec![
            // c4: `(R-Place)` builds the record into `keep` through the buffer.
            (
                "ctor-literal: n_fm_new kept a call in n_main: the return buffer is used elsewhere"
                    .to_string(),
                1
            ),
            (
                "ctor-literal: n_fm_new kept a call in n_make: the target is a parameter (the function's own return buffer)"
                    .to_string(),
                1
            ),
            // c1, c2, c3, c5, c8, c9, c11.
            ("ctor-literal: n_fm_new written in place in n_main".to_string(), 7),
            // c12: the profitable case only — a call of scalars stays a value record.
            (
                "ctor-literal: n_pt_new kept a call in n_pt_loop: no vector-literal argument (a call of scalars travels as a value record)"
                    .to_string(),
                1
            ),
        ]
    );
    let off = loft(
        &["--interpret"],
        CTOR,
        &[
            ("LOFT_TRACE_CTOR_LITERAL", "1"),
            ("LOFT_NO_CTOR_LITERAL", "1"),
        ],
    );
    assert!(
        trace(&off, "ctor-literal:").is_empty(),
        "the switch keeps every call"
    );
}

#[test]
fn the_refill_clause_keeps_the_designed_loop_records() {
    let out = loft(
        &["--native-release", "--native-emit", "/dev/null"],
        REFILL,
        &[("LOFT_TRACE_LOOP_RECORD", "1")],
    );
    let lines: Vec<String> = trace(&out, "[loop-record] n_main:")
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    let kept = lines
        .iter()
        .filter(|l| l.contains("keeps its store and its vectors"))
        .count();
    // r1, r4, r6 (r1's inner loop holds no record).
    assert_eq!(kept, 3, "{lines:#?}");
    // r2: the omitted field is written by the prefill; r5: a native append takes the record.
    assert!(
        lines
            .iter()
            .any(|l| l.ends_with("declines: a mint whose literal group the refill cannot see")),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.ends_with("declines: a native op takes it otherwise")),
        "{lines:#?}"
    );
    let off = loft(
        &["--native-release", "--native-emit", "/dev/null"],
        REFILL,
        &[
            ("LOFT_TRACE_LOOP_RECORD", "1"),
            ("LOFT_NO_LOOP_RECORD_REFILL", "1"),
        ],
    );
    assert!(
        trace(&off, "[loop-record] n_main:")
            .iter()
            .all(|(l, _)| !l.contains("its vectors")),
        "the switch keeps no vectors"
    );
}

#[test]
fn the_cells_hold_in_every_mode_under_every_switch_and_the_falsifiers() {
    for file in [CTOR, REFILL] {
        for mode in ["--interpret", "--native-release"] {
            run_ok(&[mode], file, &[]);
            for s in SWITCHES {
                run_ok(&[mode], file, &[(s, "1")]);
            }
            run_ok(&[mode], file, &[("LOFT_STRICT_STORES", "1")]);
            run_ok(&[mode], file, &[("LOFT_POISON_CLAIM", "1")]);
        }
        run_ok(&["--native"], file, &[("LOFT_NATIVE_LEAK_CHECK", "1")]);
        run_ok(
            &["--native-release"],
            file,
            &[("LOFT_NATIVE_LEAK_CHECK", "1")],
        );
    }
}
