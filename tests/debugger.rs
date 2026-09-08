// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN16 debugger — vertical tracer-bullet slice.
//!
//! The smallest end-to-end proof that the interpreter can **pause at a breakpoint
//! and read the live frame**: set a breakpoint on a source line inside a function
//! body, run a call, and assert the captured frame holds the argument the caller
//! passed. Everything the debugger needs (the REPL-at-frame, stepping, conditional
//! breaks) builds on this one capability, so this is the slice that de-risks the
//! whole plan.
//!
//! The slice captures **arguments** — always live at any body point. Non-argument
//! locals need liveness-gating (only show a local once its assignment has run),
//! which is the next slice. Built test-first; each test here is the next slice's
//! spec.

use loft::compile;
use loft::debugger::StepMode;
use loft::parser::{ParseResult, Parser};
use loft::repl::{Eval, ReplSession};
use loft::state::State;

fn repl() -> Parser {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("load stdlib");
    p
}

/// Define `def` then run `call` with a breakpoint on source `line`, returning the
/// captured frames.  `line` is relative to `def`'s own text (line 1 = its first
/// line), so a body line is post-prologue — where args sit in their slots.
fn run_with_breakpoint(
    p: &mut Parser,
    defs: &[&str],
    call: &str,
    bp_fn: &str,
    line: u32,
) -> Vec<loft::debugger::BreakHit> {
    for def in defs {
        match p.parse_statement(def) {
            ParseResult::Ready { .. } => {}
            other => panic!("def parse of {def:?} failed: {other:?}"),
        }
        let mut warm = State::new(p.database.clone());
        loft::scopes::check(&mut p.data, &mut p.database); // assign slots (locals need it)
        compile::byte_code(&mut warm, &mut p.data);
    }
    let entry = match p.parse_statement(call) {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call parse of {call:?} failed: {other:?}"),
    };
    let mut state = State::new(p.database.clone());
    // Assign slots before codegen (the REPL paths do this; bare expressions get
    // away without it, but a struct-constructing call needs its work-ref slots).
    loft::scopes::check(&mut p.data, &mut p.database);
    compile::byte_code(&mut state, &mut p.data);
    let d_nr = p.data.def_nr(&format!("n_{bp_fn}"));
    assert!(d_nr != u32::MAX, "function {bp_fn} not defined");
    assert!(
        state.set_breakpoint_fn_line(d_nr, line, &p.data).is_some(),
        "no breakpoint offset for {bp_fn} line {line}; breakable = {:?}",
        state.breakable_lines()
    );
    let name = p
        .data
        .def(entry)
        .name()
        .strip_prefix("n_")
        .expect("wrapper is n_-prefixed")
        .to_string();
    state.execute_argv(&name, &p.data, &[]);
    state.debug_hits().to_vec()
}

/// A breakpoint on a body line pauses there and captures the argument the caller
/// passed — the core "pause + read the live frame".
#[test]
fn breakpoint_on_body_line_captures_argument() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn dbl(n: integer) -> integer {\n  n + n\n}"],
        "dbl(42)",
        "dbl",
        2, // the `n + n` line
    );
    assert_eq!(hits.len(), 1, "breakpoint fired once: {hits:?}");
    assert_eq!(hits[0].function, "dbl", "frame is `dbl`: {hits:?}");
    assert!(
        hits[0].locals.iter().any(|(n, v)| n == "n" && v == "42"),
        "arg n == 42 captured: {hits:?}"
    );
}

/// A breakpoint fires on a body with **no fault-prone arithmetic op** — the bug
/// the `line_numbers` resolution fixed.  Source spans are emitted only at
/// `+ - * / % << >>`, so before the fix a pure `if`/constant body had no breakable
/// offset and silently never paused.  `pick` here has neither.
#[test]
fn breakpoint_fires_without_arithmetic() {
    let mut p = repl();
    // if-bodied, all constants — no arithmetic anywhere.
    let hits = run_with_breakpoint(
        &mut p,
        &["fn pick(b: boolean) -> integer {\n  if b { 111 } else { 222 }\n}"],
        "pick(true)",
        "pick",
        2,
    );
    assert_eq!(hits.len(), 1, "if/const body breakpoint fires: {hits:?}");
    assert_eq!(hits[0].function, "pick");
    assert!(
        hits[0].locals.iter().any(|(n, v)| n == "b" && v == "true"),
        "b == true captured: {hits:?}"
    );

    // bare-variable body — `n` is the whole body, no operator.
    let mut p2 = repl();
    let hits2 = run_with_breakpoint(
        &mut p2,
        &["fn id(n: integer) -> integer {\n  n\n}"],
        "id(42)",
        "id",
        2,
    );
    assert_eq!(hits2.len(), 1, "bare-var body breakpoint fires: {hits2:?}");
    assert!(
        hits2[0].locals.iter().any(|(n, v)| n == "n" && v == "42"),
        "n == 42 captured: {hits2:?}"
    );
}

/// `breakable_lines` reflects **every** line that emitted code (the dense
/// `line_numbers` table), not only lines with arithmetic — so each body line of a
/// multi-statement function is a valid breakpoint.
#[test]
fn breakable_lines_cover_non_arithmetic_lines() {
    let mut p = repl();
    match p.parse_statement("fn f(n: integer) -> integer {\n  m = n;\n  if m { 1 } else { 0 }\n}") {
        ParseResult::Ready { .. } => {}
        other => panic!("def failed: {other:?}"),
    }
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data, &mut p.database);
    compile::byte_code(&mut state, &mut p.data);
    let d = p.data.def_nr("n_f");
    // Line 2 (`m = n;`, no arithmetic) and line 3 (`if m {...}`, no arithmetic) are
    // both breakable — neither has a fault-prone operator.
    assert!(
        state.set_breakpoint_fn_line(d, 2, &p.data).is_some(),
        "line 2 (assignment) breakable; breakable = {:?}",
        state.breakable_lines()
    );
    assert!(
        state.set_breakpoint_fn_line(d, 3, &p.data).is_some(),
        "line 3 (if) breakable; breakable = {:?}",
        state.breakable_lines()
    );
}

/// The breakpoint fires once per call — two calls, two captured frames, each with
/// its own argument value.
#[test]
fn breakpoint_fires_per_call() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn id(n: integer) -> integer {\n  n + 0\n}"],
        "id(7) + id(9)",
        "id",
        2,
    );
    assert_eq!(hits.len(), 2, "two calls → two hits: {hits:?}");
    let vals: Vec<&str> = hits
        .iter()
        .filter_map(|h| {
            h.locals
                .iter()
                .find(|(n, _)| n == "n")
                .map(|(_, v)| v.as_str())
        })
        .collect();
    assert!(
        vals.contains(&"7") && vals.contains(&"9"),
        "both args captured: {vals:?}"
    );
}

/// Multiple arguments of a frame are all captured with their passed values.
#[test]
fn breakpoint_captures_multiple_arguments() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn add(a: integer, b: integer) -> integer {\n  a + b\n}"],
        "add(40, 2)",
        "add",
        2,
    );
    assert_eq!(hits.len(), 1, "one hit: {hits:?}");
    let frame = &hits[0].locals;
    assert!(
        frame.iter().any(|(n, v)| n == "a" && v == "40"),
        "a==40: {frame:?}"
    );
    assert!(
        frame.iter().any(|(n, v)| n == "b" && v == "2"),
        "b==2: {frame:?}"
    );
}

