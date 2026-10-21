// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 REPL.X — own-format serialization round-trip, both parser paths.
//!
//! The REPL value-snapshot (REPL.X) will serialize a runtime value to loft's
//! OWN format (`show_loft`, not built yet) and rely on it re-parsing through
//! TWO deserializers:
//!
//!   * the **language** parser (`src/parser/`) — re-compiles the session body;
//!   * the **database** parser (`Stores::parse`, lenient JSON + schema walk) —
//!     the `T.parse(...)` / data-literal path.
//!
//! This file is the falsification instrument for that claim: it feeds
//! hand-written own-format text (exactly what `show_loft` must emit) through
//! both parsers and pins which constructs round-trip and which don't *yet* —
//! mapping the three known alignment gaps (struct type-tag, enum-struct
//! `V{…}`, dotted `Enum.V`) and the value-edge corner cases (float decimal,
//! text escaping, null, empty vector).  Tests marked `#[ignore]` are the
//! identified gaps: un-ignore them as the json.rs / walk fixes land.

extern crate loft;

mod testing;

use loft::data::Value;
use loft::database::Stores;

// ─── database-parser path: Stores::parse_message (lenient) ────────────────────

/// Build `enum Category { Daily, Hourly }` + `struct Data { n: integer,
/// t: text, c: Category, amount: float }`; return the stores and `Data`'s tp.
fn data_stores() -> (Stores, u16) {
    let mut stores = Stores::new();
    let e = stores.enumerate("Category");
    stores.value(e, "Daily", u16::MAX);
    stores.value(e, "Hourly", u16::MAX);
    let s = stores.structure("Data", 0);
    stores.field(s, "n", stores.name("integer"));
    stores.field(s, "t", stores.name("text"));
    stores.field(s, "c", stores.name("Category"));
    stores.field(s, "amount", stores.name("float"));
    stores.finish();
    (stores, s)
}

/// True when `parse_message` reported success (it echoes the parsed record;
/// a failure returns a `"line N:M path:X"` diagnostic).
fn parses(msg: &str) -> bool {
    !msg.starts_with("line ")
}

/// BASELINE — debug-`show` shape already round-trips through lenient `parse`
/// (unquoted keys, bare enum tag, quoted text).  This is the existing
/// guarantee REPL.X builds on.
#[test]
fn db_baseline_unquoted_keys_bare_enum() {
    let (mut s, tp) = data_stores();
    let msg = s.parse_message("{n: 42, t: \"hi\", c: Daily}", tp);
    assert!(parses(&msg), "baseline own-format-ish must parse: {msg}");
    assert!(msg.contains("42") && msg.contains("Daily"), "got: {msg}");
}

/// CORNER — text with embedded quote + newline + backslash must round-trip
/// through the escaper.
#[test]
fn db_text_escapes() {
    let (mut s, tp) = data_stores();
    let msg = s.parse_message("{t: \"a\\\"b\\nc\\\\d\"}", tp);
    assert!(parses(&msg), "escaped text must parse: {msg}");
}

/// CORNER — a whole-number float literal (`3.0`).  Probes whether the value
/// survives as a float and whether `parse` accepts the decimal form.
#[test]
fn db_float_whole_number() {
    let (mut s, tp) = data_stores();
    let msg = s.parse_message("{amount: 3.0}", tp);
    assert!(parses(&msg), "float 3.0 must parse: {msg}");
}

/// FIXED (step 2) — a struct **type tag** `Data{…}` round-trips.  The lenient
/// parser now emits a distinct `Parsed::Constructor` for `Tag{…}` (not a
/// collapsed object), and the `Parts::Struct` walker unwraps it to the body.
/// Previously this fail-softed to an all-default record (`{}`), silently
/// dropping every field — so the test compares to the untagged parse, not just
/// "did it error".
#[test]
fn db_struct_type_tag() {
    let (mut s, tp) = data_stores();
    let tagged = s.parse_message("Data{n: 42, t: \"hi\", c: Daily}", tp);
    let untagged = s.parse_message("{n: 42, t: \"hi\", c: Daily}", tp);
    assert!(parses(&tagged), "type-tagged struct must parse: {tagged}");
    assert_eq!(
        tagged, untagged,
        "tagged must round-trip identically to untagged: tagged={tagged} untagged={untagged}"
    );
}

