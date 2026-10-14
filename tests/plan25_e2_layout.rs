// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN25 single-payload — Probe 1 (the core design claim): the synthetic
//! `__nullable<Row>` enum's `Some` variant carries a SINGLE inline `payload`
//! field whose TYPE is the dense `Row` itself (not Row's fields copied in
//! individually).  A sub-ref at the payload offset is therefore a VALID dense
//! `Row` — it shares Row's field-offset table — so the existing dense-struct
//! layout / construct / access / copy machinery carries null with NO copy.
//! See plans/25-nullable-sequences/single-payload-refactor.md.
//!
//! Single `#[test]` so the `LOFT_E2_SYNTH` gate (read during `fill_all`) is set
//! with no concurrent parsing in this binary.

extern crate loft;

use loft::data::Type;
use loft::file_access as fa;
use loft::parser::Parser;

/// `item: Row?` — the `?` is the whole trigger, and it is the point of the probe. The field
/// used to be written `item: Row`, back when the rewrite matched a bare `Reference` and so fired
/// on exactly the fields that CANNOT be absent; this fixture passed by encoding that inversion
/// (loft#896). A field with no `?` must stay dense, which `a_dense_field_is_not_rewritten`
/// below now holds down.
const SRC: &str = r#"
struct Row { id: integer, tag: text }
struct Box { item: Row?, dense: Row }

fn test() {}
"#;

#[test]
fn nullable_some_carries_a_dense_row_payload() {
    // SAFETY: this binary holds a single test, so no other thread parses while
    // the gate is set.  Edition-2024 `set_var` is `unsafe` for exactly the
    // multi-threaded-mutation hazard we avoid here.
    unsafe {
        std::env::set_var("LOFT_E2_SYNTH", "1");
    }
    // Parse a real FILE (source = MAIN_SOURCE) rather than `parse_str` (which
    // leaves source at 0 = STD_SOURCE, which the scaffolding pass skips).
    let dir = std::env::temp_dir().join("loft_e2_probe");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join("e2.loft");
    fa::write(&path, SRC).expect("write probe");
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse(&path.to_string_lossy(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "probe must parse + lay out clean (synth on): {:?}",
        p.diagnostics.lines()
    );

    // (a) The rewrite happened: Box.item is now the synthetic enum, not Row.
    let box_d = p.data.def_nr("Box");
    let item = p.data.attr(box_d, "item");
    assert_ne!(item, usize::MAX, "Box has an `item` field");
    let item_ty = p.data.attr_type(box_d, item);
    let Type::Enum(syn, true, _) = item_ty else {
        panic!("Box.item must be a nullable enum, got {item_ty:?}");
    };
    assert_eq!(
        p.data.def(syn).name,
        "__nullable<Row>",
        "the synthetic enum is named for the struct"
    );

    // (b) DATA level: the `Some` variant's single data field is `payload`, typed
    // as the dense `Row` — NOT Row's `id`/`tag` copied in individually.
    let some_d = p.data.variant_of(syn, "Some");
    let row_d = p.data.def_nr("Row");
    assert_ne!(some_d, u32::MAX, "Some variant exists");
    let payload_attr = p.data.attr(some_d, "payload");
    assert_ne!(payload_attr, usize::MAX, "Some has a `payload` field");
    let payload_ty = p.data.attr_type(some_d, payload_attr);
    assert!(
        matches!(payload_ty, Type::Reference(d, _) if d == row_d),
        "payload's type is the dense Row, got {payload_ty:?}"
    );

    // (c) DB byte layout: discriminant @0, payload rides past it, and the payload
    // region IS a dense `Row` — Row's own field offsets are intact (a payload
    // sub-ref reads them verbatim) and the whole dense `Row` fits inside `Some`.
    let db = &p.database;
    assert!(db.has_type("__nullable<Row>"), "enum type registered");
    // The synthetic variants register under a parent-enum-qualified key
    // (`__nullable<Row>::Some`) so two `__nullable<S>` enums never collide on the
    // bare name in the flat type table (@PLN25 variant-name fix).
    let some = db.name("__nullable<Row>::Some");
    let row = db.name("Row");
    assert_ne!(some, u16::MAX, "synthetic Some variant registered");
    assert_ne!(row, u16::MAX, "Row structure registered");

    // Discriminant at offset 0 (the whole representation hinges on this).
    assert_eq!(db.position(some, "enum"), 0, "synthetic discriminant @0");
    let payload_pos = db.position(some, "payload");
    assert_ne!(
        payload_pos,
        u16::MAX,
        "Some has a `payload` field in the DB layout"
    );
    assert!(payload_pos >= 1, "payload rides PAST the discriminant");

    // Row's own layout is intact — these are the offsets a payload sub-ref reads.
    assert_ne!(db.position(row, "id"), u16::MAX, "dense Row.id present");
    assert_ne!(db.position(row, "tag"), u16::MAX, "dense Row.tag present");

    // The whole dense `Row` payload fits inside the `Some` record at `payload_pos`
    // — i.e. `Some` is large enough to inline a dense `Row`, so the sub-ref never
    // runs off the record.
    assert!(
        u32::from(payload_pos) + u32::from(db.size(row)) <= u32::from(db.size(some)),
        "dense Row payload (size {}) fits in Some (size {}) at offset {payload_pos}",
        db.size(row),
        db.size(some),
    );

    // The boundary the layout claim rests on: a field declared WITHOUT `?` keeps the dense
    // `Row`. Checked in the same test because the gate is process-wide and this binary holds
    // one test on purpose.
    let dense = p.data.attr(box_d, "dense");
    assert_ne!(dense, usize::MAX, "Box has a `dense` field");
    let dense_ty = p.data.attr_type(box_d, dense);
    assert!(
        matches!(dense_ty, Type::Reference(d, _) if d == row_d),
        "a field with no `?` stays a dense Row, got {dense_ty:?}"
    );

    unsafe {
        std::env::remove_var("LOFT_E2_SYNTH");
    }
}
