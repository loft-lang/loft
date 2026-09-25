// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-WorkBuffer` — the admission of a non-escaping vector local, PINNED on the IR of the
//! cells (`tests/scripts/a-non-escaping-vector-local-is-a-caller-buffer.loft`): a promoted
//! local is an ARGUMENT of its function (the IR header names it beside the parameters), a
//! declined one stays a local.  The cells' values pass on either form, so only this pin sees
//! an admission drift; the null road is run here too, since no source can spell it.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-non-escaping-vector-local-is-a-caller-buffer.loft";

/// Per function: the locals promoted to a caller buffer (the rewrite ON; OFF: always none).
const EXPECTED: &[(&str, &[&str])] = &[
    ("n_c1", &["v"]),        // the probe
    ("n_c2", &["v"]),        // recursion
    ("n_c3", &["xs", "ks"]), // two buffers
    ("n_c4", &["v"]),        // declared inside a loop
    ("n_c5", &["v"]),        // every in-place use, the walk's alias included
    ("n_c6", &["t"]),        // beside a return buffer
    ("n_c7", &[]),           // handed to a loft-bodied call
    ("n_c8", &[]),           // a text element
    ("n_c9", &[]),           // assigned a second time
    ("n_c10", &["v", "w"]),  // the tail COPIES the local into the return buffer
    ("t_3Acc_bump", &["v"]), // a method
    ("n_c12w", &[]),         // a par worker
    ("n_c12", &["out"]),     // the par caller's own collector
    ("n_c13", &["v"]),       // an early return
    ("n_c15", &["acc"]),     // a buffer-holding caller
    ("n_c16", &["v"]),       // beside a parse-time return buffer of its own
];

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_WORK_BUFFER")
        .env_remove("LOFT_WORK_BUFFER_NULL");
    cmd
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

/// The IR and the trace of the cells, with `env` on top of the pin's own.
fn introspect(env: &[(&str, &str)]) -> (String, String) {
    let mut cmd = loft();
    cmd.arg("introspect")
        .arg(cells())
        .env("LOFT_TRACE_WORK_BUFFER", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    let ir = String::from_utf8_lossy(&out.stdout).into_owned();
    let trace = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        ir.contains("fn n_c1("),
        "no IR printed (exit {:?}): {trace}",
        out.status
    );
    (ir, trace)
}

/// The locals the trace names as promoted in `name`, in trace order.
fn promoted_in(trace: &str, name: &str) -> Vec<String> {
    let head = format!("[work-buffer] fn={name} local=");
    trace
        .lines()
        .filter_map(|l| l.strip_prefix(&head))
        .filter_map(|rest| {
            rest.strip_suffix(" PROMOTED to a caller buffer")
                .map(str::to_string)
        })
        .collect()
}

/// The IR header of `fn <name>(…)`: its parameter list as printed.
fn header_of<'a>(ir: &'a str, name: &str) -> &'a str {
    let head = format!("\nfn {name}(");
    let start = ir
        .find(&head)
        .unwrap_or_else(|| panic!("{name} was not printed"));
    let rest = &ir[start + head.len()..];
    let end = rest.find(')').unwrap_or(rest.len());
    &rest[..end]
}

/// The header names `local` as a parameter — in the bytecode spelling (`v:vector<…>`) or,
/// for a method printed only in the native section, the emitted one (`mut var_v: DbRef`).
fn names_parameter(header: &str, local: &str) -> bool {
    header
        .split(", ")
        .any(|p| p.starts_with(&format!("{local}:")) || p.starts_with(&format!("mut var_{local}:")))
}

#[test]
fn a_non_escaping_local_is_promoted_and_anything_else_keeps_its_store() {
    let (ir, trace) = introspect(&[]);
    for (name, expected) in EXPECTED {
        assert_eq!(
            promoted_in(&trace, name),
            expected.to_vec(),
            "{name}: the promoted locals — the cell's admission moved"
        );
        // And the promotion is real: a promoted local is a PARAMETER of its function.
        for local in *expected {
            assert!(
                names_parameter(header_of(&ir, name), local),
                "{name}: `{local}` was reported promoted but is no parameter"
            );
        }
    }
}

#[test]
fn the_switch_keeps_every_local() {
    let (ir, trace) = introspect(&[("LOFT_NO_WORK_BUFFER", "1")]);
    assert!(
        !trace.contains("PROMOTED"),
        "LOFT_NO_WORK_BUFFER=1 must promote nothing:\n{trace}"
    );
    for (name, expected) in EXPECTED {
        for local in *expected {
            assert!(
                !names_parameter(header_of(&ir, name), local),
                "{name}: LOFT_NO_WORK_BUFFER=1 must leave `{local}` a local"
            );
        }
    }
}

/// The null road: every caller-side buffer left at the sentinel, so each promoted callee
/// mints a store of its own and releases it at exit — the same values, no leak.
#[test]
fn the_null_road_answers_the_same_and_leaks_nothing() {
    let out = loft()
        .arg("--interpret")
        .arg(cells())
        .env("LOFT_WORK_BUFFER_NULL", "1")
        .env("LOFT_STORES", "warn")
        .output()
        .expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("done"),
        "the cells failed on the null road (exit {:?}):\n{stdout}\n{stderr}",
        out.status
    );
    assert!(
        !stderr.contains("not freed") && !stderr.contains("leak"),
        "a store leaked on the null road:\n{stderr}"
    );
}