/// A struct argument is captured as its full own-format value via `show_loft` —
/// the frame view covers heap types, not just scalars.
#[test]
fn breakpoint_captures_struct_argument() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &[
            "struct Point { x: integer, y: integer }",
            "fn area(pt: Point) -> integer {\n  pt.x * pt.y\n}",
        ],
        "area(Point { x: 3, y: 4 })",
        "area",
        2,
    );
    assert_eq!(hits.len(), 1, "one hit: {hits:?}");
    let pt = hits[0]
        .locals
        .iter()
        .find(|(n, _)| n == "pt")
        .map(|(_, v)| v.as_str());
    assert_eq!(
        pt,
        Some("Point{x:3,y:4}"),
        "struct arg rendered via show_loft: {hits:?}"
    );
}

/// Liveness-gating (Q6): a non-arg local assigned *before* the breakpoint is
/// captured with its value; one assigned *at/after* the breakpoint is not yet live
/// and is excluded (rather than read as zero/garbage).
#[test]
fn breakpoint_frame_shows_scope_and_marks_unset() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn calc(n: integer) -> integer {\n  a = n + 1;\n  b = a * 2;\n  b\n}"],
        "calc(10)",
        "calc",
        3, // the `b = a * 2;` line — `a` holds its value, `b` is not written yet
    );
    assert_eq!(hits.len(), 1, "{hits:?}");
    let frame = &hits[0].locals;
    // `n` (arg) and `a` (assigned on line 2) hold their values.
    assert!(
        frame.iter().any(|(n, _)| n == "n"),
        "arg n visible: {frame:?}"
    );
    assert!(
        frame.iter().any(|(n, v)| n == "a" && v == "11"),
        "local a == 11 (n+1) held: {frame:?}"
    );
    // @PLN120 A — `b` is in lexical scope on the line that assigns it, so it is
    // SHOWN (silence reads as a broken debugger), and shown as `<unset>` rather
    // than as its slot's contents: the write has not run, and `reserve_frame` does
    // not zero locals.  Both halves matter — presence alone would pass a fix that
    // prints stack garbage, and the value check alone would pass the old omission.
    assert!(
        frame.iter().any(|(n, v)| n == "b" && v == "<unset>"),
        "local b in scope but unwritten → `<unset>`: {frame:?}"
    );
}

/// @PLN16 D1 — the REPL-at-frame bridge: seed a session with a captured frame and
/// evaluate an expression against its variables.  Scalar frame on a fresh session.
#[test]
fn repl_at_frame_evaluates_scalar_frame() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn calc(n: integer) -> integer {\n  a = n + 1;\n  b = a * 2;\n  b\n}"],
        "calc(10)",
        "calc",
        3,
    );
    let hit = &hits[0]; // n = 10, a = 11
    let mut s = ReplSession::new("default").expect("stdlib");
    let bound = s.seed_frame(hit);
    assert!(bound >= 2, "n and a seeded: {bound}");
    // n + a == 21 holds → the frame variables are in scope with their live values.
    assert!(
        matches!(s.eval("assert(n + a == 21, \"frame eval\")"), Eval::Ran),
        "eval over frame vars ran"
    );
}

/// D1 over a heap frame: a struct value seeds and a field expression evaluates,
/// when the session carries the program's type definitions (built from its parser).
#[test]
fn repl_at_frame_evaluates_struct_frame() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &[
            "struct Point { x: integer, y: integer }",
            "fn area(pt: Point) -> integer {\n  pt.x * pt.y\n}",
        ],
        "area(Point { x: 3, y: 4 })",
        "area",
        2,
    );
    // The session must know `Point`, so build it over the program's parser.
    let hit = hits[0].clone(); // pt = Point{x:3,y:4}
    let mut s = ReplSession::from_parser(p);
    let bound = s.seed_frame(&hit);
    assert!(bound >= 1, "pt seeded: {bound}");
    assert!(
        matches!(
            s.eval("assert(pt.x * pt.y == 12, \"struct frame\")"),
            Eval::Ran
        ),
        "eval pt.x * pt.y ran"
    );
}

/// @PLN16 E — conditional / test breakpoint: a condition evaluated against each
/// captured frame selects which hits to keep ("break when `n > 1`").
#[test]
fn conditional_breakpoint_filters_by_frame_condition() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn step(n: integer) -> integer {\n  n * 10\n}"],
        "step(1) + step(2) + step(3)",
        "step",
        2,
    );
    assert_eq!(hits.len(), 3, "fires per call: {hits:?}");
    let mut s = ReplSession::from_parser(p);
    assert_eq!(
        hits.iter().filter(|h| s.frame_holds(h, "n > 1")).count(),
        2,
        "n > 1 holds for n = 2 and 3"
    );
    assert_eq!(
        hits.iter().filter(|h| s.frame_holds(h, "n > 100")).count(),
        0,
        "n > 100 holds for none"
    );
    assert_eq!(
        hits.iter().filter(|h| s.frame_holds(h, "n >= 1")).count(),
        3,
        "n >= 1 holds for all"
    );
}

/// @PLN16 F (the hard one) — change a value in the REPL at a breakpoint, then
/// resume and continue with the changed value.  `calc(5)` is normally 50; we edit
/// `n` to 99 at the breakpoint so it returns 990, and the program's assert
/// (`calc(5) == 990`) then passes — proving the edit was picked up on resume.
#[test]
fn step_picks_up_repl_edited_value() {
    let mut p = repl();
    match p.parse_statement("fn calc(n: integer) -> integer {\n  n * 10\n}") {
        ParseResult::Ready { .. } => {}
        other => panic!("def failed: {other:?}"),
    }
    let mut warm = State::new(p.database.clone());
    loft::scopes::check(&mut p.data, &mut p.database);
    compile::byte_code(&mut warm, &mut p.data);
    // The program asserts the EDITED result (990); false without the edit (50).
    let entry = match p.parse_statement("assert(calc(5) == 990, \"edited n to 99\")") {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call failed: {other:?}"),
    };
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data, &mut p.database);
    compile::byte_code(&mut state, &mut p.data);
    let calc = p.data.def_nr("n_calc");
    state.enable_stepping();
    assert!(
        state.set_breakpoint_fn_line(calc, 2, &p.data).is_some(),
        "breakpoint set"
    );
    let name = p
        .data
        .def(entry)
        .name()
        .strip_prefix("n_")
        .unwrap()
        .to_string();

    // Run → suspends at calc's body (n == 5, not yet multiplied).
    state.execute_argv(&name, &p.data, &[]);
    assert!(state.is_paused(), "suspended at breakpoint");
    let hit = state.paused_frame().expect("frame").clone();
    assert!(
        hit.locals.iter().any(|(n, v)| n == "n" && v == "5"),
        "n == 5 at pause: {hit:?}"
    );

    // The user edits the value in a REPL seeded from the paused frame.
    let mut s = ReplSession::new("default").expect("stdlib");
    s.seed_frame(&hit);
    assert!(matches!(s.eval("n = 99"), Eval::Ran), "REPL edit n = 99");
    let edited: i64 = s.value_of("n").expect("value").parse().expect("int");
    assert_eq!(edited, 99);

    // Write the edited value back into the live frame, then resume.
    assert!(
        state.set_frame_value("n", edited, &p.data),
        "write-back n = 99"
    );
    state.resume();

    // calc(5) returned 99 * 10 == 990, so the program's assert held → no error.
    assert!(
        state.database.runtime_error.is_none(),
        "edit picked up on resume (assert calc(5)==990 passed); err = {:?}",
        state.database.runtime_error
    );
}

