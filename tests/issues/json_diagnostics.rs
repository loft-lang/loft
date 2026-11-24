// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Method calls on a `JsonValue` that return a scalar, and the parser's rich diagnostics through
//! `json_errors()`.

use super::*;

// ── B7 family — method call on a JsonValue returning a scalar ────────
//
// Historical note (renamed 2026-04-14): this test was originally
// added as `b7_method_on_jsonvalue_returning_integer_crashes` —
// a regression marker for the period when every method call on a
// JsonValue local (even one returning a scalar like `len(v)`)
// double-freed the JsonValue store at scope exit.  The crash was
// resolved as a side-effect of later B-family landings (B2-runtime
// retrofit, B5 layers 1+2, the `t_9JsonValue_*` method-alias
// registrations for `n_as_*` / `n_field` / `n_item` / `n_len`).
//
// Today the test passes in both debug and release and guards the
// opposite invariant: method dispatch on a JsonValue local that
// returns a scalar must NOT crash and must NOT leak at scope exit.
// The remaining B7 symptom is narrower — the character-
// interpolation text-return path still SIGSEGVs, guarded by
// `b7_character_interpolation_return_crashes` (`#[ignore]`).
// See QUALITY-history.md § B7.
#[test]
fn b7_method_on_jsonvalue_returning_integer_works() {
    code!(
        "fn run_b7m() -> boolean {
    v_b7m = json_parse(\"null\");
    n_b7m = len(v_b7m);
    !n_b7m
}"
    )
    .expr("run_b7m()")
    .result(Value::Boolean(true));
}

/// Repeated method dispatch on the same JsonValue local.  If the
/// historical B7 double-free were still present, the second
/// `.kind()` call would trip the lifecycle bug because the
/// first call's post-dispatch cleanup had already decremented
/// the store's ref-count.  Today both calls succeed and return
/// the expected variant name.
#[test]
fn b7_repeated_method_dispatch_on_jsonvalue_works() {
    code!(
        "fn run_b7rm() -> text {
    v_b7rm = json_parse(\"true\");
    k1_b7rm = v_b7rm.kind();
    k2_b7rm = v_b7rm.kind();
    if k1_b7rm == k2_b7rm { k1_b7rm } else { \"MISMATCH\" }
}"
    )
    .expr("run_b7rm()")
    .result(Value::str("JBool"));
}

/// B7 method surface works on Q4-constructed JsonValues, not just
/// on `json_parse` results.  This locks that `json_null()` (and by
/// extension the other Q4 primitive constructors) produces a
/// JsonValue whose scope-exit cleanup doesn't conflict with method
/// dispatch — the same invariant the renamed `_works` test locks
/// for the json_parse side.
#[test]
fn b7_method_on_q4_constructed_jsonvalue_works() {
    code!(
        "fn run_b7q4() -> text {
    v_b7q4 = json_number(42.0);
    v_b7q4.kind()
}"
    )
    .expr("run_b7q4()")
    .result(Value::str("JNumber"));
}

// ── Q1: parser-side rich diagnostics through json_errors() ─────────────
//
// json_errors() now returns the full diagnostic shape:
//   parse error at line N col M (byte B):
//     path: /a/b/c
//     <message>
//     <context snippet with ^ caret>
//
// The loft testing harness has no substring matcher, so each test
// asserts the salient piece via a `text.contains(...)` check inside
// loft and returns a boolean.  Tolerant of future spacing /
// line-numbering tweaks.
#[test]
fn q1_json_errors_path_for_object_field() {
    code!(
        "fn run_q1a() -> boolean {
    v_q1a = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_q1a == v_q1a {}
    e_q1a = json_errors();
    e_q1a.contains(\"/x\")
}"
    )
    .expr("run_q1a()")
    .result(Value::Boolean(true));
}

