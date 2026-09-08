// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Emit the generated Rust for `src`, without running rustc over it.
fn emit(tag: &str, src: &str) -> String {
    let dir = std::env::temp_dir();
    let lf = dir.join(format!("loft_1132_{tag}_{}.loft", std::process::id()));
    let rf = dir.join(format!("loft_1132_{tag}_{}.rs", std::process::id()));
    std::fs::write(&lf, src).expect("write probe");
    let st = Command::new(loft_bin())
        .args(["--native-emit", rf.to_str().expect("path")])
        .arg(&lf)
        .env("LOFT_TIMEOUT", "300")
        .status()
        .expect("spawn loft");
    assert!(st.success(), "--native-emit must succeed for {tag}");
    let rs = std::fs::read_to_string(&rf).expect("read emitted Rust");
    let _ = std::fs::remove_file(&lf);
    let _ = std::fs::remove_file(&rf);
    rs
}

/// The producer refuses, and nothing else in the emit still tries to cast the value.
fn assert_refused(tag: &str, src: &str, named: &str) {
    let rs = emit(tag, src);
    assert!(
        rs.contains("has no native transport channel")
            || rs.contains("cannot be collected from a generator's LOOP body"),
        "{tag}: a yield type with no channel must be REFUSED by name, not cast — \
         the emit carries neither refusal"
    );
    assert!(
        rs.contains(named),
        "{tag}: the refusal must name the yield type `{named}` — a message the author \
         cannot match against their own source is a rustc dump with extra steps"
    );
    assert!(
        rs.contains("compile_error!"),
        "{tag}: the refusal has to STOP the build, not warn inside it"
    );
    assert!(
        rs.contains("); let _ = (") || rs.contains("{ let _ = ("),
        "{tag}: the refused value must be BOUND TO `_`, not cast — the cast is the `E0605` \
         the refusal exists to replace, and leaving it in buries the message under it"
    );
}

/// A tuple carrying a `text` element: `tuple_kinds` cannot classify it, and the legacy
/// channel it fell through to ends in `(i64, &String) as i64`.
#[test]
fn a_tuple_with_a_text_element_is_refused_by_name() {
    assert_refused(
        "text_elem",
        "fn g() -> iterator<(integer, text)> { yield (1, \"a\".to_uppercase()); }\n\
         fn main() { for t in g() { print(\"{t.0} {t.1}\\n\"); } }\n",
        "(integer, text)",
    );
}

/// A NESTED tuple — the other shape `tuple_kinds` answers `None` for, and the one that
/// shows the refusal is about the classification and not about `text`.
#[test]
fn a_nested_tuple_yield_is_refused_by_name() {
    assert_refused(
        "nested",
        "struct P1132 { n: integer }\n\
         fn g() -> iterator<((integer, P1132), integer)> { yield ((1, P1132 { n: 11 }), 5); }\n\
         fn main() { for t in g() { print(\"{t.1}\\n\"); } }\n",
        "((integer, P1132), integer)",
    );
}

/// A tuple carrying a store HANDLE, from a loop body.  The type has a channel — it is the
/// eager collector that cannot hold it, because a handle pushed once per iteration aliases
/// the work record the next iteration overwrites.  So this cell refuses for a different
/// reason than the two above, and the straight-line form of the SAME type must not.
#[test]
fn a_handle_carrying_tuple_from_a_loop_body_is_refused_but_not_straight_line() {
    assert_refused(
        "loop_handle",
        "struct P1132 { n: integer }\n\
         fn g(k: integer) -> iterator<(integer, P1132)> {\n\
         \x20 i = 0;\n\
         \x20 while i < k { yield (i, P1132 { n: i * 10 }); i += 1; }\n\
         }\n\
         fn main() { for t in g(3) { print(\"{t.0} {t.1.n}\\n\"); } }\n",
        "(integer, P1132)",
    );
    let straight = emit(
        "straight_handle",
        "struct P1132 { n: integer }\n\
         fn g() -> iterator<(integer, P1132)> { yield (1, P1132 { n: 10 }); }\n\
         fn main() { for t in g() { print(\"{t.0} {t.1.n}\\n\"); } }\n",
    );
    assert!(
        !straight.contains("compile_error!"),
        "the SAME yield type is fine straight-line — it is the eager collector that cannot \
         hold a handle, so refusing the type outright would take a working shape with it"
    );
}