/// Run `defs` + `call` in **stepping mode** with a breakpoint on `bp_fn` line
/// `line`; return the `State` suspended there, for the test to drive `debug_step`.
fn run_to_pause(p: &mut Parser, defs: &[&str], call: &str, bp_fn: &str, line: u32) -> State {
    for def in defs {
        match p.parse_statement(def) {
            ParseResult::Ready { .. } => {}
            other => panic!("def parse of {def:?} failed: {other:?}"),
        }
        let mut warm = State::new(p.database.clone());
        loft::scopes::check(&mut p.data, &mut p.database);
        compile::byte_code(&mut warm, &mut p.data);
    }
    let entry = match p.parse_statement(call) {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call parse of {call:?} failed: {other:?}"),
    };
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data, &mut p.database);
    compile::byte_code(&mut state, &mut p.data);
    let d_nr = p.data.def_nr(&format!("n_{bp_fn}"));
    state.enable_stepping();
    assert!(
        state.set_breakpoint_fn_line(d_nr, line, &p.data).is_some(),
        "breakpoint on {bp_fn}:{line}"
    );
    let name = p
        .data
        .def(entry)
        .name()
        .strip_prefix("n_")
        .unwrap()
        .to_string();
    state.execute_argv(&name, &p.data, &[]);
    assert!(state.is_paused(), "suspended at {bp_fn}:{line}");
    state
}

/// `outer` calls `inner`; line 2 of `outer` is the call, line 3 reads the result.
const NESTED: &[&str] = &[
    "fn inner(x: integer) -> integer {\n  x + 1\n}",
    "fn outer(n: integer) -> integer {\n  a = inner(n);\n  a + 100\n}",
];

/// Step *into*: from `outer`'s `inner(n)` line, descend into `inner`.
#[test]
fn step_into_descends_into_callee() {
    let mut p = repl();
    let mut state = run_to_pause(&mut p, NESTED, "outer(5)", "outer", 2);
    assert_eq!(state.paused_frame().unwrap().function, "outer");
    assert!(state.debug_step(StepMode::Into, &p.data), "still running");
    let f = state.paused_frame().unwrap();
    assert_eq!(f.function, "inner", "stepped into inner: {f:?}");
    assert!(
        f.locals.iter().any(|(n, v)| n == "x" && v == "5"),
        "inner's x == 5: {f:?}"
    );
}

/// Step *over*: run `inner(n)` to completion without pausing in it, then stop at
/// the next line in `outer` — where the result `a` is now live.
#[test]
fn step_over_runs_call_and_stays_in_frame() {
    let mut p = repl();
    let mut state = run_to_pause(&mut p, NESTED, "outer(5)", "outer", 2);
    assert!(state.debug_step(StepMode::Over, &p.data), "still running");
    let f = state.paused_frame().unwrap();
    assert_eq!(
        f.function, "outer",
        "stayed in outer (did not descend): {f:?}"
    );
    assert!(
        f.locals.iter().any(|(n, v)| n == "a" && v == "6"),
        "a == inner(5) == 6: {f:?}"
    );
}

/// Step *out*: from inside `inner`, run to its return and pause back in `outer`.
#[test]
fn step_out_returns_to_caller() {
    let mut p = repl();
    let mut state = run_to_pause(&mut p, NESTED, "outer(5)", "inner", 2);
    assert_eq!(state.paused_frame().unwrap().function, "inner");
    assert!(state.debug_step(StepMode::Out, &p.data), "still running");
    assert_eq!(
        state.paused_frame().unwrap().function,
        "outer",
        "stepped out to outer"
    );
}

/// @PLN16 B1 — the interpret-set for a breakpoint is the breakpoint fn plus its
/// transitive callers (so the whole stack to the break is introspectable); a
/// function that cannot reach it is excluded.  Pure static analysis — no run.
#[test]
fn b1_interpret_set_is_transitive_callers() {
    let mut p = repl();
    for def in NESTED {
        match p.parse_statement(def) {
            ParseResult::Ready { .. } => {}
            other => panic!("def parse failed: {other:?}"),
        }
    }
    match p.parse_statement("fn unrelated(z: integer) -> integer {\n  z + 1\n}") {
        ParseResult::Ready { .. } => {}
        other => panic!("def parse failed: {other:?}"),
    }
    let entry = match p.parse_statement("outer(5)") {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call parse failed: {other:?}"),
    };
    let inner = p.data.def_nr("n_inner");
    let outer = p.data.def_nr("n_outer");
    let unrelated = p.data.def_nr("n_unrelated");

    let set = loft::debugger::interpret_set(&p.data, inner);
    assert!(set.contains(&inner), "the breakpoint fn itself: {set:?}");
    assert!(
        set.contains(&outer),
        "outer calls inner → interpret: {set:?}"
    );
    assert!(
        set.contains(&entry),
        "the wrapper calls outer → interpret: {set:?}"
    );
    assert!(
        !set.contains(&unrelated),
        "unrelated never reaches inner: {set:?}"
    );

    // The static callees of `outer` include `inner`.
    assert!(loft::debugger::callees(&p.data, outer).contains(&inner));
    // A breakpoint in a leaf fn pulls in no callees (inner does not call unrelated).
    let leaf = loft::debugger::interpret_set(&p.data, unrelated);
    assert!(
        leaf.contains(&unrelated) && !leaf.contains(&inner),
        "{leaf:?}"
    );
}

/// @PLN16 B2 — `unmark_for_debug` switches a breakpoint's interpret-set **back to
/// interpreted** under mixed execution: it clears the default-native mark
/// (`def.native`) on the breakpoint fn + its transitive callers, leaving an
/// unrelated compiled fn marked.  The compiled-vs-interpret choice is codegen-time
/// (set `def.native` → `OpStaticCall` cdylib bridge; empty + a body → `OpCall`
/// interpreted), so this un-mark + the standard recompile is the switch — a
/// breakpoint can then fire in the interpreter loop.
#[test]
fn b2_unmark_switches_interpret_set_to_interpreted() {
    use std::collections::HashSet;
    let mut p = repl();
    for def in [
        "fn baz(n: integer) -> integer {\n  n + 1\n}",
        "fn bar(n: integer) -> integer {\n  baz(n)\n}",
        "fn foo(n: integer) -> integer {\n  bar(n)\n}",
        "fn unrelated(z: integer) -> integer {\n  z * 2\n}",
    ] {
        match p.parse_statement(def) {
            ParseResult::Ready { .. } => {}
            other => panic!("def parse failed: {other:?}"),
        }
    }
    let baz = p.data.def_nr("n_baz");
    let bar = p.data.def_nr("n_bar");
    let foo = p.data.def_nr("n_foo");
    let unrelated = p.data.def_nr("n_unrelated");
    assert!(
        [baz, bar, foo, unrelated].iter().all(|&d| d != u32::MAX),
        "all four fns defined"
    );
    // Simulate them being default-native (compiled cdylib dispatch).
    let all: HashSet<u32> = [baz, bar, foo, unrelated].into_iter().collect();
    loft::native_lib::mark_exports(&mut p.data, &all);
    assert!(
        !p.data.def(baz).native().is_empty(),
        "baz starts marked (compiled)"
    );
    // A breakpoint in baz un-marks baz + its transitive callers (so the whole chain
    // to the break interprets); `unrelated` can't reach baz, so it stays compiled.
    let n = loft::debugger::unmark_for_debug(&mut p.data, baz);
    assert_eq!(n, 3, "baz + bar + foo un-marked (transitive callers)");
    assert!(p.data.def(baz).native().is_empty(), "baz interprets");
    assert!(
        p.data.def(bar).native().is_empty(),
        "bar (direct caller) interprets"
    );
    assert!(
        p.data.def(foo).native().is_empty(),
        "foo (transitive caller) interprets"
    );
    assert!(
        !p.data.def(unrelated).native().is_empty(),
        "unrelated stays compiled — it can't reach baz"
    );
    // Idempotent: a second un-mark touches nothing (all already interpreted).
    assert_eq!(loft::debugger::unmark_for_debug(&mut p.data, baz), 0);
}