#[test]
fn q1_json_errors_path_for_array_index() {
    code!(
        "fn run_q1b() -> boolean {
    v_q1b = json_parse(\"[1, 2, 1.]\");
    if v_q1b == v_q1b {}
    e_q1b = json_errors();
    e_q1b.contains(\"/2\")
}"
    )
    .expr("run_q1b()")
    .result(Value::Boolean(true));
}

#[test]
fn q1_json_errors_includes_caret_marker() {
    code!(
        "fn run_q1c() -> boolean {
    v_q1c = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_q1c == v_q1c {}
    e_q1c = json_errors();
    e_q1c.contains(\"^\")
}"
    )
    .expr("run_q1c()")
    .result(Value::Boolean(true));
}

#[test]
fn q1_json_errors_includes_line_and_byte() {
    code!(
        "fn run_q1d() -> boolean {
    v_q1d = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_q1d == v_q1d {}
    e_q1d = json_errors();
    e_q1d.contains(\"line\") && e_q1d.contains(\"byte\")
}"
    )
    .expr("run_q1d()")
    .result(Value::Boolean(true));
}

// B7 family — character-interpolation-return regression guard.
//
// Originally a SIGSEGV reproducer (discovered while writing INC#9
// regression tests): `fn f() -> text { c = txt[0]; "{c}" }`
// crashed because the text built via n_format_text on a character
// wasn't tracked for free on the outer function's text-return path.
//
// Closed as a side-effect of the B2-runtime / B5 / dep-inference /
// lock-args fixes that landed across PR #168 → #172.  Kept as a
// regression guard.  Old `_crashes` suffix retained for
// search-back compatibility — the test now passes.
#[test]
fn b7_character_interpolation_return_crashes() {
    code!(
        "fn build_b7c() -> text {
    txt_b7c = \"hello\";
    c_b7c = txt_b7c[0];
    \"{c_b7c}\"
}"
    )
    .expr("build_b7c()")
    .result(Value::str("h"));
}

// Multiple json_parse() in the same function — currently OK
// when each result is consumed via pattern matching.  Investigated
// while writing B7 regression tests; the previous QUALITY-history.md claim
// that "multiple json_parse() corrupts memory" was a misattribution
// — the corruption observed in earlier smoke tests came from the
// kind()/len() method calls, not from json_parse() itself.  This
// guard pins the pattern-match-based shape so future B7 work
// doesn't accidentally regress it.
#[test]
fn b7_multiple_json_parse_via_match_works() {
    code!(
        "fn run_b7p() -> boolean {
    a_b7p = json_parse(\"null\");
    b_b7p = json_parse(\"true\");
    match a_b7p { JNull => match b_b7p { JBool { value } => value, _ => false }, _ => false }
}"
    )
    .expr("run_b7p()")
    .result(Value::Boolean(true));
}

/// Q4 first slice — `json_null()` constructs a `JsonValue` set to
/// the `JNull` variant.  Primitive-only; doesn't need P54 step 4's
/// arena materialisation (no payload).  Consumed via pattern-match
/// so it rides on the working path guarded by
/// `b7_multiple_json_parse_via_match_works` rather than the still-
/// open method-call surface.  When P54 step 4 lands, the companion
/// `json_bool` / `json_number` / `json_string` primitives follow
/// the same shape; the container constructors `json_array` /
/// `json_object` land with the arena allocator.
#[test]
fn q4_json_null_returns_jnull_variant() {
    code!(
        "fn run_q4n() -> boolean {
    v_q4n = json_null();
    match v_q4n { JNull => true, _ => false }
}"
    )
    .expr("run_q4n()")
    .result(Value::Boolean(true));
}

/// Q4 ↔ Q2 cross-check — every primitive constructor must write
/// the same discriminant byte that `kind()` reads back as the
/// expected variant name.  Closes the integration gap between
/// the constructor write side (Q4) and the introspection read
/// side (Q2): without these guards, a constructor that wrote
/// the wrong discriminant byte would still pass its own match
/// test (because match dispatch and kind() share the same byte
/// — but a typo in either side would silently mis-name the
/// variant).
#[test]
fn q4_constructor_kind_cross_check_null() {
    code!(
        "fn run_q4ckn() -> text {
    json_null().kind()
}"
    )
    .expr("run_q4ckn()")
    .result(Value::str("JNull"));
}

#[test]
fn q4_constructor_kind_cross_check_bool() {
    code!(
        "fn run_q4ckb() -> text {
    json_bool(true).kind()
}"
    )
    .expr("run_q4ckb()")
    .result(Value::str("JBool"));
}

#[test]
fn q4_constructor_kind_cross_check_number() {
    code!(
        "fn run_q4cknum() -> text {
    json_number(2.5).kind()
}"
    )
    .expr("run_q4cknum()")
    .result(Value::str("JNumber"));
}

