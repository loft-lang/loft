// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-PureReuse`, PINNED on its cells (`tests/scripts/a-pure-call-made-twice-is-computed-once.loft`):
//! which bodies compute an effect-free call once, read off `LOFT_TRACE_PURE_REUSE`, and how
//! many calls of it the request-check shape emits with the rule on and off.  An effect-free
//! call cannot count itself, so only this pin sees a reuse that stops firing.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-pure-call-made-twice-is-computed-once.loft";

/// Every reuse the cells make, in order: the request-check shape, the two projections, and
/// the request-check and record-field shapes in the natural spelling (`parse(v).ok`).
const TRACE: &[&str] = &[
    "[pure-reuse] n_check: n_parse computed once",
    "[pure-reuse] n_p6: n_parse computed once",
    "[pure-reuse] n_check_n: n_parse computed once",
    "[pure-reuse] n_spanned: n_parse computed once",
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
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CELLS)
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

/// The emitted body of `fn <name>(` up to the next top-level `fn`.
fn body<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} not emitted"));
    let rest = &src[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

#[test]
fn the_request_check_calls_its_parser_once() {
    for (env, want) in [(&[][..], 1usize), (&[("LOFT_NO_PURE_REUSE", "1")][..], 0)] {
        let rs = std::env::temp_dir().join(format!(
            "loft_pure_reuse_{}_{}.rs",
            std::process::id(),
            want
        ));
        let (_, err, ok) = loft(&["--native-emit", &rs.to_string_lossy(), &cells()], env);
        assert!(ok, "{env:?}: emit failed:\n{err}");
        let src = fa::read_to_string(&rs).expect("emitted source");
        let _ = fa::remove_file(&rs);
        let check = body(&src, "n_check");
        // With the rule `check` calls the parser itself, once; without it, it calls the two
        // wrappers and no parser of its own.
        assert_eq!(
            check.matches("n_parse(").count(),
            want,
            "{env:?}: n_check's own n_parse calls"
        );
        assert_eq!(
            check.matches("n_parse_ok(").count() + check.matches("n_parse_total(").count(),
            2 - 2 * want,
            "{env:?}: n_check's wrapper calls"
        );
    }
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