/// @PLN16 B2 — a **pure-cdylib** function (no loft body) is an absolute boundary:
/// `unmark_for_debug` leaves it marked, since there is no interpreted body to run
/// (a breakpoint inside it can never fire).
#[test]
fn b2_pure_cdylib_fn_stays_an_absolute_boundary() {
    let mut p = repl();
    // A no-body `#native` forward declaration — pure cdylib, no interpreted body.
    match p.parse_statement(
        "pub fn vec_sum(data: vector<integer>) -> integer not null;\n#native \"loft_shared_x\"\n",
    ) {
        ParseResult::Ready { .. } => {}
        other => panic!("native decl parse failed: {other:?}"),
    }
    let vs = p.data.def_nr("n_vec_sum");
    assert!(vs != u32::MAX, "vec_sum defined");
    assert!(
        !p.data.def(vs).native().is_empty(),
        "pure-cdylib fn is marked (its declared #native symbol)"
    );
    // Breaking "in" it un-marks nothing — there is no body to interpret.
    let n = loft::debugger::unmark_for_debug(&mut p.data, vs);
    assert_eq!(n, 0, "pure-cdylib fn stays compiled (absolute boundary)");
    assert!(
        !p.data.def(vs).native().is_empty(),
        "still marked after unmark_for_debug"
    );
}

/// The two functions whose `target` is reached only **indirectly** — `apply`
/// invokes its fn-ref parameter `f`, so the call to `target` is a `CallRef`.
const INDIRECT: &[&str] = &[
    "fn target(x: integer) -> integer {\n  x + 1\n}",
    "fn apply(f: fn(integer) -> integer, x: integer) -> integer {\n  f(x)\n}",
];

/// **Execution supports it.** A breakpoint in a function reached via an *indirect*
/// call (a fn-ref) still fires with the frame captured — the breakpoint is an
/// offset in the loop, agnostic to how the function was entered.  So an interpreted
/// function reached through a fn-ref is fully debuggable (the "middle layer
/// interpreted, called indirectly" case).
#[test]
fn breakpoint_fires_in_indirectly_called_fn() {
    let mut p = repl();
    let hits = run_with_breakpoint(&mut p, INDIRECT, "apply(target, 5)", "target", 2);
    assert_eq!(hits.len(), 1, "fired in indirectly-called target: {hits:?}");
    assert_eq!(hits[0].function, "target");
    assert!(
        hits[0].locals.iter().any(|(n, v)| n == "x" && v == "5"),
        "x == 5: {hits:?}"
    );
}

/// **Static analysis can't see it.** `apply` reaches `target` only through its
/// fn-ref parameter (a `CallRef`), which carries no static target — so B1's
/// `interpret_set(target)` does *not* include `apply`.  This is the documented
/// limitation, and the precise reason on-demand (B3) switching is needed to
/// interpret an indirectly-reached frame: the runtime call path is the only place
/// the edge is visible.
#[test]
fn b1_does_not_trace_indirect_callers() {
    let mut p = repl();
    for def in INDIRECT {
        match p.parse_statement(def) {
            ParseResult::Ready { .. } => {}
            other => panic!("def parse failed: {other:?}"),
        }
    }
    match p.parse_statement("apply(target, 5)") {
        ParseResult::Ready { .. } => {}
        other => panic!("call parse failed: {other:?}"),
    }
    let target = p.data.def_nr("n_target");
    let apply = p.data.def_nr("n_apply");
    assert!(
        target != u32::MAX && apply != u32::MAX,
        "both fns resolve (else the negative assert below is vacuous)"
    );
    let set = loft::debugger::interpret_set(&p.data, target);
    assert!(set.contains(&target), "the breakpoint fn itself: {set:?}");
    assert!(
        !set.contains(&apply),
        "apply reaches target only via CallRef → invisible to static analysis: {set:?}"
    );
}

/// @PLN16 B3 — at a breakpoint reached through an *indirect* call, the **full
/// runtime call stack** is introspectable: `break_stack` walks the live
/// `call_stack`, so the indirect caller `apply` appears with its locals — the very
/// frame B1's static analysis could not link.  The runtime call path is what
/// resolves the indirect gap.
#[test]
fn b3_full_stack_includes_indirect_caller() {
    let mut p = repl();
    let state = run_to_pause(&mut p, INDIRECT, "apply(target, 5)", "target", 2);
    let stack = state.break_stack(&p.data);
    // Top frame: target (where we broke), x == 5.
    assert_eq!(stack[0].function, "target", "{stack:?}");
    assert!(
        stack[0].locals.iter().any(|(n, v)| n == "x" && v == "5"),
        "target x == 5: {:?}",
        stack[0]
    );
    // The indirect caller `apply` is on the runtime stack with its arg x == 5 —
    // the frame B1 (b1_does_not_trace_indirect_callers) could not statically link.
    let apply = stack
        .iter()
        .find(|f| f.function == "apply")
        .expect("apply (the indirect caller) is on the runtime stack");
    assert!(
        apply.locals.iter().any(|(n, v)| n == "x" && v == "5"),
        "apply x == 5 (indirect caller introspectable via the runtime stack): {apply:?}"
    );
}

/// `set_breakpoint_fn_start` — break at a named function's body start (the
/// human-friendly `:break foo` form) without supplying a line number, and read the
/// frame correctly (post-prologue: the arg is in its slot).
#[test]
fn breakpoint_at_fn_body_start_by_name() {
    let mut p = repl();
    match p.parse_statement("fn dbl(n: integer) -> integer {\n  n + n\n}") {
        ParseResult::Ready { .. } => {}
        other => panic!("def failed: {other:?}"),
    }
    let entry = match p.parse_statement("dbl(42)") {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call failed: {other:?}"),
    };
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data, &mut p.database);
    compile::byte_code(&mut state, &mut p.data);
    assert!(
        state.set_breakpoint_fn_start("dbl", &p.data).is_some(),
        "breakpoint at dbl's body start"
    );
    assert!(
        state.set_breakpoint_fn_start("nope", &p.data).is_none(),
        "unknown fn → false"
    );
    let name = p
        .data
        .def(entry)
        .name()
        .strip_prefix("n_")
        .unwrap()
        .to_string();
    state.execute_argv(&name, &p.data, &[]);
    let hits = state.debug_hits();
    assert_eq!(hits.len(), 1, "fired once: {hits:?}");
    assert_eq!(hits[0].function, "dbl");
    assert!(
        hits[0].locals.iter().any(|(n, v)| n == "n" && v == "42"),
        "n == 42 at body start (post-prologue): {hits:?}"
    );
}