#[test]
fn q4_constructor_kind_cross_check_string() {
    code!(
        "fn run_q4cks() -> text {
    json_string(\"hi\").kind()
}"
    )
    .expr("run_q4cks()")
    .result(Value::str("JString"));
}

/// Q4 ↔ Q2 cross-check — `json_number(NaN)` resolves to `JNull`
/// (RFC 8259 disallows non-finite numbers), so kind() reports
/// `JNull`.  Locks the documented "non-finite → JNull"
/// substitution at the introspection level, not just the
/// internal storage.
#[test]
fn q4_constructor_kind_cross_check_nan_is_jnull() {
    code!(
        "fn run_q4cknan() -> text {
    json_number(null as float?).kind()
}"
    )
    .expr("run_q4cknan()")
    .result(Value::str("JNull"));
}

/// Q4 ↔ Q3 cross-check — every primitive constructor's payload
/// must serialise back to the canonical RFC 8259 text via
/// `to_json()`.  Closes the integration gap between the
/// constructor write side (Q4) and the serialiser read side
/// (Q3).  Without these, a constructor that wrote the wrong
/// payload bytes (e.g. flipped boolean polarity, wrong float
/// position) would still pass kind() and as_*() because each
/// reads the constructor-specific position — only `to_json()`
/// touches every byte and renders it as text.
#[test]
fn q4_constructor_to_json_cross_check_null() {
    code!(
        "fn run_q4ctjn() -> text {
    json_null().to_json()
}"
    )
    .expr("run_q4ctjn()")
    .result(Value::str("null"));
}

#[test]
fn q4_constructor_to_json_cross_check_bool_true() {
    code!(
        "fn run_q4ctjbt() -> text {
    json_bool(true).to_json()
}"
    )
    .expr("run_q4ctjbt()")
    .result(Value::str("true"));
}

#[test]
fn q4_constructor_to_json_cross_check_bool_false() {
    code!(
        "fn run_q4ctjbf() -> text {
    json_bool(false).to_json()
}"
    )
    .expr("run_q4ctjbf()")
    .result(Value::str("false"));
}

#[test]
fn q4_constructor_to_json_cross_check_number_integral() {
    code!(
        "fn run_q4ctjni() -> text {
    json_number(42.0).to_json()
}"
    )
    .expr("run_q4ctjni()")
    .result(Value::str("42"));
}

#[test]
fn q4_constructor_to_json_cross_check_number_fractional() {
    code!(
        "fn run_q4ctjnf() -> text {
    json_number(2.5).to_json()
}"
    )
    .expr("run_q4ctjnf()")
    .result(Value::str("2.5"));
}

#[test]
fn q4_constructor_to_json_cross_check_string() {
    code!(
        "fn run_q4ctjs() -> text {
    json_string(\"hi\").to_json()
}"
    )
    .expr("run_q4ctjs()")
    .result(Value::str("\"hi\""));
}

/// Q4 ↔ extractor cross-check — `json_X(v).as_X()` round-trips
/// the value back through the typed extractor.  Validates that
/// the constructor's payload-write position matches the
/// extractor's read position for each primitive variant.
#[test]
fn q4_constructor_as_bool_round_trips() {
    // `-> boolean?`, because `as_bool` is declared nullable now (loft#1302) and returning it
    // through a non-null `boolean` is what `(N-Store)` exists to catch — the warning it
    // raises here is correct, and silencing it by keeping the old signature would be pinning
    // the defect. The round trip is unchanged: the value still comes back.
    code!(
        "fn run_q4cab() -> boolean? {
    json_bool(true).as_bool()
}"
    )
    .expr("run_q4cab() == true")
    .result(Value::Boolean(true));
}

#[test]
fn q4_constructor_as_long_round_trips() {
    code!(
        "fn run_q4cal() -> integer {
    json_number(100.0).as_long()
}"
    )
    .expr("run_q4cal()")
    .result(Value::Long(100));
}

#[test]
fn q4_constructor_as_text_round_trips() {
    code!(
        "fn run_q4cat() -> text {
    json_string(\"abc\").as_text()
}"
    )
    .expr("run_q4cat()")
    .result(Value::str("abc"));
}