/// A refused type whose NAME carries quotes — a keyed collection renders its key list as
/// `spatial<P,["x", "y"]>`, and the message splices that name into a Rust string literal.
///
/// Spliced raw, the first quote ends the literal and the comma becomes a second macro
/// argument, so the author gets `compile_error! takes 1 argument` plus a suffix error rather
/// than the refusal — the rustc noise this path exists to replace (loft#1149).
#[test]
fn a_refused_type_whose_name_contains_quotes_still_renders_one_message() {
    let rs = emit(
        "quoted_name",
        "struct Pq { x: integer, y: integer, n: integer }\n\
         fn g() -> iterator<(spatial<Pq[x,y]>, text)> {\n\
         \x20 a: spatial<Pq[x,y]> = [Pq { x: 1, y: 2, n: 3 }];\n\
         \x20 yield (a, \"hi\");\n\
         }\n\
         fn main() { for t in g() { print(\"{t.1}\\n\"); } }\n",
    );
    assert!(
        rs.contains("has no native transport channel"),
        "the refusal must still be emitted for a type whose name carries quotes"
    );
    assert!(
        rs.contains("spatial<Pq,[\\\"x\\\", \\\"y\\\"]>"),
        "and the name must be ESCAPED into the literal — an unescaped quote ends it early, \
         which is what turned one message into two rustc errors"
    );
    assert!(
        !rs.contains("[\"x\", \"y\"]>` has no native"),
        "…so the raw, unescaped rendering must not appear inside the message literal"
    );
}

/// The control that keeps the refusal from widening: a by-value tuple from a loop body is
/// exactly what the eager buffer was taught to carry, so it must emit no refusal at all.
#[test]
fn a_by_value_tuple_from_a_loop_body_is_not_refused() {
    let rs = emit(
        "by_value",
        "fn g(k: integer) -> iterator<(integer, boolean)> {\n\
         \x20 for i in 0..k { yield (i * 10, i % 2 == 0); }\n\
         }\n\
         fn main() { for t in g(3) { print(\"{t.0} {t.1}\\n\"); } }\n",
    );
    assert!(
        !rs.contains("compile_error!"),
        "a tuple whose every element is carried BY VALUE packs into the eager buffer flat; \
         refusing it would give up the row the decided edge in formal/coroutines.md claims"
    );
    assert!(
        rs.contains("__values.push("),
        "…and it packs through the eager collector, which is what the stride pop reads back"
    );
}

/// loft#1467 — a refused generator emits its refusal AND NOTHING ELSE.
///
/// The two tests above pin the message at the EMIT level, which is where the first attempt at
/// loft#1132 was measured and found wanting.  This one runs rustc, because the defect it
/// guards is invisible until rustc reads the file: the refusal was correct and complete, and
/// the generator's `next_into` was still emitted for a channel it had just refused, ending in
/// `return v;` with `v: i64` inside a method answering `bool`.  So the author got the right
/// message with a codegen dump attached — the outcome loft#1132's write-up says must not
/// happen, reintroduced one channel over.
///
/// The refusal is planted by the eager COLLECTOR; what was missing here was never the message
/// but a BODY THAT TYPE-CHECKS beside it, so the message is not accompanied.  That is why this
/// asserts on rustc's output and not on the emitted source: an emit-level check cannot see a
/// type error, which is exactly how this shape passed the tests above.
#[test]
fn a_refused_loop_body_yield_emits_the_refusal_and_no_rustc_error() {
    let dir = std::env::temp_dir();
    let lf = dir.join(format!("loft_1467_{}.loft", std::process::id()));
    std::fs::write(
        &lf,
        "struct Ck1467 { k: integer, v: integer }\n\
         fn g(n: integer) -> iterator<(Ck1467, integer)> {\n\
         \x20 for i in 0..n { yield (Ck1467 { k: i, v: i * 11 }, i); }\n\
         }\n\
         fn main() { s = 0; for t in g(3) { s += t.1 + t.0.v; } print(\"{s}\\n\"); }\n",
    )
    .expect("write probe");
    let out = Command::new(loft_bin())
        .arg(&lf)
        .env("LOFT_TIMEOUT", "300")
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_file(&lf);
    let err = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(
        err.contains("cannot be collected from a generator's LOOP body"),
        "the refusal itself must still be delivered — without it this test would pass on a \
         build that simply stopped refusing:\n{err}"
    );
    assert!(
        !err.contains("error[E"),
        "the refusal must be the ONLY error: a rustc code beside it is generated source the \
         author cannot read, for a program `--interpret` runs correctly:\n{err}"
    );
    // ONE refusal, not two.  Planting a second `compile_error!` in the advance would also
    // remove the rustc code, and would fail the same "one message" rule a different way.
    assert_eq!(
        err.matches("cannot be collected from a generator's LOOP body")
            .count(),
        1,
        "the refusal must be delivered exactly once:\n{err}"
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
    std::fs::write(
        &lf,
        "fn g(n: integer) -> iterator<(integer, integer)> {\n\
         \x20 for i in 0..n { yield (i, i * 11); }\n\
         }\n\
         fn main() { s = 0; for t in g(3) { s += t.0 + t.1; } print(\"{s}\\n\"); }\n",
    )
    .expect("write probe");
    let out = Command::new(loft_bin())
        .arg(&lf)
        .env("LOFT_TIMEOUT", "300")
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_file(&lf);
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