/// loft#1459 — a `τ?` local reads at a paused frame like its dense twin.
///
/// `render_frame_local` matched every type in its DENSE spelling, so a nullable local
/// matched no arm and fell to the `other` catch-all, which prints the TYPE: the panel showed
/// `n = <integer?>` where every other local shows a value.  `@FR-L-Null` says
/// layout(τ) = layout(τ?), so the slot reads the same way and all the nullable case owes is
/// the ABSENT value.
///
/// The two cells are the two answers a nullable can give, and both are needed: an ABSENT one
/// proves the sentinel is recognised rather than printed raw (`i32::MIN` would render as a
/// large negative number, which looks like a value), and a PRESENT one proves the peel did
/// not swallow the number with it.  The dense `x` is the control — it never moved, and if a
/// cure broke it the peel would be doing something other than peeling.
/// Falsified by removing the `Type::Optional` arm from `State::render_frame_local`:
/// `left: "<integer?>"  right: "null"`, one assertion failure, 38 other debugger tests
/// unmoved.  The `m` and `x` cells stay green there, which is what says the peel answers the
/// ABSENT case and does not merely make the panel print something.
#[test]
fn a_nullable_local_reads_like_its_dense_twin_at_a_paused_frame() {
    let mut p = repl();
    match p.parse_statement(
        "fn probe(x: integer) -> integer {\n  n: integer? = null;\n  m: integer? = 42;\n  y = x + 1;\n  y + (n ?? 0) + (m ?? 0)\n}",
    ) {
        ParseResult::Ready { .. } => {}
        other => panic!("def failed: {other:?}"),
    }
    let entry = match p.parse_statement("probe(5)") {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call failed: {other:?}"),
    };
    // Line 4 (`y = x + 1`), so both nullable locals are ASSIGNED by the time the frame is
    // read, and line 5 READS them so neither slot is dead.  Two vacuity traps avoided, both
    // met while writing this: breaking at the body START leaves `n` and `m` nonexistent, so
    // the test passes on the broken build by asserting only the control; and leaving them
    // unread lets the slot allocator reuse one, which the panel honestly reports as
    // `n = <reused by m>` — a frame fact, not a nullability one.
    let d = p.data.def_nr("n_probe");
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data, &mut p.database);
    compile::byte_code(&mut state, &mut p.data);
    assert!(
        state.set_breakpoint_fn_line(d, 4, &p.data).is_some(),
        "line 4 breakable; breakable = {:?}",
        state.breakable_lines()
    );
    let name = p
        .data
        .def(entry)
        .name()
        .strip_prefix("n_")
        .unwrap()
        .to_string();
    state.execute_argv(&name, &p.data, &[]);
    let hits = state.debug_hits();
    assert!(!hits.is_empty(), "the breakpoint fired: {hits:?}");
    let f = &hits[hits.len() - 1];
    let show = |want: &str| {
        f.locals
            .iter()
            .find(|(n, _)| n == want)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| format!("<{want} absent>"))
    };
    assert_eq!(show("n"), "null", "an ABSENT nullable reads `null`: {f:?}");
    assert_eq!(show("m"), "42", "a PRESENT nullable reads its value: {f:?}");
    assert_eq!(show("x"), "5", "CONTROL: the dense local is unmoved: {f:?}");
}

/// Negative control: debugging on but no breakpoint registered → zero hits, and
/// the program still runs (the `assert` inside proves execution completed).
#[test]
fn debug_on_without_breakpoint_yields_no_hits() {
    let mut p = repl();
    match p.parse_statement("assert(max(3, 7) == 7, \"runs\")") {
        ParseResult::Ready { entry_def_nr } => {
            let mut state = State::new(p.database.clone());
            compile::byte_code(&mut state, &mut p.data);
            state.enable_debug(); // debugging on, but no breakpoint set
            let name = p
                .data
                .def(entry_def_nr)
                .name()
                .strip_prefix("n_")
                .unwrap()
                .to_string();
            state.execute_argv(&name, &p.data, &[]);
            assert!(state.debug_hits().is_empty(), "no breakpoint → no hits");
        }
        other => panic!("parse failed: {other:?}"),
    }
}

/// @PLN16 G1 — **text-local read** regression.  A text *argument* is a 16-byte
/// `Str` borrow; a text *local* is a 24-byte owned `String`.  The frame renderer
/// must read each at its true width — reading a local's `String` as a `Str`
/// mis-takes the capacity word for the length and renders `""` / garbage.
#[test]
fn text_local_read_shows_value() {
    let mut p = repl();
    let defs = &[
        "fn build(seed: text) -> text {\n  s = seed;\n  a = s.to_uppercase();\n  \
         b = a;\n  c = s.to_uppercase();\n  c\n}",
    ];
    let state = run_to_pause(&mut p, defs, "build(\"world\")", "build", 4);
    let f = state.paused_frame().unwrap();
    assert!(
        f.locals
            .iter()
            .any(|(n, v)| n == "seed" && v == "\"world\""),
        "text arg seed (Str): {f:?}"
    );
    assert!(
        f.locals.iter().any(|(n, v)| n == "s" && v == "\"world\""),
        "text local s (String): {f:?}"
    );
    assert!(
        f.locals.iter().any(|(n, v)| n == "a" && v == "\"WORLD\""),
        "text local a (String): {f:?}"
    );
}

/// @PLN16 G1 — **live edit of a text argument**, picked up on resume.  `greet("hi")`
/// is normally 0; we edit `msg` to `"BYE"` at the breakpoint so it returns 1, and
/// the program's assert then passes — proving the `Str` slot's repoint is read by
/// the resumed call.
#[test]
fn live_edit_text_arg_resumes_with_new_value() {
    let mut p = repl();
    let defs =
        &["fn greet(msg: text) -> integer {\n  m = 0;\n  if msg == \"BYE\" { 1 } else { 0 }\n}"];
    let mut state = run_to_pause(
        &mut p,
        defs,
        "assert(greet(\"hi\") == 1, \"edited msg to BYE\")",
        "greet",
        2,
    );
    assert!(
        state.set_frame_literal("msg", "\"BYE\"", &p.data),
        "edit text arg"
    );
    state.resume();
    assert!(
        state.database.runtime_error.is_none(),
        "text-arg edit picked up on resume; err = {:?}",
        state.database.runtime_error
    );
}

/// @PLN16 G1 — **live edit of a text local**, picked up on resume.  A text local
/// owns its `String`; the edit overwrites it (without dropping the prior — possibly
/// uninitialised — slot value).  `make()` returns 1 only if the edited `s == "BYE"`.
#[test]
fn live_edit_text_local_resumes_with_new_value() {
    let mut p = repl();
    let defs = &[
        "fn make() -> integer {\n  s = \"hi\";\n  a = s.to_uppercase();\n  \
         if s == \"BYE\" { 1 } else { 0 }\n}",
    ];
    let mut state = run_to_pause(
        &mut p,
        defs,
        "assert(make() == 1, \"edited local s to BYE\")",
        "make",
        3,
    );
    assert!(
        state.set_frame_literal("s", "\"BYE\"", &p.data),
        "edit text local"
    );
    state.resume();
    assert!(
        state.database.runtime_error.is_none(),
        "text-local edit picked up on resume; err = {:?}",
        state.database.runtime_error
    );
}