/// Q4 ↔ Q2 cross-check — `has_field()` on a Q4-built JObject finds
/// fields by name and rejects misses.  Bridges the construction
/// side (Q4) and the introspection side (Q2 has_field) — the
/// existing `q2_has_field_*` tests build via `json_parse(text)`,
/// not via the constructor surface, so this cross-check is the
/// only one that exercises the deep-copy → name-scan invariant.
#[test]
fn q4_constructor_has_field_finds_present_name() {
    code!(
        "fn run_q4chf() -> boolean {
    fields_q4chf: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_number(2.0) }
    ];
    obj_q4chf = json_object(fields_q4chf);
    obj_q4chf.has_field(\"alpha\") && !obj_q4chf.has_field(\"missing\")
}"
    )
    .expr("run_q4chf()")
    .result(Value::Boolean(true));
}

/// Q4 ↔ Q2 cross-check — `keys()` on a Q4-built JObject lists
/// every constructed field name.  Asserts both presence and
/// count via `keys().len()`.
#[test]
fn q4_constructor_keys_lists_constructed_names() {
    code!(
        "fn run_q4ckl() -> integer {
    fields_q4ckl: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_number(2.0) }
    ];
    obj_q4ckl = json_object(fields_q4ckl);
    obj_q4ckl.keys().len()
}"
    )
    .expr("run_q4ckl()")
    .result(Value::Int(2));
}

/// Q4 ↔ Q2 cross-check — `fields()` on a Q4-built JObject yields
/// every `(name, value)` entry.  Combined with the keys() test
/// above this locks both faces of the JObject introspection
/// surface against the constructor.
#[test]
fn q4_constructor_fields_lists_constructed_entries() {
    code!(
        "fn run_q4cfl() -> integer {
    fields_q4cfl: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_number(2.0) }
    ];
    obj_q4cfl = json_object(fields_q4cfl);
    obj_q4cfl.fields().len()
}"
    )
    .expr("run_q4cfl()")
    .result(Value::Int(2));
}

/// Q4 ↔ field navigation cross-check — `field()` lookup on a
/// Q4-built JObject walks the deep-copied field vector and
/// returns the embedded value.  Then `as_text()` extracts it.
/// Locks the full chain `json_object(...) → field(name) →
/// as_text()`.
#[test]
fn q4_constructor_field_lookup_extracts_value() {
    code!(
        "fn run_q4cfl2() -> text {
    fields_q4cfl2: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_string(\"B\") }
    ];
    obj_q4cfl2 = json_object(fields_q4cfl2);
    obj_q4cfl2.field(\"beta\").as_text()
}"
    )
    .expr("run_q4cfl2()")
    .result(Value::str("B"));
}

/// Q1 — `json_errors()` clears its trail on a successful parse.
/// The stdlib doc-comment spec says "Empty when the parse
/// succeeded" — the existing q1_* tests exercise the diagnostic
/// path on a single bad input but never verify the state-clearing
/// invariant: that a subsequent good parse erases the previous
/// error.  Without this guard, a regression that left stale
/// diagnostics from an earlier failure would silently mislead
/// every successive caller.
#[test]
fn q1_json_errors_cleared_after_successful_parse() {
    code!(
        "fn run_q1cls() -> boolean {
    bad_q1cls = json_parse(\"[1, 2, 1.]\");
    if bad_q1cls == bad_q1cls {}
    bad_len_q1cls = json_errors().len();
    good_q1cls = json_parse(\"[1, 2, 3]\");
    if good_q1cls == good_q1cls {}
    bad_len_q1cls > 0 && json_errors().len() == 0
}"
    )
    .expr("run_q1cls()")
    .result(Value::Boolean(true));
}

/// Q1 — `json_errors()` is empty after a fresh successful parse
/// on a never-failed JSON expression.  Pairs with the
/// state-clearing test above to lock both the "always empty on
/// success" and "transitions on failure→success" invariants.
#[test]
fn q1_json_errors_empty_after_clean_parse() {
    code!(
        "fn run_q1cep() -> integer {
    v_q1cep = json_parse(\"{{\\\"k\\\": 1}}\");
    if v_q1cep == v_q1cep {}
    json_errors().len()
}"
    )
    .expr("run_q1cep()")
    .result(Value::Int(0));
}
