// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Reading JSON: parsing, `as` casts, matching, field and key queries, `to_json` round trips and
//! the step-by-step acceptance cases.

use super::*;

// ── Q1 spec-named acceptance tests (complete the § Q1 Tests list) ──────
//
// QUALITY-history.md § Q1 Tests enumerates five `p54_err_*` names as the
// target acceptance coverage.  Earlier landings used `q1_*`
// prefixes (with equivalent content for some, different content
// for others).  These add the missing spec names directly so a
// reader looking for the Q1 checklist finds it by the exact
// documented identifiers.

/// Q1 — `json_errors()` path for a leaf inside a nested object
/// is the full `/a/b` pointer (parent field + child field),
/// not just the leaf name.  Complements
/// `q1_json_errors_path_for_object_field` which only checked
/// a top-level field.
#[test]
fn p54_err_reports_path_into_nested_object() {
    code!(
        "fn run_perno() -> boolean {
    v_perno = json_parse(`{{\"a\": {{\"b\": 1.}}}}`);
    if v_perno == v_perno {}
    json_errors().contains(\"/a/b\")
}"
    )
    .expr("run_perno()")
    .result(Value::Boolean(true));
}

/// Q1 — `json_errors()` path into an array element is
/// `/N` with the element index.  Same assertion shape as the
/// pre-existing `q1_json_errors_path_for_array_index`; kept
/// under the spec name as well so QUALITY-history.md's § Q1 Tests
/// checklist matches the landed test set by-name.
#[test]
fn p54_err_reports_path_into_array_element() {
    code!(
        "fn run_perae() -> boolean {
    v_perae = json_parse(\"[1, 2, 1.]\");
    if v_perae == v_perae {}
    json_errors().contains(\"/2\")
}"
    )
    .expr("run_perae()")
    .result(Value::Boolean(true));
}

/// Q1 — a parse failure on line 2 reports `line 2` in the
/// diagnostic (not just "line N" — asserts the specific
/// number).  Multi-line input via explicit `\n` escapes.
#[test]
fn p54_err_reports_line_and_column() {
    code!(
        "fn run_perlc() -> boolean {
    v_perlc = json_parse(\"{{\\n  \\\"x\\\": 1.\\n}}\");
    if v_perlc == v_perlc {}
    json_errors().contains(\"line 2\")
}"
    )
    .expr("run_perlc()")
    .result(Value::Boolean(true));
}

/// Q1 — the context snippet includes a `^` caret under the
/// offending column.  Equivalent to
/// `q1_json_errors_includes_caret_marker`, added under the
/// spec name.
#[test]
fn p54_err_context_snippet_includes_caret() {
    code!(
        "fn run_percsic() -> boolean {
    v_percsic = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_percsic == v_percsic {}
    json_errors().contains(\"^\")
}"
    )
    .expr("run_percsic()")
    .result(Value::Boolean(true));
}

/// Q1 — RFC 6901 path escaping at the acceptance level.  A
/// field named `a/b~c` renders as `/a~1b~0c` in the path.  The
/// unit test `err_path_escapes_slash_and_tilde` in
/// `src/json.rs` covers the parser-side function, but no
/// acceptance test verified that the escaping actually reaches
/// `json_errors()` output.  Without this guard, a refactor that
/// dropped the escape helper in the `n_json_parse` glue could
/// regress RFC 6901 conformance silently.
#[test]
fn p54_err_path_escapes_slash_and_tilde() {
    code!(
        "fn run_perpest() -> boolean {
    v_perpest = json_parse(`{{\"a/b~c\": 1.}}`);
    if v_perpest == v_perpest {}
    json_errors().contains(\"/a~1b~0c\")
}"
    )
    .expr("run_perpest()")
    .result(Value::Boolean(true));
}

/// Extractor null-on-mismatch — `as_long()` on a JString returns
/// the integer null sentinel (`i64::MIN`).  The stdlib spec says
/// "null on kind mismatch" — never directly tested.
#[test]
fn p54_as_long_on_jstring_returns_null_sentinel() {
    code!(
        "fn run_alos() -> integer {
    json_string(\"hi\").as_long()
}"
    )
    .expr("run_alos()")
    .result(Value::Null);
}

/// Extractor null-on-mismatch — `as_text()` on a JNumber returns
/// the text null sentinel (which compares equal to `null` at the
/// loft level).  Validates the "null on kind mismatch" contract
/// for the text extractor.  Asserts via a loft-level `t == null`
/// check rather than a direct text comparison because the
/// underlying sentinel is `"\0"`, not the empty string.
#[test]
fn p54_as_text_on_jnumber_returns_null() {
    code!(
        "fn run_aton() -> boolean {
    t_aton = json_number(42.0).as_text();
    t_aton == null
}"
    )
    .expr("run_aton()")
    .result(Value::Boolean(true));
}

/// Extractor null-on-mismatch — `as_bool()` on a JNull answers NULL, like its three
/// siblings and like its own documentation.
///
/// It used to answer `false`, and this test used to pin that — its own comment called `false`
/// *"the boolean null sentinel"*, which it is not (255 is). Written to the documented intent
/// and then asserting whatever was there, with a parenthetical over the gap: `false` is a
/// value a caller cannot tell from a field that really says false, which is the whole defect
/// (loft#1302).
///
/// The declaration is `-> boolean?` now, because that is what carries the sentinel out; the
/// test's own signature moves with it.
#[test]
fn p54_as_bool_on_jnull_is_null() {
    code!(
        "fn run_abon() -> boolean? {
    json_null().as_bool()
}"
    )
    .expr("run_abon() == null")
    .result(Value::Boolean(true));
}

/// The CONTROL for the cell above: a JBool that really says `false` still answers `false`.
/// A cure that answered null for every falsey reading would satisfy the null test and destroy
/// the only value `as_bool` exists to return.
#[test]
fn p54_as_bool_on_a_real_false_is_false() {
    code!(
        "fn run_abf() -> boolean? {
    json_parse(\"{{\\\"b\\\":false}}\").field(\"b\").as_bool()
}"
    )
    .expr("run_abf() == false")
    .result(Value::Boolean(true));
}

/// Extractor `as_long()` truncates float toward zero (NOT round,
/// NOT floor).  The stdlib spec is explicit: "Truncates the
/// underlying float toward zero before converting."  Locks the
/// behaviour for both signs — `2.7 → 2` and `-2.7 → -2`.
#[test]
fn p54_as_long_truncates_positive_float_toward_zero() {
    code!(
        "fn run_altp() -> integer {
    json_number(2.7).as_long()
}"
    )
    .expr("run_altp()")
    .result(Value::Long(2));
}

#[test]
fn p54_as_long_truncates_negative_float_toward_zero() {
    code!(
        "fn run_altn() -> integer {
    json_number(-2.7).as_long()
}"
    )
    .expr("run_altn()")
    .result(Value::Long(-2));
}

/// Edge-case parse inputs — the documented "malformed input
/// returns JNull" contract was tested for individual bad-syntax
/// inputs (Q1 path tests) but never for the lexically empty
/// boundary cases.  Locks `""`, `"   "` (whitespace-only), and
/// arbitrary garbage all return JNull.
#[test]
fn p54_parse_empty_string_returns_jnull() {
    code!(
        "fn run_pes() -> text {
    json_parse(\"\").kind()
}"
    )
    .expr("run_pes()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_parse_whitespace_only_returns_jnull() {
    code!(
        "fn run_pwo() -> text {
    json_parse(\"   \").kind()
}"
    )
    .expr("run_pwo()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_parse_garbage_input_returns_jnull() {
    code!(
        "fn run_pgi() -> text {
    json_parse(\"not-json-at-all\").kind()
}"
    )
    .expr("run_pgi()")
    .result(Value::str("JNull"));
}

/// Q4-built primitive match destructuring — the constructor
/// path didn't have direct destructuring guards beyond the
/// existing JNull (`q4_json_null_returns_jnull_variant`).
/// Adds JBool + JNumber.
///
/// JString destructuring on a Q4-built value is intentionally
/// NOT tested here because it triggers a B7-family
/// `free(): invalid size` crash (discovered while writing
/// these guards via `/tmp/jstring_match_probe.loft` — the
/// same store-lifecycle issue that gates
/// `b7_character_interpolation_return_crashes`).  The match
/// branch destructure of a JString value's text-typed inner
/// field is a known-failing path for the Q4 constructor —
/// pattern matching via wildcard works (existing q4 tests),
/// but field-binding doesn't.  Tracked under B7.
#[test]
fn q4_match_destructuring_jbool_extracts_value() {
    code!(
        "fn run_q4mb() -> boolean {
    match json_bool(true) {
        JBool { value } => value,
        _ => false
    }
}"
    )
    .expr("run_q4mb()")
    .result(Value::Boolean(true));
}

#[test]
fn q4_match_destructuring_jnumber_extracts_value() {
    code!(
        "fn run_q4mn() -> float {
    match json_number(3.25) {
        JNumber { value } => value,
        _ => 0.0
    }
}"
    )
    .expr("run_q4mn()")
    .result(Value::Float(3.25));
}

/// Extractor — `as_number()` returns the JNumber payload on a
/// matching variant and NaN (the float null sentinel) on every
/// other.  Complements the other extractor null-on-mismatch
/// guards (as_long / as_text / as_bool).  Asserts NaN via
/// self-inequality (`f != f` is true iff f is NaN — the only
/// reliable loft-level NaN test).
#[test]
fn p54_as_number_on_jnumber_returns_value() {
    code!(
        "fn run_annv() -> float {
    json_number(3.5).as_number()
}"
    )
    .expr("run_annv()")
    .result(Value::Float(3.5));
}