/// @PLN16 G1 — **live edit of a simple enum**, picked up on resume.  A simple enum
/// is an inline 1-based discriminant byte; the edit parses `Enum.Variant` and writes
/// the byte.  `pick(Color.Green)` returns 1 only after we edit `c` to `Color.Blue`.
#[test]
fn live_edit_simple_enum_resumes_with_new_value() {
    let mut p = repl();
    let defs = &[
        "enum Color { Red, Green, Blue }",
        "fn pick(c: Color) -> integer {\n  m = 0;\n  if c == Color.Blue { 1 } else { 0 }\n}",
    ];
    let mut state = run_to_pause(
        &mut p,
        defs,
        "assert(pick(Color.Green) == 1, \"edited c to Blue\")",
        "pick",
        2,
    );
    assert!(
        state.set_frame_literal("c", "Color.Blue", &p.data),
        "edit simple enum"
    );
    state.resume();
    assert!(
        state.database.runtime_error.is_none(),
        "enum edit picked up on resume; err = {:?}",
        state.database.runtime_error
    );
}

/// @PLN16 G1 — a **heap** local (struct / vector / struct-enum) edit is rejected:
/// reconstructing a `DbRef` value in the *live* store from a literal needs a
/// literal→store materialiser (the remaining work), so the edit returns `false`
/// rather than corrupt the slot.  The read still works.
#[test]
fn live_edit_rejects_heap_local() {
    let mut p = repl();
    let defs = &[
        "struct Point { x: integer, y: integer }",
        "fn area(pt: Point) -> integer {\n  m = pt.x;\n  pt.x * pt.y\n}",
    ];
    let mut state = run_to_pause(&mut p, defs, "area(Point{x: 3, y: 4})", "area", 2);
    assert!(
        state
            .paused_frame()
            .unwrap()
            .locals
            .iter()
            .any(|(n, v)| n == "pt" && v == "Point{x:3,y:4}"),
        "struct read still works"
    );
    assert!(
        !state.set_frame_literal("pt", "Point{x: 9, y: 9}", &p.data),
        "heap edit rejected (deferred)"
    );
}

/// @PLN16.J — **live edit of a scalar struct field** (`pt.x = 9`), picked up on
/// resume.  Resolves the struct local's `DbRef`, looks the field offset up in the
/// schema, and writes the scalar in place.  `area(Point{3,4})` is 12; editing
/// `pt.x` to 5 makes it 5*4 = 20, so the program's assert passes only if the field
/// edit was read by the resumed call.
#[test]
fn live_edit_struct_field_resumes_with_new_value() {
    let mut p = repl();
    let defs = &[
        "struct Point { x: integer, y: integer }",
        "fn area(pt: Point) -> integer {\n  m = pt.x;\n  pt.x * pt.y\n}",
    ];
    let mut state = run_to_pause(
        &mut p,
        defs,
        "assert(area(Point{x: 3, y: 4}) == 20, \"edited pt.x to 5\")",
        "area",
        2,
    );
    assert!(
        state.set_frame_field("pt", "x", "5", &p.data),
        "edit scalar struct field"
    );
    state.resume();
    assert!(
        state.database.runtime_error.is_none(),
        "field edit picked up on resume; err = {:?}",
        state.database.runtime_error
    );
}

/// @PLN16.J — field-edit rejection cases: an unknown field, a non-struct base, and
/// a non-scalar field all return `false` (no write, no corruption).
#[test]
fn field_edit_rejects_bad_targets() {
    let mut p = repl();
    let defs = &[
        "struct Point { x: integer, y: integer }",
        "fn area(pt: Point, n: integer) -> integer {\n  m = pt.x;\n  pt.x * pt.y + n\n}",
    ];
    let mut state = run_to_pause(&mut p, defs, "area(Point{x: 3, y: 4}, 7)", "area", 2);
    assert!(
        !state.set_frame_field("pt", "z", "5", &p.data),
        "unknown field"
    );
    assert!(
        !state.set_frame_field("n", "x", "5", &p.data),
        "non-struct base"
    );
    assert!(
        !state.set_frame_field("pt", "x", "notanint", &p.data),
        "unparseable literal"
    );
    // a valid edit still works after the rejections
    assert!(
        state.set_frame_field("pt", "x", "9", &p.data),
        "valid edit after rejects"
    );
}

/// @PLN16.J (M1b) — live edit of a scalar at a **nested** struct path
/// (`o.inner.a = 99`).  Nested structs are inlined, so the path resolves by summing
/// field offsets in the same record; `f(...)` returns `o.inner.a`, so the program's
/// assert passes only if the summed-offset write landed on the right field.
#[test]
fn live_edit_nested_struct_path_resumes_with_new_value() {
    let mut p = repl();
    let defs = &[
        "struct Inner { a: integer, b: integer }",
        "struct Outer { n: integer, inner: Inner }",
        "fn f(o: Outer) -> integer {\n  m = o.n;\n  o.inner.a\n}",
    ];
    let mut state = run_to_pause(
        &mut p,
        defs,
        "assert(f(Outer { n: 1, inner: Inner { a: 5, b: 6 } }) == 99, \"edited o.inner.a\")",
        "f",
        2,
    );
    assert!(
        state.set_frame_path("o", &["inner", "a"], "99", &p.data),
        "edit nested scalar path o.inner.a"
    );
    state.resume();
    assert!(
        state.database.runtime_error.is_none(),
        "nested path edit picked up on resume; err = {:?}",
        state.database.runtime_error
    );
}

/// @PLN16.J (M1b) — nested-path rejections: an unknown intermediate / leaf field and
/// a non-scalar leaf (an inline struct) all return `false`, no write.
#[test]
fn nested_path_edit_rejects_bad_paths() {
    let mut p = repl();
    let defs = &[
        "struct Inner { a: integer }",
        "struct Outer { n: integer, inner: Inner }",
        "fn f(o: Outer) -> integer {\n  m = o.n;\n  o.inner.a\n}",
    ];
    let mut state = run_to_pause(
        &mut p,
        defs,
        "f(Outer { n: 1, inner: Inner { a: 5 } })",
        "f",
        2,
    );
    assert!(
        !state.set_frame_path("o", &["bogus", "a"], "9", &p.data),
        "unknown intermediate"
    );
    assert!(
        !state.set_frame_path("o", &["inner", "z"], "9", &p.data),
        "unknown leaf"
    );
    assert!(
        !state.set_frame_path("o", &["inner"], "9", &p.data),
        "non-scalar leaf (struct)"
    );
    assert!(
        state.set_frame_path("o", &["inner", "a"], "9", &p.data),
        "valid path after rejects"
    );
}

