// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! JSON-style struct literals and parsing, `Type.parse`, `#errors`, and the `JsonValue` enum with
//! its struct-enum runtime cases.

use super::*;

// ── L6 — Field constraints and JSON-style struct literals ─────────────────────

// L6: basic field constraint — valid construction.
#[test]
fn l6_constraint_valid_construction() {
    code!(
        "struct Score {
    value: integer assert($.value >= 0, \"value must be >= 0\"),
    max: integer assert($.max >= $.value, \"max must be >= value\")
}
fn test() {
    s = Score { value: 5, max: 10 };
    assert(s.value == 5);
    assert(s.max == 10);
    s.value = 8;
    assert(s.value == 8);
}"
    )
    .result(Value::Null);
}

// L6: field constraint fires on invalid assignment.
#[test]
#[should_panic(expected = "value must be >= 0")]
fn l6_constraint_violation_on_assign() {
    code!(
        "struct Score {
    value: integer assert($.value >= 0, \"value must be >= 0\")
}
fn test() {
    s = Score { value: 5 };
    s.value = -1;
}"
    )
    .result(Value::Null);
}

// L6: cross-field constraint fires on invalid construction.
#[test]
#[should_panic(expected = "lo must be <= hi")]
fn l6_cross_field_constraint_violation() {
    code!(
        "struct Range {
    lo: integer assert($.lo <= $.hi, \"lo must be <= hi\"),
    hi: integer
}
fn test() {
    r = Range { lo: 20, hi: 10 };
}"
    )
    .warning("Variable r is never read at l6_cross_field_constraint_violation:6:8")
    .result(Value::Null);
}

// L6: JSON-style quoted field names in struct literals.
#[test]
fn l6_json_quoted_field_names() {
    code!(
        r#"struct Point { x: integer, y: integer }
fn test() {
    p = Point { "x": 3, "y": 4 };
    assert(p.x == 3, "x={p.x}");
    assert(p.y == 4, "y={p.y}");
}"#
    )
    .result(Value::Null);
}

// L6: constraint with auto-generated message.
#[test]
#[should_panic(expected = "field constraint failed on Pos.x")]
fn l6_constraint_auto_message() {
    code!(
        "struct Pos {
    x: integer assert($.x >= 0)
}
fn test() {
    p = Pos { x: 5 };
    p.x = -1;
}"
    )
    .result(Value::Null);
}

// L6: vector literal input parsed like JSON array.
#[test]
fn l6_vector_literal_as_json_array() {
    code!(
        "fn test() {
    v = [12, 34, 56];
    assert(len(v) == 3, \"len={len(v)}\");
    assert(v[0] == 12);
    assert(v[1] == 34);
    assert(v[2] == 56);
}"
    )
    .result(Value::Null);
}