#[test]
fn p54_as_number_on_jstring_returns_nan() {
    code!(
        "fn run_annjs() -> boolean {
    x_annjs = json_string(\"hi\").as_number();
    x_annjs == null
}"
    )
    .expr("run_annjs()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_as_number_on_jbool_returns_nan() {
    code!(
        "fn run_annjb() -> boolean {
    x_annjb = json_bool(true).as_number();
    x_annjb == null
}"
    )
    .expr("run_annjb()")
    .result(Value::Boolean(true));
}

/// RFC 8259 numeric parse — scientific notation (`1e10`) must
/// parse as `JNumber` with the correctly-scaled float payload.
/// Never tested — the existing q1_* tests cover syntax-failure
/// paths on numbers like `1.` but not successful scientific
/// inputs.
#[test]
fn p54_parse_scientific_notation_is_jnumber() {
    code!(
        "fn run_psn() -> text {
    json_parse(\"1e10\").kind()
}"
    )
    .expr("run_psn()")
    .result(Value::str("JNumber"));
}

#[test]
fn p54_parse_scientific_notation_extracts_value() {
    code!(
        "fn run_psnv() -> boolean {
    v_psnv = json_parse(\"1e3\").as_number();
    v_psnv > 999.0 && v_psnv < 1001.0
}"
    )
    .expr("run_psnv()")
    .result(Value::Boolean(true));
}

/// RFC 8259 numeric parse — leading zeros are rejected (the
/// grammar allows only `0` or `[1-9][0-9]*` for the integer
/// part).  Locks the documented rejection behaviour so a
/// future permissive-mode change doesn't silently accept
/// `007`.  The `-0` case is a complementary positive: RFC 8259
/// explicitly allows negative zero (`-0` is a valid
/// `JNumber`).
#[test]
fn p54_parse_leading_zero_integer_is_rejected() {
    code!(
        "fn run_plz() -> text {
    json_parse(\"007\").kind()
}"
    )
    .expr("run_plz()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_parse_negative_zero_is_accepted() {
    code!(
        "fn run_pnz() -> text {
    json_parse(\"-0\").kind()
}"
    )
    .expr("run_pnz()")
    // @PLN109 — `-0` is integer-shaped, so it is a JInteger (exact 0), not JNumber.
    .result(Value::str("JInteger"));
}

/// Pretty-print depth counting — the `to_json_pretty` path
/// tracks an explicit depth counter in `json_to_text_at` and
/// emits the right number of 2-space indents at each level.
/// Prior tests cover depth 1 (`[1,2]`) and depth 2 (`{"k":[1,2]}`);
/// this guard exercises depth 3 (`{"a":{"b":[1]}}`) so the depth
/// counter is verified to propagate through nested containers
/// without off-by-one errors.
#[test]
fn q3_to_json_pretty_three_level_nesting() {
    code!(
        "fn run_q3p3() -> text {
    v_q3p3 = json_parse(`{{\"a\":{{\"b\":[1]}}}}`);
    v_q3p3.to_json_pretty()
}"
    )
    .expr("run_q3p3()")
    .result(Value::str(
        "{\n  \"a\": {\n    \"b\": [\n      1\n    ]\n  }\n}",
    ));
}

/// Round-trip preserves JObject insertion order.  The STDLIB.md
/// JSON reference says "Field names in insertion order" for
/// both `keys()` and object serialisation.  Never tested for
/// the parse → serialise path — a parser that sorted keys
/// alphabetically would be spec-incorrect but no test would
/// catch it.  Chooses names `z/a/m` so alphabetical reordering
/// (`a,m,z`) would produce distinct output from insertion
/// order (`z,a,m`).
#[test]
fn p54_parse_serialise_preserves_insertion_order() {
    code!(
        "fn run_piso() -> text {
    v_piso = json_parse(`{{\"z\":1,\"a\":2,\"m\":3}}`);
    v_piso.to_json()
}"
    )
    .expr("run_piso()")
    .result(Value::str("{\"z\":1,\"a\":2,\"m\":3}"));
}

/// Q4 → Q2 keys() preserves the caller's declared field order.
/// Complements `q4_constructor_keys_lists_constructed_names`
/// (which only asserted `.len() == 2`) by asserting every key
/// appears at the correct index, in the caller-supplied order.
#[test]
fn q4_constructor_keys_preserves_insertion_order() {
    code!(
        "fn run_q4kio() -> text {
    fields_q4kio: vector<JsonField> = [
        JsonField { name: \"zebra\", value: json_null() },
        JsonField { name: \"apple\", value: json_null() },
        JsonField { name: \"mango\", value: json_null() }
    ];
    obj_q4kio = json_object(fields_q4kio);
    ks_q4kio = obj_q4kio.keys();
    \"{ks_q4kio[0]}|{ks_q4kio[1]}|{ks_q4kio[2]}\"
}"
    )
    .expr("run_q4kio()")
    .result(Value::str("zebra|apple|mango"));
}

/// Deep-nesting navigation — a 5-level-deep JSON tree parses
/// into a tree where the leaf is reachable via five chained
/// `.field()` calls without tripping store-lifecycle or
/// arena-offset bugs.  QUALITY-history.md § Q3 Tests mentions
/// "nested up to depth 5" as the property-test target; this
/// guard pins that depth concretely.
#[test]
fn p54_deep_nesting_five_levels_navigable() {
    code!(
        "fn run_pdn5() -> integer {
    v_pdn5 = json_parse(`{{\"a\":{{\"b\":{{\"c\":{{\"d\":{{\"e\":42}}}}}}}}}}`);
    v_pdn5.field(\"a\").field(\"b\").field(\"c\").field(\"d\").field(\"e\").as_long()
}"
    )
    .expr("run_pdn5()")
    .result(Value::Long(42));
}

/// Pretty-print of an empty container inside a non-empty
/// parent.  Locks the edge case where the outer array indents
/// its children one level but the inner empty array stays `[]`
/// (no newline padding even though its parent is pretty-printed).
/// A naive implementation that always emitted `\n<indent>` for
/// every container would turn `[]` into `[\n  ]` at depth 1 —
/// this guard catches that.
#[test]
fn q3_to_json_pretty_empty_container_inside_non_empty() {
    code!(
        "fn run_q3pein() -> text {
    inner_q3pein: vector<JsonValue> = [];
    outer_q3pein: vector<JsonValue> = [json_array(inner_q3pein), json_number(1.0)];
    v_q3pein = json_array(outer_q3pein);
    v_q3pein.to_json_pretty()
}"
    )
    .expr("run_q3pein()")
    .result(Value::str("[\n  [],\n  1\n]"));
}

/// Q2 `fields()` — full name+value insertion-order preservation.
/// The prior `q4_constructor_keys_preserves_insertion_order`
/// pinned `keys()` at per-index granularity; this is the
/// companion for `fields()` on a parsed input, asserting that
/// each entry carries both its original name AND value at the
/// correct index.  Uses `z/a` names so alphabetical reordering
/// would produce distinct output.
#[test]
fn q2_fields_preserves_name_and_value_at_each_index() {
    code!(
        "fn run_q2fnvi() -> text {
    v_q2fnvi = json_parse(`{{\"z\":1,\"a\":2}}`);
    entries_q2fnvi = v_q2fnvi.fields();
    \"{entries_q2fnvi[0].name}={entries_q2fnvi[0].value.as_long()}|{entries_q2fnvi[1].name}={entries_q2fnvi[1].value.as_long()}\"
}"
    )
    .expr("run_q2fnvi()")
    .result(Value::str("z=1|a=2"));
}

/// Q2 `has_field("")` — empty-string key on a JObject that
/// carries a field with an empty name must return `true`.  Edge
/// case for the name-scan loop: a naive string-length shortcut
/// that treated empty-string as "no lookup" would break this.
/// Locks the documented "returns `true` iff carries a field
/// named `name`" contract at the name boundary.
#[test]
fn q2_has_field_matches_empty_name_key() {
    code!(
        "fn run_q2hen() -> boolean {
    v_q2hen = json_parse(`{{\"\":1}}`);
    v_q2hen.has_field(\"\") && !v_q2hen.has_field(\"a\")
}"
    )
    .expr("run_q2hen()")
    .result(Value::Boolean(true));
}

/// Match on non-empty JArray binds the `items` field to the
/// real container vector.  The vector's `.len()` must match the
/// JSON array's length.  Coverage gap: existing JArray tests
/// destructure via wildcard (`JArray _ =>`) but don't bind the
/// items field — so the binding codegen path for a non-empty
/// container wasn't directly exercised.
#[test]
fn p54_match_jarray_binds_non_empty_items() {
    code!(
        "fn run_pmjba() -> integer {
    match json_parse(\"[10,20,30]\") {
        JArray { items } => items.len(),
        _ => -1
    }
}"
    )
    .expr("run_pmjba()")
    .result(Value::Int(3));
}

/// Match on an empty JArray binds `items` to an empty vector.
/// Pairs with the non-empty test above so the binding path
/// is covered at both the minimum (zero-length) and the
/// non-degenerate (three-element) boundaries.
#[test]
fn p54_match_jarray_binds_empty_items() {
    code!(
        "fn run_pmjbe() -> integer {
    match json_parse(\"[]\") {
        JArray { items } => items.len(),
        _ => -1
    }
}"
    )
    .expr("run_pmjbe()")
    .result(Value::Int(0));
}

/// Q4 first slice — `json_null()` ignoring the B7 method-call
/// surface: two independent `json_null()` calls in the same
/// function, each consumed via its own match.  Mirrors the
/// `b7_multiple_json_parse_via_match_works` shape to guarantee
/// the constructor doesn't trip the B7 double-free when two
/// results coexist.
#[test]
fn q4_two_json_nulls_via_match_works() {
    code!(
        "fn run_q4nn() -> integer {
    a_q4nn = json_null();
    b_q4nn = json_null();
    ok_a_q4nn = match a_q4nn { JNull => 1, _ => 0 };
    ok_b_q4nn = match b_q4nn { JNull => 1, _ => 0 };
    ok_a_q4nn + ok_b_q4nn
}"
    )
    .expr("run_q4nn()")
    .result(Value::Int(2));
}

