// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN114 — the tuple-layout differential oracle.
//!
//! Three attempts to fix the tuple layout by editing a plausible site each returned
//! "no measurable change", which is the signal that the search was at the wrong
//! altitude. This replaces guessing with a comparison.
//!
//! For a matrix of element-type shapes it computes three answers side by side:
//!
//! | column | what it is |
//! |---|---|
//! | **record** | the size of `struct { f0: T0, f1: T1, … }` — the GROUND TRUTH, because records already pack and round-trip correctly |
//! | **tuple** | the size the running system gives `(T0, T1, …)` |
//! | **new** | what `data::element_storage_size` (@PLN114 D1) computes |
//!
//! A shape where `record != tuple` is a defect. A shape where `new != record` means
//! the D1 routine is wrong and must be corrected before it is wired to anything.
//! Both are reported per shape rather than as one pass/fail, so the whole divergence
//! is visible in a single run instead of one site at a time.
//!
//! `cargo test --test pln114_layout_oracle -- --nocapture` prints the table.

use loft::data::{IntegerSpec, Type, element_storage_size};
use loft::parser::Parser;

/// One element type: the loft source spelling plus a human label.
const KINDS: &[(&str, &str)] = &[
    ("u8", "u8"),
    ("i8", "i8"),
    ("u16", "u16"),
    ("i16", "i16"),
    ("i32", "i32"),
    ("u32", "u32"),
    ("integer", "int"),
    ("float", "float"),
    ("single", "single"),
    ("character", "char"),
    ("boolean", "bool"),
    ("text", "text"),
];

/// The shapes to compare — every pair of kinds, plus a few triples/quads that
/// exercise a narrow element in first, middle and last position.
fn shapes() -> Vec<Vec<&'static str>> {
    let mut out: Vec<Vec<&'static str>> = Vec::new();
    for (a, _) in KINDS {
        for (b, _) in KINDS {
            out.push(vec![*a, *b]);
        }
    }
    out.push(vec!["u8", "u32", "u16"]);
    out.push(vec!["u16", "u8", "u32"]);
    out.push(vec!["integer", "u8", "integer"]);
    out.push(vec!["u8", "u8", "u8", "u8"]);
    out
}

/// Parse one program declaring both a record and a tuple over `kinds`, and return
/// `(record_size, tuple_size)` as the DATABASE reports them.
fn measure(kinds: &[&str]) -> Option<(u16, u16)> {
    let fields: Vec<String> = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| format!("f{i}: {k}"))
        .collect();
    let tuple_tp = format!("({})", kinds.join(", "));
    let src = format!(
        "struct Rec {{ {} }}\nfn main() {{ t: {tuple_tp} = {}; r = Rec {{ {} }}; }}\n",
        fields.join(", "),
        sample_tuple(kinds),
        kinds
            .iter()
            .enumerate()
            .map(|(i, k)| format!("f{i}: {}", sample_value(k)))
            .collect::<Vec<_>>()
            .join(", "),
    );
    let mut p = Parser::new();
    p.parse_dir("default", true, false).ok()?;
    p.parse_str(&src, "pln114_oracle", false);
    if p.diagnostics.level() >= loft::diagnostics::Level::Error {
        return None; // shape not expressible — skipped, not a finding
    }
    let rec_d = p.data.def_nr("Rec");
    if rec_d == u32::MAX {
        return None;
    }
    let rec_size = p.database.size(p.data.def(rec_d).known_type);
    let tuple_name = format!(
        "__tuple<{}>",
        kinds
            .iter()
            .map(|k| loft_type_name(k))
            .collect::<Vec<_>>()
            .join(",")
    );
    let tup_d = p.data.def_nr(&tuple_name);
    if tup_d == u32::MAX {
        return None;
    }
    let tup_size = p.database.size(p.data.def(tup_d).known_type);
    Some((rec_size, tup_size))
}

