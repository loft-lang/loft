// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN139 stage G — the `double-move` lint.
//!
//! @PLN139 made a copy into a container a MOVE: the container owns the value and its death
//! releases it. That closed loft#849 — and turned a shape that used to LEAK into a double
//! close, because `c = mk(); s1 = S { h: c }; s2 = S { h: c }` now hands one resource to two
//! owners and both release it. Rust prevents this with move checking, which loft does not
//! have, so a diagnostic catches it instead.
//!
//! Every cell asserts TWO things: whether the lint fires, and how many times the value is
//! actually released. That pairing is the point. A verdict-only test cannot tell a correct
//! silence from a missed defect, and it is exactly the silent cells that a future widening of
//! the transfer rule would break — so each of them pins the release count that makes its
//! silence correct. The two cells that release twice WITHOUT a warning (`m8`, `m13`) are the
//! lint's documented blind spot, pinned here so it stays a known boundary rather than drifting
//! into an unnoticed one.
//!
//! Binary-level, because the lint runs post-`scopes::check` from `main` (beside the dead-store
//! lint) and only a real invocation reaches it.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// The shared preamble: a droppable that announces every release, and a container.
const PRELUDE: &str = "\
struct H { id: integer }
fn OpDrop(self: H) { println(\"DROP:{self.id}\"); }
fn mk(id: integer) -> H { return H { id: id }; }
struct S { h: H }
struct Nest { s: S }
";

/// Run one cell and answer `(double_move_warnings, releases)`.
fn cell(name: &str, body: &str) -> (usize, usize) {
    let src = format!("{PRELUDE}\nfn main() {{ {body} }}\n");
    let path = std::env::temp_dir().join(format!("loft_pln139_dm_{name}.loft"));
    std::fs::write(&path, &src).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env_remove("LOFT_NO_DOUBLE_MOVE")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    (
        stderr.matches("double-move").count(),
        stdout.matches("DROP:").count(),
    )
}

/// Assert a cell's verdict and its release count together.
#[track_caller]
fn check(name: &str, body: &str, warnings: usize, releases: usize) {
    let (w, r) = cell(name, body);
    assert_eq!(w, warnings, "{name}: double-move warnings — body: {body}");
    assert_eq!(r, releases, "{name}: releases — body: {body}");
}

// ── the lint FIRES: both hand-offs certainly run ─────────────────────────────

/// Two straight-line hand-offs of one local. The headline shape, and the one @PLN139's
/// cascade converted from a leak into a double close.
#[test]
fn m1_two_fields_from_one_local() {
    check(
        "m1",
        "c = mk(1); s1 = S { h: c }; s2 = S { h: c }; println(\"{s1.h.id}{s2.h.id}\");",
        1,
        2,
    );
}

/// A collection element is an owner on the same terms as a field, so the same local
/// appearing twice in a vector literal is the same defect.
#[test]
fn m6_same_local_into_two_elements() {
    check(
        "m6",
        "c = mk(6); v: vector<H> = [c, c]; println(\"{len(v)}\");",
        1,
        2,
    );
}

/// The two owner KINDS mixed: a field takes it, then an element does. The lint counts
/// hand-offs, not shapes, so this must read the same as two fields.
#[test]
fn m7_field_then_element() {
    check(
        "m7",
        "c = mk(7); s1 = S { h: c }; v: vector<H> = [c]; println(\"{s1.h.id}{len(v)}\");",
        1,
        2,
    );
}

/// Inside ONE arm both hand-offs run whenever the arm does, so an arm is a straight line
/// like any other — this is the cell that keeps the branch rule from over-suppressing.
#[test]
fn m10_two_handoffs_inside_one_arm() {
    check(
        "m10",
        "c = mk(10); p = true; \
         if p { s1 = S { h: c }; s2 = S { h: c }; println(\"{s1.h.id}{s2.h.id}\"); }",
        1,
        2,
    );
}

// ── the lint is SILENT, and the release count proves it should be ────────────

/// One owner. The control every firing cell is read against.
#[test]
fn m2_single_handoff() {
    check(
        "m2",
        "c = mk(2); s1 = S { h: c }; println(\"{s1.h.id}\");",
        0,
        1,
    );
}

/// Opposite arms: whichever way the branch goes the value reaches exactly one owner, so
/// warning here would fail correct code — and a `warning` gates a library's CI.
#[test]
fn m3_opposite_arms() {
    check(
        "m3",
        "c = mk(3); p = true; \
         if p { s1 = S { h: c }; println(\"{s1.h.id}\"); } \
         else { s2 = S { h: c }; println(\"{s2.h.id}\"); }",
        0,
        1,
    );
}

/// Reassigned between the hand-offs, so the two containers hold two DISTINCT resources —
/// two releases of two values, which is correct.
#[test]
fn m4_reassigned_between_handoffs() {
    check(
        "m4",
        "c = mk(4); s1 = S { h: c }; c = mk(40); s2 = S { h: c }; \
         println(\"{s1.h.id}{s2.h.id}\");",
        0,
        2,
    );
}

/// Two sources, one hand-off each — a count kept per variable, not per container.
#[test]
fn m5_two_distinct_sources() {
    check(
        "m5",
        "a = mk(5); b = mk(50); s1 = S { h: a }; s2 = S { h: b }; \
         println(\"{s1.h.id}{s2.h.id}\");",
        0,
        2,
    );
}

/// No droppable anywhere: the transfer predicate asks whether an owner will RELEASE the
/// value, and nothing here does.
#[test]
fn m9_no_droppable_control() {
    check(
        "m9",
        "n = 9; v: vector<integer> = [n, n]; println(\"{len(v)}\");",
        0,
        0,
    );
}

/// Two inline temporaries are two values. Only a variable the author named can be counted,
/// and there is none here.
#[test]
fn m11_distinct_inline_temps() {
    check(
        "m11",
        "s1 = S { h: mk(11) }; s2 = S { h: mk(110) }; println(\"{s1.h.id}{s2.h.id}\");",
        0,
        2,
    );
}

/// Nesting is still ONE owner — the outer container's cascade reaches the inner one, so a
/// chain of containers must not read as a chain of owners.
#[test]
fn m12_nested_container_is_one_owner() {
    check(
        "m12",
        "c = mk(12); n = Nest { s: S { h: c } }; println(\"{n.s.h.id}\");",
        0,
        1,
    );
}

/// A conditional reassignment retires the pending hand-off: on the path that reassigns, the
/// second container takes a different value. Silent is the sound answer — `may` is not
/// `must`, and this tier gates.
#[test]
fn m14_conditional_reassignment_kills_the_pair() {
    check(
        "m14",
        "c = mk(14); s1 = S { h: c }; p = true; if p { c = mk(140); } \
         s2 = S { h: c }; println(\"{s1.h.id}{s2.h.id}\");",
        0,
        2,
    );
}

// ── the documented blind spot: released twice, no warning ────────────────────

/// A loop body is ONE static hand-off that runs N times. Seeing this needs the iteration
/// count, so it is a false NEGATIVE — the safe direction for a tier that gates, and pinned
/// here so it stays a known boundary.
#[test]
fn m8_loop_iteration_is_invisible() {
    check(
        "m8",
        "c = mk(8); for i in 0..2 { s = S { h: c }; println(\"{s.h.id}\"); }",
        0,
        2,
    );
}

/// A second hand-off inside an `if` releases twice only when the branch is taken. Warning
/// would fail the program that does not take it, so this is silent by the same rule as
/// `m3` — and is the other half of the blind spot.
#[test]
fn m13_conditional_second_handoff() {
    check(
        "m13",
        "c = mk(13); s1 = S { h: c }; p = true; \
         if p { s2 = S { h: c }; println(\"{s2.h.id}\"); } println(\"{s1.h.id}\");",
        0,
        2,
    );
}

// ── a MEMBER copied into another container (heap.md D-heap-7 family 4) ────────
//
// The source container keeps owning the member and its cascade releases it, while the
// destination releases its copy — so the FIRST copy is already the double.  The owner's call
// (2026-09-15) is a warning, not runtime bookkeeping: `OpDrop` is meant for a clear lifetime,
// and this shape is the author's to restructure.  Releases are pinned beside the verdict as
// everywhere in this file: the lint reports, it never changes what the program does.

/// Run one program with its own declarations and answer `(double_move_warnings, releases)`.
fn cell_prog(name: &str, decls: &str, body: &str) -> (usize, usize) {
    let src = format!("{PRELUDE}\n{decls}\nfn main() {{ {body} }}\n");
    let path = std::env::temp_dir().join(format!("loft_pln139_dm_{name}.loft"));
    std::fs::write(&path, &src).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env_remove("LOFT_NO_DOUBLE_MOVE")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    (
        stderr.matches("double-move").count(),
        stdout.matches("DROP:").count(),
    )
}

#[track_caller]
fn check_prog(name: &str, decls: &str, body: &str, warnings: usize, releases: usize) {
    let (w, r) = cell_prog(name, decls, body);
    assert_eq!(w, warnings, "{name}: double-move warnings — body: {body}");
    assert_eq!(r, releases, "{name}: releases — body: {body}");
}

const HOLD: &str = "struct Hold { h: H }";

/// A field into another struct.
#[test]
fn p1_field_into_a_field() {
    check_prog(
        "p1",
        HOLD,
        "s = S { h: mk(1) }; c = Hold { h: s.h }; println(\"{c.h.id}\");",
        1,
        2,
    );
}

/// A field into an enum payload.
#[test]
fn p2_field_into_an_enum_payload() {
    check_prog(
        "p2",
        "enum W { WH { h: H }, WNone }",
        "s = S { h: mk(2) }; w: W = WH { h: s.h }; \
         match w { WH { h } => println(\"{h.id}\"), WNone => {} }",
        1,
        2,
    );
}

/// A field appended to a vector.
#[test]
fn p3_field_appended() {
    check_prog(
        "p3",
        "",
        "s = S { h: mk(3) }; v: vector<H> = []; v += [s.h]; println(\"{v[0].id}\");",
        1,
        2,
    );
}

/// A field in a vector literal.
#[test]
fn p4_field_in_a_vector_literal() {
    check_prog(
        "p4",
        "",
        "s = S { h: mk(4) }; v: vector<H> = [s.h]; println(\"{v[0].id}\");",
        1,
        2,
    );
}

/// An element into a struct.
#[test]
fn p5_element_into_a_field() {
    check_prog(
        "p5",
        HOLD,
        "vs: vector<H> = [mk(5)]; c = Hold { h: vs[0] }; println(\"{c.h.id}\");",
        1,
        2,
    );
}

/// A tuple member into a struct.
#[test]
fn p6_tuple_member_into_a_field() {
    check_prog(
        "p6",
        HOLD,
        "tt = (mk(6), 1); c = Hold { h: tt.0 }; println(\"{c.h.id}\");",
        1,
        2,
    );
}

/// A nested field into a struct: the root is the outer container.
#[test]
fn p7_nested_field_into_a_field() {
    check_prog(
        "p7",
        HOLD,
        "n = Nest { s: S { h: mk(7) } }; c = Hold { h: n.s.h }; println(\"{c.h.id}\");",
        1,
        2,
    );
}

/// The source REBUILT after the copy: the rebind releases the member the container also holds.
#[test]
fn p8_source_rebuilt_after_the_copy() {
    check_prog(
        "p8",
        HOLD,
        "s = S { h: mk(8) }; c = Hold { h: s.h }; println(\"{c.h.id}\"); \
         s = S { h: mk(80) }; println(\"{s.h.id}\");",
        1,
        3,
    );
}

/// The copy inside an arm: certain on the path that runs it.
#[test]
fn p9_copy_inside_an_arm() {
    check_prog(
        "p9",
        HOLD,
        "s = S { h: mk(9) }; p = true; if p { c = Hold { h: s.h }; println(\"{c.h.id}\"); }",
        1,
        2,
    );
}

/// The copy inside a loop: one site, one warning, released once per copy and once by `s`.
#[test]
fn p10_copy_inside_a_loop() {
    check_prog(
        "p10",
        "",
        "s = S { h: mk(10) }; v: vector<H> = []; for _i in 0..2 { v += [s.h]; } \
         println(\"{len(v)}\");",
        1,
        3,
    );
}

/// The root RETURNED after the copy: it goes on owning the member in the caller.
#[test]
fn p11_root_returned_after_the_copy() {
    check_prog(
        "p11",
        "struct Hold { h: H }\n\
         fn g() -> S { s = S { h: mk(11) }; c = Hold { h: s.h }; println(\"{c.h.id}\"); return s; }",
        "t = g(); println(\"{t.h.id}\");",
        1,
        2,
    );
}

/// SILENT — a tuple literal of a member is not a copy: released once.
#[test]
fn q1_tuple_literal_is_not_a_copy() {
    check_prog(
        "q1",
        "",
        "s = S { h: mk(21) }; t = (s.h, 1); println(\"{t.0.id}\");",
        0,
        1,
    );
}

/// SILENT — a member bound to a local is a view: released once.
#[test]
fn q2_member_bound_to_a_local() {
    check_prog(
        "q2",
        "",
        "s = S { h: mk(22) }; x = s.h; println(\"{x.id}\");",
        0,
        1,
    );
}

/// SILENT — the member OVERWRITTEN after the copy: `s` releases the new value, the container
/// the copied one, each once (`(H-Drop-Not)`).
#[test]
fn q3_member_overwritten_after_the_copy() {
    check_prog(
        "q3",
        HOLD,
        "s = S { h: mk(23) }; c = Hold { h: s.h }; s.h = mk(24); \
         println(\"{c.h.id}{s.h.id}\");",
        0,
        2,
    );
}

/// SILENT — the copy in an arm, the member overwritten after it.
#[test]
fn q4_member_overwritten_after_an_arm() {
    check_prog(
        "q4",
        HOLD,
        "s = S { h: mk(25) }; p = true; if p { c = Hold { h: s.h }; println(\"{c.h.id}\"); } \
         s.h = mk(26); println(\"{s.h.id}\");",
        0,
        2,
    );
}

/// SILENT — the member overwritten inside an arm: single on the path that takes it, double on
/// the other.  `may` is not `must`, and this tier gates.
#[test]
fn q5_member_overwritten_inside_an_arm() {
    check_prog(
        "q5",
        HOLD,
        "s = S { h: mk(27) }; c = Hold { h: s.h }; p = true; if p { s.h = mk(28); } \
         println(\"{c.h.id}{s.h.id}\");",
        0,
        2,
    );
}

/// BLIND SPOT, pinned — a PARAMETER's member: the caller owns it (heap.md family 2), outside
/// this warning.  Released twice.
#[test]
fn z1_parameter_member_is_outside() {
    check_prog(
        "z1",
        "struct Hold { h: H }\nfn g(q: S) { c = Hold { h: q.h }; println(\"{c.h.id}\"); }",
        "s = S { h: mk(31) }; g(s); println(\"{s.h.id}\");",
        0,
        2,
    );
}

/// BLIND SPOT, pinned — a LOOP VARIABLE is a plain variable in the IR, not a projection
/// spelling.  Released twice.
#[test]
fn z2_loop_variable_is_outside() {
    check_prog(
        "z2",
        HOLD,
        "vs: vector<H> = [mk(32)]; for e in vs { c = Hold { h: e }; println(\"{c.h.id}\"); }",
        0,
        2,
    );
}

/// BLIND SPOT, pinned — a `match` payload binding, likewise a plain variable.  Released twice.
#[test]
fn z3_match_payload_is_outside() {
    check_prog(
        "z3",
        "struct Hold { h: H }\nenum W { WH { h: H }, WNone }",
        "w: W = WH { h: mk(33) }; \
         match w { WH { h } => { c = Hold { h: h }; println(\"{c.h.id}\"); }, WNone => {} }",
        0,
        2,
    );
}

/// The source SELF-ASSIGNED after the copy: a no-op, so the member is still `s`'s at scope end.
#[test]
fn p12_self_assignment_keeps_the_copy_pending() {
    check_prog(
        "p12",
        HOLD,
        "s = S { h: mk(12) }; c = Hold { h: s.h }; s = s; println(\"{c.h.id}{s.h.id}\");",
        1,
        2,
    );
}

/// A VIEW root: `x` views `n`'s member record, and `n`'s cascade releases what the container
/// also holds.
#[test]
fn p13_member_of_a_view_root() {
    check_prog(
        "p13",
        HOLD,
        "n = Nest { s: S { h: mk(13) } }; x = n.s; c = Hold { h: x.h }; println(\"{c.h.id}\");",
        1,
        2,
    );
}

/// BLIND SPOT, pinned — the source rebound to a value that READS it: its displaced release is
/// decided by store identity at run time, so the copy retires.  Released three times here, the
/// third from `keep` handing the caller back a second record (heap.md family 2's return shape).
#[test]
fn z4_rebind_that_reads_the_source_is_outside() {
    check_prog(
        "z4",
        "struct Hold { h: H }\nfn keep(q: S) -> S { return q; }",
        "s = S { h: mk(34) }; c = Hold { h: s.h }; s = keep(s); println(\"{c.h.id}{s.h.id}\");",
        0,
        3,
    );
}

/// BLIND SPOT, pinned — a tuple MEMBER copied into a tuple literal lands in the new tuple's
/// backing work-ref, not a container place.  Released twice.  (A FIELD in a tuple literal is not
/// a copy at all — `q1`.)
#[test]
fn z5_tuple_member_into_a_tuple_is_outside() {
    check_prog(
        "z5",
        "",
        "tt = (mk(35), 1); t2 = (tt.0, 2); println(\"{t2.0.id}\");",
        0,
        2,
    );
}

// ── the opt-out ──────────────────────────────────────────────────────────────

/// `LOFT_NO_DOUBLE_MOVE` silences it, and silencing the diagnostic changes nothing about
/// what the program does — the lint reports, it never rewrites.
#[test]
fn opt_out_silences_without_changing_behaviour() {
    let src = format!(
        "{PRELUDE}\nfn main() {{ c = mk(1); s1 = S {{ h: c }}; s2 = S {{ h: c }}; \
         println(\"{{s1.h.id}}{{s2.h.id}}\"); }}\n"
    );
    let path = std::env::temp_dir().join("loft_pln139_dm_optout.loft");
    std::fs::write(&path, &src).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_NO_DOUBLE_MOVE", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stderr.contains("double-move"),
        "LOFT_NO_DOUBLE_MOVE must silence the lint, got: {stderr}"
    );
    assert_eq!(
        stdout.matches("DROP:").count(),
        2,
        "the lint reports; it must not change what the program does"
    );
}
