// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-r (loft#1426) — the EMISSION AUDIT: the emitted Rust of every hoist-family cell
//! corpus and of the in-repo drawing bench is checked against the rewrite rules'
//! assumptions (`formal/rewrites.md`: R-State one holder per path per frame, R-Refresh no
//! mover on a held path, R-Inputs a twin handed only live holders) by
//! `scripts/emission_audit.py`, at emission time, before any run.  A clean audit that
//! resolves no holder is vacuous, so each corpus must also bind at least one; and the audit
//! must be able to FAIL, so a hand-made double holder is fed to it and must be refused.
use std::path::{Path, PathBuf};
use std::process::Command;

const CORPORA: &[&str] = &[
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-l-in-place-callee-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/P4c-scalar-hoist-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-n-view-def-header-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-o-wrapper-op-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-p-callee-inputs-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-q-hoisted-push-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-s-mint-hoist-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-t-record-push-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-j-move-append-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-x-invariant-literal-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-u-retbuf-adopt-cells.loft",
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-w-selfread-literal-cells.loft",
    "tests/scripts/157-write-hoist.loft",
    "tests/scripts/157-callee-inputs.loft",
    "tests/scripts/157-push-hoist.loft",
    "tests/scripts/157-mint-hoist.loft",
    "tests/scripts/157-record-push.loft",
    "tests/scripts/157-move-append.loft",
    "tests/scripts/157-retbuf-adopt.loft",
    "tests/scripts/157-selfread-literal.loft",
    "bench/12_drawing/bench.loft",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn emit(src: &Path, out: &Path) -> String {
    let status = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted for {} (exit {:?}): {}",
        src.display(),
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read_to_string(out).expect("read the emitted Rust")
}

/// Run the audit; answers `(exit ok, summary line, full output)`.
fn audit(rust: &Path) -> (bool, String, String) {
    let out = Command::new("python3")
        .arg(root().join("scripts/emission_audit.py"))
        .arg(rust)
        .current_dir(root())
        .output()
        .expect("spawn the emission audit");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let summary = text
        .lines()
        .find(|l| l.starts_with("emission audit:"))
        .unwrap_or("")
        .to_string();
    (out.status.success(), summary, text)
}

fn holders_bound(summary: &str) -> usize {
    summary
        .split(';')
        .nth(1)
        .and_then(|s| s.trim().split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[test]
fn every_corpus_emits_within_the_rules_and_binds_a_holder() {
    for (i, corpus) in CORPORA.iter().enumerate() {
        let out = std::env::temp_dir().join(format!("loft_emission_audit_{i}.rs"));
        emit(&root().join(corpus), &out);
        let (ok, summary, text) = audit(&out);
        assert!(
            ok,
            "{corpus}: the emission audit found a violation:\n{text}"
        );
        assert!(
            holders_bound(&summary) >= 1,
            "{corpus}: the audit resolved no holder — a vacuous pass ({summary})"
        );
        let _ = std::fs::remove_file(&out);
    }
}

#[test]
fn the_audit_refuses_a_second_holder_for_one_path() {
    let src = root().join(CORPORA[5]);
    let out = std::env::temp_dir().join("loft_emission_audit_double.rs");
    let rust = emit(&src, &out);
    // Duplicate the first plain header under a new name in the same prelude.
    let mut lines: Vec<String> = rust.lines().map(str::to_owned).collect();
    let at = lines
        .iter()
        .position(|l| {
            l.trim_start().starts_with("let __vh_") && l.contains("vector::vec_header(&(")
        })
        .expect("a plain header to duplicate");
    // `let __vh_999 = <the same path>` right below it: the original's path text, a new name.
    let (head, tail) = lines[at].split_once(" = ").expect("a binding");
    let indent = head.len() - head.trim_start().len();
    lines.insert(
        at + 1,
        format!("{}let __vh_999 = {tail}", " ".repeat(indent)),
    );
    std::fs::write(&out, lines.join("\n")).expect("write the doubled emission");
    let (ok, _, text) = audit(&out);
    assert!(
        !ok && text.contains("R-State") && text.contains("__vh_999"),
        "the audit must refuse a second holder for one path:\n{text}"
    );
    let _ = std::fs::remove_file(&out);
}
