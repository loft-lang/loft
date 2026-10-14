// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 case-D strict-index lint — the ORACLE (regression net for the gated
//! `LOFT_LINT_STRICT_INDEX` warning: `objects.rs::parse_in_range_body` capture +
//! `fields.rs` emit).
//!
//! The lint is **default OFF** (an opt-in audit): the index-trust model types `v[i]` non-null
//! for a for-loop iter var, trusting the loop bounds the vector. That trust is unchecked, so
//! `for i in 0..len(v) { w[i] }` (w != v) types non-null yet reads C80-null on overrun. When
//! `LOFT_LINT_STRICT_INDEX=1` is set, the lint warns where a loop-var index is bounded by
//! `len(<one vector>)` but indexes a DIFFERENT vector — the mismatched-vector silent-null
//! hazard. It is advisory: the element type stays non-null (a real proof would break the
//! ubiquitous `for i in 0..n { v[i] }` idiom).
//!
//! The corpus (`doc/claude/plans/102-stability-contract/strict-index-corpus.loft`) labels each
//! function FIRE (must warn under the flag) or OK (silent regardless). This locks: (1) default
//! OFF = zero warnings; (2) flag ON = exactly the three FIRE sites (read, write, field-vector),
//! every OK case silent; (3) both backends agree (it is a parse-time lint, so `--native` must
//! match `--interpret`).
//!
//! Binary-invoked (not the in-process harness) like `tests/dead_code_lint.rs`: these are
//! end-to-end compile diagnostics on stderr. `LOFT_NO_CACHE` because the warm program cache
//! skips the re-parse (hence the diagnostics) on a warm run.

use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("doc/claude/plans/102-stability-contract/strict-index-corpus.loft")
}

/// Message-only substring — NOT "strict-index" (that also appears in the corpus filename on the
/// `-->` source-location line, so it would double-count every warning).
const LINT_MSG: &str = "is typed non-null but reads null on overrun";

fn run(backend: &str, flag_on: bool) -> (String, String, Option<i32>) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg(corpus())
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1");
    if flag_on {
        cmd.env("LOFT_LINT_STRICT_INDEX", "1");
    } else {
        cmd.env_remove("LOFT_LINT_STRICT_INDEX");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn lint_count(diag: &str) -> usize {
    diag.matches(LINT_MSG).count()
}

// ── Default OFF: zero warnings, corpus runs clean ────────────────────────────────────────────

fn assert_default_off(backend: &str) {
    let (stdout, diag, code) = run(backend, false);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] corpus did not exit 0\n{stdout}\n---\n{diag}"
    );
    assert_eq!(
        lint_count(&diag),
        0,
        "[{backend}] strict-index lint must be OFF by default\n{diag}"
    );
}

#[test]
fn default_off_interpret() {
    assert_default_off("--interpret");
}

#[test]
fn default_off_native() {
    assert_default_off("--native");
}

// ── Flag ON: exactly the three FIRE sites, every OK case silent ──────────────────────────────

fn assert_flag_on(backend: &str) {
    let (stdout, diag, code) = run(backend, true);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] corpus did not exit 0 under the flag\n{stdout}\n---\n{diag}"
    );

    // Exactly three warnings — the FIRE trio. More = a false positive on an OK case (matched
    // vector, plain `0..n` range, matched field, nested); fewer = the lint regressed.
    let n = lint_count(&diag);
    assert_eq!(
        n, 3,
        "[{backend}] want exactly 3 strict-index warnings (read/write/field mismatch), got {n}\n{diag}"
    );

    // Each FIRE fires on the right index variable; the field mismatch proves VecKey::Field.
    assert!(
        diag.contains("index `i`"),
        "[{backend}] the mismatched-vector index `i` must be named\n{diag}"
    );

    // Load-bearing OK guards: no warning may land in a matched-vector, plain-range, matched-field,
    // or nested-matched loop. All OK functions read their OWN bound vector, so `v`/`w`/`s.a` are
    // never the misindexed target — the only way `n == 3` above holds is if every OK case is
    // silent, but assert the corpus shape didn't drift into an OK function warning by checking the
    // FIRE line numbers are the ones that fire.
    for (line, what) in [
        (18, "mismatch_read w[i]"),
        (24, "mismatch_write w[i]="),
        (31, "mismatch_field s.b[i]"),
    ] {
        assert!(
            diag.contains(&format!("strict-index-corpus.loft:{line}:")),
            "[{backend}] the FIRE at line {line} ({what}) must warn\n{diag}"
        );
    }
}

#[test]
fn flag_on_interpret() {
    assert_flag_on("--interpret");
}

#[test]
fn flag_on_native() {
    assert_flag_on("--native");
}