/// @PLN16 — live edit of a scalar vector **element** (`v[1] = 99`), picked up on
/// resume.  The element lives in the vector's backing record, so the edit is one
/// in-place scalar write at `8 + i*stride`; `f(...)` returns `v[1]`, so the program's
/// assert passes only if the write landed on the right element slot.
#[test]
fn live_edit_vector_element_resumes_with_new_value() {
    let mut p = repl();
    let defs = &["fn f(v: vector<integer>) -> integer {\n  m = v[0];\n  v[1]\n}"];
    let mut state = run_to_pause(
        &mut p,
        defs,
        "assert(f([10, 20, 30]) == 99, \"edited v[1]\")",
        "f",
        2,
    );
    assert!(state.set_frame_element("v", 1, "99", &p.data), "edit v[1]");
    state.resume();
    assert!(
        state.database.runtime_error.is_none(),
        "element edit picked up on resume; err = {:?}",
        state.database.runtime_error
    );
}

/// @PLN16 — vector-element edit rejections: out-of-range, negative index, a non-vector
/// base, and an unparseable literal all return `false` (no write past the end), and a
/// valid edit still works afterwards.
#[test]
fn vector_element_edit_rejects() {
    let mut p = repl();
    let defs = &["fn f(v: vector<integer>, n: integer) -> integer {\n  m = n;\n  v[0]\n}"];
    let mut state = run_to_pause(&mut p, defs, "f([10, 20, 30], 7)", "f", 2);
    assert!(
        !state.set_frame_element("v", 5, "9", &p.data),
        "out of range"
    );
    assert!(
        !state.set_frame_element("v", -1, "9", &p.data),
        "negative index"
    );
    assert!(
        !state.set_frame_element("n", 0, "9", &p.data),
        "non-vector base"
    );
    assert!(
        !state.set_frame_element("v", 0, "notanint", &p.data),
        "unparseable literal"
    );
    assert!(
        state.set_frame_element("v", 0, "42", &p.data),
        "valid edit after rejects"
    );
}

// @PLN98 P3.2 — the COOPERATIVE pause the browser tier reuses.  At a breakpoint
// `execute_argv` RETURNS control (it does not block — unlike the native
// live-dispatch pause loop) with the State-held stack preserved and the frame
// capturable; `debug_step(Continue)` re-enters and runs to completion.  This raw
// cycle — execute-yields + step-resumes, keyed on `is_paused()` — is exactly what
// a wasm debug session drives across the JS event loop (no new `debug_yield` flag:
// `is_paused()` is the signal).
#[test]
fn cooperative_pause_yields_control_then_resumes_to_completion() {
    let mut p = repl();
    match p.parse_statement("fn compute(n: integer) -> integer {\n  m = n + 2;\n  m\n}") {
        ParseResult::Ready { .. } => {}
        other => panic!("def parse failed: {other:?}"),
    }
    {
        // Warm-compile so compute's slots + line table exist before the run build.
        let mut warm = State::new(p.database.clone());
        loft::scopes::check(&mut p.data, &mut p.database);
        compile::byte_code(&mut warm, &mut p.data);
    }
    let entry = match p.parse_statement("compute(40)") {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call parse failed: {other:?}"),
    };
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data, &mut p.database);
    compile::byte_code(&mut state, &mut p.data);
    let d_nr = p.data.def_nr("n_compute");
    assert!(
        state.set_breakpoint_fn_line(d_nr, 2, &p.data).is_some(),
        "breakpoint set on compute line 2"
    );
    state.enable_stepping(); // stepping → the breakpoint SUSPENDS
    let name = p
        .data
        .def(entry)
        .name()
        .strip_prefix("n_")
        .expect("wrapper is n_-prefixed")
        .to_string();

    // 1. YIELD — execute returns control at the breakpoint (no block), frame live.
    state.execute_argv(&name, &p.data, &[]);
    assert!(state.is_paused(), "execute_argv yielded at the breakpoint");
    let frame = state.paused_frame().expect("frame capturable at the pause");
    assert_eq!(frame.function, "compute", "paused in compute: {frame:?}");
    assert!(
        frame.locals.iter().any(|(n, v)| n == "n" && v == "40"),
        "arg n == 40 captured at the pause: {:?}",
        frame.locals
    );

    // 2. RESUME one step — re-enter and execute the `m = n + 2` line; the resumed
    //    run must compute the CORRECT value, observable as `m == 42` in the frame
    //    at the next line (a frame-visible proxy for output — the entry stack is
    //    gone once the whole run completes).
    let still_paused = state.debug_step(StepMode::Into, &p.data);
    assert!(still_paused, "stepped to the next line, still paused");
    let after = state.paused_frame().expect("frame after the resumed step");
    assert!(
        after.locals.iter().any(|(n, v)| n == "m" && v == "42"),
        "the resumed step computed m == 42: {:?}",
        after.locals
    );

    // 3. RESUME to completion — the run finishes cleanly (not re-paused, no fault).
    let done = state.debug_step(StepMode::Continue, &p.data);
    assert!(!done, "resume ran to completion (not re-paused)");
    assert!(!state.is_paused(), "no longer paused after resume");
    assert!(
        state.database.runtime_error.is_none(),
        "resume completed cleanly (no fault)"
    );
}

/// The bytes a per-step heap snapshot would copy — `clone_locked` copies `len * 8` bytes per
/// store, so the whole-heap cost is that summed over every allocation (@PLN63 RX0 sizing).
fn rx0_snapshot_bytes(state: &State) -> usize {
    state
        .database
        .allocations
        .iter()
        .map(|s| s.len() as usize * 8)
        .sum()
}

/// @PLN63 RX0 — the falsification + sizing probe for reverse execution (DAP_ADVANCED § RX).
///
/// (a) FALSIFICATION: reverse execution is ABSENT today.  After a forward step, `debug_undo`
///     (the only reverse API) cannot get back — it reverts an interactive-EDIT journal, which
///     a step leaves empty.  This stays true after RX lands (RX adds a *separate* `step_back`
///     ring; `debug_undo` remains edit-scoped), so the probe is permanent, not throwaway.
/// (b) SIZING: prints the bytes a per-step heap snapshot (the RX ring's checkpoint) would
///     copy — the number that gates whether the full-heap-snapshot ring ships as-is or needs
///     the copy-on-write refinement.  Run with `--nocapture` to read it.
#[test]
fn rx0_reverse_execution_absent_and_snapshot_size() {
    let mut p = repl();
    // A step that MUTATES existing heap in place (`v[0] = 99`) — the case the RX invariant
    // cares about (a store write, not just an allocation).  Pause at line 3, v already built.
    let mut state = run_to_pause(
        &mut p,
        &["fn build() -> integer {\n  v = [10, 20, 30];\n  v[0] = 99;\n  v[0]\n}"],
        "build()",
        "build",
        3,
    );
    let line_before = state.paused_line();
    let heap_before = rx0_snapshot_bytes(&state);

    // Step over the mutating line — it advances and writes the heap.
    assert!(
        state.debug_step(StepMode::Over, &p.data),
        "the step re-pauses on a new line"
    );
    let line_after = state.paused_line();
    let heap_after = rx0_snapshot_bytes(&state);
    assert_ne!(line_before, line_after, "the step advanced the line");

    // (a) Reverse execution is absent: undo after a step is a no-op, and the state stays put.
    assert!(
        !state.debug_undo(),
        "no reverse execution today — undo cannot reverse a step (it is edit-scoped)"
    );
    assert_eq!(
        state.paused_line(),
        line_after,
        "the state did NOT move back — confirming the gap RX fills"
    );

    // (b) The per-step heap-snapshot cost, for the record (read with --nocapture).
    eprintln!(
        "RX0 heap-snapshot bytes: before-step={heap_before}, after-step={heap_after} \
         (allocations={})",
        state.database.allocations.len()
    );
    assert!(
        heap_before > 0 && heap_after > 0,
        "there is heap to snapshot"
    );
    // A tiny program's whole heap is far under a megabyte — the snapshot ring is cheap here;
    // the bound is a canary, not a target (a large-heap program is measured separately).
    assert!(
        heap_after < 4 * 1024 * 1024,
        "the snapshot is bounded: {heap_after} bytes"
    );
}

