// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 rust-writer: the yield channel's refusal is the Rust writer's — its `--native` runs as `--native-rust`.

//! loft#1132 — `--native` refuses a yield type it has no transport for, and says so.
//!
//! The positive half — every channel that DOES carry its type — is
//! `tests/scripts/1132-a-generator-yield-rides-a-channel-that-carries-it.loft`, which runs on
//! both backends.  A refused program has no run to assert on, so the refusals are pinned here
//! at the emit level instead: `--native-emit` writes the generated Rust without invoking
//! rustc, so the assertion is on the source loft produced.
//!
//! Two things are checked per shape, and the second is the one that decays: the
//! `compile_error!` is PRESENT, and the yielded value is BOUND TO `_` rather than cast.  A
//! refusal that merely adds a message while the bad cast still stands satisfies the first
//! alone, and that was the state of the first attempt at this fix — the message arrived
//! buried under an `E0605` from the producer and an `E0308` from the consumer.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Emit the generated Rust for `src`, without running rustc over it.
fn emit(tag: &str, src: &str) -> String {
    let dir = std::env::temp_dir();
    let lf = dir.join(format!("loft_1132_{tag}_{}.loft", std::process::id()));
    let rf = dir.join(format!("loft_1132_{tag}_{}.rs", std::process::id()));
    fa::write(&lf, src).expect("write probe");
    let st = loft::platform::process::harness_command(loft_bin())
        .args(["--native", "--native-emit", rf.to_str().expect("path")])
        .arg(&lf)
        .env("LOFT_TIMEOUT", "300")
        .status()
        .expect("spawn loft");
    assert!(st.success(), "--native-emit must succeed for {tag}");
    let rs = fa::read_to_string(&rf).expect("read emitted Rust");
    let _ = fa::remove_file(&lf);
    let _ = fa::remove_file(&rf);
    rs
}

/// A tuple carrying a `text` element rides the `next_into` buffer: the text is an owned
/// `String` handed over in one slot (`YieldSlot::Text`), so nothing is refused (loft#1891).
/// The values are checked on both backends by
/// `tests/scripts/a-generator-yields-a-tuple-with-a-text-member-on-both-backends.loft`.
#[test]
fn a_tuple_with_a_text_element_is_carried_not_refused() {
    let rs = emit(
        "text_elem",
        "fn g() -> iterator<(integer, text)> { yield (1, \"a\".to_uppercase()); }\n\
         fn main() { for t in g() { print(\"{t.0} {t.1}\\n\"); } }\n",
    );
    assert!(
        !rs.contains("compile_error!"),
        "a text member has a transport slot now, so no refusal may be emitted"
    );
}

/// The CONTROL for the cell above: a by-value tuple from the same loop shape is carried, not
/// refused, and still answers correctly on `--native`.
///
/// This is the half that says the fix did not close the hole by refusing more.  The branch it
/// touches is reached only when the eager stride is ZERO, and a scalar tuple has a stride — so
/// if this cell ever starts refusing, the guard above is passing for the wrong reason.
///
/// `a_by_value_tuple_from_a_loop_body_is_not_refused` asks the same shape one level shallower —
/// that the EMIT carries no `compile_error!` and packs through the collector.  This one runs
/// rustc and checks the VALUE, which is the half an emit-level read cannot reach: the defect
/// this file gained a cell for was a type error in source that emitted perfectly well.
#[test]
fn a_by_value_tuple_from_a_loop_body_still_runs_on_native() {
    let dir = std::env::temp_dir();
    let lf = dir.join(format!("loft_1467_ok_{}.loft", std::process::id()));
    fa::write(
        &lf,
        "fn g(n: integer) -> iterator<(integer, integer)> {\n\
         \x20 for i in 0..n { yield (i, i * 11); }\n\
         }\n\
         fn main() { s = 0; for t in g(3) { s += t.0 + t.1; } print(\"{s}\\n\"); }\n",
    )
    .expect("write probe");
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native")
        .arg(&lf)
        .env("LOFT_TIMEOUT", "300")
        .output()
        .expect("spawn loft");
    let _ = fa::remove_file(&lf);
    assert!(
        out.status.success(),
        "a by-value tuple yield from a loop body must still COMPILE on --native:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "36",
        "…and answer what the interpreter answers"
    );
}
