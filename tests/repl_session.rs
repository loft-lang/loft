// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 03 slice B — REPL session variable persistence.
//!
//! A variable bound in one input is visible to the next.  Each test's `assert`
//! holding (no panic) proves the prior binding was in scope with the right
//! value.  Integer and text bindings both persist.

use loft::debugger::StepMode;
use loft::repl::{Eval, ReplSession};
use std::path::PathBuf;

fn session() -> ReplSession {
    ReplSession::new("default").expect("load stdlib")
}

/// A unique temp path for a session file, keyed by the test's tag + this
/// process's id so parallel test binaries can't collide.
fn tmp_session(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("loft_test_{tag}_{}.session", std::process::id()))
}

#[test]
fn integer_variable_persists_across_inputs() {
    let mut s = session();
    assert!(matches!(s.eval("x = 1"), Eval::Ran));
    assert!(matches!(
        s.eval("assert(x + 1 == 2, \"x persists\")"),
        Eval::Ran
    ));
}

#[test]
fn variable_depends_on_earlier_variable() {
    let mut s = session();
    assert!(matches!(s.eval("a = 10"), Eval::Ran));
    assert!(matches!(s.eval("b = a * 2"), Eval::Ran));
    assert!(matches!(s.eval("assert(b == 20, \"b == a*2\")"), Eval::Ran));
}

#[test]
fn rebinding_updates_value() {
    let mut s = session();
    assert!(matches!(s.eval("n = 5"), Eval::Ran));
    assert!(matches!(s.eval("n = n + 100"), Eval::Ran));
    assert!(matches!(
        s.eval("assert(n == 105, \"n updated\")"),
        Eval::Ran
    ));
}

#[test]
fn incomplete_input_asks_for_more() {
    let mut s = session();
    assert!(matches!(s.eval("y = (1 +"), Eval::NeedMore));
}

/// A parse error is reported (and `data` is rolled back).
#[test]
fn parse_error_is_reported() {
    let mut s = session();
    assert!(matches!(s.eval("z = 3"), Eval::Ran));
    assert!(matches!(s.eval("z = 1 2 3"), Eval::Error(_)));
}

/// Recovery after a parse error: a clean input still works.  The lexer's
/// diagnostics are cleared per `parse_str`, so a prior error no longer
/// re-`fill`s into the parser and poisons the next parse.
#[test]
fn parse_error_leaves_session_usable() {
    let mut s = session();
    assert!(matches!(s.eval("z = 3"), Eval::Ran));
    assert!(matches!(s.eval("z = 1 2 3"), Eval::Error(_)));
    assert!(matches!(
        s.eval("assert(z == 3, \"z survived error\")"),
        Eval::Ran
    ));
}

/// Text bindings persist across inputs, the same as integers — the on-ramp
/// case a beginner hits first (`name = "Alice"`).  Confirms persistence is not
/// integer-only.
#[test]
fn text_variable_persists_across_inputs() {
    let mut s = session();
    assert!(matches!(s.eval("name = \"Alice\""), Eval::Ran));
    assert!(matches!(
        s.eval("assert(name == \"Alice\", \"name persists\")"),
        Eval::Ran
    ));
}

// ── REPL.S: auto-resume via text-replay ──────────────────────────────────────

/// A session's state-changing inputs (a binding + a def) are persisted, and a
/// fresh session resumes them: the variable and the function are both usable
/// again, with no entries skipped.
#[test]
fn session_persists_and_resumes() {
    let path = tmp_session("resume");
    let _ = std::fs::remove_file(&path);
    {
        let mut a = session();
        a.enable_persistence(&path).expect("enable persistence");
        assert!(matches!(a.eval("x = 41"), Eval::Ran));
        assert!(matches!(
            a.eval("fn dbl(n: integer) -> integer { n + n }"),
            Eval::Ran
        ));
    } // `a` dropped — its session file stays on disk
    let mut b = session();
    let stats = b.resume_from(&path);
    assert_eq!(stats.restored, 2, "binding + def restored: {stats:?}");
    assert_eq!(stats.skipped, 0, "nothing skipped: {stats:?}");
    assert!(matches!(
        b.eval("assert(x == 41, \"x restored\")"),
        Eval::Ran
    ));
    assert!(matches!(
        b.eval("assert(dbl(x) == 82, \"dbl restored\")"),
        Eval::Ran
    ));
    let _ = std::fs::remove_file(&path);
}

/// A stale/corrupt entry between two good ones is skipped, not fatal: the two
/// good bindings still restore and the session stays usable.  This is the
/// fault-tolerance guarantee — a poison line never bricks resume.
#[test]
fn resume_skips_poison_entry() {
    let path = tmp_session("poison");
    // good binding, garbage (a parse error, same shape as `z = 1 2 3`), good
    // binding — NUL-separated exactly as the REPL writes them.
    std::fs::write(&path, "a = 1\0c = 1 2 3\0b = 2\0").expect("write session");
    let mut s = session();
    let stats = s.resume_from(&path);
    assert_eq!(stats.restored, 2, "two good entries restored: {stats:?}");
    assert_eq!(stats.skipped, 1, "one poison entry skipped: {stats:?}");
    assert!(matches!(
        s.eval("assert(a + b == 3, \"a,b survived poison\")"),
        Eval::Ran
    ));
    let _ = std::fs::remove_file(&path);
}

// ── REPL.C: Tab-completion candidates ────────────────────────────────────────

/// `completion_names` covers the user's session (bound vars, defined fns) plus
/// stdlib globals, and excludes the synthetic generation wrappers and internal
/// `<…>`-shaped names.  The list is sorted.
#[test]
fn completion_names_cover_session_and_stdlib() {
    let mut s = session();
    assert!(matches!(s.eval("price = 10"), Eval::Ran));
    assert!(matches!(
        s.eval("fn tripled(n: integer) -> integer { n * 3 }"),
        Eval::Ran
    ));
    let names = s.completion_names();
    assert!(names.iter().any(|n| n == "price"), "bound var: {names:?}");
    assert!(names.iter().any(|n| n == "tripled"), "user fn: {names:?}");
    assert!(
        names.iter().any(|n| n == "println"),
        "stdlib global: {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with("replmain") || n.contains('<')),
        "no synthetic/internal names leak in: {names:?}"
    );
    assert!(names.windows(2).all(|w| w[0] <= w[1]), "sorted: {names:?}");
}

/// `completion_model` resolves dotted-access members from the live schema: a
/// struct-typed variable exposes its fields; a value of any type exposes its
/// methods (rendered `name(`); an enum *type* exposes its variant names.  This
/// is REPL.C member completion (the `complete_word` matching is unit-tested in
/// `repl.rs`).
#[test]
fn completion_model_resolves_members_from_schema() {
    let mut s = session();
    assert!(matches!(
        s.eval("struct Point { x: integer, y: integer }"),
        Eval::Ran
    ));
    assert!(matches!(
        s.eval("enum Color { Red, Green, Blue }"),
        Eval::Ran
    ));
    assert!(matches!(s.eval("p = Point { x: 1, y: 2 }"), Eval::Ran));
    assert!(matches!(s.eval("s = \"hi\""), Eval::Ran));
    let model = s.completion_model();

    // Struct variable → its fields (membership, not equality: a struct may also
    // carry generated methods).
    let p = model.members.get("p").expect("p has members");
    assert!(
        p.contains(&"x".to_string()) && p.contains(&"y".to_string()),
        "struct fields present: {p:?}"
    );

    // A text variable → its methods, each with a trailing `(`.
    let sm = model.members.get("s").expect("s has members");
    assert!(
        sm.contains(&"starts_with(".to_string()),
        "text method `starts_with(` present: {sm:?}"
    );
    assert!(
        sm.iter().all(|m| m.ends_with('(')),
        "every text member is a callable method: {sm:?}"
    );

    // Enum *type* → its variants (sorted, bare — no trailing paren).
    assert_eq!(
        model.members.get("Color"),
        Some(&vec![
            "Blue".to_string(),
            "Green".to_string(),
            "Red".to_string()
        ]),
        "enum type → variant names: {:?}",
        model.members
    );

    // Each member list is sorted (the order `complete_word` relies on).
    for (recv, ms) in &model.members {
        assert!(
            ms.windows(2).all(|w| w[0] <= w[1]),
            "members of `{recv}` sorted: {ms:?}"
        );
    }
}

/// Only state-changing inputs are persisted: an observing statement (a bare
/// expression) is evaluated for its echo but writes nothing to the session
/// file, so resume never re-runs it.
#[test]
fn observe_not_persisted() {
    let path = tmp_session("observe");
    let _ = std::fs::remove_file(&path);
    let mut s = session();
    s.enable_persistence(&path).expect("enable persistence");
    assert!(matches!(s.eval("k = 9"), Eval::Ran)); // binding → persisted
    assert!(matches!(s.eval("k + 1"), Eval::Ran)); // observing → NOT persisted
    drop(s);
    let contents = std::fs::read_to_string(&path).expect("session file exists");
    assert_eq!(
        contents, "k = 9\0",
        "only the binding persisted, not the observe: {contents:?}"
    );
    let _ = std::fs::remove_file(&path);
}

// ── @PLN16 G1: REPL :break command ───────────────────────────────────────────

/// `add_breakpoint("dbl")` (the `:break dbl` command) breaks at the named
/// function's body start; an observing call that runs it captures the frame into
/// `last_hits` with the argument value.
#[test]
fn repl_breakpoint_by_fn_name_captures_frame() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn dbl(n: integer) -> integer { n + n }"),
        Eval::Ran
    ));
    s.add_breakpoint("dbl");
    assert_eq!(s.breakpoints(), ["dbl"]);
    // an observing call that runs dbl fires the breakpoint
    assert!(matches!(s.eval("dbl(21)"), Eval::Ran));
    let hits = s.last_hits();
    assert_eq!(hits.len(), 1, "fired once: {hits:?}");
    assert_eq!(hits[0].function, "dbl");
    assert!(
        hits[0].locals.iter().any(|(n, v)| n == "n" && v == "21"),
        "n == 21: {hits:?}"
    );
    // clearing removes it; a later call captures nothing
    s.clear_breakpoints();
    assert!(s.breakpoints().is_empty());
    assert!(matches!(s.eval("dbl(99)"), Eval::Ran));
    assert!(s.last_hits().is_empty(), "no breakpoint → no hits");
}

