// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Reassignment locality and write-back, the link (`&`) model and its edges, and tuple layout.

use super::*;

// ── @PLN87 P2 reassignment-locality / write-back CONSISTENCY lock-ins ─────────
// The uniform model (mirrors tests/scripts/87-p2-reassign-locality.loft, which
// holds the GREEN cells): a non-`&` whole-binding reassignment of a heap param
// is LOCAL; a `&` reassignment writes BACK; field/element mutation propagates.
// Struct + scalar cells already pass (P2.1).  These pin the still-inconsistent
// VECTOR cells to their target consistent behaviour; un-ignore each when it lands.

// Vector non-`&` reassignment REBINDS locally (leaves the caller untouched),
// like the struct case — fixed by P2.4 (vector_db hands a rebind param a fresh
// `__vdb` backing; the witness frees it at exit).
/// loft#1836 / loft#1816 — a whole-value write through a `&` link to a KEYED collection writes
/// the source (`(B-Ref-Write)`), so it is neither a dead assignment nor an unread variable.  A
/// keyed link is never in `amp_vector_locals`; asked only that, the lints reported both.  No
/// warning is expected, and the value proves the write landed.
#[test]
fn a_keyed_link_whole_write_is_not_a_dead_store() {
    code!(
        "struct Ek { k: integer }
fn check() -> integer { h: hash<Ek[k]> = [Ek { k: 1 }]; c = &h; c = [Ek { k: 2 }, Ek { k: 3 }]; len(h) }"
    )
    .expr("check()")
    .result(Value::Int(2));
}

#[test]
fn pln87_vector_param_reassign_is_local() {
    code!(
        "fn vrebind(v: vector<integer>) { v = [7, 8, 9]; }
fn check() -> integer { a = [1, 2, 3]; vrebind(a); len(a) }"
    )
    .warning("The value parameter 'v' receives is never read — an assignment replaces it first, and the replacement stays in this function at pln87_vector_param_reassign_is_local:1:33")
    .warning("Dead assignment — 'v' is never read after this, and a parameter's assignment stays in this function at pln87_vector_param_reassign_is_local:1:48")
    .expr("check()")
    .result(Value::Int(3));
}

// Vector `&` reassignment WRITES BACK (caller sees the new vector), like the
// struct `&` case — fixed by P2.4 (a `&`-vector shares the caller's backing, so
// the write-back is a clear+refill in place: `OpClearVector` before the literal).
#[test]
fn pln87_vector_param_amp_writes_back() {
    code!(
        "fn vamp(v: &vector<integer>) { v = [7, 8, 9]; }
fn check() -> integer { a = [1, 2, 3]; vamp(a); a[0] }"
    )
    .expr("check()")
    .result(Value::Int(7));
}

/// @PLN87 #1 — `&` write-back from a CALL RHS (`o = mk()`). The ownership-transfer machinery
/// now routes a call RHS through a transferable owned temp, so the write-back reaches the
/// caller's place (`a.x` becomes 9). Verified clean on both backends. (Was deferred behind a
/// parse rejection that no longer exists.)
#[test]
fn pln87_amp_writeback_from_call_writes_back() {
    code!("struct Obj { x: integer } fn mk() -> Obj { Obj { x: 9 } } fn f(o: &Obj) { o = mk(); } fn check() -> integer { a = Obj { x: 1 }; f(a); a.x }")
        .expr("check()")
        .result(Value::Int(9));
}

/// @PLN87 — the `&var = 3` rejection must NOT over-fire on a valid RHS link:
/// `c = &v[0]` LINKS `c` to the element (bind-site `&`), and reading `c` sees the
/// linked value. (The `&` is on the RHS, followed by `;`, not an assignment `=`.)
#[test]
fn pln87_amp_rhs_link_is_not_rejected() {
    code!("fn check() -> integer { v = [10, 20]; c = &v[0]; c }")
        .expr("check()")
        .result(Value::Int(10));
}

// @PLN87 — the LINK-semantics ladder (corrected `&` model: `&` LINKS a binding to
// its source, read- and write-through).  Each rung is an ignored lock-in that flips
// to PASS when that rung lands.  North star: `a=3; b=&a; b=4; a == 4`.

/// L1 — scalar local, LIVE read: a link reflects the source's current value.
#[test]
fn pln87_link_l1_scalar_live_read() {
    code!("fn check() -> integer { a = 3; b = &a; a = 5; b }")
        .expr("check()")
        .result(Value::Int(5));
}