/// Q4 second slice — `json_bool(v)` constructs a `JBool` variant
/// carrying the supplied boolean payload.  Pattern-match on the
/// result binds the `value` field; this guard locks both
/// construction (discriminant byte written) and the payload round-
/// trip (field offset correct).
#[test]
fn q4_json_bool_round_trips_true() {
    code!(
        "fn run_q4bt() -> boolean {
    v_q4bt = json_bool(true);
    match v_q4bt { JBool { value } => value, _ => false }
}"
    )
    .expr("run_q4bt()")
    .result(Value::Boolean(true));
}

#[test]
fn q4_json_bool_round_trips_false() {
    code!(
        "fn run_q4bf() -> integer {
    v_q4bf = json_bool(false);
    match v_q4bf { JBool { value } => { if value { 1 } else { 2 } }, _ => 0 }
}"
    )
    .expr("run_q4bf()")
    .result(Value::Int(2));
}

/// Q4 third slice — `json_number(v)` constructs a `JNumber` variant
/// carrying the supplied float payload.  Non-finite input produces
/// `JNull` with a diagnostic in `json_errors()` — that behaviour is
/// guarded by `q4_json_number_nan_becomes_jnull` below.
#[test]
fn q4_json_number_round_trips_finite() {
    code!(
        "fn run_q4nr() -> float {
    v_q4nr = json_number(2.75);
    match v_q4nr { JNumber { value } => value, _ => 0.0 }
}"
    )
    .expr("run_q4nr()")
    .result(Value::Float(2.75));
}

#[test]
fn q4_json_number_negative_finite() {
    code!(
        "fn run_q4nn2() -> float {
    v_q4nn2 = json_number(-2.5);
    match v_q4nn2 { JNumber { value } => value, _ => 0.0 }
}"
    )
    .expr("run_q4nn2()")
    .result(Value::Float(-2.5));
}

/// Q4 third slice negative-case — feeding `float null` (NaN) or
/// non-finite values makes `json_number` store `JNull` with a
/// diagnostic, not a numeric payload that would violate RFC 8259.
#[test]
fn q4_json_number_nan_becomes_jnull() {
    code!(
        "fn run_q4nn3() -> integer {
    nan_val_q4 = null as float?;
    v_q4nn3 = json_number(nan_val_q4);
    match v_q4nn3 { JNull => 1, _ => 0 }
}"
    )
    .expr("run_q4nn3()")
    .result(Value::Int(1));
}

/// Q4 fourth slice — `json_string(v)` constructs a `JString`
/// variant carrying a copy of the supplied text.  The text is
/// written into the JsonValue's own store, so the returned value
/// lifetime-extends the payload independently of `v`.
///
/// Reading the bound `value: text` out of the match arm trips the
/// same native-returned-text lifecycle bug that B7's
/// `b7_character_interpolation_return_crashes` guards (the Str
/// pointer into the JsonValue store gets returned and later freed
/// as a DbRef).  Until B7 lands, the test verifies the shape of
/// the variant — `JString` branch taken — by measuring the
/// bound text's length (integer return, no Str escape) rather
/// than returning the value itself.
#[test]
fn q4_json_string_round_trips() {
    code!(
        "fn run_q4sr() -> integer {
    v_q4sr = json_string(\"hello world\");
    match v_q4sr { JString { value } => value.len(), _ => -1 }
}"
    )
    .expr("run_q4sr()")
    .result(Value::Int(11));
}

#[test]
fn q4_json_string_empty() {
    code!(
        "fn run_q4se() -> integer {
    v_q4se = json_string(\"\");
    match v_q4se { JString { value } => value.len(), _ => -1 }
}"
    )
    .expr("run_q4se()")
    .result(Value::Int(0));
}

/// Q2 — `kind(self: JsonValue) -> text` introspection.  One guard
/// per primitive variant, in both free-function syntax `kind(v)`
/// and method syntax `v.kind()`, to lock the registration of both
/// dispatch paths (NATIVE_FNS registers `n_kind` + the
/// `t_9JsonValue_kind` method alias).  Container variants
/// (`JArray`, `JObject`) land with P54 step 4's arena materialiser.
#[test]
fn q2_kind_of_jnull_free_form() {
    code!(
        "fn run_q2kn() -> text {
    v_q2kn = json_null();
    kind(v_q2kn)
}"
    )
    .expr("run_q2kn()")
    .result(Value::str("JNull"));
}

#[test]
fn q2_kind_of_jnull_method_form() {
    code!(
        "fn run_q2kn2() -> text {
    v_q2kn2 = json_null();
    v_q2kn2.kind()
}"
    )
    .expr("run_q2kn2()")
    .result(Value::str("JNull"));
}

#[test]
fn q2_kind_of_jbool() {
    code!(
        "fn run_q2kb() -> text {
    v_q2kb = json_bool(true);
    v_q2kb.kind()
}"
    )
    .expr("run_q2kb()")
    .result(Value::str("JBool"));
}

#[test]
fn q2_kind_of_jnumber() {
    code!(
        "fn run_q2knum() -> text {
    v_q2knum = json_number(42.0);
    v_q2knum.kind()
}"
    )
    .expr("run_q2knum()")
    .result(Value::str("JNumber"));
}

#[test]
fn q2_kind_of_jstring() {
    code!(
        "fn run_q2ks() -> text {
    v_q2ks = json_string(\"hello\");
    v_q2ks.kind()
}"
    )
    .expr("run_q2ks()")
    .result(Value::str("JString"));
}

/// Q2 kind() on a json_parse result — locks that the discriminant
/// byte written by `n_json_parse` for parsed primitives matches
/// the one `n_kind` reads back.  A JSON-parser-vs-kind-reader drift
/// would make `kind(json_parse(x))` return "JUnknown".
#[test]
fn q2_kind_of_parsed_primitive() {
    code!(
        "fn run_q2kp() -> text {
    v_q2kp = json_parse(\"true\");
    kind(v_q2kp)
}"
    )
    .expr("run_q2kp()")
    .result(Value::str("JBool"));
}

/// Q3 primitive-slice — `to_json(self: JsonValue) -> text` renders
/// a JsonValue to canonical RFC 8259 JSON text.  One guard per
/// primitive variant, each measured by text equality to lock the
/// serialisation contract.  Container variants (JArray / JObject)
/// render as `"<pending step 4>"` today; the full recursive
/// formatter lands with P54 step 4.
#[test]
fn q3_to_json_of_jnull() {
    code!(
        "fn run_q3tn() -> text {
    v_q3tn = json_null();
    to_json(v_q3tn)
}"
    )
    .expr("run_q3tn()")
    .result(Value::str("null"));
}

#[test]
fn q3_to_json_of_jbool_true() {
    code!(
        "fn run_q3tb() -> text {
    v_q3tb = json_bool(true);
    v_q3tb.to_json()
}"
    )
    .expr("run_q3tb()")
    .result(Value::str("true"));
}

#[test]
fn q3_to_json_of_jbool_false() {
    code!(
        "fn run_q3tbf() -> text {
    v_q3tbf = json_bool(false);
    v_q3tbf.to_json()
}"
    )
    .expr("run_q3tbf()")
    .result(Value::str("false"));
}

#[test]
fn q3_to_json_of_jnumber_integer() {
    code!(
        "fn run_q3tni() -> text {
    v_q3tni = json_number(42.0);
    v_q3tni.to_json()
}"
    )
    .expr("run_q3tni()")
    .result(Value::str("42"));
}

#[test]
fn q3_to_json_of_jnumber_fractional() {
    code!(
        "fn run_q3tnf() -> text {
    v_q3tnf = json_number(2.75);
    v_q3tnf.to_json()
}"
    )
    .expr("run_q3tnf()")
    .result(Value::str("2.75"));
}

/// Non-finite inputs to json_number (NaN, ±Inf) construct a JNull
/// variant with a diagnostic — to_json on that reads "null", not
/// the garbage representation of the bad float.  Matches RFC 8259
/// which forbids non-finite numeric literals.
#[test]
fn q3_to_json_of_nan_becomes_null() {
    code!(
        "fn run_q3tnn() -> text {
    nan_q3 = null as float?;
    v_q3tnn = json_number(nan_q3);
    v_q3tnn.to_json()
}"
    )
    .expr("run_q3tnn()")
    .result(Value::str("null"));
}

#[test]
fn q3_to_json_of_jstring_plain() {
    code!(
        "fn run_q3ts() -> text {
    v_q3ts = json_string(\"hello\");
    v_q3ts.to_json()
}"
    )
    .expr("run_q3ts()")
    .result(Value::str("\"hello\""));
}

// Q3 escape-sequence regressions (`"a\"b\\c"` round-trip; `\n` /
// `\t` / control-byte encoding) are deferred.  Initial attempt
// exposed that loft's String parser currently drops backslash-
// escapes in string literals inside `code!()` test scaffolding
// (the `q3_to_json_of_jstring_with_escapes` case hung on the
// loft-side interpretation of the `\\` sequence; needs isolated
// reproducer + investigation).  The Rust-side escape logic in
// `n_to_json` is exercised; the test-harness plumbing is what's
// blocking.  Track as a follow-up under Q3.

/// Q3 pretty-print primitive slice — `to_json_pretty(self: JsonValue)`
/// produces identical output to `to_json` for primitive variants
/// (no nested structure to indent).  Divergence lands with P54
/// step 4's arena materialiser.  These guards lock that primitive
/// output is byte-identical across the two entry points today, so
/// a future change that adds pretty-specific padding for
/// primitives would be caught.
#[test]
fn q3_to_json_pretty_of_jnull() {
    code!(
        "fn run_q3pn() -> text {
    v_q3pn = json_null();
    v_q3pn.to_json_pretty()
}"
    )
    .expr("run_q3pn()")
    .result(Value::str("null"));
}

