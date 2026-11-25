// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Language enhancements (bitwise operators, references, links, `is`, `break` values) and the
//! regressions found while building them.

use super::*;

// ── Language enhancements ────────────────────────────────────────────

/// Bitwise NOT operator `~` — desugars to OpBitNotSingleInt.
#[test]
fn enhancement_bitwise_not() {
    expr!("~0").result(Value::Int(-1));
}

#[test]
fn enhancement_bitwise_not_clear_bit() {
    expr!("(32 | 64) & ~32").result(Value::Int(64));
}

/// `&vector<T>` mutation detection — for-loop variable field writes
/// should propagate back to the iterated `&` collection parameter.
#[test]
fn enhancement_ref_vector_loop_mutation_detected() {
    code!(
        "struct Item { val: integer }
fn double_all(items: &vector<Item>) {
    for it in items { it.val = it.val * 2; }
}
fn test() {
    v: vector<Item> = [Item { val: 5 }];
    double_all(v);
    assert(v[0].val == 10, \"doubled\");
}"
    )
    .result(Value::Null);
}

/// Read-only loop over `&vector<T>` should still flag the `&`.
#[test]
fn enhancement_ref_vector_readonly_loop_still_flags() {
    code!(
        "struct Item { val: integer }
fn sum_vals(items: &vector<Item>) -> integer {
    total = 0;
    for it in items { total = total + it.val; }
    total
}
fn test() { }"
    )
    // 2:20 is the `&` (loft#1003).  The error shares the reference check's position with
    // the `needless-reference-parameter` warning, so it moved off the body with it.
    .error(
        "Parameter 'items' has & but is never modified; remove the & \
at enhancement_ref_vector_readonly_loop_still_flags:2:20",
    );
}

/// `@FR-B-Ref-Write` — a `&` parameter only READ through a local link (and a link to that
/// link) is still never modified, so the needless-`&` refusal still holds.  The control for the
/// fix that carries a write on a link back to its source (`a-write-through-a-link-modifies-the-
/// amp-parameter`): the link's own bind is a `Set` too, and counting it would silence this.
#[test]
fn a_link_only_read_leaves_the_amp_parameter_unmodified() {
    code!(
        "fn twice(p: &integer) -> integer {
    c = &p;
    d = &c;
    d * 2
}
fn test() { }"
    )
    .error(
        "Parameter 'p' has & but is never modified; remove the & \
at a_link_only_read_leaves_the_amp_parameter_unmodified:1:13",
    );
}

/// `@FR-T-Record` (loft#1875) — a narrow member of a by-value tuple is a field a link names: it
/// holds its field encoding once a `&` names it, so a `&u8` argument and a `&u8` bind both write
/// the member, and a whole read sees the written value.
#[test]
fn a_narrow_tuple_member_link_writes_the_member() {
    code!(
        "fn nb(p: &u8) { p += 1; }
fn run() -> integer { n: (u8, integer) = (5, 1); nb(n.0); c = &n.0; c += 10; m = n; m.0 * 1000 + n.0 * 10 + n.1 }"
    )
    .expr("run()")
    .advice(
        "a step past `0..255` takes this type's default, `0` — nothing here says which values \
the step may produce at a_narrow_tuple_member_link_writes_the_member:2:77",
    )
    .result(Value::Long(16_161));
}