/// L2 — scalar local, WRITE-THROUGH (the north star): writing the link writes the source.
#[test]
fn pln87_link_l2_scalar_write_through() {
    code!("fn check() -> integer { a = 3; b = &a; b = 4; a }")
        .expr("check()")
        .result(Value::Int(4));
}

/// L3 — scalar struct-field link: `b = &s.x; b = 4` writes `s.x`.
#[test]
fn pln87_link_l3_field_write_through() {
    code!("struct S { x: integer } fn check() -> integer { s = S { x: 3 }; b = &s.x; b = 4; s.x }")
        .expr("check()")
        .result(Value::Int(4));
}

/// L3 — the field link is LIVE in the read direction too, and works for a field at a
/// NON-zero offset (`s.b`, offset 8) as well as offset 0.
#[test]
fn pln87_link_l3_field_live_read() {
    code!(
        "struct S { a: integer, b: integer } \
         fn check() -> integer { s = S { a: 3, b: 7 }; r = &s.b; s.b = 5; r }"
    )
    .expr("check()")
    .result(Value::Int(5));
}

/// L4 — scalar vector-element link: `c = &v[0]; c = 99` writes `v[0]`.
#[test]
fn pln87_link_l4_element_write_through() {
    code!("fn check() -> integer { v = [10, 20]; c = &v[0]; c = 99; v[0] }")
        .expr("check()")
        .result(Value::Int(99));
}

/// L4 — the link is LIVE in the read direction too: `v[0]` updates show through `c`.
#[test]
fn pln87_link_l4_element_live_read() {
    code!("fn check() -> integer { v = [10, 20]; c = &v[0]; v[0] = 5; c }")
        .expr("check()")
        .result(Value::Int(5));
}

/// L6 — link as a function parameter: `fn f(b: &integer){ b = 4 }; f(a)` writes `a`.
/// The `&` parameter is called WITHOUT `&` (`f(a)`); the reference comes from the type.
#[test]
fn pln87_link_l6_param_write_through() {
    code!("fn f(b: &integer) { b = 4 } fn check() -> integer { a = 3; f(a); a }")
        .expr("check()")
        .result(Value::Int(4));
}

/// L6 — a `&`-struct parameter links to the caller's struct: a field mutation through
/// the parameter writes the caller's field.
#[test]
fn pln87_link_l6_struct_param_field_writes_back() {
    code!(
        "struct S { x: integer } fn g(obj: S) { obj.x = 5; } \
         fn check() -> integer { s = S { x: 1 }; g(s); s.x }"
    )
    .expr("check()")
    .result(Value::Int(5));
}

/// L5 — heap whole-value reference: `p = &o` ALIASES the heap local (which COPIES on a
/// plain `p = o`), so a field mutation through `p` writes `o`.  `p` reuses the #257
/// alias representation (interp stack-ref / native DbRef-by-value), non-owning.
#[test]
fn pln87_link_l5_heap_whole_value_ref() {
    code!("struct S { x: integer } fn check() -> integer { o = S { x: 1 }; p = &o; p.x = 5; o.x }")
        .expr("check()")
        .result(Value::Int(5));
}

/// L5 — the heap reference is LIVE in the read direction too: `o`'s field updates show
/// through `p`; and `p = o` WITHOUT `&` still copies (the `&` is what makes it a link).
#[test]
fn pln87_link_l5_heap_reference_live_read() {
    code!("struct S { x: integer } fn check() -> integer { o = S { x: 1 }; p = &o; o.x = 7; p.x }")
        .expr("check()")
        .result(Value::Int(7));
}

#[test]
fn pln87_plain_heap_assign_still_copies() {
    code!("struct S { x: integer } fn check() -> integer { o = S { x: 1 }; p = o; p.x = 5; o.x }")
        .expr("check()")
        .result(Value::Int(1));
}

/// @PLN87 #2 — a typed-local reference `b: &T = src` is the L1 form with the `&` on
/// the TYPE (instead of `b = &src`): a live reference to the addressable scalar `src`,
/// read- and write-through.
#[test]
fn pln87_typed_local_scalar_reference_live_read() {
    code!("fn check() -> integer { a = 3; b: &integer = a; a = 5; b }")
        .expr("check()")
        .result(Value::Int(5));
}

#[test]
fn pln87_typed_local_reference_write_through() {
    code!("fn check() -> integer { c = 10; d: &integer = c; d = 4; c }")
        .expr("check()")
        .result(Value::Int(4));
}

// @PLN87 L7 — edges of the `&`-reference model.