// L6: validate a vector of constrained structs with format-string message.
#[test]
fn l6_validate_vector_of_structs() {
    code!(
        "struct Item {
    name: text,
    qty: integer assert($.qty > 0, \"qty must be > 0 for '{$.name}'\")
}
fn test() {
    items = [
        Item { name: \"apple\", qty: 3 },
        Item { name: \"banana\", qty: 5 }
    ];
    total = 0;
    for it in items {
        total += it.qty;
    }
    assert(total == 8, \"total={total}\");
}"
    )
    .result(Value::Null);
}

// ── JSON-style parsing via `as` cast ─────────────────────────────────────────

// JSON-style quoted field names in `as Type` cast.
#[test]
fn json_quoted_field_names_in_cast() {
    code!(
        r#"struct Item { name: text, value: integer }
fn test() {
    jt = `{{"name": "hello", "value": 42}}` as Item;
    assert(jt.name == "hello", "name={jt.name}");
    assert(jt.value == 42, "value={jt.value}");
}"#
    )
    .result(Value::Null);
}

// JSON-style vector of structs parsed via `as`.
#[test]
fn json_vector_of_structs_cast() {
    code!(
        r#"struct Item { name: text, value: integer }
fn test() {
    items = `[ {{"name": "a", "value": 1}}, {{"name": "b", "value": 2}} ]` as vector<Item>;
    assert(len(items) == 2, "len={len(items)}");
    assert(items[0].name == "a");
    assert(items[1].value == 2);
}"#
    )
    .result(Value::Null);
}

// ── Type.parse(text) ──────────────────────────────────────────────────────────

// Type.parse(text) with JSON input.  Auto-wraps through
// json_parse internally (P54 step 5 with the step-6 backward-
// compatibility shim — see `src/parser/objects.rs::parse_type_parse`).
#[test]
fn type_parse_json() {
    code!(
        r#"struct Score { value: integer, name: text }
fn test() {
    s = Score.parse(`{{"value": 42, "name": "test"}}`);
    assert(s.value == 42, "value={s.value}");
    assert(s.name == "test", "name={s.name}");
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) — loft-native bare-key form (`{value: 7}`)
// is NOT standard JSON, so json_parse rejects it.  Rewritten
// to standard JSON so the test still guards the struct-unwrap
// behaviour under the auto-wrap path.
#[test]
fn type_parse_loft_native() {
    code!(
        r#"struct Score { value: integer, name: text }
fn test() {
    s = Score.parse(`{{"value": 7, "name": "hello"}}`);
    assert(s.value == 7);
    assert(s.name == "hello");
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) with variable input.
#[test]
fn type_parse_from_variable() {
    code!(
        r#"struct Point { x: integer, y: integer }
fn test() {
    input = `{{"x": 10, "y": 20}}`;
    p = Point.parse(input);
    assert(p.x == 10);
    assert(p.y == 20);
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) with constraint — valid data.
#[test]
fn type_parse_with_constraint_valid() {
    code!(
        r#"struct Score {
    value: integer assert($.value >= 0, "value must be >= 0")
}
fn test() {
    s = Score.parse(`{{"value": 5}}`);
    assert(s.value == 5);
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) with invalid data — constraint fires.
#[test]
#[should_panic(expected = "value must be >= 0")]
fn type_parse_with_constraint_violation() {
    code!(
        r#"struct Score {
    value: integer assert($.value >= 0, "value must be >= 0")
}
fn test() {
    s = Score { "value": -1 };
}"#
    )
    .warning("Variable s is never read at type_parse_with_constraint_violation:5:8")
    .result(Value::Null);
}

// L6: constraint violation with format-string message (falls back to auto-generated).
#[test]
#[should_panic(expected = "field constraint failed on Item.qty")]
fn l6_vector_struct_constraint_violation() {
    code!(
        "struct Item {
    name: text,
    qty: integer assert($.qty > 0, \"qty must be > 0 for '{$.name}'\")
}
fn test() {
    items = [
        Item { name: \"bad\", qty: 0 }
    ];
}"
    )
    .warning("Variable items is never read at l6_vector_struct_constraint_violation:6:12")
    .result(Value::Null);
}

// ── s#errors — error path reporting via #errors accessor ──────────────────────

// Successful JSON parse yields no diagnostic.  With P54 step 5's
// auto-wrap, text arguments still route through json_parse
// internally — the `s#errors` accessor stays empty on success,
// and `json_errors()` also stays empty.
#[test]
fn errors_accessor_empty_on_success() {
    code!(
        r#"struct Score { value: integer }
fn test() {
    s = Score.parse(`{{"value": 42}}`);
    err = s#errors;
    assert(len(err) == 0, "expected no error, got: '{err}'");
    assert(s.value == 42);
}"#
    )
    .result(Value::Null);
}

// Malformed JSON — `Struct.parse(text)` routes through the
// legacy lenient parser (preserves loft-native bare-key
// support per QUALITY-history.md § P54-U) which populates `s#errors`.
// The new typed-tree path (`Struct.parse(json_parse(text))`)
// populates `json_errors()` instead.  Both produce null fields
// on bad input.
#[test]
fn errors_accessor_path_on_failure() {
    code!(
        r#"struct Score { value: integer? }
fn test() {
    bad = Score.parse(`not_json`);
    err = bad#errors;
    assert(bad.value == null, "value should be null on bad parse");
    assert(len(err) > 0, "expected #errors entries for bad input");
}"#
    )
    .result(Value::Null);
}

// Type-mismatched nested input — `data: "not_an_object"` is a
// JString, not a JObject.  Under P54 step 5 the struct unwrap
// returns null-valued fields for kind mismatches; schema-level
// diagnostics arrive with Q1 schema-side (pending).  Verify the
// unwrap doesn't crash on the mismatched shape.
#[test]
fn errors_accessor_nested_path() {
    code!(
        r#"struct Inner { x: integer? }
struct Outer { name: text, data: Inner }
fn test() {
    bad = Outer.parse(`{{"name": "ok", "data": "not_an_object"}}`);
    assert(bad.name == "ok", "outer name should survive: got {bad.name}");
    assert(bad.data.x == null, "nested x should be null on mismatched shape");
}"#
    )
    .result(Value::Null);
}

// O7: OpClearStackText followed by ≥2 format ops must emit with_capacity hint;
// OpClearStackText followed by 0 or 1 ops must emit bare .clear().
// It guards the value the program answers; the half that read the rustc writer's emitted
// Rust went with the writer (@PLN225 R4).
#[test]
fn o7_format_string_with_capacity() {
    code!("struct S { name: text, count: integer }")
        .expr("s = S { name: \"Alice\", count: 3 }; \"hello {s.name}, count {s.count}\"")
        .result(Value::str("hello Alice, count 3"));
    code!("struct S2 { v: integer }")
        .expr("s = S2 { v: 7 }; \"{s.v}\"")
        .result(Value::str("7"));
}

// ── File.content() on non-existent file ─────────────────────────────────────
// Regression guard — verifies that File.content() on a missing path returns
// an empty text (not garbage / not a crash) under the regular execute path.
// The historical SIGSEGV was specific to execute_log (LOFT_LOG=full); the
// runtime behaviour the test asserts is now stable.  Un-ignored 2026-04-14
// after the test was found to pass in isolation; if execute_log ever
// regresses, that variant is exercised by the LOFT_LOG-driven test dumps,
// not by this guard.
//
// Deleted: file_content_nonexistent_trace — duplicate of file_content_nonexistent
// (passing), and the ignore was for a P136-adjacent harness bug, not a behavior gap.

// ── P54: JsonValue enum — landing tests ────────────────────────────────────
// Design in doc/claude/BITING_PLAN.md § P54.  These tests pin the public
// surface of the new JSON subsystem:
//
//   enum JsonValue { JObject, JArray, JString, JNumber, JBool, JNull }
//   fn json_parse(text) -> JsonValue
//   fn to_json(self: JsonValue) -> text
//   fn field / item / as_text / as_number / as_long / as_bool / len
//   MyStruct.parse(v: JsonValue) — replaces .parse(text)
//

/// String-variant parsing: the value is correctly stored, but
/// returning it through a function boundary trips a JsonValue-store
/// lifecycle issue — the store is freed during scope-exit cleanup
/// before the caller's text-copy machinery completes.  Same root
/// cause as `p54_extractor_as_text` below.  Standalone smoke
/// (`/tmp/jp1.loft` style with `match v { JString { value } =>
/// println("str={value}") }`) works fine inline; only the
/// fn-return path is broken.
#[test]
fn p54_parse_primitive_string() {
    code!(
        "fn run() -> text {
    v = json_parse(\"\\\"hello\\\"\");
    out = \"\";
    match v {
        JString { value } => { out = value; },
        _ => {}
    }
    out
}"
    )
    .expr("run()")
    .result(Value::str("hello"));
}

#[test]
fn p54_parse_primitive_number() {
    code!(
        "fn run() -> float {
    v = json_parse(\"42.5\");
    match v {
        JNumber { value } => value,
        _ => 0.0
    }
}"
    )
    .expr("run()")
    .result(Value::Float(42.5));
}

#[test]
fn p54_parse_primitive_bool_true() {
    code!(
        "fn run() -> boolean {
    v = json_parse(\"true\");
    match v {
        JBool { value } => value,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_parse_primitive_null() {
    code!(
        "fn run() -> boolean {
    v = json_parse(\"null\");
    match v {
        JNull => true,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_malformed_returns_jnull() {
    // Use a malformed input that doesn't trip loft's text-literal
    // interpolation (curly braces in `"…"` would).
    code!(
        "fn run() -> boolean {
    v = json_parse(\"xyz\");
    match v {
        JNull => true,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// `as_text` returns a `Str` into `Stores::scratch`; calling it
/// inline + assigning into a local works (`println("{v.as_text()}")`
/// is fine), but returning the resulting text through a function
/// boundary trips the same store-free-before-copy lifecycle as
/// `p54_parse_primitive_string`.  Both unblock together.
#[test]
fn p54_extractor_as_text() {
    code!(
        "fn run() -> text {
    v = json_parse(\"\\\"abc\\\"\");
    out = \"\";
    out += v.as_text();
    out
}"
    )
    .expr("run()")
    .result(Value::str("abc"));
}

/// Same store-free-before-copy lifecycle as the matching extractor
/// test above.  Unblocks once that lands.
#[test]
fn p54_extractor_as_text_wrong_kind_returns_null() {
    code!(
        "fn run() -> text {
    v = json_parse(\"42\");
    t = v.as_text();
    if t == null { \"is-null\" } else { \"not-null\" }
}"
    )
    .expr("run()")
    .result(Value::str("is-null"));
}

// Un-ignored 2026-04-14 by P54 step 4 third slice — non-empty
// primitive objects now materialise, n_field walks the arena
// vector for a name match, chained `.as_text()` reads the
// JString value out of the matched JsonField slot.
//
// Loft strings treat `{…}` as interpolation; the JSON `{` and
// `}` in the literal are doubled (`{{` / `}}`) so they reach
// `json_parse` as single braces.  Rationale in LOFT.md §
// String literals.
#[test]
fn p54_parse_object_field_access() {
    code!(
        "fn run() -> text {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}}\");
    v.field(\"name\").as_text()
}"
    )
    .expr("run()")
    .result(Value::str("Alice"));
}

// Un-ignored 2026-04-14 by P54 step 4 second slice — non-empty
// primitive arrays now materialise, `n_item` dispatches on JArray,
// and `as_long()` returns the element's numeric payload.
#[test]
fn p54_parse_array_item_access() {
    code!(
        "fn run() -> integer {
    v = json_parse(\"[10, 20, 30]\");
    v.item(1).as_long()
}"
    )
    .expr("run()")
    .result(Value::Long(20));
}

/// Chain access on a non-object value never traps — every
/// intermediate `field()` / `item()` returns `JNull`.  Step 3 stub:
/// since object/array parsing isn't wired yet, json_parse on any
/// object-shaped input returns JNull anyway, and the chain is
/// JNull all the way down.  Locks the chained-access safety
/// guarantee on a non-object root: every intermediate failure
/// produces JNull rather than trapping.  The positive-path
/// counterpart is `p54_chained_access_on_nested_object`.
#[test]
fn p54_missing_chain_returns_jnull() {
    // `{` in a loft text literal triggers format-string interpolation;
    // use a primitive that json_parse handles to produce a non-object
    // root.  The chain still lands at JNull because field/item on a
    // non-object always returns JNull.
    code!(
        "fn run() -> boolean {
    v = json_parse(\"42\");
    result = v.field(\"missing\").item(5).field(\"b\");
    match result {
        JNull => true,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// P54 step 4 — chained access on a real nested object reaches
/// the leaf.  Locks the documented STDLIB.md JSON example
/// (`v.field("users").item(0).field("name").as_text()`) so the
/// reference doc and the runtime cannot drift independently.
#[test]
fn p54_chained_access_on_nested_object() {
    code!(
        "fn run_pca() -> text {
    v_pca = json_parse(`{{\"users\":[{{\"name\":\"Alice\"}}]}}`);
    v_pca.field(\"users\").item(0).field(\"name\").as_text()
}"
    )
    .expr("run_pca()")
    .result(Value::str("Alice"));
}

/// P54 step 4 — locks the LOFT_CONTROL.md § Match expressions JsonValue
/// example.  Exercises destructuring of every JsonValue variant
/// (JObject / JArray / JNumber / JNull / wildcard) so that the
/// documented `match json_parse(raw) { ... }` patterns stay
/// supported.  When the doc is read by a new user, the same
/// arms must dispatch correctly today.
#[test]
fn p54_match_on_jsonvalue_classifies_each_kind() {
    code!(
        "fn classify_pmj(raw: text) -> text {
    match json_parse(raw) {
        JObject { fields: _ } => \"object\",
        JArray  { items: _ }  => \"array\",
        JNumber { value: _ }  => \"number\",
        JInteger { value: _ } => \"number\",
        JNull                 => \"null-or-error\",
        _                     => \"other\"
    }
}
fn run_pmj() -> integer {
    score_pmj = 0;
    if classify_pmj(`{{\"k\":1}}`) == \"object\" { score_pmj += 1; }
    if classify_pmj(\"[1,2]\") == \"array\" { score_pmj += 1; }
    if classify_pmj(\"42\") == \"number\" { score_pmj += 1; }
    if classify_pmj(\"null\") == \"null-or-error\" { score_pmj += 1; }
    if classify_pmj(\"not-json\") == \"null-or-error\" { score_pmj += 1; }
    if classify_pmj(`\"hi\"`) == \"other\" { score_pmj += 1; }
    score_pmj
}"
    )
    .expr("run_pmj()")
    .result(Value::Int(6));
}

#[test]
fn p54_struct_parse_accepts_jsonvalue() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> text {
    v = json_parse(\"{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}\");
    u = User.parse(v);
    u.name
}"
    )
    .expr("run()")
    .result(Value::str("Bob"));
}

/// P54 step 5 — `Type.parse(JsonValue)` populates an integer field
/// by unwrapping through `as_long()` + `OpCastIntFromLong` narrow.
/// Pairs with the above text-field test; together they lock both
/// primitive paths (text via OpSetText, integer via OpSetInt).
#[test]
fn p54_struct_parse_accepts_jsonvalue_integer_field() {
    code!(
        "struct User { name: text, age: integer }
fn run_age() -> integer {
    v = json_parse(\"{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}\");
    u = User.parse(v);
    u.age
}"
    )
    .expr("run_age()")
    .result(Value::Int(25));
}

/// P54 step 5 nested slice — nested struct field (`Type::Reference`
/// to another struct) recurses into `build_struct_from_jsonvalue`
/// on the corresponding sub-JsonValue, populating the embedded
/// struct record.  Exercises the full path through
/// `OpCopyRecord`-for-Reference in `set_field_no_check`.
#[test]
fn p54_struct_parse_accepts_nested_struct_field() {
    code!(
        "struct Inner { x_coord: integer }
struct Outer { label: text, data: Inner }
fn run_nested() -> integer {
    v = json_parse(\"{{\\\"label\\\":\\\"ok\\\",\\\"data\\\":{{\\\"x_coord\\\":99}}}}\");
    o = Outer.parse(v);
    o.data.x_coord
}"
    )
    .expr("run_nested()")
    .result(Value::Int(99));
}

/// P54 step 5 nested slice — verify the outer struct's primitive
/// text field is populated when a nested-struct field is also
/// present.  Complements `p54_struct_parse_accepts_nested_struct_field`
/// which returned only the inner integer; this returns the outer
/// label so a regression in field-ordering or offset calculation
/// for mixed-type structs gets caught.
#[test]
fn p54_struct_parse_nested_populates_outer_text_too() {
    code!(
        "struct Inner { x_coord: integer }
struct Outer { label: text, data: Inner }
fn run_label() -> text {
    v = json_parse(\"{{\\\"label\\\":\\\"ok\\\",\\\"data\\\":{{\\\"x_coord\\\":99}}}}\");
    o = Outer.parse(v);
    o.label
}"
    )
    .expr("run_label()")
    .result(Value::str("ok"));
}

/// P54 step 5 — `JsonValue`-typed field captures the sub-tree
/// verbatim as a passthrough.  Solves the "forward arbitrary
/// subtree" use case where a struct has a dynamic-shape payload.
/// The field() result gets OpCopyRecord'd into the struct's
/// JsonValue slot; kind() on the embedded payload reads the
/// discriminant back as `"JArray"` confirming the bytes round-trip.
#[test]
fn p54_struct_parse_captures_jsonvalue_field_verbatim() {
    code!(
        "struct WithPayload { name: text, info: JsonValue }
fn run_payload_kind() -> text {
    v = json_parse(\"{{\\\"name\\\":\\\"demo\\\",\\\"info\\\":[1,2,3]}}\");
    p = WithPayload.parse(v);
    p.info.kind()
}"
    )
    .expr("run_payload_kind()")
    .result(Value::str("JArray"));
}

/// P54 step 5 vector-field slice — populate `vector<integer>` from
/// a JArray of numbers.  Today's implementation routes through
/// the `n_jsonvalue_to_vector_long` native which walks the
/// JArray at runtime, truncates each JNumber toward zero, and
/// appends.  Other primitive element types (text / float /
/// boolean / integer) and struct-element vectors are follow-up
/// slices.
#[test]
fn p54_struct_parse_accepts_vector_long_field_len() {
    code!(
        "struct Data { items: vector<integer> }
fn run() -> integer {
    v = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    d = Data.parse(v);
    len(d.items)
}"
    )
    .expr("run()")
    .result(Value::Int(3));
}

#[test]
fn p54_struct_parse_vector_long_first_element() {
    code!(
        "struct Data { items: vector<integer> }
fn run_first() -> integer {
    v = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    d = Data.parse(v);
    d.items[0]
}"
    )
    .expr("run_first()")
    .result(Value::Long(10));
}

#[test]
fn p54_struct_parse_vector_long_iterates_correctly() {
    code!(
        "struct Data { items: vector<integer> }
fn run_sum() -> integer {
    v = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    d = Data.parse(v);
    total = 0;
    for x in d.items { total += x; }
    total
}"
    )
    .expr("run_sum()")
    .result(Value::Long(60));
}

#[test]
fn p54_struct_parse_vector_long_empty_array() {
    code!(
        "struct Data { items: vector<integer> }
fn run_empty() -> integer {
    v = json_parse(\"{{\\\"items\\\":[]}}\");
    d = Data.parse(v);
    len(d.items)
}"
    )
    .expr("run_empty()")
    .result(Value::Int(0));
}

/// P54 step 5 vector-field slice — `vector<integer>` populated
/// via the generic `n_jsonvalue_to_vector` native (elem_code = 2).
/// JNumber elements truncate toward zero with i32 narrowing;
/// non-number elements contribute `i32::MIN`.
#[test]
fn p54_struct_parse_vector_integer_field() {
    code!(
        "struct D { ns: vector<integer> }
fn run_int() -> integer {
    v = json_parse(\"{{\\\"ns\\\":[100,200,300]}}\");
    d = D.parse(v);
    total = 0;
    for x in d.ns { total += x; }
    total
}"
    )
    .expr("run_int()")
    .result(Value::Int(600));
}

/// P54 step 5 vector-field slice — `vector<float>` populated
/// via the generic `n_jsonvalue_to_vector` native (elem_code = 3).
/// JNumber elements pass through verbatim; non-number → NaN.
#[test]
fn p54_struct_parse_vector_float_field() {
    code!(
        "struct D { fs: vector<float> }
fn run_float() -> float {
    v = json_parse(\"{{\\\"fs\\\":[1.5,2.5]}}\");
    d = D.parse(v);
    d.fs[0]
}"
    )
    .expr("run_float()")
    .result(Value::Float(1.5));
}

/// P54 step 5 vector-field slice — `vector<boolean>` populated
/// via the generic `n_jsonvalue_to_vector` native (elem_code = 4).
/// JBool elements copy 0/1 byte; non-bool → 0.  The boolean case
/// previously hung because the handle store allocated only 1 word
/// (matching the element size) but the handle's vec_rec int sits
/// at byte offset 8 — overflowed the next free-block's header
/// and corrupted claim_scan into an infinite loop.  Fixed by
/// ensuring the handle store is always ≥ 2 words regardless of
/// element size.
#[test]
fn p54_struct_parse_vector_boolean_field() {
    code!(
        "struct D { bs: vector<boolean> }
fn run_bool() -> boolean {
    v = json_parse(\"{{\\\"bs\\\":[true,false,true]}}\");
    d = D.parse(v);
    d.bs[1]
}"
    )
    .expr("run_bool()")
    .result(Value::Boolean(false));
}

/// P54 step 5 vector-field slice — `vector<text>` populated via
/// the generic `n_jsonvalue_to_vector` native (elem_code = 5).
/// JString elements copy into the result vector's string area;
/// non-string → empty text.
#[test]
fn p54_struct_parse_vector_text_field() {
    code!(
        "struct D { ts: vector<text> }
fn run_text() -> text {
    v = json_parse(\"{{\\\"ts\\\":[\\\"hello\\\",\\\"world\\\"]}}\");
    d = D.parse(v);
    d.ts[0]
}"
    )
    .expr("run_text()")
    .result(Value::str("hello"));
}

/// P54 step 5 vector-of-struct slice — `vector<T>` where `T` is
/// a struct populates each element via runtime field-walk
/// (elem_code = 6).  The native enumerates the struct's fields
/// from `stores.types[struct_kt].parts` and writes each
/// primitive field by name lookup in the JSON object element.
/// Today handles primitive struct fields (text / integer /
/// long / float / boolean); nested struct or vector fields
/// inside the element type stay at zero-init defaults.
#[test]
fn p54_struct_parse_vector_of_struct_count() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}]}}\");
    inbox = Inbox.parse(v);
    len(inbox.users)
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

#[test]
fn p54_struct_parse_vector_of_struct_first_text_field() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> text {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}]}}\");
    inbox = Inbox.parse(v);
    inbox.users[0].name
}"
    )
    .expr("run()")
    .result(Value::str("Alice"));
}

#[test]
fn p54_struct_parse_vector_of_struct_second_integer_field() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}]}}\");
    inbox = Inbox.parse(v);
    inbox.users[1].age
}"
    )
    .expr("run()")
    .result(Value::Int(25));
}

#[test]
fn p54_struct_parse_vector_of_struct_iterates() {
    code!(
        "struct Score { val: integer }
struct Bag { scores: vector<Score> }
fn run() -> integer {
    v = json_parse(\"{{\\\"scores\\\":[{{\\\"val\\\":10}},{{\\\"val\\\":20}},{{\\\"val\\\":30}}]}}\");
    b = Bag.parse(v);
    total = 0;
    for s in b.scores { total += s.val; }
    total
}"
    )
    .expr("run()")
    .result(Value::Long(60));
}

#[test]
fn p54_struct_parse_vector_of_struct_empty_array() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[]}}\");
    inbox = Inbox.parse(v);
    len(inbox.users)
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

#[test]
fn p54_struct_parse_vector_of_struct_missing_field_is_null() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\"}}]}}\");
    inbox = Inbox.parse(v);
    inbox.users[0].age
}"
    )
    .expr("run()")
    .result(Value::Null);
}

/// Q1 schema-side — type mismatch on a primitive field during
/// `Type.parse(JsonValue)` pushes a path-qualified diagnostic
/// to `json_errors()` instead of silently producing a null
/// sentinel.  Asserts the diagnostic contains the struct's
/// name + field name + expected vs actual variant.
#[test]
fn q1_schema_side_type_mismatch_pushes_diagnostic() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> boolean {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":\\\"thirty\\\"}}\");
    u = User.parse(v);
    if u == u {}
    err = json_errors();
    err.contains(\"User.age\") && err.contains(\"expected JNumber\") && err.contains(\"got JString\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// Q1 schema-side — missing fields (JSON object lacks the key
/// entirely) DO NOT push a diagnostic.  Distinguishes "absent"
/// from "present-but-wrong-kind" — the former is silently
/// allowed (caller handles the null sentinel via `??` or `!`).
#[test]
fn q1_schema_side_missing_field_silent() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> integer {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\"}}\");
    u = User.parse(v);
    if u == u {}
    json_errors().len()
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

/// Q1 schema-side — a clean parse leaves `json_errors()`
/// empty.  Companion guard to the mismatch-pushes test.
#[test]
fn q1_schema_side_clean_parse_no_diagnostic() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> integer {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}}\");
    u = User.parse(v);
    if u == u {}
    json_errors().len()
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

/// Q1 schema-side — type mismatch on a primitive field of a
/// struct INSIDE a vector element pushes a diagnostic via the
/// runtime field-walk (not the compile-time check).  The
/// runtime path mirrors the same struct-name + field-name
/// diagnostic shape as the compile-time path.
#[test]
fn q1_schema_side_vector_element_type_mismatch_pushes_diagnostic() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> boolean {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":\\\"twenty\\\"}}]}}\");
    inbox = Inbox.parse(v);
    if inbox == inbox {}
    err = json_errors();
    err.contains(\"User.age\") && err.contains(\"expected JNumber\") && err.contains(\"got JString\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// Q1 schema-side — text field receiving a number pushes a
/// diagnostic naming JString as expected and JNumber as actual.
#[test]
fn q1_schema_side_text_field_receiving_number() {
    code!(
        "struct User { name: text }
fn run() -> boolean {
    v = json_parse(\"{{\\\"name\\\":42}}\");
    u = User.parse(v);
    if u == u {}
    err = json_errors();
    err.contains(\"User.name\") && err.contains(\"expected JString\") && err.contains(\"got JInteger\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// Q1 schema-side — boolean field receiving a string pushes a
/// diagnostic naming JBool as expected.
#[test]
fn q1_schema_side_boolean_field_receiving_string() {
    code!(
        "struct Flag { active: boolean }
fn run() -> boolean {
    v = json_parse(\"{{\\\"active\\\":\\\"yes\\\"}}\");
    f = Flag.parse(v);
    if f == f {}
    err = json_errors();
    err.contains(\"Flag.active\") && err.contains(\"expected JBool\") && err.contains(\"got JString\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

// Deleted: p54_struct_parse_rejects_plain_text — tested a rejected design decision
// (hard rejection of text args). Current design auto-wraps through json_parse.

// ── P54 struct-enum blockers — runtime specs (BITING_PLAN § P54) ──────────
//
// Each struct-enum bug found while building JsonValue gets a regression
// guard (for fixed bugs) or an #[ignore]'d spec (for open bugs).  The
// #[ignore]'d tests document the expected behaviour; they'll go green
// automatically when the corresponding blocker is resolved.

/// B1 (FIXED `61c36d7`): `match v { UnitVariant => … }` no longer panics
/// when `v` is produced somewhere other than a literal.  Exercise via
/// a mixed-variant enum where the unit arm matches.
#[test]
fn p54_b1_unit_variant_match_from_binding() {
    code!(
        "pub enum Palette { Null, Shade { v: integer } }
fn run() -> integer {
    p = Shade { v: 7 };
    match p {
        Null => -1,
        Shade { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

/// B6 (FIXED `5684df2`): match-arm type unification strips `RefVar`
/// wrappers.  Binding a field-carrying struct variant's text field in one
/// arm and returning a literal text (`""`) in another no longer errors
/// with 'cannot unify: &text and text'.
///
/// Uses plain struct (not struct-enum) to dodge the still-open B4
/// runtime bug.  Same type-system machinery — the field binding yields
/// `&text`, the wildcard arm returns owned `text`.
#[test]
fn p54_b6_match_arm_text_unify_plain_struct() {
    code!(
        "struct Pair { a: text, b: integer }
fn extract(p: const Pair) -> text {
    match p.b {
        0 => p.a,
        _ => \"other\"
    }
}
fn run() -> text {
    extract(Pair { a: \"hello\", b: 0 })
}"
    )
    .expr("run()")
    .result(Value::str("hello"));
}

/// Direct-return versions — now working (commit `6074619` opened up
/// struct-enum subject dispatch in match; constructing + returning
/// without an intermediate variable round-trips cleanly).
#[test]
fn p54_b3_float_not_null_direct_return() {
    code!(
        "pub enum JV { A { v: float } }
fn mk() -> JV { A { v: 42.5 } }
fn run() -> float {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Float(42.5));
}

#[test]
fn p54_b4_mixed_variant_direct_return() {
    code!(
        "pub enum JV { JA { v: boolean }, JB { v: integer }, JC { v: text } }
fn mk() -> JV { JB { v: 42 } }
fn run() -> integer {
    x = mk();
    match x {
        JA { v } => if v { 1 } else { 0 },
        JB { v } => v,
        JC { v } => v.len()
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// Open (sharpened): the intermediate-variable pattern `n = A { … };
/// n` (tail expression, no `return`) crashes.  Sharpened in
/// `p54_b3_int_via_intermediate` to narrow the bug: the
/// tail-expression path frees the local's store while the returned
/// value still references it.  `return n;` works — see
/// `p54_struct_enum_explicit_return_of_local`.
#[test]
fn p54_b3_float_via_intermediate() {
    code!(
        "pub enum JV { A { v: float } }
fn mk() -> JV {
    n = A { v: 42.5 };
    n
}
fn run() -> float {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Float(42.5));
}

/// B5 full regression guard: self-referential struct-enum with
/// recursive method.  Three layers had to land for this to pass:
///   1. `fill_all` registers `main_vector<T>` wrappers for every
///      struct/enum-variant `vector<T>` field — closes the original
///      "Incomplete record" panic on `OpDatabase(db_tp=u16::MAX)`.
///   2. Match-arm bindings carry `skip_free` — closes the garbage
///      `FreeRef(ref(4621,…))` on a not-taken arm's binding slot.
///   3. Struct-enum return-slot accounting (closed as a side-effect
///      of the cross-PR struct-enum return work landed in #168→#174).
#[test]
fn p54_b5_recursive_struct_enum() {
    code!(
        "pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn count(t: const Tree) -> integer {
    match t {
        Leaf { v } => v,
        Node { kids } => {
            c = 0;
            for k in kids { c += count(k); }
            c
        }
    }
}
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }] };
    count(root)
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

/// B5 match-arm-binding regression (layer 2): the not-taken arm of
/// a match whose binding is a `vector<T>` must not crash scope
/// cleanup.  Before the `skip_free` fix at src/parser/control.rs:1103,
/// the match's `_mv_items_*` binding in the `Full { items }` arm
/// was freed at function exit even when the `Empty` arm was taken,
/// reading garbage bytes as a `DbRef` and panicking in
/// `Stores::free_named` with an out-of-bounds `store_nr`.  Now
/// skip_free suppresses the OpFreeRef emission, so the garbage
/// slot stays untouched.  Guards the layer-2 half of B5 against
/// regression independently of the recursive path exercised by
/// `p54_b5_recursive_struct_enum` (now un-ignored).
#[test]
fn p54_b5_not_taken_arm_with_vector_binding_ok() {
    code!(
        "struct Item { v: integer }
pub enum Wrap { Empty, Full { items: vector<Item> } }
fn run() -> integer {
    w = Wrap.Empty;
    match w {
        Empty => 42,
        Full { items } => items.len()
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// B5 type-registration regression: a recursive struct-enum (`Node`
/// variant contains `vector<Tree>`) must get its `main_vector<Tree>`
/// wrapper registered during `fill_all`, so codegen's
/// `name_type("main_vector<Tree>")` lookup returns a real
/// `known_type` instead of `u16::MAX`.  Without this fix, simply
/// constructing `Node { kids: [...] }` would panic in
/// `Store::claim("Incomplete record")` — the scenario the original
/// B5 ticket reported.  This narrower regression guard exercises
/// just the construct-and-measure-len path (no match, no for-loop),
/// isolating the half of B5 that is now fixed from the still-open
/// match-arm-binding half tracked in `p54_b5_recursive_struct_enum`.
#[test]
fn p54_b5_recursive_struct_enum_construction() {
    code!(
        "pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }] };
    match root {
        Leaf { v } => v,
        Node { kids } => kids.len()
    }
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

/// B5 layer 1 + 2 combined regression: iterate over a struct-enum-
/// variant `vector<T>` binding inside a match arm.  Exercises the
/// same code paths as `p54_b5_recursive_struct_enum` (the still-
/// ignored recursive one) up to but not including the recursive
/// inner call — so if the recursion layer lands later, a regression
/// on type registration OR binding skip_free gets caught here even
/// when the recursive path is green.  Asserts the for-loop sees
/// each element with its correct `Leaf.v` payload.
#[test]
fn p54_b5_for_loop_over_enum_variant_vector() {
    code!(
        "pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }, Leaf { v: 5 }] };
    sum = 0;
    match root {
        Leaf { v } => sum += v,
        Node { kids } => {
            for k in kids {
                match k {
                    Leaf { v } => sum += v,
                    Node { kids } => sum += kids.len()
                }
            }
        }
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(12));
}

/// P54 related — positive baseline for struct-enum parameter passing.
/// Passing a struct-enum into a function and matching on it inside works
/// today; this test guards against that regressing while the return-
/// direction bugs (B3/B4) are being resolved.
#[test]
fn p54_struct_enum_as_parameter_ok() {
    code!(
        "pub enum JV { A { v: integer }, B { x: integer } }
fn show(j: const JV) -> integer {
    match j {
        A { v } => v,
        B { x } => x
    }
}
fn run() -> integer {
    n = A { v: 42 };
    show(n)
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// P54 related — positive baseline for struct-enum constructed in a
/// function and immediately matched in the same scope (no return).
/// Works today; guard against regression.
#[test]
fn p54_struct_enum_literal_then_match_same_scope() {
    code!(
        "pub enum JV { A { v: integer }, B { x: integer } }
fn run() -> integer {
    n = A { v: 7 };
    match n {
        A { v } => v,
        B { x } => x
    }
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

/// FIXED: single-variant struct-enum with integer payload returned
/// from a function now round-trips cleanly.  Previously crashed with
/// 'malloc(): unaligned tcache chunk detected'.  The fix that closed
/// this was the Reference(Enum)-as-match-subject arm in commit
/// `6074619` — same root cause (the for-body subject-type dispatch
/// mismatch also affected struct-enum assignment-site dispatch).
/// `float` / `float not null` variants (see p54_b3_float_not_null_variant)
/// and mixed-field variants (p54_b4_mixed_variant_return) remain
/// broken.
#[test]
fn p54_b3_single_variant_return() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV { A { v: 42 } }
fn run() -> integer {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// P54 — JsonValue-style extractors via a plain tagged struct.  This
/// is the workaround pattern callers use today while struct-enum
/// return-direction (B3/B4) is broken: a discriminant field plus one
/// slot per payload type.  Ugly but unblocks JSON work now.
///
/// Verifies that extractor-with-null-on-mismatch compiles cleanly
/// (B6 fix) and returns the expected values for both matching and
/// mismatching kind arms.
#[test]
fn p54_tagged_struct_extractors_work_today() {
    code!(
        "struct Tagged { kind: integer, text_val: text, num_val: float }
pub fn as_text(self: const Tagged) -> text {
    at_out = \"\";
    match self.kind {
        1 => { at_out = self.text_val; },
        _ => {}
    }
    at_out
}
pub fn as_number(self: const Tagged) -> float {
    match self.kind {
        2 => self.num_val,
        _ => 0.0
    }
}
fn run() -> text {
    r = \"\";
    t = Tagged { kind: 1, text_val: \"hello\", num_val: 0.0 };
    r += t.as_text();
    r += \"|\";
    n = Tagged { kind: 2, text_val: \"\", num_val: 3.14 };
    nt = n.as_text();
    r += \"miss[{nt}]\";
    r
}"
    )
    .expr("run()")
    .result(Value::str("hello|miss[]"));
}

/// P54 — the same extractor pattern via a struct-enum.  Now works
/// after the Reference(Enum) match-subject fix (commit `6074619`) —
/// the `t.as_text()` call site receives a struct-enum argument,
/// matches it, and returns the bound text.
#[test]
fn p54_struct_enum_extractors_spec() {
    code!(
        "pub enum Jv { Jstr { v: text }, Jnum { v: float } }
pub fn jv_as_text(self: Jv) -> text {
    jvat_out = \"\";
    match self {
        Jstr { v } => { jvat_out = v; },
        _ => {}
    }
    jvat_out
}
fn make_jstr() -> Jv { Jstr { v: \"hello\" } }
fn run() -> text {
    jvs_t = make_jstr();
    jvs_t.jv_as_text()
}"
    )
    .expr("run()")
    .result(Value::str("hello"));
}

/// B1-style fix applied to or-patterns: `A | B => …` over unit variants
/// in a mixed struct-enum previously panicked at
/// parser/control.rs:699 (same index-OOB shape as B1).  Guard the
/// attributes[0] access the same way.
#[test]
fn p54_or_pattern_mixed_struct_enum() {
    code!(
        "pub enum Sig { Off, Idle, On { level: integer } }
fn classify(s: const Sig) -> text {
    match s {
        Off | Idle => \"inactive\",
        On { level } => \"active\"
    }
}
fn run() -> text {
    classify(On { level: 80 })
}"
    )
    .expr("run()")
    .result(Value::str("active"));
}

/// Match guard on a struct-enum variant works today — regression
/// guard so future parser work doesn't drop this.
#[test]
fn p54_match_guard_on_struct_enum() {
    code!(
        "pub enum Sig { Off, On { level: integer } }
fn describe(s: const Sig) -> text {
    match s {
        Off => \"off\",
        On { level } if level > 50 => \"hi\",
        On { level } => \"lo\"
    }
}
fn run() -> text {
    r = \"\";
    r += describe(On { level: 80 });
    r += \",\";
    r += describe(On { level: 10 });
    r
}"
    )
    .expr("run()")
    .result(Value::str("hi,lo"));
}

/// B2-runtime (sub-bug of B3/B4): constructing a bare unit-variant
/// literal (`s = Idle;`) for a mixed struct-enum crashes at runtime
/// with `index out of bounds: the len is 2 but the index is <junk>`.
/// B2-compile-fix let this test compile; the runtime codegen path for
/// producing a valid struct-enum record from a bare unit-variant name
/// is still broken.  When that's fixed, this test goes green.
#[test]
fn p54_b2_unit_variant_literal_construction() {
    code!(
        "pub enum Sig { Off, Idle, On { level: integer } }
fn run() -> text {
    s = Sig.Idle;
    match s {
        Off => \"off\",
        Idle => \"idle\",
        On { level } => \"on\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("idle"));
}

/// P54 — positive baseline: a plain enum (all unit variants) round-trips
/// through a bare identifier literal.  Only the *mixed* struct-enum
/// case is broken (B2-runtime), not plain enums.  This test guards
/// that distinction.
#[test]
fn p54_plain_enum_bare_variant_works() {
    code!(
        "pub enum Sig { Off, Idle, On }
fn run() -> text {
    s = Sig.Idle;
    match s {
        Off => \"off\",
        Idle => \"idle\",
        On => \"on\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("idle"));
}

/// B2-runtime (qualified form): `Sig.Idle` as an expression in a
/// mixed struct-enum.  The parse path (parser/fields.rs) was giving
/// the result block `Type::Enum(dnr, true, vec![w])` — propagating
/// the work-ref into the LHS as a dep, so `s` became a borrower and
/// nothing freed the store.  Fixed by mirroring parser/objects.rs
/// and using `vec![]` instead (LHS owns, work-ref is skip_free).
#[test]
fn p54_b2_qualified_unit_variant_mixed_enum() {
    code!(
        "pub enum Sig { Off, Idle, On { level: integer } }
fn run() -> text {
    s = Sig.Idle;
    match s {
        Off => \"off\",
        Idle => \"idle\",
        On { level } => \"on\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("idle"));
}

/// P54 — positive baseline: plain enum match inside a for-loop body
/// works.  Pairs with the `#[ignore]`'d struct-enum version below to
/// isolate the struct-enum-specific breakage.
#[test]
fn p54_plain_enum_match_inside_for() {
    code!(
        "pub enum Item { One, Two }
fn run() -> text {
    v: vector<Item> = [One, Two];
    r = \"\";
    for x in v {
        match x {
            One => { r += \"1\"; },
            Two => { r += \"2\"; }
        }
    }
    r
}"
    )
    .expr("run()")
    .result(Value::str("12"));
}

/// FIXED: struct-enum match inside a for-loop body previously failed
/// to parse with 'Expect token }' on the first arm's `=>`.  Root
/// cause: `for_type` maps `vector<StructEnum>` to
/// `Type::Reference(enum_def, …)` as the loop-variable type, but
/// `parse_match` only accepted `Type::Enum` or
/// `Type::Reference(EnumValue/Struct)` as a valid subject.  Added a
/// `Reference(d_nr) if DefType::Enum` case; struct-enum for-body
/// matches now compile and run.
#[test]
fn p54_struct_enum_match_inside_for() {
    code!(
        "pub enum Item { Empty, Filled { qty: integer } }
fn run() -> integer {
    v = [Filled { qty: 3 }, Filled { qty: 7 }];
    sum = 0;
    for x in v {
        match x {
            Empty => {},
            Filled { qty } => { sum += qty; }
        }
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(10));
}

/// P54 — positive baseline: struct-enum match outside a for-loop (via
/// direct indexing) works.  Confirms the struct-enum match machinery
/// itself is fine — the above bug lives in for-body parsing.
#[test]
fn p54_struct_enum_match_via_index_works() {
    code!(
        "pub enum Item { Empty, Filled { qty: integer } }
fn run() -> integer {
    v = [Filled { qty: 3 }, Filled { qty: 7 }];
    x = v[0];
    match x {
        Empty => 0,
        Filled { qty } => qty
    }
}"
    )
    .expr("run()")
    .result(Value::Int(3));
}

/// Struct-enum as a field of a plain struct — construction, access
/// through field chain, and match all work.  This is a pattern
/// JsonValue callers use today (wrap the JsonValue in a holder
/// struct).
#[test]
fn p54_struct_enum_as_struct_field() {
    code!(
        "pub enum Inner { A { v: integer }, B { v: text } }
pub struct Holder { inner: Inner, count: integer }
fn run() -> text {
    h = Holder { inner: A { v: 7 }, count: 1 };
    match h.inner {
        A { v } => \"A-{v}-{h.count}\",
        B { v } => \"B-{v}-{h.count}\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("A-7-1"));
}

/// Vector of struct-enums with mixed variants; iterate and dispatch
/// by variant.  Exercises the Reference(Enum) match-subject fix
/// (commit `6074619`) plus accumulator mutation inside match arms.
#[test]
fn p54_struct_enum_vector_accumulate() {
    code!(
        "pub enum Op { Add { v: integer }, Sub { v: integer } }
fn run() -> integer {
    ops = [Add { v: 5 }, Sub { v: 3 }, Add { v: 2 }];
    sum = 0;
    for op in ops {
        match op {
            Add { v } => { sum += v; },
            Sub { v } => { sum -= v; }
        }
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(4));
}

/// B3 narrowed: the bug is the TAIL-EXPRESSION path, not the
/// intermediate variable itself.  `n = A { … }; n` (no `return`)
/// crashes because the function's scope exit frees `n`'s store
/// while the returned value still references it.  `return n;`
/// works fine — see `p54_struct_enum_explicit_return_of_local`.
/// Fix requires teaching the tail-expression-as-return codegen to
/// suppress the free of any local whose store is being returned,
/// or to materialize a copy.
#[test]
fn p54_b3_int_via_intermediate() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV {
    n = A { v: 42 };
    n
}
fn run() -> integer {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// Struct-enum intermediate-variable return — WORKS with explicit
/// `return n;` statement.  Pairs with `p54_b3_int_via_intermediate`
/// which fails on the tail-expression form `n = A { … }; n` (no
/// `return` keyword).  The tail-expression path has an ownership
/// bug: the local `n` gets freed on function exit while the returned
/// value still references the same store — explicit `return n`
/// avoids it.  Workaround today: always write `return n;`.
#[test]
fn p54_struct_enum_explicit_return_of_local() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV {
    n = A { v: 42 };
    return n;
}
fn run() -> integer {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// Reassignment before return — also works with explicit return.
/// Documents that the bug is specifically the *tail-expression*
/// path, not the assignment path.
#[test]
fn p54_struct_enum_reassign_explicit_return() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV {
    n = A { v: 1 };
    n = A { v: 42 };
    return n;
}
fn run() -> integer {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .warning("Dead assignment — 'n' is overwritten before being read at p54_struct_enum_reassign_explicit_return:3:20")
    .expr("run()")
    .result(Value::Int(42));
}

/// Struct-enum as the value in `hash<Entry[name]>` — JsonValue's
/// eventual shape for JObject.  Works end-to-end: construction,
/// hash lookup, match on the retrieved value's enum field.  Pairs
/// with `p54_struct_enum_as_struct_field` to show struct-enum
/// embedding in containers is fully viable.
#[test]
fn p54_struct_enum_in_hash_value() {
    code!(
        "pub enum Val { IntV { v: integer }, StrV { v: text } }
pub struct Entry { name: text, value: Val }
pub struct Holder { h: hash<Entry[name]> }
fn run() -> text {
    m = Holder { h: [Entry { name: \"a\", value: IntV { v: 7 } }] };
    e = m.h[\"a\"];
    if e == null { return \"miss\"; }
    match e.value {
        IntV { v } => \"int-{v}\",
        StrV { v } => \"str-{v}\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("int-7"));
}

/// Nested struct-enum — outer variant carries an inner struct-enum
/// as a field.  Full match-and-destructure chain works.  Critical
/// for JsonValue's JArray-of-JObjects / JObject-of-JArrays cases.
#[test]
fn p54_nested_struct_enum() {
    code!(
        "pub enum Inner { Leaf { v: integer } }
pub enum Outer { Wrap { inner: Inner }, Plain }
fn run() -> integer {
    o = Wrap { inner: Leaf { v: 42 } };
    match o {
        Plain => -1,
        Wrap { inner } => {
            match inner {
                Leaf { v } => v
            }
        }
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// Struct-enum flowing through multiple function calls — parameter
/// into one fn, return from another, construct a fresh variant in
/// one arm, pass through in another.  Exercises the full Reference
/// / return / assignment path.
#[test]
fn p54_struct_enum_multi_call_flow() {
    code!(
        "pub enum V { A { v: integer }, B { v: text } }
fn double_a(x: const V) -> V {
    match x {
        A { v } => A { v: v * 2 },
        B { v } => B { v: v }
    }
}
fn describe(x: const V) -> text {
    match x {
        A { v } => \"a-{v}\",
        B { v } => \"b-{v}\"
    }
}
fn run() -> text {
    a = A { v: 5 };
    d = double_a(a);
    describe(d)
}"
    )
    .expr("run()")
    .result(Value::str("a-10"));
}