/// `@FR-T-Record` (loft#1875) — a TEXT member of a tuple local is the tuple's own text, so a
/// `&text` argument and a `&` bind both write it, and a by-value parameter's member links to the
/// callee's copy.
#[test]
fn a_text_tuple_member_link_writes_the_member() {
    code!(
        "fn tb(p: &text) { p += \"!\"; }
fn own(t: (text, integer)) -> text { tb(t.0); t.0 }
fn run() -> text { s = (\"a\", 1); tb(s.0); c = &s.0; c += \"?\"; \"{s.0}/{own(s)}/{s.0}\" }"
    )
    .expr("run()")
    .result(Value::str("a!?/a!?!/a!?"));
}

/// `@FR-B-Ref-Lvalue` — a member of a tuple reached through a `&(…)` link is a place a link
/// names: the link's reference at the member's offset.  `c = &p.0; c += 5` writes the CALLER's
/// tuple, and the write reaches it on both backends.
#[test]
fn a_member_of_a_linked_tuple_is_a_place() {
    code!(
        "fn via(p: &(integer, integer)) -> integer { c = &p.0; c += 5; p.1 = c; p.1 }
fn run() -> integer { t = (250, 1); r = via(t); r * 1000000 + t.0 * 1000 + t.1 }"
    )
    .expr("run()")
    .result(Value::Long(255_255_255));
}

/// `@FR-Const-ScalarCollapse` / `@FR-Const-Value` — every spelling that writes past a `const`
/// scalar or a value-const field is refused.  Each wrote in silence on both backends before: a
/// `&` link to a binding-const scalar field or local, the same handed to a `&` parameter, a
/// link to a value-const scalar field, a rebind of a value-const ENUM (the collapse list named
/// five scalar types and not the enum), and an append to a value-const `text` field (routed
/// through a working variable that skipped the component check).
#[test]
fn a_link_to_a_const_scalar_field_is_refused() {
    code!(
        "struct H { const v: integer }
fn test() { h = H { v: 1 }; c = &h.v; c = 7; assert(h.v == 1, \"h\"); }"
    )
    .error(
        "Cannot modify 'c': it is a view of const field 'H.v', whose value is read-only; remove \
'const' there, or copy what you change into a local at a_link_to_a_const_scalar_field_is_refused:2:44",
    );
}

#[test]
fn a_const_scalar_local_to_an_amp_parameter_is_refused() {
    code!(
        "fn bump(p: &integer) { p += 1; }
fn test() { const x: integer = 1; bump(x); assert(x == 1, \"x\"); }"
    )
    .error(
        "Cannot pass const variable 'x' to the `&` parameter 1 of `bump`, which may modify it; its \
value is read-only — pass a local copy, or make the parameter `const` if `bump` only reads it \
at a_const_scalar_local_to_an_amp_parameter_is_refused:2:43",
    );
}

#[test]
fn a_link_to_a_value_const_scalar_field_is_refused() {
    code!(
        "struct H { v: const integer }
fn test() { h = H { v: 1 }; c = &h.v; c = 7; assert(h.v == 1, \"h\"); }"
    )
    .error(
        "Cannot modify 'c': it is a view of value-const field 'H.v', whose value is read-only; \
remove 'const' there, or copy what you change into a local at \
a_link_to_a_value_const_scalar_field_is_refused:2:44",
    );
}

#[test]
fn a_value_const_enum_field_rebind_is_refused() {
    code!(
        "enum Col { Red, Green }
struct H { v: const Col }
fn test() { h = H { v: Col.Red }; h.v = Col.Green; assert(h.v == Col.Red, \"h\"); }"
    )
    .error(
        "cannot mutate value-const field 'v' of struct 'H' — its value is read-only (rebind with \
'=' to re-point, or drop 'const') at a_value_const_enum_field_rebind_is_refused:3:51",
    );
}

#[test]
fn a_value_const_text_field_append_is_refused() {
    code!(
        "struct H { v: const text }
fn test() { h = H { v: \"a\" }; h.v += \"z\"; assert(h.v == \"a\", \"h\"); }"
    )
    .error(
        "cannot mutate value-const field 'v' of struct 'H' — its value is read-only (rebind with \
'=' to re-point, or drop 'const') at a_value_const_text_field_append_is_refused:2:41",
    );
}

/// `@FR-Const-Bind` — a binding-const field's SLOT never re-points (a rebind is refused) while
/// its contents stay mutable (`a-value-const-scalar-local-can-be-declared` holds the append and
/// the element write).  `@FR-Const-Compose` — `const v: const T` refuses the rebind AND every
/// write beneath it: the append and the element write that Const-Bind alone allows.
#[test]
fn a_binding_const_field_rebind_is_refused() {
    code!(
        "struct H { const v: vector<integer> }
fn test() { h = H { v: [1] }; h.v = [2]; }"
    )
    .error(
        "cannot reassign const field 'v' of struct 'H' — const fields are write-once-at-construction \
at a_binding_const_field_rebind_is_refused:2:41",
    );
}

#[test]
fn a_composed_const_field_refuses_its_rebind() {
    code!(
        "struct H { const v: const vector<integer> }
fn test() { h = H { v: [1] }; h.v = [2]; }"
    )
    .error(
        "cannot reassign const field 'v' of struct 'H' — const fields are write-once-at-construction \
at a_composed_const_field_refuses_its_rebind:2:41",
    );
}

#[test]
fn a_composed_const_field_refuses_its_append() {
    code!(
        "struct H { const v: const vector<integer> }
fn test() { h = H { v: [1] }; h.v += [2]; }"
    )
    .error(
        "cannot mutate value-const field 'v' of struct 'H' — its value is read-only (rebind with \
'=' to re-point, or drop 'const') at a_composed_const_field_refuses_its_append:2:42",
    );
}

#[test]
fn a_composed_const_field_refuses_an_element_write() {
    code!(
        "struct H { const v: const vector<integer> }
fn test() { h = H { v: [1] }; h.v[0] = 5; }"
    )
    .error(
        "Cannot modify value-const field 'H.v'; its value is read-only \
at a_composed_const_field_refuses_an_element_write:2:42",
    );
}

/// `@FR-F-Arity` — a compiler-inserted slot (a text return's buffer, a record's `__retbuf`) is
/// not a user parameter: an argument past the declared ones is too many.  Counted as one, the
/// extra `"c"` filled `txt`'s text buffer and the call answered `ab` in silence; a record
/// return's extra argument was refused as a type mismatch on its buffer; and a generic's refusal
/// named its mangled instance (`i_7integer_n_gen`) instead of `gen`.
#[test]
fn an_argument_past_the_parameters_never_fills_a_text_buffer() {
    code!(
        "fn txt(a: text, b: text = \"z\") -> text { \"{a}{b}\" }
fn test() { t = txt(\"a\", \"b\", \"c\"); }"
    )
    .error("Too many parameters for txt at an_argument_past_the_parameters_never_fills_a_text_buffer:2:36");
}

#[test]
fn an_argument_past_the_parameters_never_fills_a_record_buffer() {
    code!(
        "struct P { x: integer }
fn rec(a: integer) -> P { P { x: a } }
fn test() { r = rec(1, 2); }"
    )
    .error("Too many parameters for rec at an_argument_past_the_parameters_never_fills_a_record_buffer:3:27");
}

#[test]
fn a_generic_called_with_too_many_arguments_is_named_as_written() {
    code!(
        "fn gen<T>(a: T) -> T { a }
fn test() { gen(8, 1); }"
    )
    .error("Too many parameters for gen at a_generic_called_with_too_many_arguments_is_named_as_written:2:23");
}

/// `@FR-F-OneBody` with `@FR-N-Shape` — `τ` and `τ?` are one receiver for the one-body question:
/// `(F-Recv)` sends both call spellings to whichever of `m(τ)` / `m(τ?)` is a method, so a plain
/// `m(p: P?)` beside `fn m(self: P)` was reached by neither, dead in silence.  Refused in either
/// order; a `self`/`self` pair over `P` and `P?` is untouched
/// (`a-method-and-its-nullable-twin-are-two-bodies`).
#[test]
fn a_plain_function_on_the_nullable_receiver_of_a_method_is_refused() {
    code!(
        "struct P { x: integer }
fn m(self: P) -> integer { 1 }
fn m(p: P?) -> integer { 2 }
fn test() { }"
    )
    .error(
        "Cannot redefine 'm' (already defined at \
a_plain_function_on_the_nullable_receiver_of_a_method_is_refused:2:17) — a name has one body per \
receiver type, and `x.m(…)` and `m(x, …)` would reach different functions; declare it once as a \
`self` method, which takes both spellings, or rename one at \
a_plain_function_on_the_nullable_receiver_of_a_method_is_refused:3:15",
    );
}

#[test]
fn a_method_after_a_plain_function_on_its_nullable_receiver_is_refused() {
    code!(
        "struct P { x: integer }
fn m(p: P?) -> integer { 2 }
fn m(self: P) -> integer { 1 }
fn test() { }"
    )
    .error(
        "Cannot redefine 'm' (already defined at \
a_method_after_a_plain_function_on_its_nullable_receiver_is_refused:2:15) — a name has one body \
per receiver type, and `x.m(…)` and `m(x, …)` would reach different functions; declare it once \
as a `self` method, which takes both spellings, or rename one at \
a_method_after_a_plain_function_on_its_nullable_receiver_is_refused:3:17",
    );
}

/// `break value` in void function → compile error.
#[test]
fn enhancement_break_value_in_void_function_errors() {
    code!(
        "fn test() {
    for i in 0..10 {
        if i == 5 { break i; }
    }
}"
    )
    .error(
        "`break <value>` requires a non-void function — \
the value is returned from the enclosing function \
at enhancement_break_value_in_void_function_errors:3:29",
    );
}

/// `is` operator — variant check on plain enum.
#[test]
fn enhancement_is_plain_enum() {
    code!(
        "enum Dir { North, South, East, West }
fn test() {
    d = Dir.North;
    assert(d is North, \"is North\");
    assert(!(d is South), \"not South\");
}"
    )
    .result(Value::Null);
}

/// `is` operator — variant check on struct-enum + loop counting.
#[test]
fn enhancement_is_struct_enum_in_loop() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    items: vector<Shape> = [Circle { radius: 1.0 }, Rect { width: 2.0, height: 3.0 }, Circle { radius: 4.0 }];
    count = 0;
    for it in items { if it is Circle { count = count + 1; } }
    assert(count == 2, \"2 circles\");
}"
    )
    .result(Value::Null);
}

/// `is` operator with field capture — single field.
#[test]
fn enhancement_is_capture_single_field() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    s = Circle { radius: 3.14 };
    result = 0.0;
    if s is Circle { radius } {
        result = radius;
    }
    assert(result == 3.14, \"captured radius\");
}"
    )
    .result(Value::Null);
}

/// `is` operator with field capture — multiple fields + else branch.
#[test]
fn enhancement_is_capture_multiple_fields_else() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    s = Rect { width: 5.0, height: 10.0 };
    area = 0.0;
    if s is Rect { width, height } {
        area = width * height;
    } else {
        area = -1.0;
    }
    assert(area == 50.0, \"captured both\");
    c = Circle { radius: 2.0 };
    if c is Rect { width, height } {
        area = width * height;
    } else {
        area = -1.0;
    }
    assert(area == -1.0, \"else taken\");
}"
    )
    .result(Value::Null);
}

/// `is` operator with field capture in loop — sum radii from mixed vector.
#[test]
fn enhancement_is_capture_in_loop() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    items: vector<Shape> = [Circle { radius: 1.0 }, Rect { width: 2.0, height: 3.0 }, Circle { radius: 4.0 }];
    total = 0.0;
    for it in items {
        if it is Circle { radius } {
            total += radius;
        }
    }
    assert(total == 5.0, \"sum of radii\");
}"
    )
    .result(Value::Null);
}

/// `is` capture scope doesn't leak into outer scope.
#[test]
fn enhancement_is_capture_scope_isolation() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    s = Circle { radius: 99.0 };
    radius = 1.0;
    if s is Rect { width, height } {
        radius = width;
    }
    assert(radius == 1.0, \"outer radius unchanged\");
}"
    )
    .result(Value::Null);
}

/// Op table extension — emit_op handles ops >= 255 via escape prefix.
/// No specific op to test yet (all 255 primary slots used), but verify
/// the infrastructure doesn't break existing ops.
#[test]
fn enhancement_op_extension_existing_ops_unaffected() {
    expr!("~0").result(Value::Int(-1));
}

/// map/filter on &vector<T> parameter — method resolution unwraps RefVar.
#[test]
fn enhancement_map_filter_on_ref_vector() {
    code!(
        "fn process(items: &vector<integer>) {
    items += [99];
    d = items.map(|x| { x * 2 });
    assert(d[0] == 2, \"mapped\");
}
fn test() {
    v = [1, 2, 3];
    process(v);
    assert(len(v) == 4, \"appended\");
}"
    )
    .result(Value::Null);
}

/// P161 regression guard — `for it in items` where items is
/// `&vector<Struct>` used to error "Unknown type null" (field access
/// on the loop variable failed).  Root cause: `for_type` and
/// `iterator` didn't unwrap `RefVar(Vector(...))` before matching.
#[test]
fn p161_for_over_ref_vector() {
    code!(
        "struct Item { val: integer }
fn add_item(items: &vector<Item>, v: integer) {
    items += [Item { val: v }];
}
fn test() {
    v: vector<Item> = [];
    add_item(v, 42);
    assert(len(v) == 1, \"len {len(v)}\");
    assert(v[0].val == 42, \"val {v[0].val}\");
}"
    )
    .result(Value::Null);
}

/// P160 regression guard — `modify(items[1], 42)` where `modify`
/// takes `&S` used to error "Cannot pass a literal or expression
/// to a '&' parameter".  Two fixes: (1) parser accepts "addressable"
/// expressions (vector element, field access chains rooted in a Var);
/// (2) codegen handles `OpCreateStack(non-Var expr)` by generating
/// the expression first (pushes DbRef), then emitting OpCreateStack
/// with the offset pointing at the just-pushed result.
#[test]
fn p160_vec_element_as_ref_param() {
    code!(
        "struct S { x: integer }
fn modify(s: S, val: integer) { s.x = val; }
fn test() {
    items: vector<S> = [S { x: 0 }, S { x: 10 }];
    modify(items[1], 42);
    assert(items[1].x == 42, \"got {items[1].x}\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p160_nested_field_vec_element_as_ref_param() {
    code!(
        "struct Inner { val: integer }
struct Outer { items: vector<Inner> }
fn set_val(inner: Inner, v: integer) { inner.val = v; }
fn test() {
    o = Outer { items: [Inner { val: 0 }, Inner { val: 0 }] };
    set_val(o.items[1], 99);
    assert(o.items[1].val == 99, \"got {o.items[1].val}\");
}"
    )
    .result(Value::Null);
}

/// P159 regression guard — `Shape.parse(json)` used to fail for
/// struct-enums ("Unknown field Shape.parse").  Fix: added
/// `DefType::Enum` branch in `parse_var` for `.parse(` detection,
/// and added discriminant wrapper `{"Variant":{fields}}` to the
/// JSON serializer in `format.rs`.
#[test]
fn p159_struct_enum_json_roundtrip() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    c = Circle { radius: 3.14 };
    j = \"{c:j}\";
    p = Shape.parse(j);
    r = match p { Circle { radius } => radius, Rect => 0.0 };
    assert(r == 3.14, \"circle rt\");
    rect = Rect { width: 5.0, height: 10.0 };
    j2 = \"{rect:j}\";
    p2 = Shape.parse(j2);
    r2 = match p2 { Circle => 0.0, Rect { width, height } => width * height };
    assert(r2 == 50.0, \"rect rt\");
}"
    )
    .result(Value::Null);
}

/// P158 regression guard — trailing comma after the last field in a
/// struct-enum variant used to trigger "Expect attribute".  Regular
/// structs accepted trailing commas; enum variants didn't.  Fix:
/// added `|| self.lexer.peek_token("}")` to the break condition in
/// `parse_enum_values`, mirroring `parse_struct`.
#[test]
fn p158_trailing_comma_enum_variant() {
    code!(
        "enum K {
    Alpha { x: integer, y: integer, },
    Beta { z: integer }
}
fn test() {
    a = Alpha { x: 1, y: 2 };
    match a { Alpha { x, y } => assert(x + y == 3, \"sum\"), Beta => 0 };
}"
    )
    .result(Value::Null);
}

/// P155 regression guard — push/undo/mid-assert/redo/final-read used
/// to SIGSEGV in OpGetVector.  Root cause: `state/codegen.rs::generate_set`
/// (reassignment path, lines 891-932) emitted `OpCopyRecord` with the
/// 0x8000 "free source" flag around a user-fn call, but without the
/// `n_set_store_lock` bracket.  When the callee returned a DbRef
/// aliased with a caller arg — e.g. `read_at(c, idx)` returns into
/// `c.items` — the free-source flag freed the caller's arg store.
/// Later uses of that arg SIGSEGV'd.  Fix: mirror the
/// `gen_set_first_ref_call_copy` lock/unlock bracket (which the P143
/// fix added) onto the reassignment path.
#[test]
fn p155_segv_undo_redo_midassert() {
    code!(
        "struct H { m: integer }
struct Elm { prev: H }
struct Ct { items: vector<H> }
struct Ss { undo: vector<Elm>, redo: vector<Elm> }
fn read_at(c: Ct, idx: integer) -> H { c.items[idx] ?? H {} }
fn test() {
    c = Ct { items: [H{}, H{}, H{}, H{}, H{}, H{}] };
    s = Ss { undo: [], redo: [] };
    h = read_at(c, 2);
    s.undo += [Elm { prev: h }];
    nh = H {}; nh.m = 77; c.items[2] = nh;
    e = s.undo[0];
    cur = read_at(c, 2);
    s.redo += [Elm { prev: cur }];
    c.items[2] = e.prev;
    assert(read_at(c, 2).m == 0, \"reverted\");
    re = s.redo[0];
    c.items[2] = re.prev;
    assert(read_at(c, 2).m == 77, \"reapplied\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p145_text_return_multivec_struct_cross_file() {
    let mut p = Parser::new();
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_dir("default", true, false).unwrap();
    p.parse("tests/lib/p145_main2.loft", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "P145 regression: to_json on cross-file multi-vector struct crashed"
    );
}

/// P162 regression guard — native codegen emits `return let mut …` when
/// a match expression with struct-enum field bindings + guard is returned
/// directly.  `pre_declare_branch_vars` writes `let mut` declarations
/// after the `return` keyword.  Interpreter works; native compilation fails.
#[test]
fn p162_return_match_struct_enum_native() {
    code!(
        "enum GShape {
    GCircle { radius: float },
    GRect { width: float, height: float }
}
fn garea(s: GShape) -> float {
    match s {
        GCircle { radius } if radius > 0.0 => 3.14 * radius * radius,
        GCircle { radius } => 0.0,
        GRect { width, height } => width * height
    }
}
fn test() {
    assert(garea(GCircle { radius: 2.0 }) > 12.0, \"circle area\");
    assert(garea(GCircle { radius: -1.0 }) == 0.0, \"negative radius\");
    assert(garea(GRect { width: 3.0, height: 4.0 }) == 12.0, \"rect area\");
}"
    )
    .result(Value::Null);
}

/// P164 regression guard — trailing comma after the LAST VARIANT of an
/// enum declaration used to fail with `Expect name in type definition`.
/// P158 fixed trailing commas inside a variant's field list; this is the
/// sibling case on the variant list itself.  Fix: mirror the P158 guard
/// (`|| self.lexer.peek_token("}")`) onto the outer variant-list break
/// check in `parse_enum_values`.
#[test]
fn p164_trailing_comma_enum_variant_list() {
    code!(
        "enum P164Kind {
    P164Alpha { x: integer },
    P164Beta { y: integer },
}
fn test() {
    a = P164Alpha { x: 1 };
    assert(a is P164Alpha, \"alpha\");
    b = P164Beta { y: 2 };
    assert(b is P164Beta, \"beta\");
}"
    )
    .result(Value::Null);
}

/// P164 also covers plain (non-struct-field) enum declarations.
#[test]
fn p164_trailing_comma_plain_enum() {
    code!(
        "enum P164Dir {
    P164North,
    P164East,
    P164South,
    P164West,
}
fn test() {
    d = P164Dir.P164North;
    assert(d is P164North, \"north\");
}"
    )
    .result(Value::Null);
}

/// P170 regression guard — `x = Struct{}; x = vec[i]; mutate(x)` used to
/// fail with `Incorrect var x[N] versus M on n_<fn>` at codegen.
///
/// Root cause: `parser/objects.rs::parse_object` had a gap in the
/// in-place struct-literal path.  When the LHS variable's type was
/// already inferred with dependencies (because a later assignment in
/// the same function did `x = bs[i]`, giving x type
/// `Reference(Bag, [bs])`), `is_independent(x)` returned false.  The
/// in-place `v_set(x, Null) + OpDatabase(x)` init branch required both
/// `is_independent` AND `type_matches` — with `type_matches=true` and
/// `is_independent=false`, neither the if-branch nor the else-if
/// (which required `!type_matches`) fired.  The struct-literal
/// statement emitted only field-init calls into uninitialised storage,
/// codegen never saw a Set for x's first assignment, and later
/// `generate_var(x)` asserted since x's slot sat above TOS.
///
/// Fix: extend the `else if` to also fire when
/// `!is_independent && !first_pass` — routes the construction through
/// a fresh work-ref (existing "new_object" path), which emits the
/// required `v_set + OpDatabase` prelude and yields a `Block`-shaped
/// RHS that the outer assignment can then copy/alias via the normal
/// Set path.
#[test]
fn p170_struct_placeholder_then_vec_elem_reassign() {
    code!(
        "struct P170Bag { items: vector<integer> }
fn p170_mutate_bag(b: P170Bag, v: integer) { b.items += [v]; }
fn test() {
    p170_bs: vector<P170Bag> = [];
    p170_x = P170Bag {};
    p170_bs += [P170Bag {}];
    p170_x = p170_bs[len(p170_bs) - 1] ?? P170Bag {};
    p170_mutate_bag(p170_x, 1);
    assert(len(p170_bs[0].items) == 1, \"mutated through alias\");
}"
    )
    .warning("Dead assignment — 'p170_x' is overwritten before being read at p170_struct_placeholder_then_vec_elem_reassign:5:25")
    .result(Value::Null);
}

/// `dead-assignment` reads `match` arms as ALTERNATIVES, as it reads an `if`'s: a write in one
/// arm is not overwritten by the next arm's.  Until 2026-09-23 four of seven arm paths (the
/// boolean, scalar, struct and wildcard arms) carried the tracking state from one arm into the
/// next, so `acc = 1` in the first arm was reported dead — and `(B-Scope)` (loft#1600) makes this
/// bind-before-the-match spelling the way a local a later statement reads is written.  No
/// warning is declared, so any warning fails the test.
#[test]
fn dead_assignment_reads_match_arms_as_alternatives() {
    code!(
        "struct DaS { k: integer }
fn da_bool(c: boolean) -> integer { acc = 0; match c { true => { acc = 1; }, false => { acc = 2; } } acc }
fn da_int(c: integer) -> integer { acc = 0; match c { 1 => { acc = 3; }, _ => { acc = 4; } } acc }
fn da_struct(s: DaS) -> integer { acc = 0; match s { DaS { k: 1 } => { acc = 5; }, _ => { acc = 6; } } acc }
fn test() {
    assert(da_bool(true) + da_bool(false) == 3, \"bool\");
    assert(da_int(1) + da_int(2) == 7, \"int\");
    assert(da_struct(DaS { k: 1 }) + da_struct(DaS { k: 2 }) == 11, \"struct\");
}"
    )
    .result(Value::Null);
}

/// loft#2185 — a record update `w = U { f: e, ..b }` is the statement `w = b; w.f = e;`, and
/// its inner bind of `w` is that statement's own write.  The bind took back a target mention it
/// never counted, which removed the assert's real read, so `w`'s first value was reported dead;
/// no warning is declared, so any warning fails the test.  The sibling below keeps the lint.
#[test]
fn dead_assignment_reads_a_record_update_as_one_write() {
    code!(
        "struct DuU { hp: integer, x: integer }
enum DuN { DuL { v: integer }, DuB { op: integer, l: integer } }
fn test() {
    u = DuU { hp: 1, x: 2 };
    w = DuU { hp: 3, x: 4 };
    assert(w.hp == 3, \"read before the update\");
    w = DuU { hp: 5, ..u };
    n = DuB { op: 1, l: 2 };
    m: DuN = DuL { v: 4 };
    assert(m is DuL, \"read before the variant update\");
    m = DuB { l: 7, ..n };
    assert(w.hp == 5 && w.x == 2 && m is DuB, \"the updates\");
}"
    )
    .result(Value::Null);
}

/// loft#2185 — the update's earlier write that nothing read IS dead, and is reported ONCE:
/// the inner bind ran the check before the statement did, and the warning came out twice.
#[test]
fn dead_assignment_reports_a_record_update_once() {
    code!(
        "struct DoU { hp: integer, x: integer }
fn test() {
    u = DoU { hp: 1, x: 2 };
    w = DoU { hp: 3, x: 4 };
    w = DoU { hp: 5, ..u };
    assert(w.hp == 5 && w.x == 2, \"the update\");
}"
    )
    .warning("Dead assignment — 'w' is overwritten before being read at dead_assignment_reports_a_record_update_once:4:29")
    .result(Value::Null);
}

/// P170 guard — three-way: the same shape but with a conditional
/// assignment between the placeholder and the vec-elem reassign.
#[test]
fn p170_placeholder_conditional_then_reassign() {
    code!(
        "struct P170CBag { val: integer }
fn p170c_bump(b: P170CBag) { b.val = b.val + 1; }
fn test() {
    p170c_v: vector<P170CBag> = [P170CBag { val: 5 }];
    p170c_x = P170CBag { val: 0 };
    p170c_x = p170c_v[0];
    p170c_bump(p170c_x);
    assert(p170c_v[0].val == 6, \"bumped first elem\");
}"
    )
    .warning("Dead assignment — 'p170c_x' is overwritten before being read at p170_placeholder_conditional_then_reassign:5:35")
    .result(Value::Null);
}

/// P167 regression guard — trailing comma in a function-call argument
/// list used to fail with "Too many parameters for n_<fn>".  P158 fixed
/// trailing commas in struct-enum variant field lists; P164 fixed
/// trailing commas in enum variant lists; P167 covers function-call
/// argument lists (the third and final trailing-comma site).  Fix:
/// mirror the P158 guard in `parser/control.rs::parse_call` — for both
/// the positional and named argument loops.
#[test]
fn p167_trailing_comma_function_call_positional() {
    code!(
        "fn p167_add3(a: integer, b: integer, c: integer) -> integer { a + b + c }
fn test() {
    r = p167_add3(1, 2, 3,);
    assert(r == 6, \"trailing comma positional\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p167_trailing_comma_function_call_multiline() {
    // The shape that actually caught it in the wild — multi-line
    // call (rgb/vec3-style) with a trailing comma.
    code!(
        "fn p167_mix(r: integer, g: integer, b: integer) -> integer {
    (r * 65536) + (g * 256) + b
}
fn test() {
    c = p167_mix(
        10,
        20,
        30,
    );
    assert(c == 10 * 65536 + 20 * 256 + 30, \"multiline trailing comma\");
}"
    )
    .result(Value::Null);
}

/// P165 regression guard — `var: Enum = Variant { ... }` used to fail
/// with "Variable 'var' cannot change type from Enum to Variant; use a
/// new variable name or cast with 'as'".  The type-change check treated
/// a struct-enum variant as a distinct type from its parent enum.
/// Fix: in `Function::change_var_type`, accept `(Enum(p, true, _),
/// Enum(v, true, _))` when `data.def(v).parent == p` — the parent
/// relationship proves subtype compatibility.
#[test]
fn p165_enum_annotation_with_variant_rhs() {
    code!(
        "enum P165Kind {
    P165Alpha { x: integer },
    P165Beta { y: integer }
}
fn take_kind(k: P165Kind) -> boolean { k is P165Alpha }
fn test() {
    // Annotated LHS with variant RHS (the P165 shape).
    k1: P165Kind = P165Alpha { x: 1 };
    assert(take_kind(k1), \"annotated alpha\");
    // Annotated LHS with the OTHER variant — also accepted.
    k2: P165Kind = P165Beta { y: 2 };
    assert(!take_kind(k2), \"annotated beta\");
}"
    )
    .result(Value::Null);
}

/// P175 — file-scope `pub NAME: vector<T> = [...]` constants of
/// textual element types were materialised as empty vectors at
/// runtime (`len == 0`).  `vector<integer>` / `vector<float>` /
/// `vector<single>` / `vector<integer>` already worked because
/// `compile::extract_literal_values` matched `OpSetInt/Float/Single/Long`
/// calls.  `vector<text>` missed the `OpSetText` branch + the
/// `Value::Text` arm in `build_const_vectors`'s per-element match,
/// so the pre-built constant was registered with zero values and the
/// OpConstRef runtime copy produced an empty vector.  Fix: add
/// `OpSetText` to the extractor's allow-list and handle `Value::Text(s)`
/// in the build loop via `store.set_str(s)` + `set_int` (mirroring
/// runtime `OpSetText` in `src/fill.rs::set_text`).
#[test]
fn p175_vector_of_text_constant_populates() {
    code!(
        "pub P175_INTS: vector<integer> = [10, 20, 30];
pub P175_TEXTS: vector<text> = [\"aa\", \"bb\", \"cc\"];
fn test() {
    assert(len(P175_INTS) == 3, \"int vec len\");
    assert(P175_INTS[0] == 10, \"int vec [0]\");
    assert(len(P175_TEXTS) == 3, \"text vec len = {len(P175_TEXTS)}\");
    assert(P175_TEXTS[0] == \"aa\", \"text vec [0]\");
    assert(P175_TEXTS[2] == \"cc\", \"text vec [2]\");
}"
    )
    .result(Value::Null);
}

/// P178 — slot-aliasing in `is`-capture body.  When a function body's
/// IR root is a `Value::Insert` (not a `Block`/`Loop`), `assign_slots`
/// would return early from `process_scope` without placing any locals.
/// Orphans then flowed into `place_orphaned_vars`, which started
/// candidate slots at 0 — the argument area.  Arguments have
/// `stack_pos == u16::MAX` during `assign_slots` (codegen assigns
/// their positions later), so the per-var conflict check couldn't
/// see them; orphans happily claimed slot 0, 4, 12, ..., overlapping
/// the args at runtime.  Writing to the captured `tb_id` slot
/// corrupted the caller-owned `&tools` ref bytes sharing those
/// offsets; the downstream `SetInt` on a now-null ref silently wrote
/// to store 0 (observed as `ft_cur == 0` instead of the expected
/// captured value).
///
/// Fix: `place_orphaned_vars` now takes `local_start` and starts the
/// candidate slot there, so orphan locals can't overlap the
/// argument + return-address region.
#[test]
fn p178_is_capture_slot_alias() {
    code!(
        "enum P178Ui { P178UhToolButton { tb_id: integer } }
struct P178Tools { ft_cur: integer }
fn p178_hit() -> P178Ui { P178UhToolButton { tb_id: 2 } }
fn p178_router(dummy: integer, tools: P178Tools) -> P178Ui {
    _ = dummy;
    rc = p178_hit();
    if rc is P178UhToolButton { tb_id } {
        tools.ft_cur = tb_id;
    }
    rc
}
fn test() {
    ft = P178Tools { ft_cur: 0 };
    p178_router(99, ft);
    assert(ft.ft_cur == 2, \"captured was {ft.ft_cur} (expected 2)\");
}"
    )
    .result(Value::Null);
}

/// P179 — passing `&struct.field` as a non-sole argument silently
/// corrupted both the `&` destination and the preceding by-value
/// arg.  Fixed by routing non-Var `&T` sources through a work-ref
/// local in `src/parser/mod.rs::convert()`, emitting a
/// `Value::Insert([Set(__ref_N, expr), OpCreateStack(Var(__ref_N))])`
/// that `src/scopes.rs::scan_args` hoists into the enclosing
/// statement list (so the work-ref lives at function scope, not
/// block scope), plus `set_skip_free` to keep the borrowed DbRef
/// from freeing the owning store at scope exit.
///
/// Fixture lives at
/// `tests/lib/p179_ref_field_arg_corrupts_siblings.loft`.
#[test]
fn p179_ref_field_arg_corrupts_sibling() {
    code!(
        "struct P179Inner { pin_n: integer }
struct P179Outer { po_x: P179Inner, po_q: integer }
fn p179_int_ref(n: integer, r: P179Inner) { r.pin_n = n; }
fn test() {
    o = P179Outer { po_x: P179Inner { pin_n: 0 }, po_q: 0 };
    p179_int_ref(42, o.po_x);
    assert(o.po_x.pin_n == 42, \"int+&field: expected 42, got {o.po_x.pin_n}\");
}"
    )
    .result(Value::Null);
}

/// P176 — method-style callee: `fn add(self: Box, x) { self.items += [x]; }`
/// called from a `&Box` parameter caller should compile.  Before the fix,
/// `find_written_vars` did not recurse into the callee's body, so the
/// caller's `&Box` param looked unused and the "Parameter ... has `&`
/// but is never modified" error fired.  Fix: interprocedural
/// `callee_param_writes` analysis in `src/parser/mod.rs`.
#[test]
fn p176_ref_param_method_style_mutation() {
    code!(
        "struct P176Box { items: vector<integer> }
fn p176_add(self: P176Box, x: integer) { self.items += [x]; }
fn p176_caller(b: P176Box) { p176_add(b, 1); }
fn test() {
    bx = P176Box { items: [] };
    p176_caller(bx);
    assert(len(bx.items) == 1, \"expected 1 got {len(bx.items)}\");
}"
    )
    .result(Value::Null);
}

/// P176 — 3-level forwarding: only the innermost callee writes a
/// field, two intermediate callers just forward the value.  The
/// outermost `&T` must still be accepted.  Exercises the fixpoint /
/// monotone-merge code path.
#[test]
fn p176_transitive_forwarding_three_levels() {
    code!(
        "struct P176Tx { val: integer }
fn p176_inner(self: P176Tx)   { self.val = self.val + 1; }
fn p176_mid(self: P176Tx)     { p176_inner(self); }
fn p176_outer(b: P176Tx)     { p176_mid(b); }
fn test() {
    b = P176Tx { val: 0 };
    p176_outer(b);
    assert(b.val == 1, \"expected 1 got {b.val}\");
}"
    )
    .result(Value::Null);
}

/// P176 — recursive self-call must terminate the analysis.  The
/// `callee_param_writes` placeholder breaks cycles before descending
/// into the body.  Without the placeholder, the fn would recurse
/// infinitely while computing its own param-writes.
#[test]
fn p176_recursive_self_call_terminates() {
    code!(
        "struct P176Rec { val: integer }
fn p176_bump(n: P176Rec, depth: integer) {
    n.val = n.val + 1;
    if depth > 0 { p176_bump(n, depth - 1); }
}
fn test() {
    n = P176Rec { val: 0 };
    p176_bump(n, 3);
    assert(n.val == 4, \"expected 4 got {n.val}\");
}"
    )
    .result(Value::Null);
}

/// P180 — assigning a `single` (f32) literal to a `float` (f64) struct
/// field used to be silently accepted and corrupted the record at
/// runtime.  Fix: `src/parser/expressions.rs` now funnels simple-
/// assignment RHS through the same `convert()` machinery the
/// constructor and return-type paths already use, which widens via
/// `OpConvFloatFromSingle` and rejects narrowing with a diagnostic.
#[test]
fn p180_single_literal_into_float_field() {
    code!(
        "struct P180Box { a: float, b: integer }
fn test() {
    p = P180Box { a: 1.0, b: 42 };
    p.a = 1.2f;
    // f32 1.2 widened to f64 is not exactly 1.2; allow a tolerance
    // that covers the unavoidable precision loss.
    assert(p.a > 1.19 && p.a < 1.21, \"expected ~1.2 got {p.a}\");
    assert(p.b == 42, \"b untouched, got {p.b}\");
}"
    )
    .result(Value::Null);
}

/// P180 companion — widening an `integer` RHS into a `long` field
/// still works after the int→long hand-rolled branch in the
/// assignment path was replaced with a generic `convert()` funnel.
/// Guards against regressing the prior auto-widen behaviour.
#[test]
fn p180_int_widens_to_long_field() {
    code!(
        "struct P180Long { n: integer }
fn test() {
    p = P180Long { n: 0 };
    p.n = 42;
    assert(p.n == 42, \"expected 42 got {p.n}\");
}"
    )
    .result(Value::Null);
}

/// P181 — inline struct-returning call inside a format-string
/// interpolation used to SIGSEGV when the call's arg was a
/// field-access expression (not a plain Var) and the callee
/// returned a borrowed view into one of its args.  Root cause:
/// `OpCopyRecord` was emitted with the `0x8000` free-source flag
/// set unconditionally, freeing the view's source store.
///
/// Fix in `src/state/codegen.rs` (two sites — first-assignment and
/// reassignment paths both touch OpCopyRecord): clear the flag
/// when the callee's return type carries a non-empty `dep` chain.
/// Inference already tags these correctly for consistent-view
/// callees; a deeper issue with MIXED-return callees
/// (some paths view, some owned) is tracked separately in
/// `doc/claude/plans/finished/00-inline-lift-safety/01b-return-dep-inference.md`.
///
/// Tests: `tests/lib/p181_inline_field_access.loft`.
#[test]
fn p181_inline_field_access_format_string() {
    code!(
        "struct P181Inner { n: integer }
struct P181Container { items: vector<P181Inner> }
struct P181Holder { c: P181Container, sentinel: integer }
fn p181_first_inner(c: P181Container) -> P181Inner {
    c.items[0]
}
fn test() {
    h = P181Holder {
        c: P181Container { items: [P181Inner { n: 1 }] },
        sentinel: 42,
    };
    assert(p181_first_inner(h.c).n == 1,
           \"inline; got {p181_first_inner(h.c).n}\");
    assert(h.sentinel == 42, \"sentinel preserved; got {h.sentinel}\");
}"
    )
    .result(Value::Null);
}

/// P184: `vector<i32>` honours the `size(4)` annotation on the alias.
/// Before Phase 3, indexing returned `(v[i+1] << 32) | v[i]` because
/// storage was 4-byte stride but `OpGetVector` / `get_val` emitted
/// 8-byte reads.
#[test]
fn p184_vector_i32_narrow_read() {
    code!(
        "struct P184Box { v: vector<i32> }
fn test() {
    b = P184Box { v: [] };
    b.v += [1 as i32, 2 as i32, 3 as i32];
    assert(b.v[0] == 1, \"v[0] expected 1, got {b.v[0]}\");
    assert(b.v[1] == 2, \"v[1] expected 2, got {b.v[1]}\");
    assert(b.v[2] == 3, \"v[2] expected 3, got {b.v[2]}\");
    assert(len(b.v) == 3, \"len expected 3, got {len(b.v)}\");
}"
    )
    .result(Value::Null);
}

/// P184 control: `vector<integer>` still uses 8-byte-stride storage.
#[test]
fn p184_vector_integer_wide_control() {
    code!(
        "struct P184WideBox { v: vector<integer> }
fn test() {
    b = P184WideBox { v: [] };
    b.v += [1, 2, 3];
    assert(b.v[0] == 1, \"v[0] expected 1, got {b.v[0]}\");
    assert(b.v[1] == 2, \"v[1] expected 2, got {b.v[1]}\");
    assert(b.v[2] == 3, \"v[2] expected 3, got {b.v[2]}\");
}"
    )
    .result(Value::Null);
}

/// P184: `vector<u16>` — currently stored wide (8-byte) because
/// `Parts::Short`'s legacy `val - min + 1` encoding diverges from
/// the raw-byte vector copy path; narrow 2-byte storage awaits a
/// later Phase 4 round.  This guard confirms the wide-fallback
/// behaviour is consistent (reads + writes agree) so values
/// round-trip correctly even without the narrowing optimisation.
#[test]
fn p184_vector_u16_round_trip() {
    code!(
        "struct P184U16Box { v: vector<u16> }
fn test() {
    b = P184U16Box { v: [] };
    b.v += [1 as u16, 2 as u16, 300 as u16, 65000 as u16];
    assert(b.v[0] == 1, \"v[0]\");
    assert(b.v[1] == 2, \"v[1]\");
    assert(b.v[2] == 300, \"v[2]\");
    assert(b.v[3] == 65000, \"v[3]\");
}"
    )
    .result(Value::Null);
}

/// P184: `vector<u8>` narrow storage — 1-byte stride.
#[test]
fn p184_vector_u8_narrow_read() {
    code!(
        "struct P184U8Box { v: vector<u8> }
fn test() {
    b = P184U8Box { v: [] };
    b.v += [1 as u8, 2 as u8, 255 as u8];
    assert(b.v[0] == 1, \"v[0]\");
    assert(b.v[1] == 2, \"v[1]\");
    assert(b.v[2] == 255, \"v[2]\");
}"
    )
    .result(Value::Null);
}

/// P184 Phase 5: `vector<i32>` as a LOCAL variable narrows to 4-byte
/// storage.  Before Phase 5 locals fell back to wide (8-byte) storage
/// because `fill_database` only ran on struct definitions — the
/// literal-append path in `parser/vectors.rs::build_vector_code` and
/// `get_type`'s Type::Vector arm used the default wide `integer` slot.
/// Phase 5 routes both paths through `Parser::vector_of` which
/// consults `Data::narrow_vector_content`.
#[test]
fn p184_vector_i32_local_narrow_read() {
    code!(
        "fn test() {
    result: vector<i32> = [];
    result += [1 as i32, 2 as i32, 3 as i32];
    assert(result[0] == 1, \"r[0] expected 1, got {result[0]}\");
    assert(result[1] == 2, \"r[1] expected 2, got {result[1]}\");
    assert(result[2] == 3, \"r[2] expected 3, got {result[2]}\");
    assert(len(result) == 3, \"len\");
}"
    )
    .result(Value::Null);
}

/// P184 Phase 5: `vector<i32>` as a function RETURN type narrows.
/// The caller sees 4-byte-stride storage because `parse_type`'s
/// alias-resolution path stamps forced_size and `get_type` /
/// `vector_of` register the narrow vector db type on demand.
#[test]
fn p184_vector_i32_return_narrow_read() {
    code!(
        "fn make_i32_vec() -> vector<i32> {
    result: vector<i32> = [];
    result += [10 as i32, 20 as i32, 30 as i32];
    result
}
fn test() {
    v = make_i32_vec();
    assert(v[0] == 10, \"v[0]\");
    assert(v[1] == 20, \"v[1]\");
    assert(v[2] == 30, \"v[2]\");
}"
    )
    .result(Value::Null);
}

/// P184 Phase 6: `hash<Row[narrow_key]>` already works via the
/// struct-field narrowing path — the key field's narrow storage
/// is handled by the same `fill_database` Integer arm that
/// Phase 0 extended.  Primitive-content `hash<i32>` /
/// `sorted<i32>` / `index<i32>` are parse errors (the grammar
/// requires a `[key]` suffix), so Phase 6 has no new narrowing
/// code to add.  This guard locks down the happy path:
/// `hash` + `sorted` collections with `u32`-typed key fields.
#[test]
fn p184_hash_sorted_narrow_key_field() {
    code!(
        "struct P184Row { rid: u32, name: text }
struct P184HashDb { rows: hash<P184Row[rid]> }
struct P184SortedDb { rows: sorted<P184Row[rid]> }
fn test() {
    h = P184HashDb { rows: [] };
    h.rows += [P184Row { rid: 42, name: \"forty-two\" }];
    h.rows += [P184Row { rid: 7, name: \"seven\" }];
    assert(h.rows[42].name == \"forty-two\", \"hash[42]\");
    assert(h.rows[7].name == \"seven\", \"hash[7]\");
    s = P184SortedDb { rows: [] };
    s.rows += [P184Row { rid: 3, name: \"three\" }];
    s.rows += [P184Row { rid: 1, name: \"one\" }];
    s.rows += [P184Row { rid: 2, name: \"two\" }];
    assert(s.rows[1].name == \"one\", \"sorted[1]\");
    assert(s.rows[2].name == \"two\", \"sorted[2]\");
    assert(s.rows[3].name == \"three\", \"sorted[3]\");
}"
    )
    .result(Value::Null);
}

/// P293 regression — `hash<Row[i32_field]>` lookup silently returned
/// null because (a) `determine_keys` mapped any non-built-in content
/// type (including Parts::Int) to `type_nr = 7` (the byte fallback),
/// (b) `read_key`'s catch-all popped 1 byte off the stack while the
/// lookup value was pushed as a full i64, and (c) the `hash_ref` /
/// `compare_key` / `get_key` paths in keys.rs only knew about the
/// legacy 8-byte `integer` storage.  Fixed by extending all four
/// paths to recognise Parts::Int / Short / ShortRaw / Byte.  Same
/// bug surfaced from `f#read as u32` (the original P293 report) once
/// `u32` was given `size(4)` to make file-I/O width predictable; the
/// narrow-key hash fix landed alongside.
#[test]
fn p293_narrow_key_hash_lookup() {
    code!(
        "struct P293Row { rid: i32, name: text }
struct P293Db { rows: hash<P293Row[rid]> }
fn test() {
    h = P293Db { rows: [] };
    h.rows += [P293Row { rid: 42, name: \"forty-two\" }];
    h.rows += [P293Row { rid: 7, name: \"seven\" }];
    h.rows += [P293Row { rid: 100, name: \"hundred\" }];
    found42 = h.rows[42];
    found7  = h.rows[7];
    foundX  = h.rows[100];
    miss    = h.rows[999];
    assert(found42 != null, \"42 present\");
    assert(found7  != null, \"7 present\");
    assert(foundX  != null, \"100 present\");
    assert(miss    == null, \"999 absent\");
    assert(found42.name == \"forty-two\", \"hash[42] value\");
    assert(found7.name  == \"seven\",     \"hash[7] value\");
    assert(foundX.name  == \"hundred\",   \"hash[100] value\");
}"
    )
    .result(Value::Null);
}

/// P284 — `for f in vector<float>` looped forever past the end yielding
/// a garbage subnormal (~2.8e-282).  Root cause: `Store::get_float` /
/// `get_single` skipped the `rec != 0` guard that `get_int` already has,
/// so a null DbRef (rec=0, returned by `OpGetVectorNullable` for OOB)
/// read `*addr(0, 0)` (the store's free-list header) in release mode —
/// the `valid()` asserts inside are debug-only.  Fixed by adding the
/// `rec != 0` guard to both float getters; null DbRefs now return
/// `f64::NAN` / `f32::NAN`, the for-loop's value-truthiness check then
/// evaluates to false and breaks.
#[test]
fn p284_vector_float_iteration_terminates() {
    code!(
        "fn test() {
    v: vector<float> = [1.0, 2.0, 3.0];
    sum = 0.0;
    count = 0;
    for f in v {
        sum = sum + f;
        count = count + 1;
        if count > 100 { return; }
    }
    assert(count == 3, \"loop terminates after 3 elements\");
    assert(sum > 5.9 && sum < 6.1, \"sum approximately 6.0\");
    sv: vector<single> = [1.0f, 2.0f, 3.0f];
    sc = 0;
    for _ in sv {
        sc = sc + 1;
        if sc > 100 { return; }
    }
    assert(sc == 3, \"single iteration terminates after 3 elements\");
}"
    )
    .result(Value::Null);
}

/// P277 — local `sorted<T[K]> = []; += [T{…}]` panicked with
/// "Variable 'x' cannot change type from sorted<…> to vector<…>" —
/// `parse_vector` re-typed the LHS local to vector<T> before
/// `parse_assign_op` could route through the keyed-collection element
/// dispatch.  Same shape would have affected hash / index / spatial
/// locals.  Fixed by an early intercept in `parse_assign_op` that
/// detects `local_keyed += [literal]` BEFORE `parse_operators` runs,
/// then per-element parses + dispatches via `new_record` (which
/// already routes per-kind via the P188-followup `lhs_known` lookup).
#[test]
fn p277_local_sorted_pluseq_single_literal() {
    code!(
        "struct TagSlot { name: text, count: integer }
fn test() {
    s: sorted<TagSlot[name]> = [];
    s += [TagSlot{name: \"alpha\", count: 1}];
    assert(len(s) == 1, \"len == 1\");
    assert(s[\"alpha\"].count == 1, \"alpha lookup\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p277_local_sorted_pluseq_multi_literal() {
    code!(
        "struct TagSlot { name: text, count: integer }
fn test() {
    s: sorted<TagSlot[name]> = [];
    s += [TagSlot{name: \"zeta\", count: 1},
          TagSlot{name: \"alpha\", count: 5},
          TagSlot{name: \"mike\", count: 3}];
    assert(len(s) == 3, \"three elements\");
    // sorted by name ascending — iterate and collect in order.
    out: vector<text> = [];
    for t in s { out += [t.name]; }
    assert(out[0] == \"alpha\", \"first = alpha\");
    assert(out[1] == \"mike\",  \"middle = mike\");
    assert(out[2] == \"zeta\",  \"last = zeta\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p277_local_hash_pluseq_multi_literal() {
    code!(
        "struct Row { id: integer, name: text }
fn test() {
    h: hash<Row[id]> = [];
    h += [Row{id: 1, name: \"one\"},
          Row{id: 7, name: \"seven\"},
          Row{id: 42, name: \"forty-two\"}];
    assert(len(h) == 3, \"three entries\");
    assert(h[1].name  == \"one\",       \"h[1]\");
    assert(h[7].name  == \"seven\",     \"h[7]\");
    assert(h[42].name == \"forty-two\", \"h[42]\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p277_local_index_pluseq_multi_literal() {
    code!(
        "struct Score { player: text, value: integer }
fn test() {
    ix: index<Score[player]> = [];
    ix += [Score{player: \"a\", value: 10},
           Score{player: \"b\", value: 20},
           Score{player: \"c\", value: 30}];
    assert(len(ix) == 3, \"three entries\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p277_local_sorted_mixed_scalar_and_literal() {
    code!(
        "struct TagSlot { name: text, count: integer }
fn test() {
    s: sorted<TagSlot[name]> = [];
    // Scalar `+= elem` (P188 path) — must still work.
    s += TagSlot{name: \"alpha\", count: 1};
    // Literal `+= [...]` (P277 path).
    s += [TagSlot{name: \"beta\", count: 2}, TagSlot{name: \"gamma\", count: 3}];
    // Another scalar — paths interleave.
    s += TagSlot{name: \"delta\", count: 4};
    assert(len(s) == 4, \"four entries from interleaved scalar + literal\");
    assert(s[\"alpha\"].count == 1, \"alpha\");
    assert(s[\"beta\"].count  == 2, \"beta\");
    assert(s[\"gamma\"].count == 3, \"gamma\");
    assert(s[\"delta\"].count == 4, \"delta\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p277_local_sorted_pluseq_empty_literal() {
    // Defensive regression — `+= []` is a no-op append.
    code!(
        "struct TagSlot { name: text, count: integer }
fn test() {
    s: sorted<TagSlot[name]> = [];
    s += [TagSlot{name: \"a\", count: 1}];
    s += [];
    s += [TagSlot{name: \"b\", count: 2}];
    assert(len(s) == 2, \"empty append is no-op\");
}"
    )
    .result(Value::Null);
}

/// @P300 — assigning the result of a call that RETURNS a keyed
/// collection to a local (`x = mk()`) used to panic in codegen
/// (`Incorrect var x[65535]`): the @P295 reassignment lowering fired
/// for the FIRST assignment too and emitted `Insert([OpReplaceKeyed])`
/// with no `Set` node, so `compute_intervals` recorded no `first_def`
/// and `x` got no stack slot.  Fixed by prepending `Set(x, Null)` in
/// the keyed-assignment branch (`parser/expressions.rs`); `scan_set`
/// keeps it on a first assignment (→ store init) and elides it on a
/// reassignment (→ bare `OpReplaceKeyed`).
#[test]
fn p300_hash_return_assign_untyped() {
    code!(
        "struct R { ck: integer, v: integer }
fn mk() -> hash<R[ck]> { h: hash<R[ck]> = []; h += [R{ck: 5, v: 9}]; h }
fn test() {
    x = mk();
    assert(len(x) == 1, \"len 1\");
    assert(x[5].v == 9, \"key 5 -> 9\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p300_hash_return_assign_typed() {
    code!(
        "struct R { ck: integer, v: integer }
fn mk() -> hash<R[ck]> { h: hash<R[ck]> = []; h += [R{ck: 5, v: 9}]; h }
fn test() {
    x: hash<R[ck]> = mk();
    assert(len(x) == 1, \"len 1\");
    assert(x[5].v == 9, \"key 5 -> 9\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p300_sorted_return_assign() {
    code!(
        "struct Item { k: integer }
fn build() -> sorted<Item[k]> { s: sorted<Item[k]> = []; s += [Item{k: 3}]; s += [Item{k: 1}]; s += [Item{k: 2}]; s }
fn test() {
    x = build();
    assert(len(x) == 3, \"len 3\");
    out = \"\";
    for it in x { out += \"{it.k} \"; }
    assert(out == \"1 2 3 \", \"ordered: {out}\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p300_index_return_assign() {
    code!(
        "struct IRec { n: integer }
fn build() -> index<IRec[n]> { ix: index<IRec[n]> = []; ix += [IRec{n: 7}]; ix += [IRec{n: 3}]; ix }
fn test() {
    x = build();
    assert(len(x) == 2, \"len 2\");
}"
    )
    .result(Value::Null);
}

/// @P300 sibling — first assignment from another keyed LOCAL (Var-RHS
/// alias, no prior declaration of `x`).  Same no-slot panic shape; the
/// deep-copy gives `x` its own store so `ns` stays usable.
#[test]
fn p300_var_rhs_alias_first() {
    code!(
        "struct R { ck: integer, v: integer }
fn build() -> hash<R[ck]> { h: hash<R[ck]> = []; h += [R{ck: 1, v: 10}]; h += [R{ck: 2, v: 20}]; h }
fn test() {
    ns: hash<R[ck]> = build();
    x = ns;
    assert(len(x) == 2, \"x len 2\");
    assert(len(ns) == 2, \"ns still readable\");
    assert(x[1].v == 10 && ns[2].v == 20, \"both independent\");
}"
    )
    .result(Value::Null);
}

/// P279 — caller defined BEFORE a text-returning callee in the
/// same file: pass-1 types the call result as Unknown(0), the
/// receiving local is Unknown, and using it as a struct-field
/// value (`out += [Struct{field: unknown_local, …}]`) trips
/// "Cannot assign unknown(0) to field …" in pass-1.  Pass-2
/// re-runs the check with the fn registered and would have
/// resolved correctly, but the pass-1 diagnostic surfaces.
///
/// Same architectural shape as P281/P278: pass-1 mustn't emit
/// errors pass-2 would naturally resolve.  Fix in
/// `src/parser/objects.rs::handle_field` gates the diagnostic
/// on `!first_pass` when the value type is Unknown.
#[test]
fn p279_forward_text_fn_into_struct_field() {
    code!(
        "struct R { name: text }
fn test() {
    out: vector<R> = [];
    s = forward_fn(\"hello\");
    out += [R{name: s}];
    assert(len(out) == 1, \"struct constructed\");
    assert(out[0].name == \"hello\", \"forward-fn text reached struct field\");
}
fn forward_fn(s: text) -> text { s }"
    )
    .result(Value::Null);
}

#[test]
fn p279_forward_fn_via_intermediate_local() {
    // Real scan_link_line shape: while loop + if-arm reassign +
    // forward call wrapped in an intermediate local before the
    // struct-field init.
    code!(
        "struct R { name: text }
fn test() {
    out: vector<R> = [];
    line = \"x#tag\";
    nl = line.len();
    name = \"\";
    lh = 0;
    while lh < nl {
        if line.byte_at(lh) == 35 {
            name = line[lh + 1..nl];
            break;
        }
        lh = lh + 1;
    }
    a_esc = json_escape(name);
    out += [R{name: a_esc}];
    assert(len(out) == 1, \"loop + if-reassign + forward-fn + struct works\");
    assert(out[0].name == \"tag\", \"correct slice + forward-fn\");
}
fn json_escape(s: text) -> text { s }"
    )
    // The shape under test is the codegen one (loop + if-arm reassign + forward
    // call + struct field), so the source stays exactly as `scan_link_line` wrote
    // it — including `nl = line.len()` used as a slice bound.  That mixes units
    // (loft#749: `len()` counts characters, a slice bound is a byte offset) and
    // the @PLN110 lint now says so.  It is right to: this only computes "tag"
    // because the literal is ASCII.  Declared rather than corrected — rewriting
    // the source to `size(line)` would quietly change the shape this test exists
    // to pin.
    .warning(
        "a text slice ends at `len(text)` (a character count) but slice bounds are byte \
         offsets — this stops short on multi-byte text at \
         p279_forward_fn_via_intermediate_local:10:36",
    )
    .result(Value::Null);
}

/// P281 — caller defined BEFORE callee in the same file: the
/// caller's body parses with the callee unregistered, the call
/// returns `Type::Unknown(0)`, the receiving local stays Unknown,
/// and any `.method(args)` chain on it trips "Expect token ;"
/// in pass-1 (because pass-1's `field()` Unknown-receiver branch
/// consumed the method name but not the trailing `(args)`).
///
/// The two-pass parser ALREADY has the right architecture — pass
/// 2 sees every fn registered and re-parses bodies cleanly.  The
/// fix in `src/parser/fields.rs::field` makes pass-1 also consume
/// the `(args)` so no spurious pass-1 parse error escapes.
#[test]
fn p281_forward_text_returning_fn_method_chain() {
    code!(
        "fn test() {
    s = helper(\"foo.loft\");
    assert(s.len() == 3, \"forward text fn + .len() on receiver works\");
}
fn helper(s: text) -> text {
    i = s.find(\".\");
    if i == null { s } else { s[0..i] }
}"
    )
    .result(Value::Null);
}

#[test]
fn p281_forward_text_returning_fn_method_with_args() {
    code!(
        "fn test() {
    s = upper(\"abc\");
    n = s.find(\"BC\");
    assert(n == 1, \"forward fn + .find(arg) returns position 1\");
}
fn upper(s: text) -> text {
    out = \"\";
    for c in s { out = out + (c as integer - 32) as character; }
    out
}"
    )
    .result(Value::Null);
}

#[test]
fn p281_mutual_forward_text_fns() {
    code!(
        "fn test() {
    s = first(\"hello\");
    assert(s.len() == 5, \"first + second + len chain works\");
}
fn first(s: text) -> text {
    second(s)
}
fn second(s: text) -> text {
    s
}"
    )
    .result(Value::Null);
}

#[test]
fn p281_forward_fn_slice_on_text_return() {
    // P278/P281 sibling — slice expression on forward-text-fn result.
    // Was the second cascade after my P281 fix; tolerance extended to
    // parse_index for Unknown receivers covers `[a..b]` shapes too.
    code!(
        "fn test() {
    s = forward(\"hello world\");
    sp = s.find(\" \");
    if sp != null {
        first = s[0..sp];
        assert(first == \"hello\", \"slice on forward-text-fn result works\");
    }
}
fn forward(s: text) -> text {
    s
}"
    )
    .result(Value::Null);
}

#[test]
fn p281_forward_fn_in_println_format() {
    // Forward fn used inside a format string interpolation —
    // pass-1's body parse must also tolerate Unknown receivers
    // inside the format-string expression position.
    code!(
        "fn test() {
    out = \"len={shorten(\\\"hello world\\\").len()}\";
    assert(out == \"len=5\", \"format interp with forward fn + .len() works\");
}
fn shorten(s: text) -> text {
    sp = s.find(\" \");
    if sp == null { s } else { s[0..sp] }
}"
    )
    .result(Value::Null);
}

/// P292 — `v = new_v` where `new_v` is loop-scoped and `v` is outer-scoped
/// used to corrupt `v`'s storage on the next iteration: subsequent reads
/// of `v[j]` reported `index J out of bounds for length 0` (or SEGV when
/// the next iter's `new_v: vector<integer> = []` allocation landed on the
/// just-freed storage).  The accumulator pattern
/// `v: vector<T> = []; for i { new_v: vector<T> = []; for j { new_v += [v[j]] }; new_v += [i]; v = new_v }`
/// is the canonical reproducer.  Fixed by extending the field-replace
/// `OpClearVector + OpAppendVector` path to cover local-var vector
/// re-assignment when the RHS is a Var read (the only shape that aliases —
/// fresh-storage calls / comprehensions go through the standard Set path).
#[test]
fn p292_vector_reassign_from_loop_local() {
    code!(
        "fn test() {
    v: vector<integer> = [];
    v += [10];
    for i in 0..3 {
        new_v: vector<integer> = [];
        for j in 0..len(v) { new_v += [v[j]]; }
        new_v += [i];
        v = new_v;
    }
    assert(len(v) == 4, \"v has 4 elements after 3 iters of (copy + append)\");
    assert(v[0] == 10, \"v[0] = 10 (initial)\");
    assert(v[1] == 0,  \"v[1] = 0 (iter 0)\");
    assert(v[2] == 1,  \"v[2] = 1 (iter 1)\");
    assert(v[3] == 2,  \"v[3] = 2 (iter 2)\");
}"
    )
    .result(Value::Null);
}

/// @P295 — reassigning a keyed-collection LOCAL (`s = ns`) for
/// sorted/hash/index.  Before the fix this panicked in codegen
/// (`gen_put_var` has no `OpPut*` arm for keyed kinds).  Fixed by
/// emitting a deep-copy `OpReplaceKeyed` (remove_claims + copy_claims,
/// per-kind index rebuild) and stripping the `s["ns"]` lifetime dep so
/// scope analysis frees both `s` (its own copy) and `ns` (its scope).
/// The loop-rebuild shape (insertion-sort idiom) is the canonical case;
/// it must not accumulate across iterations.  This `code!` harness runs
/// interp; `tests/scripts/119-keyed-local-reassign.loft` covers both
/// backends (native keyed locals were unblocked by the @P296 fix).
#[test]
fn p295_sorted_reassign_from_loop_local() {
    code!(
        "struct Item { k: integer }
fn test() {
    s: sorted<Item[k]> = [];
    s += [Item{k: 100}];
    for i in 1..5 {
        ns: sorted<Item[k]> = [];
        for it in s { ns += [Item{k: it.k}]; }
        ns += [Item{k: i}];
        s = ns;
    }
    out = \"\";
    for it in s { out += \"{it.k} \"; }
    assert(out == \"1 2 3 4 100 \", \"sorted rebuild: {out}\");
}"
    )
    .result(Value::Null);
}

/// @P295 — hash + index variants of the keyed-local reassignment, plus
/// the fresh-storage call RHS (`s = build()`) that exercises the
/// 0x8000 source-free path.
#[test]
fn p295_hash_index_reassign() {
    code!(
        "struct H { k: text, v: integer }
struct I { n: integer }
fn test() {
    h: hash<H[k]> = [];
    h += [H{k: \"a\", v: 1}];
    nh: hash<H[k]> = [];
    nh += [H{k: \"a\", v: 1}];
    nh += [H{k: \"b\", v: 2}];
    h = nh;
    assert(len(h) == 2, \"hash reassign len\");
    assert(h[\"b\"].v == 2, \"hash reassign lookup\");

    ix: index<I[n]> = [];
    ix += [I{n: 5}];
    nx: index<I[n]> = [];
    nx += [I{n: 3}];
    nx += [I{n: 7}];
    ix = nx;
    assert(len(ix) == 2, \"index reassign len\");
}"
    )
    .result(Value::Null);
}

/// @P285 — a keyed-collection membership test (`hash[key] == null`) must
/// NOT fire the "Redundant null check" warning when the KEY is a
/// `not null` field.  The lookup RESULT is nullable (absent key → null);
/// the bug attributed the key's not-null-ness to the comparison.  The
/// test declares NO expected warnings, so the harness's
/// `assert_diagnostics` fails if any spurious warning is emitted.
#[test]
fn p285_hash_lookup_null_no_spurious_warning() {
    code!(
        "struct P285Ent { name: text, v: integer }
struct P285Box { items: hash<P285Ent[name]> }
fn test() {
    b = P285Box{items: []};
    b.items += [P285Ent{name: \"x\", v: 9}];
    key = P285Ent{name: \"x\", v: 0};
    miss = P285Ent{name: \"y\", v: 0};
    found = 0;
    if b.items[key.name] == null { found = -1; } else { found = b.items[key.name].v; }
    if b.items[miss.name] != null { found = found + 100; }
    assert(found == 9, \"present + absent membership tests resolve correctly\");
}"
    )
    .result(Value::Null);
}

/// @P285 control — a GENUINE redundant check (`not_null_field == null`,
/// no lookup) must STILL warn.  Guards against the fix over-suppressing.
/// A narrow `u8` field, because that is a slot with no null (C127); a plain
/// `text` or `integer` field holds the null `(N-Store)` lets into it, so the
/// same check there is a real test (`Type::non_null_slot_reads_null`).
#[test]
fn p285_genuine_redundant_check_still_warns() {
    code!(
        "struct P285G { level: u8 }
fn test() {
    g = P285G{level: 3};
    if g.level == null { assert(false, \"unreachable\"); }
}"
    )
    .warning("Redundant null check — 'level' is 'not null', so this is false unless a null reached the slot anyway (an overflow, a NaN, or an out-of-range read) at p285_genuine_redundant_check_still_warns:4:25")
    .result(Value::Null);
}

/// @PLN102 — the redundant-null-check warning must NOT fire on a field of a
/// NULLABLE receiver: `s.name` where `s: S?` reads null when `s` is absent (C80),
/// so `s.name == null` is a genuine check, not "always false".  No `.warning()` —
/// an unexpected diagnostic fails the harness, so this guards the suppression (the
/// p285 test above guards the complement: a non-null receiver still warns).
#[test]
fn nullable_receiver_field_null_check_no_warning() {
    code!(
        "struct NRF { name: text }
fn test() {
    s: NRF? = null;
    if s.name == null { assert(true, \"reachable\"); }
}"
    )
    .result(Value::Null);
}

/// P185 — slot-aliasing bug: a local (`key`) declared AFTER an inner
/// `body += <format-string>` accumulator loop, inside an outer
/// `for _ in file(...).files()` that uses an inline temporary as the
/// iterator source, gets assigned a slot that overlaps a still-live
/// text buffer.  Scope teardown runs `OpFreeText` (fill.rs op 118)
/// on the aliased slot → SIGSEGV or `realloc(): invalid pointer`.
///
/// Two independent workarounds (each alone suppresses the crash):
///   (a) hoist `key` above the inner loop;
///   (b) hoist `file(...)` into a named variable `d`.
///
/// Same class as P178 (orphan-placer reused argument slots).  The
/// structural fix is a rework of slot allocation — see
/// `doc/claude/plans/finished/04-slot-assignment-redesign/`.
///
/// The test depends on `tests/docs/*.loft` existing (it does in-tree)
/// because the bug requires a real `file(...).files()` iterator —
/// synthetic vector iteration doesn't reproduce the slot overlap.
// The slice bound discharges its `find` (`?? 0`): since @PLN153 phase 3 a nullable INDEX is
// an `(N-Store)` slot and warns, and this fixture expects no diagnostics.
#[test]
fn p185_slot_alias_on_late_local_in_nested_for() {
    code!(
        "fn test() {
    out = file(\"/tmp/p185_out.txt\");
    for f in file(\"tests/docs\").files() {
        path = \"{f.path}\";
        if !path.ends_with(\".loft\") or path.ends_with(\"/.loft\") { continue; }
        body = \"\";
        for i in 0..3 {
            body += \"{i}\";
        }
        key = path[(path.find(\"/\") ?? 0) + 1..path.len() - 5];
        out += `
          {key}
        `;
        break;
    }
}"
    )
    .result(Value::Null);
}

/// P186 — struct-typed block / if expressions rejected as `void`.
///
/// `x = { S { ... } }`, `x = { mk() }` (where `mk()` returns a struct),
/// `if cond { S {…} } else { S {…} }`, and blocks with intermediate
/// statements before a struct-literal result all used to fail with
/// `Variable 'x' cannot change type from void to S` (CLI) or a
/// cascade of `Unknown type void` errors (test harness).
///
/// Root cause: `parse_object` in first_pass returns
/// `Type::Rewritten(Type::Reference(_))` with `*code = Value::Insert(…)`
/// because the struct-init IR can't be fully materialised until type
/// resolution in second_pass.  `parse_block`'s Insert-flattening then
/// unconditionally reset `t = Type::Void`, discarding the Rewritten
/// tag, so the block's inferred type was `void` in first_pass.
/// Second_pass then produced the real `Reference(S)` and
/// `change_var_type` fired the "cannot change type from void" error.
///
/// Fix: `src/parser/control.rs::parse_block` preserves the Rewritten
/// type across Insert flattening.  A companion fix in the same
/// function disambiguates struct-body `{ field: val, … }` from
/// block-expression `{ expr }` by peeking the first two tokens after
/// `{` — only `ident :` / `ident ,` route to `parse_object`.
///
/// The four shapes below reproduce the four PROBLEMS.md variants;
/// they must all compile and execute cleanly.
#[test]
fn p186_struct_typed_block_expressions() {
    code!(
        "struct P04Sb { sb_a: integer, sb_b: integer }
fn p04_mkbox(n: integer) -> P04Sb { P04Sb { sb_a: n, sb_b: n * 2 } }
fn test() {
    b1 = { P04Sb { sb_a: 3, sb_b: 4 } };
    assert(b1.sb_a == 3 and b1.sb_b == 4, \"b1\");
    b2 = { n = 5; P04Sb { sb_a: n, sb_b: n + 1 } };
    assert(b2.sb_a == 5 and b2.sb_b == 6, \"b2\");
    b3 = { p04_mkbox(7) };
    assert(b3.sb_a == 7 and b3.sb_b == 14, \"b3\");
    cond = true;
    b4 = if cond { P04Sb { sb_a: 1, sb_b: 2 } } else { P04Sb { sb_a: 0, sb_b: 0 } };
    assert(b4.sb_a == 1 and b4.sb_b == 2, \"b4\");
}"
    )
    .result(Value::Null);
}

/// P187 — struct scalar fields read as corrupt after a later vector
/// allocation in a sibling function.
///
/// Surface symptom (Brick Buster):
///   `graphics::create_sprite_sheet(atlas, 4, 5, graphics::painter_vao(p))`
///   receives `atlas` with `width=null`, `height=null`, `data.len=0`,
///   even though the caller just printed the same struct with correct
///   values (width=128, height=160, data.len=20480).  `gl_upload_canvas`
///   sees `w=0, h=0, count=1` and bails.  The entire title / HUD / text
///   rendering disappears.
///
/// Irreducible core (this test):
///   1. A builder returns a struct whose `vector<T>` field was
///      populated via a for-comprehension (`result.data = [for _ in ..]`).
///   2. A mutator method on that returned struct is called inside the
///      builder before `return`.
///   3. A subsequent sibling function allocates a local vector of its
///      own (any element type).
///
/// After those three things, `atlas.width` — a scalar field set at
/// struct-literal time — reads as a corrupt value.
///
/// Drop any of the three and the bug disappears: no comprehension,
/// no mutator call, or no post-return vector allocation → correct read.
///
/// Likely root cause: P184's narrow collection storage interacting
/// with store reallocation triggered by the later vector literal.
/// The returned struct's record is relocated but the caller's local
/// still references the old (stale) address, so the width read walks
/// into unrelated bytes.  See `doc/claude/PROBLEMS.md` § P187.
#[test]
fn p187_struct_scalar_field_corrupted_after_sibling_vector_alloc() {
    code!(
        "struct P187Canvas {
    width: integer,
    height: integer,
    data: vector<integer>
}
fn p187_canvas() -> P187Canvas {
    result = P187Canvas { width: 128, height: 160 };
    result.data = [for _ in 0..10 { 0 }];
    result
}
fn touch(self: P187Canvas) {
    self.data[0] = 42;
}
fn p187_build() -> P187Canvas {
    at = p187_canvas();
    at.touch();
    at
}
fn p187_alloc_local_vec() -> integer {
    a = [0.0f, 0.0f];
    a.len()
}
fn test() {
    atlas = p187_build();
    _unused = p187_alloc_local_vec();
    assert(atlas.width == 128, \"width={atlas.width}, expected 128\");
    assert(atlas.height == 160, \"height={atlas.height}, expected 160\");
    assert(atlas.data.len() == 10, \"data.len={atlas.data.len()}, expected 10\");
}"
    )
    .result(Value::Null);
}