/// A `<fn>:<line>` spec breaks at a specific line.  An unknown function is skipped,
/// not an error.  A **bare line** isn't unique in the REPL (every input restarts
/// line numbering under `<repl>`), so it resolves to nothing — function-scoped
/// specs are the only unique form.
#[test]
fn repl_breakpoint_fn_line_and_unknown() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn step(n: integer) -> integer {\n  m = n + 1;\n  m * 2\n}"),
        Eval::Ran
    ));
    s.add_breakpoint("step:2"); // the `m = n + 1` line
    assert!(matches!(s.eval("step(4)"), Eval::Ran));
    assert!(
        s.last_hits().iter().any(|h| h.function == "step"),
        "fn:line breakpoint fired: {:?}",
        s.last_hits()
    );
    // an unknown function is skipped (no panic, no hit), session stays usable
    s.clear_breakpoints();
    s.add_breakpoint("does_not_exist");
    assert!(matches!(s.eval("step(4)"), Eval::Ran));
    assert!(s.last_hits().is_empty(), "unknown fn → no hit");
    // a bare line resolves to nothing (not unique) — no hit
    s.clear_breakpoints();
    s.add_breakpoint("2");
    assert!(matches!(s.eval("step(4)"), Eval::Ran));
    assert!(s.last_hits().is_empty(), "bare line → no hit (not unique)");
}

/// @PLN16 G1 (interactive) — with stepping on, observing a call that hits a
/// breakpoint **suspends** into the paused sub-mode; edit a value in the frame,
/// continue, and the edit is picked up — the full pause → edit → resume cycle
/// driven through one held session.  `calc(5)` is normally 50; editing `n` to 99
/// makes it 990 on resume, which the program's assert then confirms.
#[test]
fn repl_interactive_break_edit_continue() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn calc(n: integer) -> integer {\n  n * 10\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("calc");
    // The assert is true only with the edit (990), false without it (50) — so a
    // clean run proves the edited value was used.
    assert!(matches!(
        s.eval("assert(calc(5) == 990, \"edited\")"),
        Eval::Paused
    ));
    assert!(s.is_debugging(), "suspended inside calc");
    let f = s.paused_frame().expect("frame");
    assert_eq!(f.function, "calc");
    assert!(
        f.locals.iter().any(|(n, v)| n == "n" && v == "5"),
        "n == 5 at the pause (pre-multiply): {f:?}"
    );
    // Edit `n` in the live frame; the frame view refreshes to the new value.
    assert!(s.debug_set("n", "99"), "write-back n = 99");
    assert!(
        s.paused_frame()
            .unwrap()
            .locals
            .iter()
            .any(|(n, v)| n == "n" && v == "99"),
        "frame refreshed to n == 99: {:?}",
        s.paused_frame()
    );
    // Continue → resumes with the edit; the run finishes and the sub-mode is left.
    assert!(!s.debug_continue(), "run finished (no further pause)");
    assert!(!s.is_debugging(), "back to normal mode");
    // Breakpoints persist across runs, so calc(2) would suspend again; clear them
    // and confirm the session evaluates normally after a debug run.
    s.clear_breakpoints();
    assert!(
        matches!(s.eval("calc(2)"), Eval::Ran),
        "session still works"
    );
}