/// L7 — a reference to a reference (`c = &b`, both scalar): `c` links to the same source
/// `b` does, so writing `c` writes the original and reads are live.
#[test]
fn pln87_l7_ref_to_ref_scalar() {
    code!("fn check() -> integer { a = 3; b = &a; c = &b; c = 5; a }")
        .expr("check()")
        .result(Value::Int(5));
}

/// L7 — a reference to a struct reference (`q = &p`): a field mutation through `q`
/// reaches the original struct.
#[test]
fn pln87_l7_ref_to_ref_struct() {
    code!("struct S { x: integer } fn check() -> integer { o = S { x: 1 }; p = &o; q = &p; q.x = 7; o.x }")
        .expr("check()")
        .result(Value::Int(7));
}

/// L7 — a reference in an inner scope to an OUTER local (the source outlives the
/// reference): write-through is safe.
#[test]
fn pln87_l7_inner_scope_reference_to_outer() {
    code!("fn check() -> integer { a = 3; if true { b = &a; b = 5; } a }")
        .expr("check()")
        .result(Value::Int(5));
}

/// L7 — a heap reference stays LIVE across a whole-value reassignment of the source
/// (`o = S{..}` reuses the record in place, so the alias sees the new field values).
#[test]
fn pln87_l7_heap_reference_live_across_reassign() {
    code!("struct S { x: integer } fn check() -> integer { o = S { x: 1 }; p = &o; o = S { x: 9 }; p.x }")
        .expr("check()")
        .result(Value::Int(9));
}

