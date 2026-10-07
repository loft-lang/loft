// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-Rebind` — its admissions PINNED by the trace: the own-buffer clause
//! (`tests/scripts/a-builder-rebinds-into-its-own-return-buffer.loft`) and the live-view
//! decline (`tests/fixtures/rebind-view-across-call.loft`).  A wrong admission changes no
//! value in the cells that exercise it, so only this pin sees it drift.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-builder-rebinds-into-its-own-return-buffer.loft";
const VIEW: &str = "tests/fixtures/rebind-view-across-call.loft";

/// `(fn, callee, verdict)` for the local `d`, as `LOFT_TRACE_REBIND=1` prints them.
fn verdicts(file: &str, env: &[(&str, &str)]) -> Vec<(String, String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_TRACE_REBIND", "1")
        .env_remove("LOFT_NO_REBIND_OWN_BUFFER")
        .env_remove("LOFT_NO_REBIND_PLACE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    let err = String::from_utf8_lossy(&out.stderr);
    let mut found: Vec<(String, String, String)> = err
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("[rebind] fn=")?;
            let (func, rest) = rest.split_once(" x=d callee=")?;
            let (callee, verdict) = rest.split_once(' ')?;
            Some((func.into(), callee.into(), verdict.into()))
        })
        .collect();
    found.sort();
    found.dedup();
    assert!(!found.is_empty(), "no rebind trace for {file}: {err}");
    found
}

fn expect(found: &[(String, String, String)], want: &[(&str, &str, &str)]) {
    for (func, callee, verdict) in want {
        assert!(
            found
                .iter()
                .any(|(f, c, v)| f == func && c == callee && v.contains(verdict)),
            "{func} → {callee}: expected `{verdict}`, the trace said {found:?}"
        );
    }
}

#[test]
fn a_builder_hands_its_own_buffer_to_the_step_it_rebinds_from() {
    let found = verdicts(CELLS, &[]);
    expect(
        &found,
        &[
            ("n_build", "n_step", "ADMITTED"),
            ("n_from", "n_step", "ADMITTED"),
            ("n_from", "n_bump", "ADMITTED"),
            ("n_held", "n_shift", "ADMITTED"),
            ("n_twice", "n_step", "ADMITTED"),
            (
                "n_twice",
                "n_merge",
                "DECLINED: the local is handed in twice",
            ),
        ],
    );
}

#[test]
fn the_switch_keeps_the_own_buffer_copied() {
    let found = verdicts(CELLS, &[("LOFT_NO_REBIND_OWN_BUFFER", "1")]);
    expect(
        &found,
        &[
            ("n_build", "n_step", "DECLINED: the local is a parameter"),
            ("n_held", "n_shift", "DECLINED: the local is a parameter"),
            // a plain local is the rule's original case, and stays admitted
            ("n_viewed", "n_step", "ADMITTED"),
        ],
    );
}

#[test]
fn a_view_of_the_local_read_across_the_call_declines() {
    let found = verdicts(VIEW, &[]);
    expect(
        &found,
        &[(
            "n_plain",
            "n_shift",
            "DECLINED: a view of the local is read across the call",
        )],
    );
}