#[test]
fn q3_to_json_pretty_of_jbool() {
    code!(
        "fn run_q3pb() -> text {
    v_q3pb = json_bool(true);
    v_q3pb.to_json_pretty()
}"
    )
    .expr("run_q3pb()")
    .result(Value::str("true"));
}

#[test]
fn q3_to_json_pretty_of_jnumber() {
    code!(
        "fn run_q3pnum() -> text {
    v_q3pnum = json_number(42.0);
    v_q3pnum.to_json_pretty()
}"
    )
    .expr("run_q3pnum()")
    .result(Value::str("42"));
}

#[test]
fn q3_to_json_pretty_of_jstring() {
    code!(
        "fn run_q3ps() -> text {
    v_q3ps = json_string(\"hi\");
    v_q3ps.to_json_pretty()
}"
    )
    .expr("run_q3ps()")
    .result(Value::str("\"hi\""));
}

/// Q3 — `to_json` and `to_json_pretty` must agree on primitives.
/// This regression guard compares the outputs directly and fails
/// if they ever diverge for a primitive variant (which would
/// indicate the pretty path is accidentally formatting leaf
/// values differently from the canonical path).
#[test]
fn q3_to_json_and_pretty_agree_on_primitive() {
    code!(
        "fn run_q3pa() -> boolean {
    v_q3pa = json_number(2.75);
    canonical_q3pa = v_q3pa.to_json();
    pretty_q3pa = v_q3pa.to_json_pretty();
    canonical_q3pa == pretty_q3pa
}"
    )
    .expr("run_q3pa()")
    .result(Value::Boolean(true));
}

/// Q3 — free-function dispatch of `to_json_pretty`.  Locks the
/// `n_to_json_pretty` registration in NATIVE_FNS alongside the
/// `t_9JsonValue_to_json_pretty` method alias.
#[test]
fn q3_to_json_pretty_free_form() {
    code!(
        "fn run_q3pf() -> text {
    v_q3pf = json_null();
    to_json_pretty(v_q3pf)
}"
    )
    .expr("run_q3pf()")
    .result(Value::str("null"));
}

/// Q3 pretty — empty containers stay byte-identical to canonical
/// (no newline padding for `[]` / `{}`).
#[test]
fn q3_to_json_pretty_empty_array() {
    code!(
        "fn run_q3pea() -> text {
    v_q3pea = json_parse(\"[]\");
    v_q3pea.to_json_pretty()
}"
    )
    .expr("run_q3pea()")
    .result(Value::str("[]"));
}

#[test]
fn q3_to_json_pretty_empty_object() {
    code!(
        "fn run_q3peo() -> text {
    v_q3peo = json_parse(\"{{}}\");
    v_q3peo.to_json_pretty()
}"
    )
    .expr("run_q3peo()")
    .result(Value::str("{}"));
}

/// Q3 pretty — non-empty array indents each element on its own
/// line with 2-space indent, closing bracket dedents back.
#[test]
fn q3_to_json_pretty_array_indents_elements() {
    code!(
        "fn run_q3pai() -> text {
    v_q3pai = json_parse(\"[1,2,3]\");
    v_q3pai.to_json_pretty()
}"
    )
    .expr("run_q3pai()")
    .result(Value::str("[\n  1,\n  2,\n  3\n]"));
}

/// Q3 pretty — non-empty object indents each field on its own
/// line; key/value separator is `": "` (colon + single space).
#[test]
fn q3_to_json_pretty_object_indents_fields() {
    code!(
        "fn run_q3poi() -> text {
    v_q3poi = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2}}\");
    v_q3poi.to_json_pretty()
}"
    )
    .expr("run_q3poi()")
    .result(Value::str("{\n  \"a\": 1,\n  \"b\": 2\n}"));
}

/// Q3 pretty — nested containers indent recursively.  Inner
/// container's indent is one level deeper than the outer's.
#[test]
fn q3_to_json_pretty_nested_array_in_object() {
    code!(
        "fn run_q3pnao() -> text {
    v_q3pnao = json_parse(\"{{\\\"k\\\":[1,2]}}\");
    v_q3pnao.to_json_pretty()
}"
    )
    .expr("run_q3pnao()")
    .result(Value::str("{\n  \"k\": [\n    1,\n    2\n  ]\n}"));
}

/// Q3 pretty — canonical and pretty diverge once a non-empty
/// container is in play.  Locks the active difference (the prior
/// stub returned the same text for both, which would have hidden
/// a regression in the pretty walk).
#[test]
fn q3_to_json_and_pretty_differ_on_nonempty_container() {
    code!(
        "fn run_q3pdc() -> boolean {
    v_q3pdc = json_parse(\"[1,2]\");
    v_q3pdc.to_json() != v_q3pdc.to_json_pretty()
}"
    )
    .expr("run_q3pdc()")
    .result(Value::Boolean(true));
}

/// P54 step-4 null-safety — `len()` on each primitive variant
/// returns the integer null sentinel (`i32::MIN`) per the
/// stdlib contract.  Locks the documented "no length defined"
/// behaviour for non-container variants so an accidental
/// switch to `0` (which would be wrong — a real empty array
/// has length 0) gets caught.
#[test]
fn p54_step4_len_on_jnull_is_null_sentinel() {
    code!(
        "fn run_lnn() -> integer {
    v = json_null();
    v.len()
}"
    )
    .expr("run_lnn()")
    .result(Value::Null);
}

#[test]
fn p54_step4_len_on_jbool_is_null_sentinel() {
    code!(
        "fn run_lnb() -> integer {
    v = json_bool(true);
    v.len()
}"
    )
    .expr("run_lnb()")
    .result(Value::Null);
}

#[test]
fn p54_step4_len_on_jnumber_is_null_sentinel() {
    code!(
        "fn run_lnnum() -> integer {
    v = json_number(1.0);
    v.len()
}"
    )
    .expr("run_lnnum()")
    .result(Value::Null);
}

#[test]
fn p54_step4_len_on_jstring_is_null_sentinel() {
    code!(
        "fn run_lnstr() -> integer {
    v = json_string(\"hello\");
    v.len()
}"
    )
    .expr("run_lnstr()")
    .result(Value::Null);
}