/// Regression (routing consumer, docs/loft-feedback.md 2026-07-08 "Incorrect loop
/// finish"): a `for` loop variable reused across two sequential loops of DIFFERENT
/// element types is invalid under loft's flat scoping — the reused name keeps the
/// FIRST loop's type (`add_variable` only refines an unknown type for a user var),
/// so a field access on the SECOND loop variable resolves against the stale type,
/// fails, and leaves the loop with no iterable.  That error path in `parse_for`
/// ("Need an iterable expression") used to `return` WITHOUT `finish_loop`-ing the
/// loop scope it had already opened, so the ENCLOSING loop's `finish_loop` tripped
/// the `assert_eq!(current_loop, loop_nr)` "Incorrect loop finish" panic — masking
/// the real diagnostic.  The parse must now diagnose cleanly instead of panicking;
/// this test would panic inside `parse_str` before the fix.
///
/// loft#825 — what it diagnoses CLEANLY moved one step upstream: it used to answer
/// "Unknown field Pt.roads", which is the stale binding's CONSEQUENCE, and then named
/// the loop-variable conflict itself.
///
/// loft#915 removed the conflict rather than reporting it — `t` in the second loop is
/// its own variable carrying `Rt`, so `t.roads` resolves and the program COMPILES.  That
/// is what this now pins, and it is the same guarantee from the other side: the field
/// read only works if the second loop failed to inherit the first's type, so a
/// regression to a shared binding fails this test as "Unknown field Pt.roads" — the
/// error the whole chain started from.
#[test]
fn loop_var_reuse_different_type_binds_per_loop() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    // `t` is a `Pt` in the first loop and an `Rt` in the second; `t.roads` in the
    // second body resolves against the stale `Pt` type (which has no `roads`
    // field) → no iterable → the previously-leaking error path.
    p.parse_str(
        "struct Pt { tkey: integer, areas: vector<integer> }
struct Rt { tkey: integer, roads: vector<integer> }
fn main() {
  layout: hash<Pt[tkey]> = [];
  roads: hash<Rt[tkey]> = [];
  for t in layout { for a in t.areas { println(\"{a}\"); } }
  for t in roads { for r in t.roads { println(\"{r}\"); } }
}",
        "loop_var_reuse_different_type",
        false,
    );
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "each loop binds its own `t`, so `t.roads` resolves against `Rt` and this must \
         compile; a shared binding shows up here as \"Unknown field Pt.roads\".  Got: {:?}",
        p.diagnostics.lines()
    );
}

// @PLN102 keystone step 4 — honest nullable stdlib returns.
//
// `find`/`rfind` (text) and `min_of`/`max_of` (vector) can produce null on a
// reachable path (value absent / vector empty) and are now typed `integer?` /
// `T?` respectively, so the static type no longer LIES about the null case.
// The runtime representation is identical to before (in-band sentinel), so a
// runtime test cannot distinguish `-> integer` from `-> integer?`; the honest
// type is only observable at a declared-non-null BOUNDARY, where assigning the
// nullable result to a non-null declaration is reported — `(N-Store)`'s WARNING at full
// width (@PLN153 phase 3b: the declared local used to be a hard error).  This is the
// non-vacuous guard: it FAILS if a future edit reverts a signature to non-null.
#[test]
fn pln102_stdlib_reachable_null_returns_are_typed_nullable() {
    // Each of these assigns a nullable-returning stdlib call to a declared
    // NON-NULL local.  With honest `?` return types this is reported; if the
    // return type were reverted to non-null it would compile clean (the guard
    // would then fail to see the expected diagnostic → test fails).
    let cases = [
        ("find", "n: integer = \"abc\".find(\"z\");"),
        ("rfind", "n: integer = \"abc\".rfind(\"z\");"),
        ("min_of", "n: integer = min_of([3, 1, 2]);"),
        ("max_of", "n: integer = max_of([3, 1, 2]);"),
    ];
    for (name, stmt) in cases {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            &format!("fn main() {{\n  {stmt}\n  println(\"{{n}}\");\n}}"),
            "pln102_nullable_return",
            false,
        );
        assert!(
            p.diagnostics.level() >= loft::diagnostics::Level::Warning,
            "{name}: nullable return assigned to a non-null decl should be reported, got: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.diagnostics
                .lines()
                .iter()
                .any(|l| l.contains("is stored into the local `n`") && l.contains("integer?")),
            "{name}: expected the (N-Store) diagnostic naming `integer?`, got: {:?}",
            p.diagnostics.lines()
        );
    }
    // Positive control: the INFERRED (nullable) form compiles clean — proves the
    // error above is specifically the non-null-declaration mismatch, not a broken
    // program, and that the honest type stays fully usable via inference / `??`.
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(
        "fn main() {\n  \
           a = \"abc\".find(\"z\");\n  \
           b = min_of([3, 1, 2]) ?? 0;\n  \
           assert(a == null, \"find absent is null\");\n  \
           assert(b == 1, \"min_of ?? default\");\n\
         }",
        "pln102_nullable_return_ok",
        false,
    );
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "inferred/`??` nullable use should compile clean, got: {:?}",
        p.diagnostics.lines()
    );
}

// @PLN102 pre-freeze — comparison operators are NON-ASSOCIATIVE.  A chain like
// `a == b == c` / `a < b < c` parses left-associatively as `(a OP b) OP c`, silently
// comparing a BOOLEAN to the third operand — the classic C footgun.  The parser now
// rejects a second comparison at the same level; the explicit `(a == b) == c`, an `&&`
// join, and a single comparison all stay legal.  This is the non-vacuous guard (proven
// to fail if the rule is reverted — the chains would then parse clean).
#[test]
fn pln102_comparison_is_non_associative() {
    let chains = [
        "b = 2 == 2 == 1;",
        "b = 1 < 5 < 10;",
        "b = true == true == true;",
        "b = 1 == 2 != 3;",
    ];
    for stmt in chains {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            &format!("fn main() {{\n  {stmt}\n}}"),
            "pln102_nonassoc",
            false,
        );
        assert!(
            p.diagnostics
                .lines()
                .iter()
                .any(|l| l.contains("do not chain")),
            "expected a 'do not chain' error for `{stmt}`, got: {:?}",
            p.diagnostics.lines()
        );
    }
    // Positive control: explicit parens, an `&&` join, and a single comparison compile
    // clean — proves the rule rejects only the CHAIN, not comparison itself.
    let ok = [
        "b = (2 == 2) == true;",
        "b = 1 < 5 && 5 < 10;",
        "b = 2 == 2;",
    ];
    for stmt in ok {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            &format!("fn main() {{\n  {stmt}\n}}"),
            "pln102_nonassoc_ok",
            false,
        );
        assert!(
            p.diagnostics.level() < loft::diagnostics::Level::Error,
            "expected `{stmt}` to compile clean, got: {:?}",
            p.diagnostics.lines()
        );
    }
}

