// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Call reachability, and an arm after a total `_` naming the rule it breaks.

use super::*;

// ── loft#815 ─────────────────────────────────────────────────────────────────
// Native output emits only the functions `generation::reachable_functions`
// marks, and the three walkers feeding it each re-derived `Value`'s tree shape
// as a whitelist ending in `_ => {}`.  `Tuple` was absent from all three, so a
// callee reached ONLY from a tuple element was pruned while its call site was
// still emitted — rustc then failed E0425 on the emitted call and the whole
// library refused to build (`hex_way`'s `(0.0 - sin(a) * dir, cos(a) * dir)`
// took down every program in its dependency cone).
//
// The walkers now delegate recursion to `IrNode::for_each_child` /
// `Value::for_each_child`, so the walk is total by construction.

// A struct that contains ITSELF through a non-null field is reported as such, in a program
// that also CONSTRUCTS it — the diagnostic names the field and the cure (`τ?`,
// `@FR-T-Recursive`).  The report must not depend on whether the type is used: a gate on
// `def_referenced` once silenced it for every cyclic type a real program writes (loft#929).
#[test]
fn a_struct_containing_itself_by_value_says_so() {
    code!(
        "struct PENode { val: integer, next: PENode }
fn test() {
    _n = PENode { val: 1 };
}"
    )
    .error(
        "`PENode.next: PENode` makes every PENode contain another PENode — make it `PENode?` \
at a_struct_containing_itself_by_value_says_so:1:16",
    );
}