/// @PLN16 G1 (interactive) — the step verbs at the REPL: from `outer`'s call
/// line, step **into** `inner`, then step **out** back to `outer`, then continue
/// to completion — all through the held session.
#[test]
fn repl_interactive_step_into_and_out() {
    let mut s = session();
    for d in [
        "fn inner(x: integer) -> integer {\n  x + 1\n}",
        "fn outer(n: integer) -> integer {\n  a = inner(n);\n  a + 100\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("outer:2"); // the `a = inner(n)` line
    assert!(matches!(s.eval("outer(5)"), Eval::Paused));
    assert_eq!(s.paused_frame().unwrap().function, "outer");
    assert!(s.debug_step(StepMode::Into), "into inner");
    let f = s.paused_frame().unwrap();
    assert_eq!(f.function, "inner", "stepped into inner: {f:?}");
    assert!(
        f.locals.iter().any(|(n, v)| n == "x" && v == "5"),
        "inner's x == 5: {f:?}"
    );
    assert!(s.debug_step(StepMode::Out), "out back to outer");
    assert_eq!(s.paused_frame().unwrap().function, "outer");
    assert!(!s.debug_continue(), "finishes");
    assert!(!s.is_debugging());
}

/// @PLN16 G1 (interactive) — the REPL-at-frame: at a pause, evaluate arbitrary
/// expressions against the live frame (not just read a value back).  Covers a
/// heap (struct) argument and an integer arg, reads compound expressions, and
/// confirms a live edit is reflected in a subsequent frame eval.
#[test]
fn repl_interactive_eval_at_frame() {
    let mut s = session();
    for d in [
        "struct Point { x: integer, y: integer }",
        "fn area(pt: Point, k: integer) -> integer {\n  pt.x * pt.y * k\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("area");
    assert!(matches!(
        s.eval("area(Point { x: 3, y: 4 }, 2)"),
        Eval::Paused
    ));
    // Read expressions against the frame's live variables.
    assert_eq!(s.debug_eval("k").as_deref(), Some("2"), "bare arg");
    assert_eq!(
        s.debug_eval("pt.x * pt.y").as_deref(),
        Some("12"),
        "struct field arithmetic"
    );
    assert_eq!(s.debug_eval("pt.x + k").as_deref(), Some("5"), "mixed args");
    // A live integer edit is reflected in a later frame eval.
    assert!(s.debug_set("k", "10"), "edit k");
    assert_eq!(
        s.debug_eval("pt.x * pt.y * k").as_deref(),
        Some("120"),
        "eval reflects the edit"
    );
    // A nonsense expression yields None, not a panic; the session stays paused.
    assert_eq!(s.debug_eval("no_such_var + 1"), None);
    assert!(s.is_debugging(), "still paused after a failed eval");
    assert!(!s.debug_continue());
}

/// @PLN16 D2 — a **bare heap local** (struct / vector) is read **live, in place** at the
/// pause: `show_loft` renders the actual `DbRef` from the store, with no reconstruct and
/// no fn-return deep-copy.  A bare `vector` is the case the reconstruct-eval path could
/// not return (it faulted), so this proves the live read both fixes it and is faithful.
#[test]
fn repl_interactive_eval_bare_heap_live() {
    let mut s = session();
    for d in [
        "struct Mob { hp: integer }",
        "fn run(v: vector<integer>, m: Mob) -> integer {\n  v[0] + m.hp\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("run");
    assert!(matches!(
        s.eval("run([10, 20, 30], Mob { hp: 7 })"),
        Eval::Paused
    ));
    assert_eq!(
        s.debug_eval("v").as_deref(),
        Some("[10,20,30]"),
        "bare vector — live read (the case the reconstruct path faulted on)"
    );
    assert_eq!(
        s.debug_eval("m").as_deref(),
        Some("Mob{hp:7}"),
        "bare struct — live read"
    );
    assert!(!s.debug_continue());
}

/// @PLN16 M1a (interactive) — **whole-value heap edit**: replace a struct local with
/// a freshly-constructed one at the pause (`pt = Point{...}`), then resume and observe
/// the program use the new value.  The value is built on a throwaway State above the
/// live store's high-water and grafted into the paused stores with no `DbRef` remap.
/// `use_pt(Point{1,2})` is normally 3; editing `pt` to `{10,20}` makes it 30, so a
/// clean continue (the `assert(==30)` holds) proves the resumed program read the edit.
#[test]
fn repl_interactive_edit_whole_struct() {
    let mut s = session();
    for d in [
        "struct Point { x: integer, y: integer }",
        "fn use_pt(pt: Point) -> integer {\n  pt.x + pt.y\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("use_pt");
    assert!(matches!(
        s.eval("assert(use_pt(Point { x: 1, y: 2 }) == 30, \"edited\")"),
        Eval::Paused
    ));
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("1"), "pre-edit pt.x");
    // The whole-struct edit: build a fresh Point and point the frame local at it.
    assert!(
        s.debug_set("pt", "Point { x: 10, y: 20 }"),
        "whole-struct edit"
    );
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("10"), "edited pt.x");
    assert_eq!(s.debug_eval("pt.y").as_deref(), Some("20"), "edited pt.y");
    // Continue → the assert(==30) holds only with the edit, so a clean finish proves
    // the resumed program used the materialised value.
    assert!(
        !s.debug_continue(),
        "finishes — assert(==30) holds with the edit"
    );
    assert!(!s.is_debugging());
}

/// @PLN16 M1a (interactive) — the heap-edit **matrix**: nested (inlined) struct, a
/// vector local, and a struct with a text field.  Each shape is built fresh at the
/// pause and read back via `debug_eval`; a second untouched heap local in the same
/// frame is asserted intact, proving the graft leaves the suspended frame's other
/// values alone (store-level adoption above the high-water, not a frame rewrite).
#[test]
fn repl_interactive_edit_whole_value_matrix() {
    let mut s = session();
    for d in [
        "struct Point { x: integer, y: integer }",
        "struct Line { a: Point, b: Point }",
        "struct Tagged { name: text, n: integer }",
        // Two heap locals: `lead` is edited, `keep` must stay intact across the edit.
        "fn shapes(ln: Line, keep: Point) -> integer {\n  ln.a.x + keep.x\n}",
        "fn vecfn(v: vector<integer>, keep: Point) -> integer {\n  v[0] + keep.x\n}",
        "fn textfn(t: Tagged, keep: Point) -> integer {\n  t.n + keep.x\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);

    // --- nested (inlined) struct ---
    s.add_breakpoint("shapes");
    assert!(matches!(
        s.eval("shapes(Line { a: Point { x: 1, y: 2 }, b: Point { x: 3, y: 4 } }, Point { x: 7, y: 8 })"),
        Eval::Paused
    ));
    assert!(
        s.debug_set(
            "ln",
            "Line { a: Point { x: 100, y: 2 }, b: Point { x: 3, y: 4 } }"
        ),
        "nested struct edit"
    );
    assert_eq!(
        s.debug_eval("ln.a.x").as_deref(),
        Some("100"),
        "nested field"
    );
    assert_eq!(
        s.debug_eval("keep.x").as_deref(),
        Some("7"),
        "other local intact"
    );
    assert!(!s.debug_continue());
    s.clear_breakpoints();

    // --- vector ---
    s.add_breakpoint("vecfn");
    assert!(matches!(
        s.eval("vecfn([10, 20, 30], Point { x: 7, y: 8 })"),
        Eval::Paused
    ));
    assert!(s.debug_set("v", "[40, 50, 60]"), "vector edit");
    assert_eq!(
        s.debug_eval("v[0]").as_deref(),
        Some("40"),
        "edited element"
    );
    assert_eq!(
        s.debug_eval("v[2]").as_deref(),
        Some("60"),
        "edited element"
    );
    assert_eq!(
        s.debug_eval("keep.x").as_deref(),
        Some("7"),
        "other local intact"
    );
    assert!(!s.debug_continue());
    s.clear_breakpoints();

    // --- struct with a text field ---
    s.add_breakpoint("textfn");
    assert!(matches!(
        s.eval("textfn(Tagged { name: \"a\", n: 5 }, Point { x: 7, y: 8 })"),
        Eval::Paused
    ));
    assert!(
        s.debug_set("t", "Tagged { name: \"hello\", n: 9 }"),
        "struct-with-text edit"
    );
    assert_eq!(
        s.debug_eval("t.n").as_deref(),
        Some("9"),
        "edited scalar field"
    );
    assert_eq!(
        s.debug_eval("t.name").as_deref(),
        Some("\"hello\""),
        "edited text field"
    );
    assert_eq!(
        s.debug_eval("keep.x").as_deref(),
        Some("7"),
        "other local intact"
    );
    assert!(!s.debug_continue());
}

/// @PLN16 M1a (interactive) — a whole-value heap edit whose constructor **references
/// frame locals** (`Point{x: pt.y, y: pt.x}` swaps the live fields), and clean
/// **rejection** of a malformed / type-mismatched heap edit (no corruption; the
/// session stays paused and usable).  The frame-reference path proves `debug_eval`
/// resolves the RHS against the frame to a self-contained literal before the build.
#[test]
fn repl_interactive_edit_whole_struct_frame_ref_and_reject() {
    let mut s = session();
    for d in [
        "struct Point { x: integer, y: integer }",
        "fn use_pt(pt: Point) -> integer {\n  pt.x * 10 + pt.y\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("use_pt");
    // use_pt(Point{3,4}) = 34; swapping to Point{4,3} gives 43.
    assert!(matches!(
        s.eval("assert(use_pt(Point { x: 3, y: 4 }) == 43, \"swapped\")"),
        Eval::Paused
    ));
    // Constructor references the live frame fields (resolved by debug_eval).
    assert!(
        s.debug_set("pt", "Point { x: pt.y, y: pt.x }"),
        "frame-referencing whole-struct edit"
    );
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("4"), "swapped x");
    assert_eq!(s.debug_eval("pt.y").as_deref(), Some("3"), "swapped y");
    // An unknown reference (rejected when `debug_eval` can't resolve the RHS) and a
    // type-mismatched RHS (rejected when the typed build won't compile) are both clean
    // — no write, the value stays the swapped one, the session stays paused.  (A
    // partial constructor like `Point{x:1}` is *valid* loft — y defaults — so it is
    // not a rejection case.)
    assert!(!s.debug_set("pt", "no_such_local"), "unknown ref rejected");
    assert!(!s.debug_set("pt", "42"), "scalar-for-struct rejected");
    assert_eq!(
        s.debug_eval("pt.x").as_deref(),
        Some("4"),
        "unchanged after reject"
    );
    assert!(s.is_debugging(), "still paused after rejects");
    assert!(
        !s.debug_continue(),
        "finishes — assert(==43) holds with the swap"
    );
    assert!(!s.is_debugging());
}

/// @PLN16 M2 (interactive) — **undo/redo** mechanics on a scalar local: empty-stack
/// no-op, undo restores the pre-edit value, redo re-applies it, a stack of edits
/// unwinds/rewinds in order, and a fresh edit after an undo **forks** the timeline
/// (clears redo).  All within one suspension (undo is per-pause-point).
#[test]
fn repl_interactive_undo_redo_scalar() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn calc(n: integer) -> integer {\n  n * 10\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("calc");
    assert!(matches!(s.eval("calc(5)"), Eval::Paused));
    assert_eq!(s.debug_eval("n").as_deref(), Some("5"));
    // Nothing recorded yet.
    assert!(!s.debug_undo(), "nothing to undo initially");
    assert!(!s.debug_redo(), "nothing to redo initially");
    // One edit, undo, redo.
    assert!(s.debug_set("n", "99"));
    assert_eq!(s.debug_eval("n").as_deref(), Some("99"));
    assert!(s.debug_undo(), "undo");
    assert_eq!(s.debug_eval("n").as_deref(), Some("5"), "reverted");
    assert!(s.debug_redo(), "redo");
    assert_eq!(s.debug_eval("n").as_deref(), Some("99"), "re-applied");
    // Stack of edits unwinds and rewinds in order.
    assert!(s.debug_set("n", "7"));
    assert!(s.debug_set("n", "8"));
    assert_eq!(s.debug_eval("n").as_deref(), Some("8"));
    assert!(s.debug_undo());
    assert_eq!(s.debug_eval("n").as_deref(), Some("7"), "undo to 7");
    assert!(s.debug_undo());
    assert_eq!(s.debug_eval("n").as_deref(), Some("99"), "undo to 99");
    assert!(s.debug_redo());
    assert_eq!(s.debug_eval("n").as_deref(), Some("7"), "redo to 7");
    // A new edit after an undo forks the timeline: the redo (→ 8) is dropped.
    assert!(s.debug_set("n", "100"));
    assert!(!s.debug_redo(), "fresh edit cleared the redo stack");
    assert_eq!(s.debug_eval("n").as_deref(), Some("100"));
    assert!(!s.debug_continue());
}

/// @PLN16 M2 (interactive) — an undo's revert reaches **resume**: edit `n` to 99,
/// undo it, continue; the `assert(== 50)` holds only if the resumed call used the
/// reverted `n == 5` (a failed undo would leave 99 → 990 → the assert would panic).
#[test]
fn repl_interactive_undo_resumes_with_reverted_value() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn calc(n: integer) -> integer {\n  n * 10\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("calc");
    assert!(matches!(
        s.eval("assert(calc(5) == 50, \"undone resumes to original\")"),
        Eval::Paused
    ));
    assert!(s.debug_set("n", "99"));
    assert!(s.debug_undo(), "undo the edit");
    assert!(
        !s.debug_continue(),
        "finishes — assert(==50) holds with the revert"
    );
    assert!(!s.is_debugging());
}

/// @PLN16 M2 (interactive) — undo/redo across the **heap edit kinds**: a scalar
/// struct field (`pt.x`), a scalar vector element (`v[i]`), a struct field on a
/// text-bearing struct (the text must stay intact), and a **whole-value** replacement
/// (`pt = Point{…}`).  Each reverts to its pre-edit value and re-applies — proving the
/// journaled `Modify` (and, for whole-value, the frame-slot `DbRef` swap) round-trips.
#[test]
fn repl_interactive_undo_redo_heap_kinds() {
    let mut s = session();
    for d in [
        "struct Point { x: integer, y: integer }",
        "struct Tagged { name: text, n: integer }",
        "fn field_fn(pt: Point) -> integer {\n  pt.x\n}",
        "fn vec_fn(v: vector<integer>) -> integer {\n  v[0]\n}",
        "fn text_fn(t: Tagged) -> integer {\n  t.n\n}",
        "fn whole_fn(pt: Point) -> integer {\n  pt.x + pt.y\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);

    // --- scalar struct field ---
    s.add_breakpoint("field_fn");
    assert!(matches!(
        s.eval("field_fn(Point { x: 1, y: 2 })"),
        Eval::Paused
    ));
    assert!(s.debug_set("pt.x", "9"));
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("9"));
    assert!(s.debug_undo());
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("1"), "field reverted");
    assert!(s.debug_redo());
    assert_eq!(
        s.debug_eval("pt.x").as_deref(),
        Some("9"),
        "field re-applied"
    );
    assert!(!s.debug_continue());
    s.clear_breakpoints();

    // --- scalar vector element ---
    s.add_breakpoint("vec_fn");
    assert!(matches!(s.eval("vec_fn([10, 20, 30])"), Eval::Paused));
    assert!(s.debug_set("v[0]", "99"));
    assert_eq!(s.debug_eval("v[0]").as_deref(), Some("99"));
    assert!(s.debug_undo());
    assert_eq!(
        s.debug_eval("v[0]").as_deref(),
        Some("10"),
        "element reverted"
    );
    assert!(s.debug_redo());
    assert_eq!(
        s.debug_eval("v[0]").as_deref(),
        Some("99"),
        "element re-applied"
    );
    assert!(!s.debug_continue());
    s.clear_breakpoints();

    // --- scalar field on a text-bearing struct: text must survive undo/redo ---
    s.add_breakpoint("text_fn");
    assert!(matches!(
        s.eval("text_fn(Tagged { name: \"a\", n: 5 })"),
        Eval::Paused
    ));
    assert!(s.debug_set("t.n", "9"));
    assert!(s.debug_undo());
    assert_eq!(s.debug_eval("t.n").as_deref(), Some("5"), "n reverted");
    assert_eq!(
        s.debug_eval("t.name").as_deref(),
        Some("\"a\""),
        "text intact"
    );
    assert!(s.debug_redo());
    assert_eq!(s.debug_eval("t.n").as_deref(), Some("9"), "n re-applied");
    assert!(!s.debug_continue());
    s.clear_breakpoints();

    // --- whole-value replacement ---
    s.add_breakpoint("whole_fn");
    assert!(matches!(
        s.eval("whole_fn(Point { x: 1, y: 2 })"),
        Eval::Paused
    ));
    assert!(s.debug_set("pt", "Point { x: 10, y: 20 }"));
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("10"));
    assert!(s.debug_undo());
    assert_eq!(
        s.debug_eval("pt.x").as_deref(),
        Some("1"),
        "whole-value reverted"
    );
    assert_eq!(
        s.debug_eval("pt.y").as_deref(),
        Some("2"),
        "whole-value reverted"
    );
    assert!(s.debug_redo());
    assert_eq!(
        s.debug_eval("pt.x").as_deref(),
        Some("10"),
        "whole-value re-applied"
    );
    assert!(!s.debug_continue());
}

/// @PLN16 M2 (interactive) — undo/redo of a **text** edit (the trickiest case: the
/// 16-byte `Str` slot is restored by a raw-byte `Modify`, so undo must repoint it at
/// the original argument text and redo back at the debugger-owned buffer, with no
/// double-free).  Edit a text argument, undo to the original, redo to the edit.
#[test]
fn repl_interactive_undo_redo_text() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn greet(msg: text) -> integer {\n  msg.len()\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("greet");
    assert!(matches!(s.eval("greet(\"hi\")"), Eval::Paused));
    assert_eq!(s.debug_eval("msg").as_deref(), Some("\"hi\""));
    assert!(s.debug_set("msg", "\"hello\""), "text edit");
    assert_eq!(s.debug_eval("msg").as_deref(), Some("\"hello\""));
    assert!(s.debug_undo(), "undo text edit");
    assert_eq!(
        s.debug_eval("msg").as_deref(),
        Some("\"hi\""),
        "text reverted"
    );
    assert!(s.debug_redo(), "redo text edit");
    assert_eq!(
        s.debug_eval("msg").as_deref(),
        Some("\"hello\""),
        "text re-applied"
    );
    assert!(!s.debug_continue());
}

/// @PLN16 G1 (interactive) — edit-and-continue across **scalar types**, not just
/// integers: a live `integer` / `float` / `boolean` local is each edited at the
/// pause (one by literal, one by an expression evaluated against the frame), the
/// reads reflect the edits, and a **text** argument is editable too (its `Str` slot
/// repoints at a debugger-owned buffer), with `debug_eval` confirming the new value.
#[test]
fn repl_interactive_edit_scalar_types() {
    let mut s = session();
    assert!(matches!(
        s.eval(
            "fn mix(n: integer, f: float, b: boolean, msg: text) -> float {\n  \
             if b { n * f } else { 0.0 }\n}"
        ),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("mix");
    assert!(matches!(s.eval("mix(2, 1.5, true, \"hi\")"), Eval::Paused));
    // integer (literal), float (literal), boolean (expression `!b`).
    assert!(s.debug_set("n", "10"), "int edit");
    assert!(s.debug_set("f", "2.0"), "float edit");
    assert!(s.debug_set("b", "!b"), "bool expr edit (true → false)");
    assert_eq!(s.debug_eval("n").as_deref(), Some("10"));
    assert_eq!(s.debug_eval("f").as_deref(), Some("2.0"));
    assert_eq!(s.debug_eval("b").as_deref(), Some("false"));
    // A type-mismatched edit is rejected (float literal into an integer local).
    assert!(!s.debug_set("n", "3.5"), "type-mismatched edit rejected");
    assert_eq!(s.debug_eval("n").as_deref(), Some("10"), "n unchanged");
    // A text argument is editable via a literal: its `Str` slot repoints at a
    // stable buffer, and the edit is visible both in the frame view and via eval.
    assert!(s.debug_set("msg", "\"bye\""), "text arg edit");
    assert_eq!(
        s.debug_eval("msg").as_deref(),
        Some("\"bye\""),
        "msg reflects edit"
    );
    assert!(!s.debug_continue());
}

/// Without stepping enabled, breakpoints stay in **record-and-continue** mode: an
/// observing run completes (`Eval::Ran`, not `Paused`) and the hits land in
/// `last_hits` — the programmatic mode the conditional-breakpoint sweep relies on.
#[test]
fn repl_breakpoints_record_when_not_stepping() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn calc(n: integer) -> integer {\n  n * 10\n}"),
        Eval::Ran
    ));
    s.add_breakpoint("calc"); // stepping NOT enabled
    assert!(matches!(s.eval("calc(5)"), Eval::Ran), "run completes");
    assert!(!s.is_debugging(), "never suspends without stepping");
    assert!(
        s.last_hits().iter().any(|h| h.function == "calc"),
        "hit recorded: {:?}",
        s.last_hits()
    );
}