/// DATA PRESERVATION — an unknown enum tag (e.g. a variant removed by a schema
/// edit) must degrade to the **null sentinel**, never silently to variant 1.
/// This is the "keep as much data as possible alive across structure changes"
/// guarantee: a stale value reads back as visibly-absent null, not as a
/// plausible-but-wrong variant — and the parse says so, naming the field
/// (`@FR-D-NoEnumF`: a variant the enum lacks is listed under `#errors`, @C148).
#[test]
fn db_unknown_enum_degrades_to_null() {
    let (mut s, tp) = data_stores();
    let msg = s.parse_message("{c: Weekly}", tp); // `Weekly` ∉ {Daily, Hourly}
    assert!(
        msg.contains("path:c"),
        "an unknown enum tag is reported against its field: {msg}"
    );
    assert!(
        !msg.contains("c:Daily") && !msg.contains("c:Hourly"),
        "unknown variant must NOT map to a real variant: {msg}"
    );
}

/// FIXED (step 1) — **qualified enum** `Category.Daily` (dotted Ident).
/// `show_loft` emits the qualified form so the language parser can infer the
/// type; the DB parser accepts it, mapping by the last segment.  Verifies the
/// *value* round-trips (not just that it parses) — `Hourly`, the SECOND variant,
/// would silently map to variant 1 if the qualifier weren't stripped.
#[test]
fn db_qualified_enum() {
    let (mut s, tp) = data_stores();
    let daily = s.parse_message("{n: 1, t: \"x\", c: Category.Daily}", tp);
    assert!(parses(&daily), "qualified enum must parse: {daily}");
    assert!(daily.contains("c:Daily"), "Category.Daily → Daily: {daily}");
    // The discriminating case: the qualifier must be stripped, not ignored —
    // `Category.Hourly` must land on Hourly, not default to the first variant.
    let hourly = s.parse_message("{n: 1, t: \"x\", c: Category.Hourly}", tp);
    assert!(
        hourly.contains("c:Hourly"),
        "Category.Hourly → Hourly: {hourly}"
    );
}

// ─── serializer: show_loft → own-format text → round-trip ─────────────────────

/// `show_loft` emits re-parseable native loft for a struct value — type tag,
/// qualified enum, forced-decimal float, escaped text — and that output
/// **round-trips** back through the database parser to the same record.  This is
/// the serializer half completing the loop (the parser fixes were steps 1–2).
#[test]
fn show_loft_round_trips_struct() {
    let (mut s, tp) = data_stores();
    let input = "{n: 42, t: \"hi\\nthere\", c: Daily, amount: 3.0}";
    // Build the record, serialise it to own format.
    let rec = s.database(256);
    s.parse(input, tp, &rec);
    let mut loft = String::new();
    s.show_loft(&mut loft, &rec, tp);
    // Native-loft shape: type tag, qualified enum, decimal float, escaped text.
    assert!(loft.starts_with("Data{"), "struct type tag: {loft}");
    assert!(loft.contains("Category.Daily"), "qualified enum: {loft}");
    assert!(
        loft.contains("3.0"),
        "forced-decimal float (not `3`): {loft}"
    );
    assert!(loft.contains("\\n"), "escaped text: {loft}");
    // Round-trip: show_loft output re-parses to the same record as the input.
    let from_loft = s.parse_message(&loft, tp);
    let from_input = s.parse_message(input, tp);
    assert_eq!(
        from_loft, from_input,
        "show_loft output must round-trip to the same value: loft={loft}"
    );
}

// ─── language-parser path: code! (re-compiles loft source) ────────────────────

/// Struct **type tag** parses fine in the language (this is normal loft
/// construction) — the asymmetry with the DB parser above is the whole point.
#[test]
fn lang_struct_type_tag() {
    code!(
        "struct Point { x: integer, y: integer }
fn run() -> integer { a = Point{x: 1, y: 2}; a.x + a.y }"
    )
    .expr("run()")
    .result(Value::Int(3));
}

/// Qualified enum `Color.Red` parses in the language (stdlib uses this form
/// pervasively, e.g. `FileResult.Ok`).
#[test]
fn lang_qualified_enum() {
    code!(
        "enum Color { Red, Green }
fn run() -> boolean { c = Color.Red; c == Color.Red }"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// Text with embedded quote/newline/backslash round-trips as a language literal.
#[test]
fn lang_text_escapes() {
    code!("fn run() -> text { a = \"a\\\"b\\nc\\\\d\"; a }")
        .expr("run()")
        .result(Value::Text("a\"b\nc\\d".to_string()));
}

/// A whole-number float literal stays a float and is usable as one.
#[test]
fn lang_float_whole_number() {
    code!("fn run() -> float { a = 3.0; a + 0.5 }")
        .expr("run()")
        .result(Value::Float(3.5));
}