/// @PLN63 RX1 — the checkpoint primitive: a snapshot captures the heap + execution registers,
/// and a restore makes the state byte-identical to when it was taken.  Mutate a heap value AND
/// registers after the snapshot, restore, and assert BOTH reverted; the checkpoint is reusable.
#[test]
fn rx1_checkpoint_restores_heap_and_registers() {
    let mut p = repl();
    // Pause with the arg `n` and a heap vector `v` both live (line 3, `n + v[0]`).
    let mut state = run_to_pause(
        &mut p,
        &["fn f(n: integer) -> integer {\n  v = [1, 2, 3];\n  n + v[0]\n}"],
        "f(42)",
        "f",
        3,
    );
    let cp = state
        .snapshot_checkpoint()
        .expect("an all-in-memory heap snapshots");
    let code_pos_at_snapshot = state.code_pos;
    let frames_at_snapshot = state.call_stack.len();

    // MUTATE the heap: edit the stack-local `n` (writes the stack store).
    assert!(state.set_frame_value("n", 999, &p.data), "edit n = 999");
    state.refresh_paused_frame(&p.data);
    assert!(
        n_is(&state, "999"),
        "the edit took: {:?}",
        state.paused_frame()
    );
    // MUTATE registers: advance the PC and push a synthetic frame.
    state.code_pos += 8;
    let dup = state.call_stack.last().cloned().expect("a live frame");
    state.call_stack.push(dup);

    // RESTORE — heap + registers revert to the snapshot.
    state.restore_checkpoint(&cp);
    state.refresh_paused_frame(&p.data);
    assert!(
        n_is(&state, "42"),
        "heap restored — n back to 42: {:?}",
        state.paused_frame()
    );
    assert_eq!(state.code_pos, code_pos_at_snapshot, "code_pos restored");
    assert_eq!(
        state.call_stack.len(),
        frames_at_snapshot,
        "call_stack restored"
    );

    // The checkpoint is left intact (copied, not consumed) — a second restore also works.
    state.code_pos += 16;
    state.restore_checkpoint(&cp);
    assert_eq!(
        state.code_pos, code_pos_at_snapshot,
        "checkpoint is reusable"
    );
}

/// Whether the paused frame's local `n` currently renders as `val`.
fn n_is(state: &State, val: &str) -> bool {
    state
        .paused_frame()
        .is_some_and(|f| f.locals.iter().any(|(k, v)| k == "n" && v == val))
}

/// @PLN63 RX2 — `step_back` reverses a forward step: with reverse armed, stepping over a
/// heap-mutating line then stepping back returns to the exact prior state — line + local value
/// restored (byte-level).  This is the RX0 falsification FLIPPED: what `debug_undo` could not
/// do, the snapshot ring does.  A step_back past the floor is a clean no-op.
#[test]
fn rx2_step_back_reverses_a_step() {
    let mut p = repl();
    // `a` is a scalar local mutated by the step; it is read on lines 3 AND 5, so its liveness
    // range spans the mutation and it stays visible at both stops (avoiding the read-dominated
    // under-show).  `b` keeps the frame non-trivial.
    let mut state = run_to_pause(
        &mut p,
        &["fn build() -> integer {\n  a = 5;\n  b = a;\n  a = 99;\n  a + b\n}"],
        "build()",
        "build",
        4, // pause at `a = 99;` — a == 5, not yet mutated
    );
    state.set_reverse(true);
    let read_a = |s: &State| {
        s.paused_frame().and_then(|f| {
            f.locals
                .iter()
                .find(|(k, _)| k == "a")
                .map(|(_, v)| v.clone())
        })
    };
    let line_before = state.paused_line();
    assert_eq!(
        read_a(&state).as_deref(),
        Some("5"),
        "a == 5 before the step"
    );

    // Step over the mutating line → a becomes 99, the line advances.
    assert!(
        state.debug_step(StepMode::Over, &p.data),
        "the step re-pauses"
    );
    assert_ne!(
        state.paused_line(),
        line_before,
        "the step advanced the line"
    );
    assert_eq!(
        read_a(&state).as_deref(),
        Some("99"),
        "the step mutated a to 99"
    );

    // STEP BACK — line + local value return to exactly the pre-step state.
    assert!(state.step_back(&p.data), "step_back succeeds");
    assert_eq!(state.paused_line(), line_before, "the line reverted");
    assert_eq!(
        read_a(&state).as_deref(),
        Some("5"),
        "a reverted to its pre-step value (byte-level heap restore)"
    );

    // The ring floor: nothing earlier retained → a clean no-op, state unchanged.
    assert!(
        !state.step_back(&p.data),
        "no earlier state → clean floor, no corruption"
    );
    assert_eq!(
        state.paused_line(),
        line_before,
        "state unchanged at the floor"
    );
}

/// @PLN63 RX3 — the reverse ring is BOUNDED to its depth: with a cap of 2, three forward steps
/// drop the oldest, so exactly two step_backs succeed and the third hits a clean floor — and
/// the dropped (oldest) step is NOT reachable, proving the cap held (no unbounded growth, no
/// corruption).
#[test]
fn rx3_ring_is_bounded_to_the_depth() {
    let mut p = repl();
    // Each line READS `a`, so none is a dead store elided by codegen — every line is a step.
    let mut state = run_to_pause(
        &mut p,
        &[
            "fn build() -> integer {\n  a = 1;\n  a = a + 1;\n  a = a + 2;\n  a = a + 4;\n  a + 0\n}",
        ],
        "build()",
        "build",
        2, // pause at `a = 1;`
    );
    state.set_reverse(true);
    state.set_reverse_depth(2); // retain only the last 2 steps
    let origin_line = state.paused_line();

    // Three forward steps → the ring keeps only the most recent 2 (the oldest is dropped).
    for _ in 0..3 {
        assert!(
            state.debug_step(StepMode::Over, &p.data),
            "each step re-pauses"
        );
    }

    // Exactly two step_backs succeed (the retained window); the third hits the bounded floor.
    assert!(
        state.step_back(&p.data),
        "step back 1 of 2 (within the cap)"
    );
    assert!(
        state.step_back(&p.data),
        "step back 2 of 2 (within the cap)"
    );
    let floor_line = state.paused_line();
    assert!(
        !state.step_back(&p.data),
        "the 3rd step_back hits the bounded floor — a clean no-op"
    );
    assert_eq!(
        state.paused_line(),
        floor_line,
        "state unchanged at the floor"
    );

    // The oldest step was dropped by the cap, so the origin line is NOT reachable.
    assert_ne!(
        floor_line, origin_line,
        "the dropped step (the origin line) is unreachable — the cap held"
    );
}