// The other direction of the same rule: `reference<T>` is the documented cure, so a
// self-reference THROUGH it stays legal.  Removing the `def_referenced` gate must not
// start reporting the shape the diagnostic recommends.
#[test]
fn a_reference_self_field_is_not_a_cycle() {
    code!(
        "struct RefNode { val: integer, next: reference<RefNode> }
fn test() {
    n = RefNode { val: 7, next: null };
    assert(n.val == 7, \"reference<Self> field is legal\");
}"
    )
    // `@FR-D-NoRef` — the bare pointer has no default, so its null is written out (loft#1931).
    .warning(
        "`null` is stored into the field of the non-null type `reference<RefNode>` — the slot \
         holds null; declare it `reference<RefNode>?` to make that explicit at \
         a_reference_self_field_is_not_a_cycle:3:33",
    )
    .result(Value::Null);
}

// ── An arm after a total `_` names the rule it breaks ────────────────────
//
// A total `_` matches everything, so an arm written after it can never be selected. Both match
// paths — the enum one in `parse_match` and the scalar one in `parse_scalar_match` — used to
// `break` out of the arm loop at that point and let the next arm meet the closing-brace
// expectation, which reported `Expect token }`: the right caret with the wrong reason, and on
// the scalar path it cascaded into four more errors about the rest of the line, none of which
// mentioned the wildcard. The Match chapter states this rule as "put it last", so the compiler
// is what a reader meets when they get it wrong.
#[test]
fn an_arm_after_a_total_wildcard_says_so_on_the_scalar_path() {
    code!(
        "fn test() { r = match 2 { 1 => \"one\", _ => \"other\", 2 => \"two\" }; print(\"{r}\"); }"
    )
    .error(
        "a `_` arm matches everything, so this arm can never be selected — move `_` to the end \
at an_arm_after_a_total_wildcard_says_so_on_the_scalar_path:1:54",
    );
}

#[test]
fn an_arm_after_a_total_wildcard_says_so_on_the_enum_path() {
    code!(
        "enum D { North, South, East }
fn test() { r = match D.South { North => \"n\", _ => \"other\", South => \"s\" }; print(\"{r}\"); }"
    )
    .error(
        "a `_` arm matches everything, so this arm can never be selected — move `_` to the end \
at an_arm_after_a_total_wildcard_says_so_on_the_enum_path:2:66",
    );
}

/// The carve-out that keeps the check above from being wrong: a GUARDED `_ if cond` is NOT
/// total — the guard can reject — so arms are expected to follow it and must stay legal. This
/// is the same distinction `(M-Total)` draws for exhaustiveness, asked at the parse site.
#[test]
fn a_guarded_wildcard_still_admits_the_arms_after_it() {
    code!(
        "fn test() {
  r = match 7 { _ if 7 < 0 => \"neg\", _ if 7 > 100 => \"big\", 7 => \"seven\", _ => \"other\" };
  assert(r == \"seven\", \"a guarded _ does not close the arm list: {r}\");
}"
    );
}

/// loft#1528 — `τ? ⤳ σ?` is `τ ⤳ σ` inside the wrapper.  A `Fireball?` handed to an `Entity?`
/// parameter, local or field, and an `i8?` handed to an `i16?` parameter, compile with NO
/// diagnostic — `code!` fails on any it was not told to expect, which is what makes this the
/// absence pin a `.loft` `@EXPECT_WARNING` cannot be — and the values arrive.
#[test]
fn a_nullable_variant_into_its_enums_nullable_slot_reports_nothing() {
    code!(
        "enum Entity { Fireball { n: integer }, IceWall { n: integer } }
struct Holder { e: Entity? }
fn tag(e: Entity?) -> text { if e { \"any {e.n}\" } else { \"absent\" } }
fn wider(v: i16?) -> text { if v { \"v {v}\" } else { \"v-null\" } }
fn probe() -> text {
  p: Fireball? = Fireball { n: 4 };
  z: Fireball? = null;
  l: Entity? = p;
  h = Holder { e: p };
  s: i8? = 3;
  \"{tag(p)}|{tag(z)}|{tag(l)}|{tag(h.e)}|{wider(s)}\"
}"
    )
    .expr("probe()")
    .result(Value::str("any 4|absent|any 4|any 4|v 3"));
}

/// loft#1531 — a `&`-source tuple local's record takes the LOCAL's member types only where the
/// literal's members convert into them.  A member the declaration does not accept stays the
/// bind's refusal: the record is never laid out for a type its value does not have.
#[test]
fn a_reference_source_tuple_whose_member_the_declaration_refuses_is_still_refused() {
    code!(
        "enum Entity { Fireball { n: integer }, IceWall { n: integer } }
fn bump(t: &(integer, Entity)) { t.0 = t.0 + 1; }
fn test() { w: (integer, Entity) = (1, \"x\"); bump(w); print(\"{w.0}\"); }"
    )
    .error(
        "Variable 'w' cannot change type from (integer, Entity) to (integer, text); use a new \
variable name or cast with 'as' at a_reference_source_tuple_whose_member_the_declaration_refuses_is_still_refused:3:45",
    );
}

/// An element of an unresolved call's result reads as `never`, and a store into it is not a
/// second mistake: the program reports the unknown function once, with its cure, and no
/// `Cannot assign float to a field of type never — use 'as never'` beside it.
#[test]
fn a_store_into_an_unresolved_calls_element_reports_only_the_call() {
    code!(
        "fn run() -> float {
    v = nope_rest(3);
    v[0] = 2.5;
    v[0]
}"
    )
    .error("Unknown function nope_rest at a_store_into_an_unresolved_calls_element_reports_only_the_call:2:9");
}

/// A misspelled type used both in a construction and in an annotation is ONE mistake.  The
/// construction's speculative stub is named by the annotation too, and the annotation's
/// report was skipped as if only the construction had named it — which left `d` unknown and
/// cascaded into `Unknown variable 'd'`, `Unknown in expression type never` and `cannot
/// iterate over never`.  The multi-file form, where the same gap was an internal compiler
/// error: `imports::an_unimported_type_both_constructed_and_annotated_names_the_import_cure`.
#[test]
fn a_type_both_constructed_and_annotated_is_reported_once() {
    code!(
        "fn nb(h: integer, k: integer) -> integer { h + k }
fn a(q: integer) -> integer {
    x = nb(Hexq { q: q, r: 0 }, 1);
    x
}
fn g() -> integer {
    d: vector<Hexq> = [];
    n = 0;
    for h in d { n += h.q; }
    n
}"
    )
    .error("Undefined type Hexq — did you mean 'text'? at a_type_both_constructed_and_annotated_is_reported_once:3:12");
}

/// A tuple return of three or more members whose member names an undefined type is one
/// mistake, reported once — never an internal compiler error.  Such a tuple has no record
/// behind it (its reference is `u32::MAX`), and the tail's value hint read `def(u32::MAX)`.
/// Two members took another path and always reported; 2026.9.0 crashed on three.
#[test]
fn a_tuple_return_naming_an_undefined_type_is_reported_not_a_crash() {
    code!("fn f(k: integer) -> (integer, text, Hexz) { (k, \"a\", Hexz { q: 1 }) }")
        .error("Undefined type Hexz — did you mean 'text'? at a_tuple_return_naming_an_undefined_type_is_reported_not_a_crash:1:42");
}

/// A tuple RETURN naming a struct declared LATER in the file is refused with its cure, and
/// one naming an UNDEFINED type is reported as undefined — neither is an internal compiler
/// error.  Boxing the heap-carrying return asked `tuple_def` for a record it cannot build
/// while a member is unresolved; its `u32::MAX` became the return type, the between-passes
/// check never saw a tuple to judge, and the first read of the result dereferenced no
/// definition (2026.9.0 crashed on both, and on dryopea's `r3_run` under C98).
#[test]
fn a_boxed_tuple_return_with_a_later_member_is_refused_with_its_cure() {
    code!(
        "fn f(k: integer) -> (integer, text, Qz) { (k, \"a\", Qz { q: k * 2 }) }
fn g() -> integer { t = f(3); t.0 }
struct Qz { q: integer }"
    )
    .error(
        "`f` returns a tuple containing `Qz`, which is declared later in the file — move the \
declaration of `Qz` above `f`. A tuple RETURN has to know at its declaration whether it carries \
heap storage, and it cannot ask a type that does not exist yet; every other use of a \
forward-declared type in a tuple (a local, a vector element, a struct field, a parameter) works \
at a_boxed_tuple_return_with_a_later_member_is_refused_with_its_cure:1:20",
    );
}

#[test]
fn a_boxed_tuple_return_with_an_undefined_member_is_reported_when_bound() {
    code!("fn f(k: integer) -> (integer, text, Hexz) { (k, \"a\", Hexz { q: 1 }) }
fn g() -> integer { t = f(3); t.0 }")
    .error("Undefined type Hexz — did you mean 'text'? at a_boxed_tuple_return_with_an_undefined_member_is_reported_when_bound:1:42");
}

/// Walking a vector literal of an UNDEFINED struct reports the undefined name, and stops at
/// the record that cannot be built — never an internal compiler error from the `for`.
#[test]
fn a_for_over_a_literal_of_an_undefined_struct_reports_the_name() {
    code!("fn g() -> integer {
    hs = [Hexz { q: 3 }, Hexz { q: 4 }];
    n = 0;
    for h in hs { n += h.q; }
    n
}")
    .error("unknown type 'Hexz' — did you mean 'text'? at a_for_over_a_literal_of_an_undefined_struct_reports_the_name:2:11")
    .error("unknown type 'Hexz' — did you mean 'text'? at a_for_over_a_literal_of_an_undefined_struct_reports_the_name:2:26")
    .fatal("cannot build this record — its type never resolved at a_for_over_a_literal_of_an_undefined_struct_reports_the_name:2:40");
}

/// `@FR-Const-Foreign` (loft#1897, @C139) — a binding of a `-> const T` producer's value is
/// value-const, so a write through it is refused before the program runs, with the copy cure.
#[test]
fn a_const_returning_function_makes_its_binding_read_only() {
    code!(
        "fn mk() -> const vector<u8> { [1, 2] }
fn test() { v = mk(); v[0] = 7; }"
    )
    .error(
        "Cannot modify 'v': it holds read-only data the program does not own (a mapped file or a \
library's buffer) — copy it first (`w = v`, a bind copies) and write the copy at \
a_const_returning_function_makes_its_binding_read_only:2:32",
    );
}

/// `@FR-Const-Foreign` — foreign data bound into a WRITABLE variable would present it as
/// writable; refused, with the cure of a name of its own.
#[test]
fn a_foreign_value_bound_into_a_writable_variable_is_refused() {
    code!(
        "fn mk() -> const vector<u8> { [1, 2] }
fn test() { v: vector<u8> = [3]; v[0] = 4; v = mk(); }"
    )
    .error(
        "'v' is writable, and this value is read-only data the program does not own (a mapped \
file or a library's buffer) — bind it to a name of its own (`m = file_map(p)`), and copy that \
where a writable value is needed (`v = m`) at \
a_foreign_value_bound_into_a_writable_variable_is_refused:2:53",
    );
}

/// `@FR-Const-Foreign` — a function returning foreign data declares it `-> const T`.
#[test]
fn a_function_returning_foreign_data_must_say_const() {
    code!(
        "fn mk() -> const vector<u8> { [1, 2] }
fn wrap() -> vector<u8> { mk() }
fn test() { w = wrap(); }"
    )
    .error(
        "'wrap' returns read-only data the program does not own (a mapped file or a library's \
buffer) as a writable `vector<u8>` — declare it `-> const vector<u8>`, or return a copy at \
a_function_returning_foreign_data_must_say_const:2:33",
    )
    .warning("Variable w is never read at a_function_returning_foreign_data_must_say_const:3:16");
}

/// loft#2536 — a function that rebinds its tail local from a call READING it
/// (`cv = grow(cv, 2); cv`) answers the call's result.  The return-buffer handover gave the
/// callee `cv` as its buffer, so it built into the record it read: `1 2 1 0`, not `5 4 1 4`.
#[test]
fn a_call_reading_the_local_it_rebinds_answers_its_result() {
    code!(
        "struct Canvas { data: vector<integer>, w: integer }
fn grow(c: const Canvas, extra: integer) -> Canvas {
  g = Canvas { data: [], w: c.w + 1 };
  for x in c.data { g.data += [x]; }
  for _k in 0..extra { g.data += [c.w]; }
  g
}
fn mk(t: integer) -> Canvas { Canvas { data: [t, t], w: t + 3 } }
fn one(t: integer) -> Canvas { cv = mk(t); cv = grow(cv, 2); cv }
fn two(t: integer) -> Canvas {
  cv = mk(t);
  cv = grow(cv, 2);
  cv
}
fn sig(c: Canvas) -> integer { c.w * 1000 + len(c.data) * 100 + (c.data[0] ?? 9) * 10 + (c.data[3] ?? 9) }
fn run() -> integer { sig(one(1)) * 10000 + sig(two(1)) }"
    )
    .expr("run()")
    .result(Value::Long(54_145_414));
}

/// loft#2544 — an `is` test of a unit variant whose enum is declared further down, followed by
/// a one-name body and `else`, is the body, not a capture list: only a body stands before an
/// `else`.  Pass 1 cannot see the enum, read `{ b }` as a capture list by its shape and
/// refused the program with `Expect token {`.
#[test]
fn a_forward_unit_variant_test_reads_a_one_name_body_before_else() {
    code!(
        "fn pick(a: Later, b: integer, c: integer) -> integer {
  v = if a.k is KA { b } else { c };
  v
}
fn chain(a: Later, b: integer, c: integer) -> integer {
  if a.k is KA { b } else if a.k is KB { c } else { 9 }
}
enum K { KA, KB, KC }
struct Later { k: K }
fn run() -> integer {
  pick(Later { k: KA }, 1, 2) * 10000 + pick(Later { k: KB }, 1, 2) * 1000
    + chain(Later { k: KA }, 3, 4) * 100 + chain(Later { k: KB }, 3, 4) * 10 + chain(Later { k: KC }, 3, 4)
}"
    )
    .expr("run()")
    .result(Value::Long(12_349));
}

/// `@FR-Const-Foreign` — asking whether a body's tail is foreign data follows each local's
/// assignments once.  Locals assigned from each other form a cycle (`y = n; x = y; y = x; y`),
/// and following it again and again overflowed the compiler's stack: a crash on a valid program.
#[test]
fn locals_assigned_from_each_other_are_followed_once() {
    code!(
        "fn two(n: integer) -> integer { y = n; x = y; y = x; y }
fn three(n: integer) -> integer { y = n; x = y; z = x; y = z; y }
fn run() -> integer { two(3) * 10 + three(7) }"
    )
    .expr("run()")
    .result(Value::Int(37));
}

/// `@FR-Const-Foreign` — followed once, a local still brings every assignment it has: foreign
/// data reaching the tail only through the OTHER local of a cycle is still refused.
#[test]
fn foreign_data_through_a_cycle_of_locals_is_still_refused() {
    code!(
        "fn h(n: const integer) -> const integer { n }
fn g(n: integer) -> integer { y = n; x = h(n); y = x; x = y; y }"
    )
    .warning(
        "Parameter 'n' is const but is never modified; 'const' has no effect on an unmodified \
primitive parameter at foreign_data_through_a_cycle_of_locals_is_still_refused:1:9",
    )
    .warning("Dead assignment — 'y' is overwritten before being read at foreign_data_through_a_cycle_of_locals_is_still_refused:2:37")
    .error(
        "Cannot modify 'x': it holds read-only data the program does not own (a mapped file or a \
library's buffer) — copy it first (`w = x`, a bind copies) and write the copy at foreign_data_through_a_cycle_of_locals_is_still_refused:2:43",
    )
    .error(
        "Cannot modify 'x': it holds read-only data the program does not own (a mapped file or a \
library's buffer) — copy it first (`w = x`, a bind copies) and write the copy at foreign_data_through_a_cycle_of_locals_is_still_refused:2:60",
    )
    // the tail `y` reaches `h(n)` only through `x`, across the cycle
    .error(
        "'g' returns read-only data the program does not own (a mapped file or a library's \
buffer) as a writable `integer` — declare it `-> const integer`, or return a copy at foreign_data_through_a_cycle_of_locals_is_still_refused:2:65",
    );
}

/// A failed call leaves the destination's own name as the value (`y = y`), the shape of a
/// cycle: an unknown function whose answer is the tail is reported, not a crash.
#[test]
fn an_unknown_function_bound_and_answered_is_reported() {
    code!("fn pp() -> integer { y = swap(pr(3.0)); y }")
        .error("Unknown function pr at an_unknown_function_bound_and_answered_is_reported:1:31");
}

/// `@FR-Const-Foreign` — the cure: a bind copies (`w = v`) into a writable value, and a `const`
/// parameter reads the original; the original is untouched.
#[test]
fn a_copy_of_foreign_data_is_writable() {
    code!(
        "fn mk() -> const vector<u8> { [1, 2] }
fn rd(p: const vector<u8>) -> integer { p[1] }
fn run() -> integer { v = mk(); w = v; w[0] = 9; w += [5]; v[0] * 1000 + w[0] * 100 + len(w) * 10 + rd(v) }"
    )
    .expr("run()")
    .result(Value::Long(1932));
}

/// A body local two loops bind at types that differ only in absence is two bindings
/// (`@FR-B-Scope`, `@FR-N-Join`), and the second is defined where it is bound.  Pass 1 cannot
/// see the difference — a field's value is `float?` only once the struct is laid out — so the
/// split is first met on pass 2, and the `=` there defined the ENDED binding: every read of
/// the new one was "Unknown variable 'l#b1'".  By hand: the first loop adds 1 and 3; the
/// second, the root of (2 - 0)², a hundredfold: 4 + 200.
///
/// @falsified-at ef39df483 + this change: without the `defined` at the split, the test's
/// program does not compile.
#[test]
fn a_loop_local_split_on_absence_alone_is_defined_where_it_is_bound() {
    code!(
        "struct SlW { p: float, q: float }
fn sl_faces(xs: const vector<float>, inner: const vector<SlW>) -> float {
    t = 0.0;
    for i in 0..len(xs) {
        bx = xs[i] ?? 0.0;
        l = sqrt(bx * bx);
        t += l;
    }
    for iw in inner {
        l = sqrt((iw.q - iw.p) * (iw.q - iw.p));
        t += (l ?? 0.0) * 100.0;
    }
    t
}
fn test() {
    assert(sl_faces([1.0, 3.0], [SlW { p: 0.0, q: 2.0 }]) == 204.0, \"4 + 200\");
}"
    )
    .result(Value::Null);
}

/// loft#2509 — the same split from the other side: a local bound in an inner loop, and bound
/// again after it from a value pass 1 types differently (`sqrt` of a local the inner loop
/// fed).  The bind after the inner block is a binding of its own (`@FR-B-Scope`), first split
/// on pass 2, and it is defined there.  By hand: `near` is 1.0 after the inner loop, twice.
///
/// @falsified-at 6ae07aa59: without the `defined` at the split the program is refused with
/// "Unknown variable 'd#b0'" (measured on main @ 39701d652).
#[test]
fn a_local_rebound_after_its_inner_loop_is_defined_where_it_is_bound() {
    code!(
        "fn rb_sum() -> float {
    t = 0.0;
    for r in 0..2 {
        near = 9.0;
        for f in [1.0] { d = f; near = d; }
        d = sqrt(near);
        t += (d ?? 0.0) + (r as float) * 10.0;
    }
    t
}
fn test() {
    assert(rb_sum() == 12.0, \"1 + 0, then 1 + 10\");
}"
    )
    .result(Value::Null);
}