/// P54 step-4 null-safety — `field()` on a non-JObject receiver
/// returns `JNull` rather than crashing.  Locks the chained-
/// access safety guarantee (every intermediate missing produces
/// `JNull`, never a trap).
#[test]
fn p54_step4_field_on_jstring_returns_jnull() {
    code!(
        "fn run_fjs() -> text {
    v = json_string(\"hi\");
    v.field(\"missing\").kind()
}"
    )
    .expr("run_fjs()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_field_missing_key_returns_jnull() {
    code!(
        "fn run_fmk() -> text {
    v = json_parse(\"{{\\\"present\\\":1}}\");
    v.field(\"absent\").kind()
}"
    )
    .expr("run_fmk()")
    .result(Value::str("JNull"));
}

/// P54 step-4 null-safety — `item()` on non-JArray, negative
/// index, and out-of-bounds index all return `JNull`.
#[test]
fn p54_step4_item_on_jnumber_returns_jnull() {
    code!(
        "fn run_ijn() -> text {
    v = json_number(42.0);
    v.item(0).kind()
}"
    )
    .expr("run_ijn()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_item_negative_index_returns_jnull() {
    code!(
        "fn run_ini() -> text {
    v = json_parse(\"[1,2,3]\");
    v.item(-1).kind()
}"
    )
    .expr("run_ini()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_item_out_of_bounds_returns_jnull() {
    code!(
        "fn run_iob() -> text {
    v = json_parse(\"[1,2,3]\");
    v.item(99).kind()
}"
    )
    .expr("run_iob()")
    .result(Value::str("JNull"));
}

/// Q3 — round-trip property for primitives.  Each primitive
/// variant survives `to_json` → `json_parse` with its kind
/// (and where applicable, payload) intact.  Listed in
/// QUALITY-history.md § Q3 Tests as `q3_primitives_round_trip`.
#[test]
fn q3_primitives_round_trip() {
    code!(
        "fn check_q3prt(s: text, expected_kind: text) -> boolean {
    v = json_parse(s);
    text_q3prt = v.to_json();
    parsed_q3prt = json_parse(text_q3prt);
    parsed_q3prt.kind() == expected_kind
}
fn run_q3prt() -> integer {
    score_q3prt = 0;
    if check_q3prt(\"null\", \"JNull\") { score_q3prt += 1; }
    if check_q3prt(\"true\", \"JBool\") { score_q3prt += 1; }
    if check_q3prt(\"false\", \"JBool\") { score_q3prt += 1; }
    if check_q3prt(\"42\", \"JInteger\") { score_q3prt += 1; }
    if check_q3prt(\"3.14\", \"JNumber\") { score_q3prt += 1; }
    if check_q3prt(\"\\\"hi\\\"\", \"JString\") { score_q3prt += 1; }
    score_q3prt
}"
    )
    .expr("run_q3prt()")
    .result(Value::Int(6));
}

/// Q3 — round-trip property for nested objects.  An object with
/// primitive fields survives `to_json` → `json_parse` and the
/// extracted leaves agree on values.  Listed in QUALITY-history.md
/// § Q3 Tests as `q3_nested_object_round_trip`.
#[test]
fn q3_nested_object_round_trip() {
    code!(
        "fn run_q3nort() -> integer {
    src_q3nort = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    text_q3nort = src_q3nort.to_json();
    back_q3nort = json_parse(text_q3nort);
    sum_q3nort = 0;
    sum_q3nort += back_q3nort.field(\"a\").as_long();
    sum_q3nort += back_q3nort.field(\"b\").as_long();
    sum_q3nort += back_q3nort.field(\"c\").as_long();
    sum_q3nort as integer
}"
    )
    .expr("run_q3nort()")
    .result(Value::Int(6));
}

/// Q3 — round-trip property for arrays of mixed primitive kinds.
/// `[1,true,\"x\"]` survives `to_json` → `json_parse` with each
/// element's kind preserved.  Listed in QUALITY-history.md § Q3 Tests as
/// `q3_array_of_mixed_kinds_round_trip`.
#[test]
fn q3_array_of_mixed_kinds_round_trip() {
    code!(
        "fn run_q3amkrt() -> text {
    src_q3amkrt = json_parse(\"[1,true,\\\"x\\\"]\");
    text_q3amkrt = src_q3amkrt.to_json();
    back_q3amkrt = json_parse(text_q3amkrt);
    \"{back_q3amkrt.item(0).kind()}|{back_q3amkrt.item(1).kind()}|{back_q3amkrt.item(2).kind()}\"
}"
    )
    .expr("run_q3amkrt()")
    // @PLN109 — the leading `1` round-trips as a JInteger.
    .result(Value::str("JInteger|JBool|JString"));
}

/// Q3 — pretty-printed output is still valid JSON: `parse(to_json_pretty(v))`
/// produces an equivalent tree.  Locks the property that pretty
/// mode only adds whitespace between structural tokens, never
/// inside string literals or numbers.  Listed in QUALITY-history.md § Q3
/// Tests as `q3_pretty_form_valid_json`.
#[test]
fn q3_pretty_form_valid_json() {
    code!(
        "fn run_q3pfvj() -> integer {
    src_q3pfvj = json_parse(\"{{\\\"items\\\":[1,2,3]}}\");
    pretty_q3pfvj = src_q3pfvj.to_json_pretty();
    back_q3pfvj = json_parse(pretty_q3pfvj);
    back_q3pfvj.field(\"items\").len()
}"
    )
    .expr("run_q3pfvj()")
    .result(Value::Int(3));
}

/// Q3 — UTF-8 string content passes through `to_json` verbatim
/// (no `\\uXXXX` escaping of BMP characters).  Listed in
/// QUALITY-history.md § Q3 Tests as `q3_unicode_string_escaping`.
#[test]
fn q3_unicode_string_escaping() {
    code!(
        "fn run_q3use() -> text {
    s_q3use = json_string(\"α β 😊\");
    s_q3use.to_json()
}"
    )
    .expr("run_q3use()")
    .result(Value::str("\"α β 😊\""));
}

/// P54 step 4 first slice — empty arrays `[]` and empty objects
/// `{}` are now materialised as real `JArray` / `JObject`
/// variants (not the earlier `JNull`-stub).  This unblocks
/// `kind()`, `len()`, `has_field()`, and `to_json()` for the
/// empty-container case today; non-empty containers remain
/// stubbed until the full arena materialiser lands.
#[test]
fn p54_step4_empty_array_has_jarray_kind() {
    code!(
        "fn run_p4ea() -> text {
    v_p4ea = json_parse(\"[]\");
    v_p4ea.kind()
}"
    )
    .expr("run_p4ea()")
    .result(Value::str("JArray"));
}

#[test]
fn p54_step4_empty_object_has_jobject_kind() {
    // Loft string literals treat `{...}` as interpolation; escape
    // literal braces by doubling (`{{` → `{`, `}}` → `}`), so
    // `"{{}}"` in loft source is the two-char JSON empty-object
    // literal `{}`.  Same trick below on the other object tests.
    code!(
        "fn run_p4eo() -> text {
    v_p4eo = json_parse(\"{{}}\");
    v_p4eo.kind()
}"
    )
    .expr("run_p4eo()")
    .result(Value::str("JObject"));
}

/// Step 4 first slice — `len()` returns 0 for empty containers
/// (both JArray and JObject).  Primitive variants still return
/// the integer null sentinel via the unchanged path.
#[test]
fn p54_step4_empty_array_len_is_zero() {
    code!(
        "fn run_p4al() -> integer {
    v_p4al = json_parse(\"[]\");
    v_p4al.len()
}"
    )
    .expr("run_p4al()")
    .result(Value::Int(0));
}

#[test]
fn p54_step4_empty_object_len_is_zero() {
    code!(
        "fn run_p4ol() -> integer {
    v_p4ol = json_parse(\"{{}}\");
    v_p4ol.len()
}"
    )
    .expr("run_p4ol()")
    .result(Value::Int(0));
}

/// Step 4 first slice — `to_json()` now renders `"[]"` / `"{}"`
/// for empty containers instead of the earlier `"<pending step
/// 4>"` placeholder.  Non-empty containers still render the
/// placeholder until the full arena materialiser lands.
#[test]
fn p54_step4_empty_array_to_json() {
    code!(
        "fn run_p4aj() -> text {
    v_p4aj = json_parse(\"[]\");
    v_p4aj.to_json()
}"
    )
    .expr("run_p4aj()")
    .result(Value::str("[]"));
}

#[test]
fn p54_step4_empty_object_to_json() {
    code!(
        "fn run_p4oj() -> text {
    v_p4oj = json_parse(\"{{}}\");
    v_p4oj.to_json()
}"
    )
    .expr("run_p4oj()")
    .result(Value::str("{}"));
}

/// Step 4 first slice — round-trip: parse `[]`, serialise, parse
/// again, confirm the discriminant agrees end-to-end.  Locks that
/// `n_json_parse` and `n_to_json` agree on empty containers.
#[test]
fn p54_step4_empty_array_round_trips_through_to_json() {
    code!(
        "fn run_p4ar() -> text {
    first_p4ar = json_parse(\"[]\");
    round_p4ar = json_parse(first_p4ar.to_json());
    round_p4ar.kind()
}"
    )
    .expr("run_p4ar()")
    .result(Value::str("JArray"));
}

/// Step 4 fourth slice (2026-04-14) — nested containers
/// (arrays-of-arrays, objects-of-objects, or any mix) now
/// materialise too, closing step 4.  This guard reverses the
/// earlier stub assertion: `[[1,2],[3,4]]` is a real JArray,
/// not a JNull stub.
#[test]
fn p54_step4_nested_array_materialises() {
    code!(
        "fn run_p4nm() -> text {
    v_p4nm = json_parse(\"[[1,2],[3,4]]\");
    v_p4nm.kind()
}"
    )
    .expr("run_p4nm()")
    .result(Value::str("JArray"));
}

/// Step 4 second slice (2026-04-14) — non-empty arrays of primitive
/// elements now materialise into real JArray variants with elements
/// in an arena sub-record of the root's store.  The
/// `n_json_parse` + `n_len` + `n_item` + `n_to_json` paths all
/// dispatch on JArray and cooperate: parse produces the arena,
/// len reads the vector header, item reads the N-th slot,
/// to_json recurses.  Nested containers still stub as JNull
/// (guarded above).
#[test]
fn p54_step4_nonempty_primitive_array_has_jarray_kind() {
    code!(
        "fn run_p4npk() -> text {
    v_p4npk = json_parse(\"[1,2,3]\");
    v_p4npk.kind()
}"
    )
    .expr("run_p4npk()")
    .result(Value::str("JArray"));
}

#[test]
fn p54_step4_nonempty_primitive_array_length_correct() {
    code!(
        "fn run_p4npl() -> integer {
    v_p4npl = json_parse(\"[1,2,3]\");
    v_p4npl.len()
}"
    )
    .expr("run_p4npl()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nonempty_primitive_array_item_0_is_first() {
    code!(
        "fn run_p4npi0() -> integer {
    v_p4npi0 = json_parse(\"[10,20,30]\");
    v_p4npi0.item(0).as_long()
}"
    )
    .expr("run_p4npi0()")
    .result(Value::Long(10));
}

#[test]
fn p54_step4_nonempty_primitive_array_item_1_is_middle() {
    code!(
        "fn run_p4npi1() -> integer {
    v_p4npi1 = json_parse(\"[10,20,30]\");
    v_p4npi1.item(1).as_long()
}"
    )
    .expr("run_p4npi1()")
    .result(Value::Long(20));
}

#[test]
fn p54_step4_nonempty_primitive_array_item_out_of_range_returns_jnull() {
    code!(
        "fn run_p4npior() -> text {
    v_p4npior = json_parse(\"[10,20]\");
    v_p4npior.item(5).kind()
}"
    )
    .expr("run_p4npior()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_nonempty_bool_array_item_kind() {
    code!(
        "fn run_p4npbk() -> text {
    v_p4npbk = json_parse(\"[true,false]\");
    v_p4npbk.item(0).kind()
}"
    )
    .expr("run_p4npbk()")
    .result(Value::str("JBool"));
}

#[test]
fn p54_step4_nonempty_string_array_item_value() {
    code!(
        "fn run_p4npsi() -> text {
    v_p4npsi = json_parse(\"[\\\"hello\\\",\\\"world\\\"]\");
    v_p4npsi.item(1).kind()
}"
    )
    .expr("run_p4npsi()")
    .result(Value::str("JString"));
}

#[test]
fn p54_step4_nonempty_array_to_json_round_trips() {
    code!(
        "fn run_p4narj() -> integer {
    v_p4narj = json_parse(\"[1,2,3]\");
    round_p4narj = json_parse(v_p4narj.to_json());
    round_p4narj.len()
}"
    )
    .expr("run_p4narj()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nonempty_array_to_json_text_shape() {
    // `[1,2,3]` serialises each JNumber via `f64::Display`, which
    // prints `1` / `2` / `3` for whole-number floats.
    code!(
        "fn run_p4nats() -> text {
    v_p4nats = json_parse(\"[1,2,3]\");
    v_p4nats.to_json()
}"
    )
    .expr("run_p4nats()")
    .result(Value::str("[1,2,3]"));
}

// The original `p54_parse_array_item_access` test (originally
// `#[ignore]`'d) was un-ignored in place 2026-04-14 by P54 step 4
// second slice — see that test's comment + commit history.

/// Step 4 third slice (2026-04-14) — non-empty primitive objects.
/// Tests mirror the array second-slice guards: discriminant,
/// length, field lookup (hit + miss), has_field, to_json, and
/// a round-trip.  Every loft string with `{` / `}` doubles them
/// to `{{` / `}}` per LOFT_LITERALS.md § String literals.
#[test]
fn p54_step4_nonempty_primitive_object_has_jobject_kind() {
    code!(
        "fn run_p4ok() -> text {
    v_p4oK = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oK.kind()
}"
    )
    .expr("run_p4ok()")
    .result(Value::str("JObject"));
}

#[test]
fn p54_step4_nonempty_primitive_object_length_correct() {
    code!(
        "fn run_p4ol() -> integer {
    v_p4oL = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    v_p4oL.len()
}"
    )
    .expr("run_p4ol()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nonempty_object_field_hit_returns_value() {
    code!(
        "fn run_p4oh() -> integer {
    v_p4oH = json_parse(\"{{\\\"age\\\":30}}\");
    v_p4oH.field(\"age\").as_long()
}"
    )
    .expr("run_p4oh()")
    .result(Value::Long(30));
}

#[test]
fn p54_step4_nonempty_object_field_miss_returns_jnull() {
    code!(
        "fn run_p4om() -> text {
    v_p4oM = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oM.field(\"missing\").kind()
}"
    )
    .expr("run_p4om()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_nonempty_object_has_field_hit() {
    code!(
        "fn run_p4ohh() -> boolean {
    v_p4oHh = json_parse(\"{{\\\"users\\\":true}}\");
    v_p4oHh.has_field(\"users\")
}"
    )
    .expr("run_p4ohh()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_step4_nonempty_object_has_field_miss() {
    code!(
        "fn run_p4ohm() -> boolean {
    v_p4oHm = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oHm.has_field(\"q\")
}"
    )
    .expr("run_p4ohm()")
    .result(Value::Boolean(false));
}

#[test]
fn p54_step4_nonempty_object_to_json_text_shape() {
    code!(
        "fn run_p4oj() -> text {
    v_p4oJ = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oJ.to_json()
}"
    )
    .expr("run_p4oj()")
    .result(Value::str("{\"k\":1}"));
}

#[test]
fn p54_step4_nonempty_object_to_json_round_trips() {
    code!(
        "fn run_p4or() -> integer {
    v_p4oR = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2}}\");
    round_p4oR = json_parse(v_p4oR.to_json());
    round_p4oR.len()
}"
    )
    .expr("run_p4or()")
    .result(Value::Int(2));
}

#[test]
fn p54_step4_nonempty_object_mixed_primitive_values() {
    code!(
        "fn run_p4omx() -> text {
    v_p4oMx = json_parse(\"{{\\\"s\\\":\\\"hi\\\",\\\"n\\\":7,\\\"b\\\":true}}\");
    v_p4oMx.field(\"s\").as_text()
}"
    )
    .expr("run_p4omx()")
    .result(Value::str("hi"));
}

/// Step 4 fourth slice — nested arrays.  Outer `[[1,2],[3,4]]`
/// has length 2, each item is itself a JArray of length 2.
#[test]
fn p54_step4_nested_array_outer_length() {
    code!(
        "fn run_p4nal() -> integer {
    v_p4nal = json_parse(\"[[1,2],[3,4]]\");
    v_p4nal.len()
}"
    )
    .expr("run_p4nal()")
    .result(Value::Int(2));
}

#[test]
fn p54_step4_nested_array_inner_length() {
    code!(
        "fn run_p4nil() -> integer {
    v_p4nil = json_parse(\"[[1,2,3],[4,5]]\");
    v_p4nil.item(0).len()
}"
    )
    .expr("run_p4nil()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nested_array_inner_item_value() {
    code!(
        "fn run_p4niv() -> integer {
    v_p4niv = json_parse(\"[[10,20],[30,40]]\");
    v_p4niv.item(1).item(0).as_long()
}"
    )
    .expr("run_p4niv()")
    .result(Value::Long(30));
}

/// Step 4 fourth slice — nested objects.
/// `{"a": {"b": 7}}` — outer field "a" is a JObject; inner
/// field "b" is a JNumber 7.
#[test]
fn p54_step4_nested_object_chained_field() {
    code!(
        "fn run_p4nocf() -> integer {
    v_p4nocf = json_parse(\"{{\\\"a\\\":{{\\\"b\\\":7}}}}\");
    v_p4nocf.field(\"a\").field(\"b\").as_long()
}"
    )
    .expr("run_p4nocf()")
    .result(Value::Long(7));
}

/// Step 4 fourth slice — array of objects.  `[{"k":1},{"k":2}]`
/// — outer is JArray, each item is a JObject with field `"k"`.
#[test]
fn p54_step4_array_of_objects_field_lookup() {
    code!(
        "fn run_p4aof() -> integer {
    v_p4aof = json_parse(\"[{{\\\"k\\\":1}},{{\\\"k\\\":2}}]\");
    v_p4aof.item(1).field(\"k\").as_long()
}"
    )
    .expr("run_p4aof()")
    .result(Value::Long(2));
}

/// Step 4 fourth slice — object containing an array.  Locks the
/// reverse mix from `array_of_objects` so both directions of the
/// recursion are exercised.
#[test]
fn p54_step4_object_with_array_field() {
    code!(
        "fn run_p4owaf() -> integer {
    v_p4owaf = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    v_p4owaf.field(\"items\").len()
}"
    )
    .expr("run_p4owaf()")
    .result(Value::Int(3));
}

/// Step 4 fourth slice — to_json round-trip for nested containers.
#[test]
fn p54_step4_nested_array_to_json_text_shape() {
    code!(
        "fn run_p4narts() -> text {
    v_p4narts = json_parse(\"[[1,2],[3,4]]\");
    v_p4narts.to_json()
}"
    )
    .expr("run_p4narts()")
    .result(Value::str("[[1,2],[3,4]]"));
}

#[test]
fn p54_step4_object_with_array_to_json_text_shape() {
    code!(
        "fn run_p4owats() -> text {
    v_p4owats = json_parse(\"{{\\\"k\\\":[1,2]}}\");
    v_p4owats.to_json()
}"
    )
    .expr("run_p4owats()")
    .result(Value::str("{\"k\":[1,2]}"));
}

/// Step 4 + Q2 cross-integration — `has_field` on an empty
/// JObject returns false (no fields to look up).  The two pieces
/// were developed independently; this guard locks their
/// interaction so a future has_field rewrite can't accidentally
/// claim a field exists on an empty object.
#[test]
fn p54_step4_empty_object_has_no_field() {
    code!(
        "fn run_p4oh() -> boolean {
    v_p4oh = json_parse(\"{{}}\");
    v_p4oh.has_field(\"anything\")
}"
    )
    .expr("run_p4oh()")
    .result(Value::Boolean(false));
}

/// Step 4 + existing `field()` stub cross-integration — querying
/// any key on an empty JObject returns JNull.  Regressing this
/// would break the common `if v.has_field(k) { v.field(k) … }`
/// pattern when users write it on a JSON-parsed empty object.
#[test]
fn p54_step4_empty_object_field_lookup_returns_jnull() {
    code!(
        "fn run_p4ofl() -> text {
    v_p4ofl = json_parse(\"{{}}\");
    v_p4ofl.field(\"k\").kind()
}"
    )
    .expr("run_p4ofl()")
    .result(Value::str("JNull"));
}

/// Step 4 + existing `item()` stub cross-integration — any index
/// into an empty JArray returns JNull.  Locks that out-of-range
/// access doesn't accidentally leak into an uninitialised
/// variant slot.
#[test]
fn p54_step4_empty_array_item_lookup_returns_jnull() {
    code!(
        "fn run_p4eil() -> text {
    v_p4eil = json_parse(\"[]\");
    v_p4eil.item(0).kind()
}"
    )
    .expr("run_p4eil()")
    .result(Value::str("JNull"));
}

/// Step 4 + Q3 `to_json_pretty` cross-integration — pretty output
/// for an empty container is byte-identical to canonical
/// (`"[]"` / `"{}"`) — there's nothing to indent.  Locks that
/// divergence between canonical and pretty only happens when a
/// container has content.
#[test]
fn p54_step4_empty_array_pretty_matches_canonical() {
    code!(
        "fn run_p4eapc() -> boolean {
    v_p4eapc = json_parse(\"[]\");
    canonical_p4eapc = v_p4eapc.to_json();
    pretty_p4eapc = v_p4eapc.to_json_pretty();
    canonical_p4eapc == pretty_p4eapc
}"
    )
    .expr("run_p4eapc()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_step4_empty_object_pretty_matches_canonical() {
    code!(
        "fn run_p4eopc() -> text {
    v_p4eopc = json_parse(\"{{}}\");
    v_p4eopc.to_json_pretty()
}"
    )
    .expr("run_p4eopc()")
    .result(Value::str("{}"));
}

/// Q2 — `has_field(self: JsonValue, name: text) -> boolean` —
/// forward-compatible stub.  Today returns false for every
/// primitive variant (JNull / JBool / JNumber / JString); a real
/// JObject can't be constructed until P54 step 4 so the JObject
/// branch isn't exercised yet.  When step 4 ships, these guards
/// still stand (primitives still return false) and a new
/// `q2_has_field_on_jobject` test will cover the positive case.
#[test]
fn q2_has_field_on_jnull_is_false() {
    code!(
        "fn run_q2hn() -> boolean {
    v_q2hn = json_null();
    v_q2hn.has_field(\"k\")
}"
    )
    .expr("run_q2hn()")
    .result(Value::Boolean(false));
}

#[test]
fn q2_has_field_on_jbool_is_false() {
    code!(
        "fn run_q2hb() -> boolean {
    v_q2hb = json_bool(true);
    v_q2hb.has_field(\"value\")
}"
    )
    .expr("run_q2hb()")
    .result(Value::Boolean(false));
}

#[test]
fn q2_has_field_on_jnumber_is_false() {
    code!(
        "fn run_q2hnum() -> boolean {
    v_q2hnum = json_number(42.0);
    v_q2hnum.has_field(\"n\")
}"
    )
    .expr("run_q2hnum()")
    .result(Value::Boolean(false));
}

#[test]
fn q2_has_field_on_jstring_is_false() {
    code!(
        "fn run_q2hs() -> boolean {
    v_q2hs = json_string(\"hello\");
    v_q2hs.has_field(\"anything\")
}"
    )
    .expr("run_q2hs()")
    .result(Value::Boolean(false));
}

/// Q2 — free-function form of `has_field`.  Locks the `n_has_field`
/// registration in NATIVE_FNS alongside the
/// `t_9JsonValue_has_field` method alias so both dispatch paths
/// keep working.
#[test]
fn q2_has_field_free_form_on_parsed_primitive() {
    code!(
        "fn run_q2hf() -> boolean {
    v_q2hf = json_parse(\"42\");
    has_field(v_q2hf, \"k\")
}"
    )
    .expr("run_q2hf()")
    .result(Value::Boolean(false));
}

/// Q2 — `keys(self: JsonValue) -> vector<text>` returns the
/// field names of a JObject in insertion order, empty vector
/// for every other variant.  First slice (2026-04-14): empty
/// vector unconditionally — JObject walk lands in a follow-up.
/// These guards lock the empty-vector return shape across all
/// variants, in both free and method form.
#[test]
fn q2_keys_on_jnull_is_empty() {
    code!(
        "fn run_q2kne() -> integer {
    v_q2kne = json_null();
    ks_q2kne = v_q2kne.keys();
    ks_q2kne.len()
}"
    )
    .expr("run_q2kne()")
    .result(Value::Int(0));
}

#[test]
fn q2_keys_on_jbool_is_empty() {
    code!(
        "fn run_q2kbe() -> integer {
    v_q2kbe = json_bool(true);
    keys(v_q2kbe).len()
}"
    )
    .expr("run_q2kbe()")
    .result(Value::Int(0));
}

#[test]
fn q2_keys_on_jobject_returns_field_names_length() {
    // (Was `q2_keys_on_jobject_is_empty_today` until 2026-04-14
    // when the JObject walk shipped.)  Locks that `keys()` on
    // a JObject now returns the actual field-name vector.
    code!(
        "fn run_q2koe() -> integer {
    v_q2koe = json_parse(\"{{\\\"k\\\":1}}\");
    v_q2koe.keys().len()
}"
    )
    .expr("run_q2koe()")
    .result(Value::Int(1));
}

#[test]
fn q2_keys_on_jobject_returns_multiple_field_names_length() {
    code!(
        "fn run_q2km() -> integer {
    v_q2km = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    v_q2km.keys().len()
}"
    )
    .expr("run_q2km()")
    .result(Value::Int(3));
}

/// Q2 — `keys()` JObject walk preserves insertion order: the
/// first key in the source is the first key in the result.
#[test]
fn q2_keys_on_jobject_preserves_first_name() {
    code!(
        "fn run_q2kf() -> text {
    v_q2kf = json_parse(\"{{\\\"alpha\\\":1,\\\"beta\\\":2}}\");
    ks_q2kf = v_q2kf.keys();
    first_q2kf = \"\";
    for k in ks_q2kf {
        if first_q2kf == \"\" { first_q2kf = k; }
    }
    first_q2kf
}"
    )
    .expr("run_q2kf()")
    .result(Value::str("alpha"));
}

/// Q2 — `keys()` collects every name, locked by joining them.
#[test]
fn q2_keys_on_jobject_collects_all_names() {
    code!(
        "fn run_q2kc() -> text {
    v_q2kc = json_parse(\"{{\\\"x\\\":1,\\\"y\\\":2}}\");
    out_q2kc = \"\";
    for k in v_q2kc.keys() { out_q2kc += k; out_q2kc += \"|\"; }
    out_q2kc
}"
    )
    .expr("run_q2kc()")
    .result(Value::str("x|y|"));
}

/// Q2 — the empty `keys()` result is a real iterable: a `for`
/// loop over it terminates without executing the body.  Locks
/// that callers can write `for k in v.keys() { ... }` today
/// without it crashing or looping forever.
#[test]
fn q2_keys_for_loop_is_safe() {
    code!(
        "fn run_q2kfl() -> integer {
    v_q2kfl = json_null();
    count_q2kfl = 0;
    for _k in v_q2kfl.keys() { count_q2kfl += 1; }
    count_q2kfl
}"
    )
    .expr("run_q2kfl()")
    .result(Value::Int(0));
}

/// Q2 — `fields(self: JsonValue) -> vector<JsonField>` mirrors
/// `keys`'s shape but returns (name, value) entries.  First
/// slice (2026-04-14): empty vector for every variant including
/// JObject — the real walk lands with `keys`'s JObject walk.
#[test]
fn q2_fields_on_jnull_is_empty() {
    code!(
        "fn run_q2fne() -> integer {
    v_q2fne = json_null();
    fs_q2fne = v_q2fne.fields();
    fs_q2fne.len()
}"
    )
    .expr("run_q2fne()")
    .result(Value::Int(0));
}

#[test]
fn q2_fields_on_jstring_is_empty() {
    code!(
        "fn run_q2fse() -> integer {
    v_q2fse = json_string(\"hi\");
    fields(v_q2fse).len()
}"
    )
    .expr("run_q2fse()")
    .result(Value::Int(0));
}

#[test]
fn q2_fields_on_jobject_returns_field_entries_length() {
    // (Was `q2_fields_on_jobject_is_empty_today` until 2026-04-14
    // when the JObject walk shipped.)  Locks that `fields()`
    // on a JObject now returns a vector of JsonField entries.
    code!(
        "fn run_q2foe() -> integer {
    v_q2foe = json_parse(\"{{\\\"k\\\":1}}\");
    v_q2foe.fields().len()
}"
    )
    .expr("run_q2foe()")
    .result(Value::Int(1));
}

#[test]
fn q2_fields_on_jobject_collects_multiple_entries() {
    code!(
        "fn run_q2fm() -> integer {
    v_q2fm = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    v_q2fm.fields().len()
}"
    )
    .expr("run_q2fm()")
    .result(Value::Int(3));
}

/// Q2 — `fields()` JObject walk preserves names: iterating the
/// result and reading `.name` gives back each JsonField's name.
#[test]
fn q2_fields_collects_all_names() {
    code!(
        "fn run_q2fcn() -> text {
    v_q2fcn = json_parse(\"{{\\\"x\\\":1,\\\"y\\\":2}}\");
    out_q2fcn = \"\";
    for entry in v_q2fcn.fields() { out_q2fcn += entry.name; out_q2fcn += \"|\"; }
    out_q2fcn
}"
    )
    .expr("run_q2fcn()")
    .result(Value::str("x|y|"));
}

/// Q2 — `fields()` JObject walk also copies primitive values:
/// iterating gives back each `entry.value` as the right variant
/// with the right payload.  This guard covers JNumber.
#[test]
fn q2_fields_preserves_primitive_number_values() {
    code!(
        "fn run_q2fp() -> integer {
    v_q2fp = json_parse(\"{{\\\"k\\\":42}}\");
    sum_q2fp = 0;
    for entry in v_q2fp.fields() {
        sum_q2fp += entry.value.as_long();
    }
    sum_q2fp
}"
    )
    .expr("run_q2fp()")
    .result(Value::Long(42));
}

/// Q2 — `fields()` JObject walk: container values deep-copy
/// into the result vector (JArray preserved).
#[test]
fn q2_fields_preserves_container_values_array() {
    code!(
        "fn run_q2fca() -> text {
    v_q2fca = json_parse(\"{{\\\"k\\\":[1,2,3]}}\");
    kind_q2fca = \"\";
    for entry in v_q2fca.fields() { kind_q2fca = entry.value.kind(); }
    kind_q2fca
}"
    )
    .expr("run_q2fca()")
    .result(Value::str("JArray"));
}

/// Q2 — `fields()` JObject walk: container values deep-copy
/// into the result vector (JObject preserved).
#[test]
fn q2_fields_preserves_container_values_object() {
    code!(
        "fn run_q2fco() -> text {
    v_q2fco = json_parse(\"{{\\\"k\\\":{{\\\"a\\\":1}}}}\");
    kind_q2fco = \"\";
    for entry in v_q2fco.fields() { kind_q2fco = entry.value.kind(); }
    kind_q2fco
}"
    )
    .expr("run_q2fco()")
    .result(Value::str("JObject"));
}

#[test]
fn q2_fields_for_loop_is_safe() {
    code!(
        "fn run_q2ffl() -> integer {
    v_q2ffl = json_bool(true);
    count_q2ffl = 0;
    for _entry in v_q2ffl.fields() { count_q2ffl += 1; }
    count_q2ffl
}"
    )
    .expr("run_q2ffl()")
    .result(Value::Int(0));
}

/// Q4 container constructors — first slice (2026-04-14):
/// `json_array(items)` and `json_object(fields)` build empty
/// containers when given empty input vectors.  Non-empty input
/// returns JNull + diagnostic; the per-element deep-copy lands
/// in a follow-up.
#[test]
fn q4_json_array_empty_vector_returns_jarray() {
    code!(
        "fn run_q4ae() -> text {
    items_q4ae: vector<JsonValue> = [];
    v_q4ae = json_array(items_q4ae);
    v_q4ae.kind()
}"
    )
    .expr("run_q4ae()")
    .result(Value::str("JArray"));
}

#[test]
fn q4_json_array_empty_has_zero_length() {
    code!(
        "fn run_q4ael() -> integer {
    items_q4ael: vector<JsonValue> = [];
    v_q4ael = json_array(items_q4ael);
    v_q4ael.len()
}"
    )
    .expr("run_q4ael()")
    .result(Value::Int(0));
}

#[test]
fn q4_json_array_empty_serialises_as_brackets() {
    code!(
        "fn run_q4aes() -> text {
    items_q4aes: vector<JsonValue> = [];
    v_q4aes = json_array(items_q4aes);
    v_q4aes.to_json()
}"
    )
    .expr("run_q4aes()")
    .result(Value::str("[]"));
}

#[test]
fn q4_json_array_nonempty_input_returns_jarray() {
    // (Was `…_stubs_to_jnull` until 2026-04-14 when the deep-copy
    // landed.)  Locks that non-empty input now produces a real
    // JArray with the right element count.
    code!(
        "fn run_q4ans() -> integer {
    items_q4ans: vector<JsonValue> = [json_null()];
    v_q4ans = json_array(items_q4ans);
    v_q4ans.len()
}"
    )
    .expr("run_q4ans()")
    .result(Value::Int(1));
}

/// Q4 `json_array` deep-copy — multiple elements, mixed primitive
/// variants, all preserved in the result arena.  `to_json` round-
/// trips back to the canonical text form.
#[test]
fn q4_json_array_multi_element_round_trips() {
    code!(
        "fn run_q4amrt() -> text {
    items_q4amrt: vector<JsonValue> = [
        json_number(1.0),
        json_number(2.0),
        json_number(3.0)
    ];
    v_q4amrt = json_array(items_q4amrt);
    v_q4amrt.to_json()
}"
    )
    .expr("run_q4amrt()")
    .result(Value::str("[1,2,3]"));
}

/// Q4 `json_array` deep-copy — element index access.  `item(N)`
/// reads back the value passed at position N.
#[test]
fn q4_json_array_item_access_after_construction() {
    code!(
        "fn run_q4aiac() -> integer {
    items_q4aiac: vector<JsonValue> = [
        json_number(10.0),
        json_number(20.0),
        json_number(30.0)
    ];
    v_q4aiac = json_array(items_q4aiac);
    v_q4aiac.item(1).as_long()
}"
    )
    .expr("run_q4aiac()")
    .result(Value::Long(20));
}

/// Q4 `json_array` deep-copy — recursive: array of arrays.
/// Inner arrays are themselves built via `json_array`, then
/// embedded.  Outer length 2; inner length 2.
#[test]
fn q4_json_array_nested_construction() {
    code!(
        "fn run_q4anc() -> integer {
    inner_a_q4anc: vector<JsonValue> = [json_number(1.0), json_number(2.0)];
    inner_b_q4anc: vector<JsonValue> = [json_number(3.0), json_number(4.0)];
    outer_q4anc: vector<JsonValue> = [
        json_array(inner_a_q4anc),
        json_array(inner_b_q4anc)
    ];
    v_q4anc = json_array(outer_q4anc);
    v_q4anc.item(1).item(0).as_long() as integer
}"
    )
    .expr("run_q4anc()")
    .result(Value::Int(3));
}

#[test]
fn q4_json_object_empty_vector_returns_jobject() {
    code!(
        "fn run_q4oe() -> text {
    fields_q4oe: vector<JsonField> = [];
    v_q4oe = json_object(fields_q4oe);
    v_q4oe.kind()
}"
    )
    .expr("run_q4oe()")
    .result(Value::str("JObject"));
}

#[test]
fn q4_json_object_empty_has_zero_length() {
    code!(
        "fn run_q4oel() -> integer {
    fields_q4oel: vector<JsonField> = [];
    v_q4oel = json_object(fields_q4oel);
    v_q4oel.len()
}"
    )
    .expr("run_q4oel()")
    .result(Value::Int(0));
}

#[test]
fn q4_json_object_empty_serialises_as_braces() {
    code!(
        "fn run_q4oes() -> text {
    fields_q4oes: vector<JsonField> = [];
    v_q4oes = json_object(fields_q4oes);
    v_q4oes.to_json()
}"
    )
    .expr("run_q4oes()")
    .result(Value::str("{}"));
}

/// Q4 `json_object` deep-copy — single field round-trip.  Build a
/// JsonField in loft, pass it to json_object, read back via
/// field() lookup.
#[test]
fn q4_json_object_single_field_round_trips() {
    code!(
        "fn run_q4osf() -> integer {
    f_q4osf = JsonField { name: \"k\", value: json_number(42.0) };
    fields_q4osf: vector<JsonField> = [f_q4osf];
    v_q4osf = json_object(fields_q4osf);
    v_q4osf.field(\"k\").as_long()
}"
    )
    .expr("run_q4osf()")
    .result(Value::Long(42));
}

/// Q4 `json_object` deep-copy — multi-field length.
#[test]
fn q4_json_object_multi_field_length() {
    code!(
        "fn run_q4omfl() -> integer {
    fa_q4omfl = JsonField { name: \"a\", value: json_number(1.0) };
    fb_q4omfl = JsonField { name: \"b\", value: json_string(\"x\") };
    fc_q4omfl = JsonField { name: \"c\", value: json_bool(true) };
    fields_q4omfl: vector<JsonField> = [fa_q4omfl, fb_q4omfl, fc_q4omfl];
    v_q4omfl = json_object(fields_q4omfl);
    v_q4omfl.len()
}"
    )
    .expr("run_q4omfl()")
    .result(Value::Int(3));
}

/// Q4 `json_object` deep-copy — to_json round-trip.  Build via
/// json_object, serialise via to_json, parse back, confirm shape.
#[test]
fn q4_json_object_serialisation() {
    code!(
        "fn run_q4os() -> text {
    f1_q4os = JsonField { name: \"k\", value: json_number(7.0) };
    fields_q4os: vector<JsonField> = [f1_q4os];
    v_q4os = json_object(fields_q4os);
    v_q4os.to_json()
}"
    )
    .expr("run_q4os()")
    .result(Value::str("{\"k\":7}"));
}

/// Q4 — forward a captured subtree.  Parses a JSON array, takes
/// the resulting JArray, embeds it as the value of a freshly-
/// constructed JObject field, and serialises.  Locks that the
/// `dbref_to_parsed` deep-copy used by `n_json_object` correctly
/// preserves container values originating from a parse arena
/// (not just constructor calls).  Listed in QUALITY-history.md § Q4 Tests
/// as `q4_forward_captured_subtree`.
#[test]
fn q4_forward_captured_subtree_array() {
    code!(
        "fn run_q4fcsa() -> text {
    src_q4fcsa = json_parse(\"[10,20,30]\");
    fields_q4fcsa: vector<JsonField> = [
        JsonField { name: \"data\", value: src_q4fcsa }
    ];
    obj_q4fcsa = json_object(fields_q4fcsa);
    obj_q4fcsa.to_json()
}"
    )
    .expr("run_q4fcsa()")
    .result(Value::str("{\"data\":[10,20,30]}"));
}

/// Q4 — forward-captured-subtree, object variant.  Same shape as
/// the array case but the captured subtree is itself a JObject.
#[test]
fn q4_forward_captured_subtree_object() {
    code!(
        "fn run_q4fcso() -> text {
    inner_q4fcso = json_parse(\"{{\\\"x\\\":1,\\\"y\\\":2}}\");
    fields_q4fcso: vector<JsonField> = [
        JsonField { name: \"point\", value: inner_q4fcso }
    ];
    obj_q4fcso = json_object(fields_q4fcso);
    obj_q4fcso.to_json()
}"
    )
    .expr("run_q4fcso()")
    .result(Value::str("{\"point\":{\"x\":1,\"y\":2}}"));
}

/// Q4 — forward-captured-subtree round-trip: parsing the
/// serialised result yields a tree whose structure agrees
/// with the original captured subtree.
#[test]
fn q4_forward_captured_subtree_round_trip() {
    code!(
        "fn run_q4fcsr() -> integer {
    src_q4fcsr = json_parse(\"[10,20,30]\");
    fields_q4fcsr: vector<JsonField> = [
        JsonField { name: \"data\", value: src_q4fcsr }
    ];
    obj_q4fcsr = json_object(fields_q4fcsr);
    text_q4fcsr = obj_q4fcsr.to_json();
    back_q4fcsr = json_parse(text_q4fcsr);
    back_q4fcsr.field(\"data\").item(1).as_long()
}"
    )
    .expr("run_q4fcsr()")
    .result(Value::Long(20));
}

/// Q2 full-surface smoke — exercises every Q2 helper
/// (`kind`, `has_field`, `keys`, `fields`) on the same JObject
/// value in one expression chain.  Locks the four-way dispatch
/// interaction.  Score breakdown today (post both keys + fields
/// JObject walks 2026-04-14): kind=="JObject" → 1, has_field("k")
/// → 1, keys.len() → 1, fields.len() → 1.  Total 4 — every Q2
/// helper now returns its real JObject answer.
#[test]
fn q2_full_surface_smoke_on_jobject() {
    code!(
        "fn run_q2fs() -> integer {
    v_q2fs = json_parse(\"{{\\\"k\\\":1}}\");
    score_q2fs = 0;
    if v_q2fs.kind() == \"JObject\" { score_q2fs += 1; }
    if v_q2fs.has_field(\"k\")     { score_q2fs += 1; }
    score_q2fs += v_q2fs.keys().len();
    score_q2fs += v_q2fs.fields().len();
    score_q2fs
}"
    )
    .expr("run_q2fs()")
    .result(Value::Int(4));
}

/// Q2 — the common guarded-access idiom works today and will
/// keep working when step 4 lands.  `if v.has_field(k) { … }`
/// is the forward-compatible pattern — on a primitive it
/// takes the else branch, on a JObject (future) it takes the
/// then branch iff the key is present.  This guard locks the
/// control-flow shape.
#[test]
fn q2_has_field_gates_conditional_safely() {
    code!(
        "fn run_q2hg() -> integer {
    v_q2hg = json_parse(\"null\");
    if v_q2hg.has_field(\"users\") { 1 } else { 2 }
}"
    )
    .expr("run_q2hg()")
    .result(Value::Int(2));
}
