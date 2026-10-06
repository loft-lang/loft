// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @C138 — `(L-Align)`: every scalar in a store sits on its natural boundary.
//!
//! A record starts on a word boundary, so two facts decide it: each field's offset is a
//! multiple of its type's alignment, and each record's size is a multiple of the record's own
//! alignment (the stride of that record as a collection element, and how far the next field
//! starts when it is inlined).  `Stores::alignment_violations` asks both of every finished type.
//!
//! A tuple's STORAGE layout is also restated by hand (`data::element_storage_offsets` /
//! `element_storage_size`), for the `par` paths that copy a stored tuple row into a worker's
//! argument slot without the type table at hand.  `(L-Tuple)` requires the two to agree, and
//! they disagreed once already: the hand copy stayed packed tight when the layout was aligned,
//! and a `par` over `vector<(u8, u16)>` summed to 1540 where the answer is 10.

mod common;
extern crate loft;

use common::cached_default;
use loft::data::{Data, Type};
use loft::database::Stores;
use loft::parser::Parser;

/// Every spelling that reaches the layout differently: tail padding, an inline record, a
/// tuple group inside a struct, nested tuples, narrow members, text and record members.
const SHAPES: &str = r#"
struct IB { n: integer, b: boolean }
struct IT { n: integer, t: text }
struct FC { f: float, c: character }
struct U8I { a: u8, n: integer }
struct Lab { name: text, n: integer }
struct Card { label: Lab, id: integer }
struct TupInStruct { a: boolean, p: (integer, integer) }
struct Rows {
  a: vector<(u8, u16)>,
  b: vector<(u8, u32, u16)>,
  c: vector<(text, integer)>,
  d: vector<(u8, text)>,
  e: vector<(integer, (u8, u16))>,
  f: vector<(u8, (u16, u8), integer)>,
  g: vector<(single, u8)>,
  h: vector<(float, u8)>,
  i: vector<(boolean, integer)>,
  j: vector<(character, u8)>,
  k: vector<(Lab, integer)>,
  l: vector<IB>,
}
"#;

fn shapes() -> (Data, Stores) {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(SHAPES, "layout_alignment", false);
    assert!(
        p.diagnostics
            .entries()
            .iter()
            .all(|e| e.level < loft::diagnostics::Level::Error),
        "the shape corpus must parse: {:?}",
        p.diagnostics
            .entries()
            .iter()
            .map(|e| e.to_string_compact())
            .collect::<Vec<_>>()
    );
    (p.data, p.database)
}

/// `@C138` — no finished type, in the standard library or the shape corpus, places a scalar at
/// an offset its alignment does not divide, or has a size its alignment does not divide.
#[test]
fn every_field_and_every_stride_is_aligned() {
    let (_, db) = shapes();
    let violations = db.alignment_violations();
    assert!(
        violations.is_empty(),
        "{} misaligned place(s):\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// `@C138` — the hand-written storage view of a tuple answers what the `__tuple<…>` record's
/// finished layout answers: the same member offsets, and the same size.
#[test]
fn the_storage_view_of_a_tuple_is_its_record_layout() {
    let (data, db) = shapes();
    let mut checked = 0;
    let mut wrong = Vec::new();
    for (nr, t) in db.types.iter().enumerate() {
        if !t.name.starts_with("__tuple<") {
            continue;
        }
        let t_size = db.size(nr as u16);
        let def_nr = data.def_nr(&t.name);
        if def_nr == u32::MAX {
            continue;
        }
        let elems: Vec<Type> = data
            .def(def_nr)
            .attributes()
            .iter()
            .map(|a| a.typedef.clone())
            .collect();
        let Some(stored) =
            loft::data::stored_tuple_offsets_for_def(&data, &db, def_nr, elems.len())
        else {
            continue;
        };
        let by_hand: Vec<u16> = loft::data::element_storage_offsets(&elems)
            .into_iter()
            .map(|o| o as u16)
            .collect();
        let hand_size = loft::data::element_storage_size(&Type::Tuple(elems.clone()));
        if by_hand != stored || hand_size != usize::from(t_size) {
            wrong.push(format!(
                "{}: layout offsets {stored:?} size {t_size}, storage view {by_hand:?} size {hand_size}",
                t.name
            ));
        }
        checked += 1;
    }
    assert!(checked >= 10, "only {checked} tuple records were compared");
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// The rule's own arithmetic, hand-computed: a struct reorders largest-first and pads its tail;
/// a tuple keeps its order and pads before each member.
#[test]
fn the_layouts_are_the_hand_computed_ones() {
    let (_, db) = shapes();
    let size = |name: &str| db.size(db.name(name));
    // integer at 0, boolean at 8: 9 bytes, padded to 16.
    assert_eq!(size("IB"), 16);
    // integer at 0, text handle at 8: 12 bytes, padded to 16.
    assert_eq!(size("IT"), 16);
    // Lab (16, aligned 8) inline at 0, integer at 16.
    assert_eq!(size("Card"), 24);
    // (u8, u16): u8 at 0, u16 at 2 -> 4.  (u8, u32, u16): 0, 4, 8 -> 10, padded to 12.
    assert_eq!(size("__tuple<integer(0, 255),integer(0, 65535)>"), 4);
    assert_eq!(
        size("__tuple<integer(0, 255),integer(0, 4294967294),integer(0, 65535)>"),
        12
    );
    // (text, integer): the handle at 0, the integer at 8 -> 16.
    assert_eq!(size("__tuple<text,integer>"), 16);
    // (u8, (u16, u8), integer): u8 at 0, the inner tuple (4 bytes, aligned 2) at 2, the
    // integer at 8 -> 16.
    assert_eq!(
        size("__tuple<integer(0, 255),(integer(0, 65535), integer(0, 255)),integer>"),
        16
    );
}