/// The synthetic struct is named after the RESOLVED element types, not the alias.
fn loft_type_name(kind: &str) -> String {
    match kind {
        "u8" => "integer(0, 255)".into(),
        "i8" => "integer(-128, 127)".into(),
        "u16" => "integer(0, 65535)".into(),
        "i16" => "integer(-32768, 32767)".into(),
        "i32" | "integer" => "integer".into(),
        "text" => "text".into(),
        "u32" => "integer(0, 4294967294)".into(),
        other => other.into(),
    }
}

fn sample_value(kind: &str) -> &'static str {
    match kind {
        "float" => "1.0",
        "single" => "1.0f",
        "character" => "'a'",
        "text" => "\"x\"",
        "boolean" => "true",
        _ => "1",
    }
}

fn sample_tuple(kinds: &[&str]) -> String {
    format!(
        "({})",
        kinds
            .iter()
            .map(|k| sample_value(k))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The element `Type` a kind resolves to — mirrors what `parse_type` stamps
/// (`definitions.rs:1869`: the alias's `size(N)` lands in `IntegerSpec.forced_size`).
fn kind_type(kind: &str) -> Type {
    let int = |min: i32, max: u32, forced: Option<u8>| {
        Type::Integer(IntegerSpec {
            min,
            max: i64::from(max),
            not_null: false,
            forced_size: forced.and_then(std::num::NonZeroU8::new),
        })
    };
    match kind {
        "u8" => int(0, 255, Some(1)),
        "i8" => int(-128, 127, Some(1)),
        "u16" => int(0, 65535, Some(2)),
        "i16" => int(-32768, 32767, Some(2)),
        "i32" => int(i32::MIN + 1, i32::MAX as u32, Some(4)),
        "u32" => int(0, 4_294_967_294, Some(4)),
        "integer" => int(i32::MIN + 1, i32::MAX as u32, None),
        "float" => Type::Float,
        "single" => Type::Single,
        "character" => Type::Character,
        "boolean" => Type::Boolean,
        "text" => Type::Text(loft::data::Deps::none()),
        other => panic!("unmapped kind {other}"),
    }
}

/// The D1 routine's answer for this shape.
fn new_routine(kinds: &[&str]) -> Option<usize> {
    let elems: Vec<Type> = kinds.iter().map(|k| kind_type(k)).collect();
    Some(element_storage_size(&Type::Tuple(elems)))
}

// @speed 5.2
#[test]
fn tuple_layout_oracle() {
    let mut rows = Vec::new();
    let mut defects = 0usize;
    let mut routine_wrong = 0usize;
    let mut skipped = 0usize;

    for kinds in shapes() {
        let Some((rec, tup)) = measure(&kinds) else {
            skipped += 1;
            continue;
        };
        let new = new_routine(&kinds);
        let label = kinds.join(",");
        let matches_record = rec == tup;
        let routine_ok = new.map(|n| n as u16 == rec);
        if !matches_record {
            defects += 1;
        }
        if routine_ok == Some(false) {
            routine_wrong += 1;
        }
        rows.push((label, rec, tup, new, matches_record, routine_ok));
    }

    println!("\n  shape                          record  tuple  D1-new   verdict");
    println!("  ------------------------------ ------  -----  ------   -------");
    for (label, rec, tup, new, ok, routine_ok) in &rows {
        let verdict = if !ok {
            "TUPLE != RECORD"
        } else if *routine_ok == Some(false) {
            "D1 wrong"
        } else {
            ""
        };
        println!(
            "  {label:<30} {rec:>6}  {tup:>5}  {:>6}   {verdict}",
            new.map_or("-".into(), |n| n.to_string())
        );
    }
    println!(
        "\n  {} shapes compared · {defects} where tuple != record · {routine_wrong} where D1 != record · {skipped} skipped\n",
        rows.len()
    );

    // Inventory mode: this test REPORTS.  It becomes an assertion when the rewrite
    // lands — flip these to assert_eq!(defects, 0) and assert_eq!(routine_wrong, 0).
    assert!(
        !rows.is_empty(),
        "the oracle measured nothing — harness broken"
    );
}
