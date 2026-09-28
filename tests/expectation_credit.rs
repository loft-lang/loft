// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! An `@EXPECT_ERROR` is credited only by an error that matches it (loft#1261).
//!
//! TESTING.md states the contract as **"Every expectation must match"**, and the
//! file-level check has always held it.  The per-function check settled the same
//! question with `!file_result.errors.is_empty()` — "did the file produce SOME
//! error?" — an existential standing in for a universal.  One firing cell therefore
//! credited every other annotation in the file, whatever its text said.
//!
//! Only the FIRST annotation in a file is file-level (`in_header` ends at the first
//! `fn`/`struct`/`enum`), so in a file with two or more, exactly one was checked and
//! the rest were credited on sight.
//!
//! The reach of that is narrower than it first looks, and the rows below pin the
//! boundary rather than the headline.  A *reworded* diagnostic is still caught, by
//! the `unexpected_errors` filter walking the other way: the error nothing claims is
//! rejected.  What nothing could catch is a refusal that stops being emitted at all
//! while another cell in the file still errors — the annotation goes unmatched, every
//! error present is claimed, and the file passes.  That is
//! `a_refusal_that_stopped_firing_is_caught`, and it is the row that matters: a
//! guarantee quietly stops being enforced and the suite keeps saying it holds.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Write `src` as a single-file package and run `loft test` over it.
fn run_file(tag: &str, src: &str) -> (i32, String) {
    let root = std::env::temp_dir().join(format!("loft_1261_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    let file = root.join("cells.loft");
    std::fs::write(&file, src).expect("write cells.loft");
    let out = Command::new(loft_bin())
        .current_dir(&root)
        .args(["--tests", "cells.loft"])
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run loft --tests");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
    (out.status.code().unwrap_or(-1), text)
}

/// A cell that really refuses.  `const` parameters are used throughout because the
/// refusal is a pass-2 diagnostic with a stable wording that names the parameter, so
/// each cell's expected text is distinct and a cross-match cannot pass by accident.
const REAL: &str = "// @EXPECT_ERROR: Cannot modify const parameter 'pv'\n\
                    fn cell_real(pv: const vector<integer>) { pv += [9]; }\n";

/// A cell that produces no diagnostic at all, declaring one no build has printed.
const BOGUS: &str = "// @EXPECT_ERROR: never printed alpha\n\
                     fn cell_bogus() { aa = 1; assert(aa == 1, \"x\"); }\n";

/// A cell that neither errors nor declares anything — it only ends `in_header`, so
/// the annotations that follow are per-function rather than file-level.
const PLAIN: &str = "fn cell_plain() { z = 1; assert(z == 1, \"z\"); }\n";

fn assert_refused(what: &str, code: i32, out: &str, needle: &str) {
    assert_ne!(code, 0, "{what} must not exit 0:\n{out}");
    assert!(
        out.contains("expected error never emitted"),
        "{what} must be reported as an unmatched expectation:\n{out}"
    );
    assert!(
        out.contains(needle),
        "{what} must name the annotation that went unmatched ({needle}):\n{out}"
    );
    assert!(
        !out.contains("test result: ok."),
        "{what} must not print a green:\n{out}"
    );
}

/// The filed repro: a bogus declaration AFTER a firing one.
#[test]
fn a_bogus_expectation_after_a_firing_cell_is_refused() {
    let (code, out) = run_file("after", &format!("{REAL}{BOGUS}"));
    assert_refused(
        "a bogus expectation after a firing cell",
        code,
        &out,
        "cell_bogus: never printed alpha",
    );
}

/// The axis the report named is not the one that governs.  The issue described the
/// hole as crediting declarations that follow a firing cell; the predicate never
/// looked at position at all, so a bogus declaration BEFORE the firing one was
/// credited just the same.  Only being first in the FILE — ahead of every `fn` —
/// ever protected an annotation, and this row exists so a future fix that restores
/// an order-sensitive rule fails here.
#[test]
fn a_bogus_expectation_before_a_firing_cell_is_refused_too() {
    let (code, out) = run_file("before", &format!("{PLAIN}{BOGUS}{REAL}"));
    assert_refused(
        "a bogus expectation before a firing cell",
        code,
        &out,
        "cell_bogus: never printed alpha",
    );
}

/// The case no other check can reach, and the reason this matters.
///
/// Every error the file produces is claimed by an annotation, so `unexpected_errors`
/// finds nothing to reject; one cell has simply stopped refusing.  Here the `const`
/// is dropped from `cell_quiet`, which is what a real regression looks like — the
/// guarantee lapses, its annotation still asserts it, and before this the suite
/// reported the file green.
#[test]
fn a_refusal_that_stopped_firing_is_caught() {
    let quiet = "// @EXPECT_ERROR: Cannot modify const parameter 'qv'\n\
                 fn cell_quiet(qv: vector<integer>) { qv += [7]; }\n";
    let (code, out) = run_file("lapsed", &format!("{PLAIN}{REAL}{quiet}"));
    assert_refused(
        "a refusal that stopped being emitted",
        code,
        &out,
        "cell_quiet: Cannot modify const parameter 'qv'",
    );
}

/// Every unmatched substring is named, not just the first, so one run tells the
/// reader the whole of what lapsed.
#[test]
fn each_unmatched_substring_is_named() {
    let two = "// @EXPECT_ERROR: never printed alpha\n\
               // @EXPECT_ERROR: never printed beta\n\
               fn cell_bogus() { aa = 1; assert(aa == 1, \"x\"); }\n";
    let (code, out) = run_file("both", &format!("{REAL}{two}"));
    assert_ne!(code, 0, "two bogus substrings must not exit 0:\n{out}");
    assert!(
        out.contains("cell_bogus: never printed alpha")
            && out.contains("cell_bogus: never printed beta"),
        "both unmatched substrings must be named:\n{out}"
    );
}

/// The control.  Without it, a harness that refused every annotated file would pass
/// every row above — which is the failure mode this whole change is about, one level
/// down.  Both cells are per-function (`PLAIN` ends the header), both really refuse,
/// and the file must still be green.
#[test]
fn genuine_expectations_still_pass() {
    let second = "// @EXPECT_ERROR: Cannot modify const parameter 'sv'\n\
                  fn cell_second(sv: const vector<integer>) { sv += [1]; }\n";
    let (code, out) = run_file("genuine", &format!("{PLAIN}{REAL}{second}"));
    assert_eq!(code, 0, "genuine expectations must still pass:\n{out}");
    assert!(
        out.contains("test result: ok."),
        "the file must be reported green:\n{out}"
    );
    assert!(
        !out.contains("expected error never emitted"),
        "nothing may be reported unmatched:\n{out}"
    );
}

/// loft#1727 — one report answers ONE expectation.  Both cells below expect the same
/// text, and only the first still refuses: answered from every diagnostic of the file,
/// the one report satisfied both, so a guard whose cells share a rule's single message
/// checked only that SOME cell fired.  The corpus harness (`tests/wrap.rs`) has always
/// matched one-to-one; this is the `loft test` runner agreeing with it.
#[test]
fn two_cells_sharing_a_message_each_need_their_own_report() {
    let first = "// @EXPECT_ERROR: Cannot modify const parameter\n\
                 fn cell_first(fv: const vector<integer>) { fv += [1]; }\n";
    let lapsed = "// @EXPECT_ERROR: Cannot modify const parameter\n\
                  fn cell_lapsed(lv: vector<integer>) { lv += [2]; }\n";
    let (code, out) = run_file("shared", &format!("{PLAIN}{first}{lapsed}"));
    assert_refused(
        "a second cell sharing the first's message",
        code,
        &out,
        "cell_lapsed: Cannot modify const parameter",
    );
}

/// The control for the row above: both cells refuse, so two reports answer the two
/// expectations and the file is green.
#[test]
fn two_cells_sharing_a_message_pass_when_both_report() {
    let first = "// @EXPECT_ERROR: Cannot modify const parameter\n\
                 fn cell_first(fv: const vector<integer>) { fv += [1]; }\n";
    let second = "// @EXPECT_ERROR: Cannot modify const parameter\n\
                  fn cell_second(sv: const vector<integer>) { sv += [2]; }\n";
    let (code, out) = run_file("shared_ok", &format!("{PLAIN}{first}{second}"));
    assert_eq!(
        code, 0,
        "two reports for two expectations must pass:\n{out}"
    );
    assert!(
        out.contains("test result: ok."),
        "the file must be reported green:\n{out}"
    );
}

/// The warning twin of the shared-message row: `@EXPECT_WARNING` is answered one report
/// per expectation as well.
#[test]
fn two_cells_sharing_a_warning_each_need_their_own_report() {
    let warns = "// @EXPECT_WARNING: Parameter a is never read\n\
                 fn cell_warns(a: integer) { }\n";
    let reads = "// @EXPECT_WARNING: Parameter a is never read\n\
                 fn cell_reads(a: integer) -> integer { a }\n";
    let (code, out) = run_file("shared_warn", &format!("{PLAIN}{warns}{reads}"));
    assert_ne!(
        code, 0,
        "a second warning cell with no report must not exit 0:\n{out}"
    );
    assert!(
        out.contains("expected warnings not found for: cell_reads"),
        "the silent cell must be named:\n{out}"
    );
}
