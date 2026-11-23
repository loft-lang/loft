// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-PureReuse`, PINNED on its cells (`tests/scripts/a-pure-call-made-twice-is-computed-once.loft`):
//! which bodies compute an effect-free call once, read off `LOFT_TRACE_PURE_REUSE`, and how
//! many calls of it the request-check shape emits with the rule on and off.  An effect-free
//! call cannot count itself, so only this pin sees a reuse that stops firing.
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-pure-call-made-twice-is-computed-once.loft";

/// The shared-call clause's cells.
const SHARED: &str = "tests/scripts/a-call-two-wrappers-share-is-made-once.loft";

/// Every body the shared-call cells share a call in, in order.  c3 (an argument rebound), c4
/// (a parameter's store written), c5w (a work buffer written), c6 (a wrapper calling twice),
/// c7 (a function that is not total), c9 (no call always evaluated) and c12 (a wrapper that
/// writes) share nothing.
const SHARED_TRACE: &[&str] = &[
    "[pure-reuse] n_c1: n_names computed once, shared",
    "[pure-reuse] n_c2: n_names computed once, shared",
    "[pure-reuse] n_c5: n_names computed once, shared",
    "[pure-reuse] n_c8: n_names computed once, shared",
    "[pure-reuse] n_c10: n_names computed once, shared",
    "[pure-reuse] n_c11: n_names computed once, shared",
    "[pure-reuse] n_c13: n_names computed once, shared",
];

/// Every reuse the cells make, in order: the request-check shape, the two projections, the
/// request-check and record-field shapes in the natural spelling (`parse(v).ok`), and the
/// request check over a parser that reads a slice value.
const TRACE: &[&str] = &[
    "[pure-reuse] n_check: n_parse computed once",
    "[pure-reuse] n_p6: n_parse computed once",
    "[pure-reuse] n_check_n: n_parse computed once",
    "[pure-reuse] n_spanned: n_parse computed once",
    "[pure-reuse] n_check_s: n_parse_s computed once",
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> (String, String, bool) {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_PURE_REUSE")
        .env_remove("LOFT_TRACE_PURE_REUSE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn cells() -> String {
    script(CELLS)
}

fn script(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(name)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn the_reusing_bodies_are_the_admitted_shapes() {
    let (out, err, ok) = loft(
        &["--interpret", &cells()],
        &[("LOFT_TRACE_PURE_REUSE", "1")],
    );
    assert!(ok && out.contains("done"), "cells failed:\n{out}\n{err}");
    let got: Vec<&str> = err
        .lines()
        .filter(|l| l.starts_with("[pure-reuse]"))
        .collect();
    assert_eq!(got, TRACE, "the reusing bodies moved:\n{err}");
}

#[test]
fn the_cells_answer_the_same_with_the_rule_off() {
    for b in ["--interpret", "--native"] {
        let (out, err, ok) = loft(&[b, &cells()], &[("LOFT_NO_PURE_REUSE", "1")]);
        assert!(
            ok && out.contains("done"),
            "{b} with the rule off:\n{out}\n{err}"
        );
    }
}

#[test]
fn the_sharing_bodies_are_the_admitted_shapes() {
    let (out, err, ok) = loft(
        &["--interpret", &script(SHARED)],
        &[("LOFT_TRACE_PURE_REUSE", "1")],
    );
    assert!(ok && out.contains("done"), "cells failed:\n{out}\n{err}");
    let got: Vec<&str> = err
        .lines()
        .filter(|l| l.starts_with("[pure-reuse]"))
        .collect();
    assert_eq!(got, SHARED_TRACE, "the sharing bodies moved:\n{err}");
}

#[test]
fn the_shared_cells_answer_the_same_on_both_backends_either_way() {
    for b in ["--interpret", "--native"] {
        for env in [&[][..], &[("LOFT_NO_PURE_REUSE", "1")][..]] {
            let (out, err, ok) = loft(&[b, &script(SHARED)], env);
            assert!(ok && out.contains("done"), "{b} {env:?}:\n{out}\n{err}");
        }
    }
}