// @PLN102 pre-freeze — a boolean and an integer are not comparable.  The old `bool == int`
// coerced the integer to boolean by "is non-null", so `true == 0` was TRUE and `true == 2`
// was TRUE (nonsense), while `bool < int` already errored.  `==`/`!=` now reject it too.
#[test]
fn pln102_boolean_integer_comparison_rejected() {
    let bad = [
        "b = true == 1;",
        "b = true != 0;",
        "b = 1 == false;",
        "b = false == 0;",
    ];
    for stmt in bad {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            &format!("fn main() {{\n  {stmt}\n}}"),
            "pln102_bool_int",
            false,
        );
        assert!(
            p.diagnostics
                .lines()
                .iter()
                .any(|l| l.contains("cannot compare a boolean and an integer")),
            "expected `{stmt}` to be rejected, got: {:?}",
            p.diagnostics.lines()
        );
    }
    // Still valid: bool==bool, int==int, and the three-state `boolean? == null`.
    let ok = [
        "b = true == false;",
        "b = 1 == 2;",
        "n: boolean? = true; b = n == null;",
    ];
    for stmt in ok {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            &format!("fn main() {{\n  {stmt}\n}}"),
            "pln102_bool_int_ok",
            false,
        );
        assert!(
            p.diagnostics.level() < loft::diagnostics::Level::Error,
            "expected `{stmt}` to compile clean, got: {:?}",
            p.diagnostics.lines()
        );
    }
}

// ─── @PLN114 — tuple layout must match the record layout ─────────────────────
//
// Both measure a tuple against the `struct` of identical fields, because that record
// is the oracle the tuple layout was moved onto.  They landed `#[ignore]`d — asserting
// a target that failed — and were un-ignored when the fix made them pass.

/// Instrument 3 — record-vs-tuple stride parity.
///
/// `(u8, u32, u16)` must occupy what `struct { a: u8, b: u32, c: u16 }` occupies.
/// Today the record packs to 7 bytes and the tuple reserves 24, because
/// `data::element_size` reports STACK widths (`Integer` is 8B regardless of
/// `forced_size`) where storage needs database widths.
#[test]
fn pln114_tuple_stride_matches_record() {
    // `@C138` / `@FR-L-Tuple` — every member sits on its natural boundary, and a stored tuple
    // is a record (@C139): both reorder largest-first (u32, u16, u8: 7 bytes, padded to 8), so
    // the two strides are equal and element 1 of either vector is aligned.
    code!(
        "struct M { a: u8, b: u32, c: u16 }
fn test() {
    vs: vector<M> = [M{a:1,b:2,c:3}, M{a:4,b:5,c:6}];
    vt: vector<(u8, u32, u16)> = [(1,2,3), (4,5,6)];
    assert(sizeof(M) == 8, \"struct stride {sizeof(M)}\");
    assert(sizeof((u8, u32, u16)) == 8, \"tuple stride {sizeof((u8, u32, u16))}\");
    assert(vs[1].b == 5 && vt[1].1 == 5 && vt[1].2 == 6, \"element 1 of each\");
}"
    );
}

/// Instrument 4 — a mixed-width tuple must round-trip every element.
///
/// Today the narrow trailing element reads back `+1`: `(1,2,3)` returns `(1,2,4)`,
/// reproducible in `vector<(u8,u16)>` and `vector<(u32,u16)>` too.  Silent
/// corruption — exit 0, no diagnostic.
#[test]
fn pln114_mixed_width_tuple_round_trip() {
    code!(
        "fn test() {
    v: vector<(u8, u32, u16)> = [(1,2,3), (4,5,6)];
    assert(v[0].0 == 1 && v[0].1 == 2 && v[0].2 == 3, \"element 0 round-trips\");
    assert(v[1].0 == 4 && v[1].1 == 5 && v[1].2 == 6, \"element 1 round-trips\");
    w: vector<(u8, u16)> = [(1,2)];
    assert(w[0].1 == 2, \"u16 after u8 round-trips\");
}"
    );
}

/// A3 — the two alignment tables must agree.
///
/// `data::element_align` and the inline table in `Data::tuple_def` encode the same
/// rule twice.  They ALREADY disagree about `Function` (8 vs 4), and nothing detects
/// it because nothing compares them — the drift this plan exists to remove.  Delete
/// this test when the inline copy is gone (A4) and there is nothing to compare.
#[test]
fn pln114_alignment_tables_agree() {
    use loft::data::{IntegerSpec, Type, element_stack_align};
    let int = Type::Integer(IntegerSpec {
        min: i32::MIN + 1,
        max: i64::from(i32::MAX),
        not_null: false,
        forced_size: None,
    });
    for (name, tp, expect) in [
        ("boolean", Type::Boolean, 1u8),
        ("character", Type::Character, 4),
        ("single", Type::Single, 4),
        ("integer", int, 8),
        ("float", Type::Float, 8),
    ] {
        assert_eq!(
            element_stack_align(&tp),
            expect,
            "element_stack_align({name}) — the tuple_def inline table must agree"
        );
    }
}
