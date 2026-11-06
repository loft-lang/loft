// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! A join's arm ORDER does not change how its callers lower (`(O-Move)`, `(O-Deps)`).
//!
//! `fn f(s: S, i) -> S { if c { S { … } } else { s } }` borrows `s` on every path, exactly as
//! the arms written the other way round do, and both signatures read `S["s"]`.  The literal
//! arm's built-in-place marker (`Rewritten`) is a fact about that arm alone; kept on the whole
//! join on pass 1, it hid the join's borrow from the record-return delivery, the borrow reached
//! the signature a pass late, and a caller rebinding a literal-holding local to the call built
//! that literal in place for one order and apart for the other.  Both lowerings were sound, so
//! no value shows the difference — `tests/scripts/a-join-arms-order-does-not-change-the-callers-lowering.loft`
//! asserts the values on both backends; this pins the IR.
use std::path::{Path, PathBuf};

const GUARD: &str = "tests/scripts/a-join-arms-order-does-not-change-the-callers-lowering.loft";

/// One function's IR as `loft introspect` prints it, its own name, its callee's name and the
/// source line markers neutralised, so two callers on different lines compare equal.
fn caller_ir(text: &str, caller: &str, callee: &str) -> String {
    let head = format!("fn n_{caller}(");
    let start = text
        .match_indices(&head)
        .map(|(i, _)| i)
        .find(|i| {
            text[*i..]
                .lines()
                .next()
                .is_some_and(|l| l.contains("{#block"))
        })
        .unwrap_or_else(|| panic!("{caller}'s IR"));
    let rest = &text[start..];
    let end = rest.find("}#block(1)").expect("the closing block");
    let body = rest[..end]
        .replace(&format!("n_{caller}("), "n_CALLER(")
        .replace(&format!("n_{callee}("), "n_CALLEE(");
    body.lines()
        .map(|l| {
            let t = l.trim_start();
            let indent = &l[..l.len() - t.len()];
            match t.strip_prefix('[').and_then(|r| r.split_once("] ")) {
                Some((n, after)) if n.bytes().all(|b| b.is_ascii_digit()) => {
                    format!("{indent}{after}")
                }
                _ => l.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_arm_order_and_spelling_lowers_its_caller_alike() {
    let guard = Path::new(env!("CARGO_MANIFEST_DIR")).join(GUARD);
    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("introspect")
        .arg(&guard)
        .env("LOFT_TIMEOUT", "240")
        .output()
        .expect("spawn loft");
    assert!(
        out.status.success(),
        "introspect failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let base = caller_ir(&text, "run_par_first", "par_first");
    // The parameter-first order always published its borrow on pass 1, so its caller builds
    // the rebound local's literal APART — the lowering every other spelling must match.
    assert!(
        base.contains("{#Object(6):ref(S)[\"__ref_p2_1\"]"),
        "the parameter-first caller no longer builds its literal apart:\n{base}"
    );
    for (caller, callee) in [
        ("run_lit_first", "lit_first"),
        ("run_chain", "chain"),
        ("run_by_match", "by_match"),
    ] {
        let ir = caller_ir(&text, caller, callee);
        assert_eq!(
            ir, base,
            "{caller} lowers differently from run_par_first: the join in {callee} hid its \
             parameter borrow from pass 1"
        );
    }
}