/// @P293 — value-snapshotting a **text** binding must not crash.  Re-binding a text
/// *variable* (`y = t`) or a *concat* (`z = t + "!"`) once double-freed the buffer
/// the synthetic capture fn returned (the entry-fn text return borrowed a local
/// `String` freed on teardown).  The capture now routes text through a store-resident
/// single-element vector, so the value is snapshotted correctly and re-binding works.
#[test]
fn text_binding_capture_does_not_crash() {
    let mut s = session();
    assert!(matches!(s.eval("t = \"hi\""), Eval::Ran));
    // bare variable read
    assert!(matches!(s.eval("y = t"), Eval::Ran));
    assert!(matches!(
        s.eval("assert(y == \"hi\", \"y == t\")"),
        Eval::Ran
    ));
    // concat (a borrowed work-text — the second crash shape)
    assert!(matches!(s.eval("z = t + \"!\""), Eval::Ran));
    assert!(matches!(
        s.eval("assert(z == \"hi!\", \"z == t + !\")"),
        Eval::Ran
    ));
    // chained rebind keeps the value
    assert!(matches!(s.eval("w = z"), Eval::Ran));
    assert!(matches!(
        s.eval("assert(w == \"hi!\", \"w == z\")"),
        Eval::Ran
    ));
}

/// @P293 — the value-snapshot API and the debugger's eval-at-frame both render a
/// text value (a variable, a concat, a borrowed `text[self]`) without crashing.
#[test]
fn value_of_renders_text_expressions() {
    let mut s = session();
    assert!(matches!(s.eval("msg = \"hi\""), Eval::Ran));
    assert_eq!(s.value_of("msg").as_deref(), Some("\"hi\""));
    assert_eq!(s.value_of("msg + \"!\"").as_deref(), Some("\"hi!\""));
    assert_eq!(s.value_of("msg.to_uppercase()").as_deref(), Some("\"HI\""));
    // eval-at-frame on a text argument (the @PLN16 debugger surface)
    assert!(matches!(
        s.eval("fn g(m: text) -> integer {\n  if m == \"x\" { 1 } else { 0 }\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("g");
    assert!(matches!(s.eval("g(\"hi\")"), Eval::Paused));
    assert_eq!(s.debug_eval("m").as_deref(), Some("\"hi\""));
    assert_eq!(s.debug_eval("m + \"!\"").as_deref(), Some("\"hi!\""));
    assert!(!s.debug_continue());
}

/// @PLN16.J — a **struct field** edited at the `(dbg)` prompt (`pt.x = 5`) routes
/// through `debug_set` to the in-place field write, and the refreshed frame reflects
/// it.  `debug_set` evaluates the RHS against the frame first, so an expression RHS
/// (`pt.x = pt.y + 1`) works too.
#[test]
fn repl_interactive_edit_struct_field() {
    let mut s = session();
    assert!(matches!(
        s.eval("struct Point { x: integer, y: integer }"),
        Eval::Ran
    ));
    assert!(matches!(
        s.eval("fn area(pt: Point) -> integer {\n  m = pt.x;\n  pt.x * pt.y\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("area");
    assert!(matches!(s.eval("area(Point { x: 3, y: 4 })"), Eval::Paused));

    assert!(s.debug_set("pt.x", "5"), "literal field edit");
    assert_eq!(
        s.debug_eval("pt.x").as_deref(),
        Some("5"),
        "field reflects edit"
    );
    assert!(s.debug_set("pt.y", "pt.x + 1"), "expression field edit");
    assert_eq!(s.debug_eval("pt.y").as_deref(), Some("6"), "y = x + 1");
    // a bad field is rejected, frame intact
    assert!(!s.debug_set("pt.z", "9"), "unknown field rejected");
    assert!(!s.debug_continue());
}

/// @PLN16.J (M1b) — a **nested** struct path edited at the `(dbg)` prompt
/// (`o.inner.a = 9`) routes through `debug_set` → `set_frame_path` and the refreshed
/// frame reflects it.
#[test]
fn repl_interactive_edit_nested_field() {
    let mut s = session();
    assert!(matches!(
        s.eval("struct Inner { a: integer, b: integer }"),
        Eval::Ran
    ));
    assert!(matches!(
        s.eval("struct Outer { n: integer, inner: Inner }"),
        Eval::Ran
    ));
    assert!(matches!(
        s.eval("fn f(o: Outer) -> integer {\n  m = o.n;\n  o.inner.a\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("f");
    assert!(matches!(
        s.eval("f(Outer { n: 1, inner: Inner { a: 5, b: 6 } })"),
        Eval::Paused
    ));
    assert!(s.debug_set("o.inner.a", "9"), "nested path edit");
    assert_eq!(
        s.debug_eval("o.inner.a").as_deref(),
        Some("9"),
        "nested field reflects edit"
    );
    assert_eq!(
        s.debug_eval("o.inner.b").as_deref(),
        Some("6"),
        "sibling untouched"
    );
    assert!(!s.debug_continue());
}

/// @PLN16 M5a (file-run debugger) — load a real program under its own filename, break
/// at `file:line`, run `main()`, and pause in the right function.  Proves the file:line
/// breakpoint resolves to the **user file's** function (not a stdlib function that
/// happens to share the line number), reads the live local, and resumes cleanly.
#[test]
fn repl_file_debugger_breaks_at_file_line() {
    let mut s = session();
    // Line 2 (`n * 2`) is inside `helper`; line 5 calls it from `main`.
    let prog = "fn helper(n: integer) -> integer {\n  \
                n * 2\n}\nfn main() {\n  \
                a = helper(21);\n  \
                assert(a == 42, \"ok\")\n}\n";
    assert!(
        s.load_program_str(prog, "prog.loft").is_ok(),
        "program loads"
    );
    assert!(s.defines_function("main"), "main is defined");
    assert!(
        s.breakable_lines_in_file("prog.loft").contains(&2),
        "line 2 is breakable: {:?}",
        s.breakable_lines_in_file("prog.loft")
    );
    s.debug_stepping(true);
    s.add_file_breakpoint("prog.loft", 2);
    // Running main() pauses inside helper at line 2 — scoped to prog.loft, so stdlib's
    // own line 2 is never a breakpoint.
    assert!(
        matches!(s.eval("main()"), Eval::Paused),
        "paused at prog.loft:2"
    );
    let f = s.paused_frame().expect("frame");
    assert_eq!(
        f.function, "helper",
        "paused in the user's helper, not stdlib"
    );
    assert_eq!(s.debug_eval("n").as_deref(), Some("21"), "helper's live n");
    // Continue with the original value → assert(42 == 42) holds → clean finish.
    assert!(!s.debug_continue(), "run finishes cleanly");
    assert!(!s.is_debugging());
}

/// @PLN16 M5a — a `file:line` with no breakable code resolves to nothing (the file-run
/// debugger reports it + hints), and a line in a different file (the stdlib) is never
/// matched.
#[test]
fn repl_file_debugger_unbreakable_line() {
    let mut s = session();
    let prog = "fn main() {\n  a = 1;\n  print(\"{a}\")\n}\n";
    assert!(s.load_program_str(prog, "prog.loft").is_ok());
    // Line 4 is the closing `}` — no emitted code → not breakable.
    assert!(
        !s.breakable_lines_in_file("prog.loft").contains(&4),
        "line 4 (the `}}`) is not breakable"
    );
    s.debug_stepping(true);
    s.add_file_breakpoint("prog.loft", 4);
    // The breakpoint silently doesn't resolve, so the run finishes without pausing.
    assert!(
        matches!(s.eval("main()"), Eval::Ran),
        "no pause on an unbreakable line"
    );
    assert!(!s.is_debugging());
}

/// @PLN16 M5a — the full `loft debug <file>:<line>` flow through `run_file_debug`:
/// write a real `.loft` file, break at a line, auto-run `main()` to it, then drive the
/// interactive `(dbg)` prompt from piped input (read the frame, continue, quit).  The
/// pause announcement + frame go to the output stream; the run then resumes to the end.
#[test]
fn repl_file_debugger_end_to_end() {
    let path = tmp_session("filedebug");
    let prog = "fn helper(n: integer) -> integer {\n  \
                n * 2\n}\nfn main() {\n  \
                a = helper(21);\n  \
                assert(a == 42, \"ok\")\n}\n";
    std::fs::write(&path, prog).expect("write temp program");
    let file = path.to_str().unwrap();
    // Piped session: read `n` at the frame, continue to the end, quit.
    let input = std::io::Cursor::new(b"n\n:continue\n:quit\n".to_vec());
    let mut out: Vec<u8> = Vec::new();
    loft::repl::run_file_debug("default", &[], file, 2, input, &mut out).expect("debug run");
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("break at"),
        "announces the breakpoint: {text}"
    );
    assert!(
        text.contains("paused in helper") && text.contains("n = 21"),
        "pauses in helper with n = 21: {text}"
    );
    assert!(text.contains("resumed"), "continues to the end: {text}");
    let _ = std::fs::remove_file(&path);
}

/// @PLN16 M3 (watchpoints) — set a watch on a struct field, resume, and the run pauses
/// the moment a later write changes it (reporting old → new).  Proves the per-op poll
/// in the resume loop catches an in-place heap mutation and stops one op past the write.
#[test]
fn repl_watchpoint_fires_on_field_change() {
    let mut s = session();
    for d in [
        "struct Counter { n: integer }",
        "fn bump(c: Counter) -> integer {\n  c.n = c.n + 1;\n  c.n = c.n + 10;\n  c.n\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("bump"); // first body line, before `c.n = c.n + 1`
    assert!(matches!(s.eval("bump(Counter { n: 5 })"), Eval::Paused));
    assert_eq!(s.debug_eval("c.n").as_deref(), Some("5"), "c.n starts at 5");
    assert!(s.add_watchpoint("c.n"), "watch the field");
    assert_eq!(s.watchpoints(), vec!["c.n".to_string()], "watch registered");
    // Continue → the run pauses when `c.n = c.n + 1` changes it to 6.
    assert!(s.debug_continue(), "watch fired → paused again");
    let hit = s.take_watch_hit().expect("first watch hit");
    assert_eq!(
        (hit.label.as_str(), hit.old.as_str(), hit.new.as_str()),
        ("c.n", "5", "6"),
        "reports the change"
    );
    assert!(s.take_watch_hit().is_none(), "hit is taken once");
    // Continue → `c.n = c.n + 10` fires it again (6 → 16).
    assert!(s.debug_continue(), "second change fires the watch");
    let hit2 = s.take_watch_hit().expect("second watch hit");
    assert_eq!((hit2.old.as_str(), hit2.new.as_str()), ("6", "16"));
    // Continue → no more writes; the run finishes.
    assert!(!s.debug_continue(), "run finishes");
    assert!(!s.is_debugging());
}

/// @PLN16 M3 — a watch on a **vector element** fires when that element is written, and
/// an unwatchable expression (a bare local) is rejected.
#[test]
fn repl_watchpoint_vector_element_and_reject() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn edit(v: vector<integer>) -> integer {\n  v[0] = 99;\n  v[0]\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("edit");
    assert!(matches!(s.eval("edit([10, 20, 30])"), Eval::Paused));
    // A bare local is a stack slot — not a watchable region.
    assert!(!s.add_watchpoint("v"), "bare local rejected");
    assert!(s.add_watchpoint("v[0]"), "element watchable");
    assert!(s.debug_continue(), "v[0] = 99 fires the watch");
    let hit = s.take_watch_hit().expect("watch hit");
    assert_eq!(
        (hit.label.as_str(), hit.old.as_str(), hit.new.as_str()),
        ("v[0]", "10", "99")
    );
    assert!(!s.debug_continue());
}

/// @PLN16 rich-bp — a **conditional breakpoint** (`:break f if n == 3`) breaks only on
/// the call where the condition holds, silently auto-resuming past the others — the
/// "fails on one call in N" case.
#[test]
fn repl_conditional_breakpoint_breaks_only_when_true() {
    let mut s = session();
    for d in [
        "fn f(n: integer) -> integer {\n  n + 100\n}",
        "fn run() -> integer {\n  f(1);\n  f(2);\n  f(3);\n  f(4)\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("f if n == 3");
    // f(1)/f(2) auto-resume (condition false); the run suspends at f(3).
    assert!(
        matches!(s.eval("run()"), Eval::Paused),
        "paused only at f(3)"
    );
    assert_eq!(
        s.debug_eval("n").as_deref(),
        Some("3"),
        "broke at the matching call"
    );
    // f(4)'s condition is false → no further break → the run finishes.
    assert!(!s.debug_continue(), "no further break");
    assert!(!s.is_debugging());
}

/// @PLN16 rich-bp — a **tracepoint** (`:trace f n`) logs the expression on each hit and
/// **continues** (never pauses); the emitted lines are collected for the driver.
#[test]
fn repl_tracepoint_logs_and_continues() {
    let mut s = session();
    for d in [
        "fn f(n: integer) -> integer {\n  n + 100\n}",
        "fn run() -> integer {\n  f(1);\n  f(2)\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_tracepoint("f n");
    // The tracepoint logs n at each f call and the run continues to the end.
    assert!(
        matches!(s.eval("run()"), Eval::Ran),
        "a tracepoint never pauses"
    );
    assert!(!s.is_debugging());
    assert_eq!(
        s.take_trace_output(),
        vec!["n = 1".to_string(), "n = 2".to_string()],
        "logged both calls"
    );
}

/// `ReplSession::value_of` renders EVERY binding kind — a vector, a text, a wide
/// vector, a struct — and never panics (loft#618).  The three tests below are its
/// guards; each was a reproduction of a crash while #618 was open.
///
/// `capture_typed` runs `fn replmain_N() -> vector<τ> { … v }`, i.e. it returns a
/// bare heap value that borrows a body local.  The fn-return deep-copy then
/// targets a store the capture generation never allocated, so the destination
/// `DbRef` is the null sentinel and `append_vector` indexes `allocations[65535]`
/// — `index out of bounds: the len is 2 but the index is 65535` at `keys.rs`.
/// A struct binding was unaffected; a vector faulted on a single bind.
///
/// Same family as @P293 (which hit this for `text`), and the same remedy: wrap
/// the value in a one-element vector so it is COPIED into store-resident memory,
/// then strip the outer brackets.  The in-tree caller (`frame_value_of`) guards
/// with `catch_unwind`, but an embedder calling the public `value_of` directly
/// has no such guard and the process aborts — so this must not merely degrade to
/// `None`, it must return the value.
#[test]
fn value_of_renders_vector_bindings_618() {
    let mut s = session();
    assert!(matches!(s.eval("v = [1, 2, 3]"), Eval::Ran));
    assert_eq!(s.value_of("v").as_deref(), Some("[1,2,3]"));

    // Re-binding was in the reported boundary table too.
    assert!(matches!(s.eval("v = [4, 5]"), Eval::Ran));
    assert_eq!(s.value_of("v").as_deref(), Some("[4,5]"));

    // Two separate vector bindings — reading the second used to panic as well.
    let mut t = session();
    assert!(matches!(t.eval("a = [1, 2]"), Eval::Ran));
    assert!(matches!(t.eval("b = [3, 4]"), Eval::Ran));
    assert_eq!(t.value_of("b").as_deref(), Some("[3,4]"));
    assert_eq!(t.value_of("a").as_deref(), Some("[1,2]"));
}

/// #618 — the same capture path for a `vector<text>` and for an element type
/// wider than 32 bits.  The report's "possibly related second signature" was a
/// `v = [9000000000, 0]` binding panicking at `database/types.rs` instead of
/// `keys.rs`; both are the fn-return copy, so guard the wide element here too.
#[test]
fn value_of_renders_text_and_wide_vector_bindings_618() {
    let mut s = session();
    assert!(matches!(s.eval("vt = [\"a\", \"b\"]"), Eval::Ran));
    assert_eq!(s.value_of("vt").as_deref(), Some("[\"a\",\"b\"]"));

    let mut w = session();
    assert!(matches!(w.eval("vw = [9000000000, 0]"), Eval::Ran));
    assert_eq!(w.value_of("vw").as_deref(), Some("[9000000000,0]"));
}

/// #618 — a struct binding was already correct; keep it in the guard so the
/// vector wrap can't be "fixed" by regressing the path that worked.
#[test]
fn value_of_struct_binding_still_renders_618() {
    let mut s = session();
    assert!(matches!(s.eval("struct P { x: integer }"), Eval::Ran));
    assert!(matches!(s.eval("p = P { x: 1 }"), Eval::Ran));
    assert_eq!(s.value_of("p").as_deref(), Some("P{x:1}"));
}

// ── FU.3: one typo must produce one message ─────────────────────────────────

/// A failed input reported diagnostics about the *replayed session body* as if
/// the user had just typed them: `prnt("hi")` after `x = 5` answered with the
/// real error **and** `Variable x is never read`, which is false — `x` was read,
/// by the very input that failed to compile.  Every prelude line clamps onto the
/// user's line 1, so N earlier bindings produced N false warnings all pointing
/// at the same wrong column.
#[test]
fn a_failed_input_does_not_warn_about_the_replayed_session() {
    let mut s = session();
    assert!(matches!(s.eval("x = 5"), Eval::Ran));
    assert!(matches!(s.eval("y = 6"), Eval::Ran));
    assert!(matches!(s.eval("z = 7"), Eval::Ran));

    let Eval::Error(diags) = s.eval("prnt(\"hi\")") else {
        panic!("a typo'd function name must be an error");
    };

    // The real error survives — dropping it would leave the REPL rejecting the
    // input while saying nothing.
    assert!(
        diags
            .iter()
            .any(|d| d.level >= loft::diagnostics::Level::Error),
        "the error itself must still reach the user; got {diags:?}"
    );
    // ...and it is the ONLY thing reported.
    let noise: Vec<String> = diags
        .iter()
        .filter(|d| d.level < loft::diagnostics::Level::Error)
        .map(loft::diagnostics::DiagEntry::to_string_compact)
        .collect();
    assert!(
        noise.is_empty(),
        "a typo must not warn about earlier, correct bindings; got {noise:?}"
    );

    // The session is unharmed: x really was readable all along.
    assert!(
        matches!(
            s.eval("assert(x + y + z == 18, \"bindings survived\")"),
            Eval::Ran
        ),
        "the replayed bindings must still be live after the failed input"
    );
}

/// @PLN120 E1 — `loft debug prog.loft:N --lib <dir>` must resolve a library the
/// same way running the program does.
///
/// `run_file_debug` built its session with `ReplSession::new` (stdlib only) while
/// the `--rpc` and `--serve` paths used `new_with_libs`, so the interactive
/// debugger — and only it — reported `Library '<x>' not found` on a file that runs
/// clean.  moros read that as "the debugger does not work on real programs": every
/// project whose libraries live in a local `lib/` was undebuggable by this route.
#[test]
fn file_debugger_resolves_a_library_from_lib_dirs() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lib");
    let lib_dirs = vec![dir.to_string_lossy().into_owned()];

    let path = tmp_session("filedebug_lib").with_extension("loft");
    std::fs::write(
        &path,
        "use typeshift;\nfn main() {\n  v = ts_touch();\n  assert(v == 7, \"lib call\")\n}\n",
    )
    .expect("write temp program");
    let file = path.to_str().unwrap();

    let input = std::io::Cursor::new(b":continue\n:quit\n".to_vec());
    let mut out: Vec<u8> = Vec::new();
    loft::repl::run_file_debug("default", &lib_dirs, file, 3, input, &mut out).expect("debug run");
    let text = String::from_utf8_lossy(&out);

    assert!(
        !text.contains("not found"),
        "the library must resolve under --lib: {text}"
    );
    assert!(
        text.contains("break at"),
        "the breakpoint must be reached: {text}"
    );
    let _ = std::fs::remove_file(&path);
}

/// The control for the test above: with NO `lib_dirs` the same program cannot
/// resolve its library, so the assertion there is about the wiring and not about
/// `typeshift` happening to be findable some other way.
#[test]
fn file_debugger_without_lib_dirs_cannot_resolve_the_library() {
    let path = tmp_session("filedebug_nolib").with_extension("loft");
    std::fs::write(
        &path,
        "use typeshift;\nfn main() {\n  v = ts_touch();\n  assert(v == 7, \"lib call\")\n}\n",
    )
    .expect("write temp program");
    let file = path.to_str().unwrap();

    let input = std::io::Cursor::new(b":quit\n".to_vec());
    let mut out: Vec<u8> = Vec::new();
    loft::repl::run_file_debug("default", &[], file, 3, input, &mut out).expect("debug run");
    let text = String::from_utf8_lossy(&out);

    assert!(
        text.contains("not found"),
        "without --lib the library must NOT resolve — else the E1 test proves nothing: {text}"
    );
    let _ = std::fs::remove_file(&path);
}

/// @PLN120 E3b — a call crossing into NATIVE code must work under the debugger.
///
/// The REPL's execute path ran `compile::byte_code` (which registers native
/// STUBS) but never loaded the packages' cdylibs, so the first `#native` call
/// panicked with *"native function not loaded"* and the session was abandoned.
/// The CLI run path and `loft test` both wired them; the debugger did not — which
/// is why importing a registry package was harmless and calling the part of it
/// that is native was fatal (moros H13: `use web; sleep_ms(5)` died while the same
/// file ran clean under `--interpret`).
#[test]
fn file_debugger_can_call_into_a_native_library() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lib");
    let lib_dirs = vec![dir.to_string_lossy().into_owned()];

    let path = tmp_session("filedebug_native").with_extension("loft");
    std::fs::write(
        &path,
        "use native_pkg;\nfn main() {\n  v = ext_add_one(41);\n  \
         assert(v == 42, \"native call under the debugger\")\n}\n",
    )
    .expect("write temp program");
    let file = path.to_str().unwrap();

    let input = std::io::Cursor::new(b":continue\n:quit\n".to_vec());
    let mut out: Vec<u8> = Vec::new();
    loft::repl::run_file_debug("default", &lib_dirs, file, 3, input, &mut out).expect("debug run");
    let text = String::from_utf8_lossy(&out);

    assert!(
        !text.contains("native function not loaded"),
        "the cdylib must be wired for the debug run: {text}"
    );
    assert!(
        !text.contains("debug session ended"),
        "the session must survive the native call: {text}"
    );
    // The program's own assert is the value check — reaching the end proves it held.
    assert!(
        text.contains("run finished"),
        "the run must complete past the native call: {text}"
    );
    let _ = std::fs::remove_file(&path);
}

/// @PLN120 B — a breakpoint condition has THREE outcomes, not two.
///
/// `resolve_pause` compared `debug_eval(cond) != Some("true")`, so a condition
/// that could not be EVALUATED (a typo, or a name not in scope at that line) was
/// indistinguishable from one that evaluated FALSE: the breakpoint reported
/// `verified: true` and then never fired, silently. The client was promised a
/// breakpoint that could not exist.
///
/// The three cells below are the whole contract. The false-but-valid row is the
/// control — without it, a "fix" that simply always breaks would pass.
fn cond_session(cond: &str, file: &std::path::Path) -> (bool, Vec<String>) {
    let mut s = ReplSession::new("default").expect("stdlib");
    let src = "fn main() {\n  total = 0;\n  for i in 0..4 {\n    \
               step = i * 10;\n    total = total + step;\n  }\n  \
               assert(total == 60, \"ran\")\n}\n";
    std::fs::write(file, src).expect("write");
    s.load_program(file.to_str().unwrap())
        .expect("read")
        .expect("parse");
    s.debug_stepping(true);
    s.add_file_breakpoint_rich(
        file.to_str().unwrap(),
        5,
        Some(cond.to_string()),
        Vec::new(),
        true,
    );
    let paused = matches!(s.eval("main()"), Eval::Paused);
    (paused, s.take_trace_output())
}

#[test]
fn an_unevaluable_breakpoint_condition_is_reported_and_stops() {
    let f = tmp_session("cond_bad").with_extension("loft");
    // `i` is not in the frame at line 5, so the condition cannot be evaluated.
    let (paused, trace) = cond_session("i == 2", &f);
    assert!(
        paused,
        "an unevaluable condition must STOP — silently never firing is the bug"
    );
    assert!(
        trace.iter().any(|t| t.contains("cannot be evaluated")),
        "and must say so: {trace:?}"
    );
    let _ = std::fs::remove_file(&f);
}

#[test]
fn a_false_breakpoint_condition_stays_silent_and_does_not_stop() {
    let f = tmp_session("cond_false").with_extension("loft");
    // `total` IS in the frame; the condition is simply never true.
    let (paused, trace) = cond_session("total == 999", &f);
    assert!(!paused, "a false condition must not stop: {trace:?}");
    assert!(
        !trace.iter().any(|t| t.contains("cannot be evaluated")),
        "and must not be reported as unevaluable: {trace:?}"
    );
    let _ = std::fs::remove_file(&f);
}

#[test]
fn a_true_breakpoint_condition_stops_without_a_diagnostic() {
    let f = tmp_session("cond_true").with_extension("loft");
    let (paused, trace) = cond_session("total == 10", &f);
    assert!(paused, "a true condition must stop: {trace:?}");
    assert!(
        !trace.iter().any(|t| t.contains("cannot be evaluated")),
        "a valid condition draws no complaint: {trace:?}"
    );
    let _ = std::fs::remove_file(&f);
}

/// @PLN120 A — the frame-liveness gate: the plan's three-row probe table, driven
/// through the real `loft debug <file>:<line>` path.
///
/// `for i in 0..4 { step = i * 10; total = total + step; }` is the shape the design
/// was built on, and it is deliberately one where `i` and `step` **share a stack
/// slot** (`loft introspect` shows both at `var[48]`; the allocator is scope-blind,
/// so two locals whose live ranges do not overlap coalesce even inside one scope).
/// That is what makes the three rows below controls rather than decoration:
///
/// * line 4 — `step` is in scope with no completed write, so it must read `<unset>`
///   and **not** `0`. `0` is `i`'s value in the shared slot: a scope-only fix (show
///   every in-scope local with its slot's contents) prints exactly that, which is a
///   silent lie of the same family arc B closed for conditions.
/// * line 5 — `step` must read its value (the one the line consumes; it was missing
///   entirely before A, because the reference-span filter asked whether the pc was
///   past its first *read*), and `i` must be reported as `<reused by step>` — in
///   scope, but its bytes are gone. Not silently dropped, not shown as `0`.
/// * line 7 — after the loop, `i` and `step` must be **absent**. Without this row a
///   fix that shows every local everywhere passes the other two.
#[test]
fn file_debugger_frame_shows_scope_with_unset_and_reused_markers() {
    let path = std::env::temp_dir().join(format!("loft_pln120a_{}.loft", std::process::id()));
    let prog = "fn main() {\n  \
                total = 0;\n  \
                for i in 0..4 {\n    \
                step = i * 10;\n    \
                total = total + step;\n  \
                }\n  \
                assert(total == 60, \"loop\")\n}\n";
    std::fs::write(&path, prog).expect("write temp program");
    let file = path.to_str().unwrap();
    let at = |line: u32, cmds: &str| -> String {
        let input = std::io::Cursor::new(cmds.as_bytes().to_vec());
        let mut out: Vec<u8> = Vec::new();
        loft::repl::run_file_debug("default", &[], file, line, input, &mut out).expect("debug run");
        String::from_utf8_lossy(&out).to_string()
    };

    // Row 1 — line 4 `step = i * 10`.
    let l4 = at(4, ":vars\n:quit\n");
    assert!(l4.contains("i = 0"), "line 4 shows the user's `i`: {l4}");
    assert!(
        l4.contains("step = <unset>"),
        "line 4 marks `step` unset — NOT `step = 0`, which is `i`'s value in the \
         slot they share: {l4}"
    );

    // Row 2 — line 5 `total = total + step`.
    let l5 = at(5, ":vars\n:quit\n");
    assert!(
        l5.contains("step = 0"),
        "line 5 shows `step`, the value the line reads (absent before A): {l5}"
    );
    assert!(
        l5.contains("i = <reused by step>"),
        "line 5 names why `i` is unreadable rather than dropping it: {l5}"
    );

    // Row 3 (the control) — line 7, after the loop.
    let l7 = at(7, ":vars all\n:quit\n");
    assert!(l7.contains("total = 60"), "line 7 shows `total`: {l7}");
    assert!(
        !l7.contains("step ") && !l7.contains("i = "),
        "line 7 must NOT gain `i`/`step` — they are out of scope: {l7}"
    );

    // The unreadable local names its reason on read AND refuses the edit — the edit
    // half is the memory-safety property: writing through a name whose slot belongs
    // to another local corrupts that local (and for `text`, `Drop`s a `String` that
    // was never constructed).
    let refused = at(5, "i\ni = 42\n:quit\n");
    assert!(
        refused.contains("slot was reused by `step`"),
        "reading an unheld local explains itself: {refused}"
    );
    assert!(
        refused.contains("can't set it:"),
        "editing an unheld local is refused, with the reason: {refused}"
    );
    let _ = std::fs::remove_file(&path);
}

/// @PLN120 A — the `text` half of the edit gate, which is the one failure mode of
/// this arc that corrupts memory rather than merely misinforming: overwriting a text
/// local runs `Drop` on its old `String`, so the slot must hold a constructed one.
/// Before A the narrow reference-span filter hid an unwritten text local, and the
/// gate rode on that; A shows it, so the gate has to be explicit.
#[test]
fn file_debugger_refuses_to_edit_an_unset_text_local() {
    let path = std::env::temp_dir().join(format!("loft_pln120a_txt_{}.loft", std::process::id()));
    let prog = "fn main() {\n  \
                total = 0;\n  \
                for i in 0..3 {\n    \
                msg = \"n\";\n    \
                total = total + i;\n  \
                }\n  \
                assert(total == 3, \"loop\")\n}\n";
    std::fs::write(&path, prog).expect("write temp program");
    let file = path.to_str().unwrap();
    let drive = |line: u32, cmds: &str| -> String {
        let input = std::io::Cursor::new(cmds.as_bytes().to_vec());
        let mut out: Vec<u8> = Vec::new();
        loft::repl::run_file_debug("default", &[], file, line, input, &mut out).expect("debug run");
        String::from_utf8_lossy(&out).to_string()
    };
    // On its own assignment line the text local is `<unset>` and the edit is refused.
    let unset = drive(4, "msg = \"hi\"\n:quit\n");
    assert!(
        unset.contains("msg = <unset>"),
        "an unwritten text local is shown as unset: {unset}"
    );
    assert!(
        unset.contains("has no value yet at this line"),
        "and the refused edit says why: {unset}"
    );
    // The control: one line later it IS held, so the same edit must succeed —
    // without this, a gate that refuses every text edit would pass.
    let held = drive(5, "msg = \"hi\"\n:quit\n");
    assert!(
        held.contains("msg = \"n\"") && held.contains("msg = \"hi\""),
        "one line later the edit lands: {held}"
    );
    let _ = std::fs::remove_file(&path);
}

/// @PLN120 F — the undo/redo history survives a step exactly as far as its storage
/// does.  Six cells; the consumer report is cell 1 and the safety property is cell 2.
///
/// The probe is the arc-A shape on purpose: `i` and `step` **share** slot 48 (the slot
/// allocator is scope-blind), so an undo of `i` replayed one line later would write
/// `step`'s value — which is exactly the hazard that made `debug_step` clear the whole
/// history, discarding the valid entries with the stale one.
#[test]
fn file_debugger_undo_survives_a_step_only_while_its_storage_does() {
    let path = std::env::temp_dir().join(format!("loft_pln120f_{}.loft", std::process::id()));
    let prog = "fn main() {\n  \
                total = 0;\n  \
                for i in 0..4 {\n    \
                step = i * 10;\n    \
                total = total + step;\n  \
                }\n  \
                assert(total > 0, \"loop\")\n}\n";
    std::fs::write(&path, prog).expect("write temp program");
    let file = path.to_str().unwrap();
    let drive = |line: u32, cmds: &str| -> String {
        let input = std::io::Cursor::new(cmds.as_bytes().to_vec());
        let mut out: Vec<u8> = Vec::new();
        loft::repl::run_file_debug("default", &[], file, line, input, &mut out).expect("debug run");
        String::from_utf8_lossy(&out).to_string()
    };

    // Cell 1 — THE REPORTED CASE.  `total` keeps slot 32 for the whole function, so its
    // undo entry is still valid after a step; before F this answered "nothing to undo".
    let c1 = drive(5, "total = 99\n:next\n:undo\n:quit\n");
    assert!(c1.contains("total = 99"), "the edit lands: {c1}");
    assert!(
        c1.matches("total = 0").count() >= 2,
        "…and `:undo` after a step restores it: {c1}"
    );

    // Cell 2 — THE SAFETY CONTROL.  Editing `i` on line 4 and stepping to line 5 hands
    // slot 48 to `step`; the entry must be dropped WITH ITS REASON, and `step`'s value
    // must be left alone.  Without this cell, "stop clearing the history" passes cell 1
    // and ships the stale-slot write.
    let c2 = drive(4, "i = 7\n:next\n:undo\n:quit\n");
    assert!(
        c2.contains("no longer undoable") && c2.contains("`i`'s stack slot is now `step`'s"),
        "the drop names the local that took the slot: {c2}"
    );
    assert!(
        c2.contains("step = 70"),
        "…and `step` (7 * 10, the edit's own effect) is NOT overwritten by the undo: {c2}"
    );

    // Cell 5 — no edits at all.  Correct behaviour, previously indistinguishable from a
    // broken feature: `:undo` is edit-scoped, and it now says so.
    let c5 = drive(5, ":next\n:undo\n:quit\n");
    assert!(
        c5.contains("no edits to undo at this pause") && c5.contains("stepBack"),
        "an empty stack names the edit/step boundary and the tool that does step back: {c5}"
    );
    let _ = std::fs::remove_file(&path);
}

/// @PLN120 F — the two classes the blanket clear discarded needlessly.
///
/// A **heap** edit's address survives any number of steps, so its entry must too
/// (cell 3); an edit made in a frame that has since **returned** must be dropped, named
/// (cell 4).  Cell 3 is what proves the frame/heap split earns its keep rather than
/// being decoration — with one rule for everything, either heap edits stay lost or
/// returned-frame edits get replayed into a dead frame.
#[test]
fn file_debugger_undo_keeps_heap_edits_and_drops_returned_frames() {
    let path = std::env::temp_dir().join(format!("loft_pln120f_h_{}.loft", std::process::id()));
    let prog = "struct Pt { x: integer, y: integer }\n\n\
                fn bump(p: Pt) -> integer {\n  \
                inner = p.x + 1;\n  \
                return inner;\n}\n\n\
                fn main() {\n  \
                pt = Pt { x: 5, y: 6 };\n  \
                a = bump(pt);\n  \
                b = pt.x + a;\n  \
                assert(b > 0, \"ok\")\n}\n";
    std::fs::write(&path, prog).expect("write temp program");
    let file = path.to_str().unwrap();
    let drive = |line: u32, cmds: &str| -> String {
        let input = std::io::Cursor::new(cmds.as_bytes().to_vec());
        let mut out: Vec<u8> = Vec::new();
        loft::repl::run_file_debug("default", &[], file, line, input, &mut out).expect("debug run");
        String::from_utf8_lossy(&out).to_string()
    };

    // Cell 3 — a heap field edit, then a step, then undo: restored.  No frame binding is
    // recorded for it, because no frame change can invalidate a heap record's address.
    let c3 = drive(10, "pt.x = 40\n:next\n:undo\n:quit\n");
    assert!(c3.contains("Pt{x:40,y:6}"), "the field edit lands: {c3}");
    assert!(
        c3.contains("a = 41"),
        "…and takes effect in the program (5 → 40, +1): {c3}"
    );
    assert!(
        c3.matches("Pt{x:5,y:6}").count() >= 2,
        "…and `:undo` after the step restores it: {c3}"
    );

    // Cell 4 — an edit inside `bump`, then `:finish` back into `main`: the frame it was
    // made in is gone, so the entry is dropped and says which frame.  Replaying it would
    // write a dead frame's slot.
    let c4 = drive(5, "inner = 99\n:finish\n:undo\n:quit\n");
    assert!(c4.contains("inner = 99"), "the callee edit lands: {c4}");
    assert!(
        c4.contains("no longer undoable") && c4.contains("frame it was made in has returned"),
        "…and returning drops it, naming the reason: {c4}"
    );
    let _ = std::fs::remove_file(&path);
}

/// @PLN120 A follow-up — a name that IS a local of this function but is not in scope on
/// the paused line says so, instead of the generic "couldn't evaluate" a typo gets.
/// The consumer's case verbatim: reading the loop iterator while stopped on the `for`.
#[test]
fn file_debugger_names_an_out_of_scope_local() {
    let path = std::env::temp_dir().join(format!("loft_pln120a_oos_{}.loft", std::process::id()));
    let prog = "fn main() {\n  \
                total = 0;\n  \
                for i in 0..3 {\n    \
                total = total + i;\n  \
                }\n  \
                assert(total == 3, \"loop\")\n}\n";
    std::fs::write(&path, prog).expect("write temp program");
    let file = path.to_str().unwrap();
    let input = std::io::Cursor::new(b"i\nnosuchname\n:quit\n".to_vec());
    let mut out: Vec<u8> = Vec::new();
    loft::repl::run_file_debug("default", &[], file, 3, input, &mut out).expect("debug run");
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("not in scope at this line"),
        "an out-of-scope local explains itself: {text}"
    );
    // The control: a name that is not a local at all must still get the generic message,
    // or the new one would be claiming knowledge it does not have.
    assert!(
        text.contains("couldn't evaluate `nosuchname`"),
        "an unknown name stays a plain failure: {text}"
    );
    let _ = std::fs::remove_file(&path);
}

/// moros H13's residual — a call to a `use`d LIBRARY's function inside `eval` answered
/// "couldn't evaluate", while a same-file call and arithmetic over locals both worked.
///
/// The cause was not in the debugger. An import is an ALIAS in `def_names` —
/// `(name, importing_source) → def_nr` — which is not derivable from `definitions`,
/// since each definition knows only its own source. `Data::rebuild_indices` rebuilt
/// from `definitions` alone and therefore dropped every import, and the REPL rebuilds
/// on each `savepoint`/`rewind` — so the FIRST eval probe silently un-imported every
/// library name for the rest of the session. `Data` now retains the applied imports
/// and replays them after a rebuild.
///
/// The two controls are what place the blame: a same-file call and a local expression
/// must keep working, so a fix that broke ordinary resolution to make libraries
/// resolve could not pass.
#[test]
fn debug_eval_can_call_a_library_function_at_a_frame() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lib");
    let lib_dirs = vec![dir.to_string_lossy().into_owned()];

    let path = tmp_session("eval_libcall").with_extension("loft");
    std::fs::write(
        &path,
        "use typeshift;\n\
         fn local_double(n: integer) -> integer { return n * 2; }\n\
         fn main() {\n  \
         base = 5;\n  \
         v = 0;\n  \
         v = ts_touch() + base;\n  \
         assert(v == 12, \"lib call\")\n}\n",
    )
    .expect("write temp program");
    let file = path.to_str().unwrap();

    // Break on the line that uses the library call, then exercise three reads and two
    // edits.  A bare read prints its value to stdout, which this harness does not
    // capture — but a FAILED read prints "couldn't evaluate" to the chrome stream, and
    // an EDIT reprints the frame there, so the edits are what pin the actual VALUES.
    let input = std::io::Cursor::new(
        b"base\nlocal_double(base)\nts_touch()\nv = local_double(base)\nv = ts_touch() + base\n:quit\n"
            .to_vec(),
    );
    let mut out: Vec<u8> = Vec::new();
    loft::repl::run_file_debug("default", &lib_dirs, file, 6, input, &mut out).expect("debug run");
    let text = String::from_utf8_lossy(&out);

    assert!(
        !text.contains("couldn't evaluate"),
        "every expression must evaluate at the frame: {text}"
    );
    assert!(
        !text.contains("couldn't set"),
        "every edit must land: {text}"
    );
    // The same-file call is the control: `local_double(5)` is 10.
    assert!(
        text.contains("v = 10"),
        "the same-file call produced its value (control): {text}"
    );
    // The regression: `ts_touch()` is 7, so 7 + 5 = 12.  A VALUE, not merely the
    // absence of an error — a library call that silently yielded 0 would pass the two
    // negative assertions above.
    assert!(
        text.contains("v = 12"),
        "the LIBRARY call produced its value at the frame: {text}"
    );
    let _ = std::fs::remove_file(&path);
}

/// The two silent resolution sites, closed by one `ResolutionContext` value.
///
/// `--lib` used to be threaded into three entry points and forgotten in the fourth
/// (@PLN120 E.1 — `loft repl --lib d` answered "Library 'x' not found"), and `:reset`
/// rebuilt a live session from `stdlib_dir` alone, silently dropping the libraries it
/// had. The second was LATENT BEHIND the first: with no way to get a library into a
/// REPL session, there was no way to watch a reset lose one — which is why fixing only
/// the flag would have left a live fault behind.
///
/// Both are asserted here, in order, in one session: a library call resolves, a
/// `:reset` happens, and a library call resolves again.
#[test]
fn repl_keeps_its_resolution_context_across_a_reset() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lib");
    let ctx = loft::repl::ResolutionContext {
        stdlib_dir: "default".to_string(),
        lib_dirs: vec![dir.to_string_lossy().into_owned()],
    };
    // The VALUE is pinned with loft's own `assert`, not by reading the printed result:
    // an expression's value goes to stdout, which this harness does not capture, while
    // a failed `assert` reports to the chrome stream it does.  So a wrong value fails
    // loudly here, and a resolution failure shows as "Unknown function".
    let input = std::io::Cursor::new(
        b"use typeshift;\nassert(ts_touch() == 7, \"before reset\")\n:reset\n          use typeshift;\nassert(ts_touch() == 7, \"after reset\")\n:quit\n"
            .to_vec(),
    );
    let mut out: Vec<u8> = Vec::new();
    loft::repl::run_repl(&ctx, input, &mut out).expect("repl run");
    let text = String::from_utf8_lossy(&out);

    assert!(
        !text.contains("not found") && !text.contains("Unknown function"),
        "the library must resolve in a plain REPL session, before AND after the \
         reset: {text}"
    );
    assert!(
        text.contains("session reset."),
        "the reset must actually have happened, or the second call proves nothing: \
         {text}"
    );
    assert!(
        !text.contains("assertion failed"),
        "both library calls must return 7 — a resolved call with a wrong value would \
         otherwise pass the checks above: {text}"
    );
}

/// loft#1459 site 2 — `eval` at a paused frame answers an ABSENT nullable local, for
/// every base type, instead of declining it.
///
/// The panel (`State::render_frame_local`) and `eval` are two renderers of one value, and
/// only the panel had learned `Optional`.  `render_capture` matched bare `Type` variants, so
/// every `Optional` fell to its `_ => None` — and a `None` there is the SAME answer as "this
/// expression does not evaluate", so `ni` reported *"couldn't evaluate `ni` at the frame"*
/// for a local the panel one site over printed as `null`.  Two independent paths had to be
/// opened for it to show at all: `base_type_name` dropped the `?` from `text["x"]?` and could
/// not reduce `ref(P)?`, and the capture fn's SOURCE spelling was also being used as the
/// SCHEMA key, where `vector<text?>` and `P?` name nothing.
///
/// `formatting.md` `(F-Render)` is the contract this pins: **a null of any type renders as
/// the literal word `null`.**  So the cells are one per base type, and the PRESENT twin of
/// each is what says a cure answered the value rather than answering `null` to everything —
/// the failure mode a one-line `Some("null")` would sail through.
///
/// Falsified by removing the `Type::Optional` arm from `render_capture`: **7 of 18** cells
/// fail, every one of them an ABSENT read answering `None`, with all 8 present cells and
/// both sentinel neighbours green.  Seven and not eight is the informative part — absent
/// `text` stays green there, because a `text?` reaches its answer through @P293's
/// single-element vector wrapper and never asks the `Optional` arm at all.  Falsified again
/// by reverting `base_type_name` alone: **2 of 18**, `nt` and `np` — the two cells whose
/// type spelling carries a decoration (`text["x"]?`, `ref(P)?`).  So the two halves of the
/// fix are load-bearing for disjoint cells, and neither subsumes the other.
#[test]
fn a_nullable_local_evaluates_at_a_paused_frame() {
    let mut s = session();
    for d in [
        "struct P { a: integer, b: text }",
        "enum Col { Red, Green }",
        // Every local is READ on the breakpoint line, so no slot is dead and none is
        // recycled — a reused slot is reported as `<reused by …>`, a frame fact that would
        // make these cells vacuous instead of failing.
        "fn probe(k: integer) -> integer {\n  \
           ni: integer? = null;  vi: integer? = 42;\n  \
           nf: float? = null;    vf: float? = 1.5;\n  \
           ns: single? = null;   vs: single? = 2.5f;\n  \
           nb: boolean? = null;  vb: boolean? = true;\n  \
           nc: character? = null; vc: character? = 'z';\n  \
           nt: text? = null;     vt: text? = \"hi\";  et: text? = \"\";\n  \
           np: P? = null;        vp: P? = P { a: 1, b: \"x\" };\n  \
           ne: Col? = null;      ve: Col? = Col.Green;\n  \
           fi: integer? = -9223372036854775807;\n  \
           all = \"{ni ?? 0}{vi ?? 0}{nf ?? 0.0}{vf ?? 0.0}{ns ?? 0.0f}{vs ?? 0.0f}\"\n    \
             + \"{nb ?? false}{vb ?? false}{nc ?? '?'}{vc ?? '?'}{nt ?? \"\"}{vt ?? \"\"}{et ?? \"z\"}\"\n    \
             + \"{(np ?? P {}).a}{(vp ?? P {}).a}{ne ?? Col.Red}{ve ?? Col.Red}{fi ?? 0}\";\n  \
           k + size(all)\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    // Line 11 of `probe` — the `all = …` statement: every local is ASSIGNED by then and
    // lines 11-13 READ each one, so no slot is dead and none has been recycled.  Breaking
    // at the body START instead would leave every local unset and pass this test on the
    // broken build, which is the vacuity trap site 1's guard also had to dodge.
    s.add_breakpoint("probe:11");
    assert!(matches!(s.eval("probe(1)"), Eval::Paused), "paused");

    // Every cell is collected before anything is asserted, so ONE run names every broken
    // one.  A per-cell `assert_eq!` stops at the first, which cannot tell "the nullable
    // family is declined" from "one base type is" — and it is the ratio between the two
    // halves below that says a cure read the value rather than answering `null` to
    // everything.
    //
    // ABSENT — `(F-Render)`: a null of any type is the word `null`, never the sentinel's
    // own numeric form and never a declined evaluation.  PRESENT — the same locals with a
    // value.  The last two are a sentinel's NEIGHBOURS: a guessed sentinel would report a
    // real value as null, which is the one lie a debugger must not tell.
    let cells: &[(&str, &str, &str)] = &[
        ("ni", "null", "absent integer"),
        ("nf", "null", "absent float"),
        ("ns", "null", "absent single"),
        ("nb", "null", "absent boolean"),
        ("nc", "null", "absent character"),
        ("nt", "null", "absent text"),
        ("np", "null", "absent struct"),
        ("ne", "null", "absent value enum"),
        ("vi", "42", "present integer"),
        ("vf", "1.5", "present float"),
        ("vs", "2.5f", "present single"),
        ("vb", "true", "present boolean"),
        ("vc", "'z'", "present character"),
        ("vt", "\"hi\"", "present text"),
        ("vp", "P{a:1,b:\"x\"}", "present struct"),
        ("ve", "Col.Green", "present value enum"),
        (
            "et",
            "\"\"",
            "an EMPTY text is a value, not the STRING_NULL handle",
        ),
        (
            "fi",
            "-9223372036854775807",
            "i64::MIN + 1 is a value, not the integer sentinel",
        ),
    ];
    let broken: Vec<String> = cells
        .iter()
        .filter_map(|(name, want, what)| {
            let got = s.debug_eval(name);
            (got.as_deref() != Some(*want))
                .then(|| format!("{what} (`{name}`): want {want:?}, got {got:?}"))
        })
        .collect();
    assert!(
        broken.is_empty(),
        "{} of {} cells wrong:\n  {}",
        broken.len(),
        cells.len(),
        broken.join("\n  ")
    );
    // CONTROL: a nonsense expression is still declined, so `None` has not been
    // repurposed into "null" for everything.
    assert_eq!(
        s.debug_eval("no_such + 1"),
        None,
        "an unknown name declines"
    );
    assert!(!s.debug_continue());
}
