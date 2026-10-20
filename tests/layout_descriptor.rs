// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 Phase 0 — the layout-descriptor emitter is a FAITHFUL and SUFFICIENT
// transcription of the store layout. Three independent oracles, no FFI:
//
//   * faithfulness — `LayoutDesc::render_dump` reproduces `Stores::layout_dump`
//     byte-for-byte over a rich corpus (so its hash IS `layout_algo_hash`): the
//     descriptor loses none of the layout facts the @PLN97 contract pins.
//   * sufficiency  — `read_via_descriptor`, driven ONLY by the descriptor, matches
//     `read_data` AND a hand-computed byte string for a nested live value.
//   * anti-vacuity — a deliberately corrupted descriptor DIVERGES from `read_data`
//     (the harness can fail), and the store-internal boundary (keyed collections)
//     is refused by the reader but present as an `Iterated` node in the descriptor.

mod common;
extern crate loft;

use common::cached_default;
use loft::database::{Iterated, LayoutNode, Parts};
use loft::parser::Parser;

/// A corpus spanning every user-writable storage kind: scalars, narrow ints,
/// inline & nested vectors, vector<text>, hash / index / sorted collections, a
/// nested struct (inline), a value enum, and a struct-enum. (Mirrors the @PLN97
/// layout-golden corpus so faithfulness is checked on the same shapes.)
const CORPUS: &str = r#"
struct Scalars { b: boolean, c: character, s: single, f: float, i: integer, t: text }
struct Narrow { a: i32, b: u8, c: u16 }
struct Vec1 { v: vector<integer> }
struct VecNest { vv: vector<vector<integer>> }
struct VecText { v: vector<text> }
struct Item { ik: integer }
struct Bag { items: hash<Item[ik]> }
struct SortedBag { items: sorted<Item[ik]> }
struct IndexBag { items: index<Item[ik]> }
struct RefHost { child: Scalars }
struct Word { w: text }
struct Lexicon { words: trie<Word[w]> }
struct Pin { x: integer, y: integer }
struct Map { pins: spatial<Pin[x, y]> }
enum Color { Red, Green, Blue }
enum Shape { Circle { radius: integer }, Rect { width: integer, height: integer } }
"#;

const CORPUS_TYPES: &[&str] = &[
    "Scalars",
    "Narrow",
    "Vec1",
    "VecNest",
    "VecText",
    "Item",
    "Bag",
    "SortedBag",
    "IndexBag",
    "RefHost",
    "Lexicon",
    "Map",
    "Color",
    "Shape",
];

fn parse(src: &str) -> Parser {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(src, "descriptor_test", false);
    p
}

fn known(p: &Parser, name: &str) -> u16 {
    let kt = p.data.def(p.data.def_nr(name)).known_type;
    assert!(
        kt != u16::MAX,
        "type `{name}` did not resolve — parse drifted"
    );
    kt
}

// ── Oracle 1: faithfulness — descriptor render == layout_dump (⇒ hash == F9) ────

#[test]
fn descriptor_render_reproduces_layout_dump() {
    let p = parse(CORPUS);
    let roots: Vec<u16> = CORPUS_TYPES.iter().map(|n| known(&p, n)).collect();

    let desc = p.database.layout_descriptor(&roots);

    assert_eq!(
        desc.render_dump(),
        p.database.layout_dump(&roots),
        "descriptor render must reproduce the layout dump byte-for-byte (a lost/altered \
         layout fact = a red diff here)"
    );
    assert_eq!(
        desc.layout_hash(),
        p.database.layout_algo_hash(&roots),
        "descriptor layout hash must equal the @PLN97 F9 layout-algo hash"
    );
}

// ── Oracle 1b: positive controls — keyed collections become Iterated, not panic ─

#[test]
fn keyed_collections_emit_iterated_nodes() {
    let p = parse(CORPUS);
    let roots: Vec<u16> = CORPUS_TYPES.iter().map(|n| known(&p, n)).collect();
    let desc = p.database.layout_descriptor(&roots);

    let has = |pred: &dyn Fn(&Iterated) -> bool| {
        desc.nodes
            .values()
            .any(|n| matches!(n, LayoutNode::Iterated(it) if pred(it)))
    };
    assert!(
        has(&|it| matches!(it, Iterated::Hash { .. })),
        "hash<> → Iterated::Hash"
    );
    assert!(
        has(&|it| matches!(it, Iterated::Index { .. })),
        "index<> → Iterated::Index"
    );
    assert!(
        has(&|it| matches!(it, Iterated::Ordered { .. })),
        "struct-field sorted<> is array-backed → Iterated::Ordered"
    );

    // The `Item` element type of the hash is reachable in the descriptor (a cursor
    // will yield records of it in a later phase).
    let item = known(&p, "Item");
    assert!(
        desc.nodes.contains_key(&item),
        "hash element type Item is in the descriptor"
    );
    assert!(
        matches!(desc.nodes.get(&item), Some(LayoutNode::Record(_))),
        "Item is a Record node"
    );
}

// ── Oracle 2: sufficiency — read_via_descriptor == read_data == hand-computed ───

/// A record with a nested (inline) struct field and an inline scalar vector — the
/// meaty `read_data` walk: Record → {Text, Record → {Integer, Single}, Vector →
/// Integer, Boolean}.
const NESTED: &str = r#"
struct Inner { a: integer, b: single }
struct Outer { label: text, inner: Inner, nums: vector<integer>, flag: boolean }
"#;

/// Byte position of a named field within a struct type (from its `Parts::Struct`).
fn field(p: &Parser, tp: u16, name: &str) -> (u32, u16) {
    if let Parts::Struct(fields) = &p.database.types[tp as usize].parts {
        let f = fields
            .iter()
            .find(|f| f.name == name)
            .expect("field exists");
        (u32::from(f.position), f.content)
    } else {
        panic!("type {tp} is not a struct");
    }
}

#[test]
fn read_via_descriptor_matches_read_data_and_truth() {
    let mut p = parse(NESTED);
    let outer_tp = known(&p, "Outer");
    let inner_tp = known(&p, "Inner");

    let (label_pos, _) = field(&p, outer_tp, "label");
    let (inner_pos, inner_content) = field(&p, outer_tp, "inner");
    let (nums_pos, _) = field(&p, outer_tp, "nums");
    let (flag_pos, _) = field(&p, outer_tp, "flag");
    let (a_pos, _) = field(&p, inner_tp, "a");
    let (b_pos, _) = field(&p, inner_tp, "b");
    assert_eq!(
        inner_content, inner_tp,
        "the `inner` field is the inline Inner struct"
    );

    // Build a live Outer value in one store.
    let outer = p.database.database(64);
    {
        let store = &mut p.database.allocations[outer.store_nr as usize];
        let sr = store.set_str("hi");
        store.set_u32_raw(outer.rec, outer.pos + label_pos, sr);
        store.set_int(outer.rec, outer.pos + inner_pos + a_pos, 7);
        store.set_single(outer.rec, outer.pos + inner_pos + b_pos, 1.5);
        let vec_rec = loft::vector::alloc_vector_from_bytes(store, 8, 3, &[]);
        for (i, v) in [10i64, 20, 30].iter().enumerate() {
            store.set_int(vec_rec, 8 + 8 * i as u32, *v);
        }
        store.set_u32_raw(outer.rec, outer.pos + nums_pos, vec_rec);
        store.set_byte(outer.rec, outer.pos + flag_pos, 0, 1); // flag = true
    }

    // Hand-computed truth, in declaration order (== Parts::Struct field order):
    //   label "hi" | inner.a=7 (i64) | inner.b=1.5 (f32) | nums 10,20,30 (i64) | flag=1
    let mut truth: Vec<u8> = Vec::new();
    truth.extend_from_slice(b"hi");
    truth.extend_from_slice(&7i64.to_le_bytes());
    truth.extend_from_slice(&1.5f32.to_le_bytes());
    for v in [10i64, 20, 30] {
        truth.extend_from_slice(&v.to_le_bytes());
    }
    truth.push(1);

    let mut via_read_data = Vec::new();
    p.database
        .read_data(&outer, outer_tp, true, &mut via_read_data);

    let desc = p.database.layout_descriptor(&[outer_tp]);
    let mut via_desc = Vec::new();
    p.database
        .read_via_descriptor(&desc, &outer, outer_tp, true, &mut via_desc)
        .expect("nested value is in the serializable subset");

    assert_eq!(
        via_read_data, truth,
        "read_data must produce the hand-computed bytes"
    );
    assert_eq!(
        via_desc, truth,
        "read_via_descriptor must produce the hand-computed bytes"
    );

    // ── anti-vacuity: a corrupted descriptor MUST diverge from read_data ────────
    let mut bad = desc.clone();
    if let Some(LayoutNode::Record(fields)) = bad.nodes.get_mut(&inner_tp) {
        // point Inner.a at Inner.b's byte offset → reads the wrong bytes
        let bpos = fields.iter().find(|f| f.name == "b").unwrap().position;
        fields.iter_mut().find(|f| f.name == "a").unwrap().position = bpos;
    }
    let mut via_bad = Vec::new();
    p.database
        .read_via_descriptor(&bad, &outer, outer_tp, true, &mut via_bad)
        .unwrap();
    assert_ne!(
        via_bad, via_read_data,
        "a corrupted descriptor must diverge from read_data (proves the round-trip can fail)"
    );
}

// ── Oracle 2b: the store-internal boundary is refused by the byte reader ────────

#[test]
fn keyed_collection_read_is_refused_not_panicked() {
    let mut p = parse(CORPUS);
    let bag_tp = known(&p, "Bag");
    let desc = p.database.layout_descriptor(&[bag_tp]);

    // A zeroed Bag record is enough: the reader reaches the hash field and returns
    // Err (the walkable-subset boundary) before dereferencing anything — never a
    // panic like read_data does.
    let bag = p.database.database(16);
    let err = p
        .database
        .read_via_descriptor(&desc, &bag, bag_tp, true, &mut Vec::new());
    assert!(
        err.is_err(),
        "a hash<> field is refused by the byte reader (store-internal)"
    );
    assert!(
        err.unwrap_err().contains("store-internal"),
        "the refusal names the store-internal boundary"
    );
}

// ── @PLN105 Phase 2: descriptor → JSON (the JS reader's contract) ────────────────

/// Balanced-brace / string-aware JSON well-formedness check — no parser dependency.
fn json_balanced(s: &str) -> bool {
    let (mut braces, mut brackets, mut in_str, mut esc) = (0i32, 0i32, false, false);
    for c in s.chars() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => braces += 1,
            '}' => braces -= 1,
            '[' => brackets += 1,
            ']' => brackets -= 1,
            _ => {}
        }
        if braces < 0 || brackets < 0 {
            return false;
        }
    }
    braces == 0 && brackets == 0 && !in_str
}

#[test]
fn descriptor_to_json_is_well_formed_and_faithful() {
    let p = parse(CORPUS);
    let roots: Vec<u16> = CORPUS_TYPES.iter().map(|n| known(&p, n)).collect();
    let js = p.database.layout_descriptor(&roots).to_json();

    // Well-formed (the JS side does `JSON.parse`), with the three top-level tables.
    assert!(json_balanced(&js), "descriptor JSON is not balanced:\n{js}");
    assert!(js.starts_with("{\"nodes\":{"), "top-level shape:\n{js}");
    for key in ["\"nodes\":{", "\"names\":{", "\"sizes\":{"] {
        assert!(js.contains(key), "missing top-level {key}\n{js}");
    }

    // Every node kind the JS reader (§2) dispatches on is present in the corpus.
    for kind in [
        "\"kind\":\"base\"",     // scalar leaves (text/char/…)
        "\"kind\":\"record\"",   // Scalars / Narrow / RefHost
        "\"kind\":\"vector\"",   // Vec1
        "\"kind\":\"enum\"",     // Color
        "\"kind\":\"iterated\"", // hash / index / sorted
    ] {
        assert!(js.contains(kind), "corpus JSON missing node {kind}\n{js}");
    }

    // Faithful details: a base carries its wire scalar name; the value enum keeps its variants
    // in discriminant order; a keyed collection is iterated with its sub-kind (never structural).
    assert!(
        js.contains("\"kind\":\"base\",\"base\":\"text\""),
        "text base wire name\n{js}"
    );
    for v in ["Red", "Green", "Blue"] {
        assert!(
            js.contains(&format!("\"name\":\"{v}\"")),
            "Color.{v} variant\n{js}"
        );
    }
    assert!(
        js.contains("\"kind\":\"iterated\",\"sub\":\"hash\""),
        "hash<> → iterated/hash\n{js}"
    );
}

// ── @PLN196 step 0: the dump reads back into the description it was rendered from ──

/// The corpus description read back from its own dump, for one program's roots: equal to the
/// live description once the facts the dump does not record are set aside.  The sum of the
/// facts set aside is the report step 0 asks for — what a version-2 sidecar must add.
#[test]
fn the_corpus_dump_reads_back_into_its_description() {
    let p = parse(CORPUS);
    let roots: Vec<u16> = CORPUS_TYPES.iter().map(|n| known(&p, n)).collect();
    let live = p.database.layout_descriptor(&roots);
    let dump = p.database.layout_dump(&roots);
    let back = loft::database::LayoutDesc::from_dump(&dump).expect("the corpus dump parses");
    assert_eq!(
        back.render_dump(),
        dump,
        "the parsed description re-renders the dump"
    );
    assert_eq!(
        back.canonical().unwrap(),
        live.as_dumped().canonical().unwrap(),
        "name-keyed, the dump carries every layout fact the live table has"
    );
}

/// Anti-vacuity for the comparison above: a description whose one field moved compares
/// unequal, and a dump with a position edited reads back as a different description.
#[test]
fn a_moved_field_is_seen_by_the_comparison() {
    let p = parse(CORPUS);
    let roots = [known(&p, "Scalars")];
    let dump = p.database.layout_dump(&roots);
    let edited = dump.replacen("i@", "i@9", 1);
    assert_ne!(edited, dump, "the edit applied");
    match loft::database::LayoutDesc::from_dump(&edited) {
        Ok(back) => assert_ne!(
            back.canonical().unwrap(),
            p.database
                .layout_descriptor(&roots)
                .as_dumped()
                .canonical()
                .unwrap(),
            "a moved field must not compare equal"
        ),
        Err(e) => panic!("an edited position is still a parseable dump: {e}"),
    }
}

/// Step 0 over every program: each program's layout reads back from its own dump, and the
/// facts the dump does not carry are counted per kind.  A program whose dump does not read
/// back, or reads back as a different description, fails here with its name.
#[test]
fn every_corpus_layout_reads_back_from_its_dump() {
    common::on_cli_stack(every_corpus_layout_body);
}

fn every_corpus_layout_body() {
    let (data, db) = cached_default();
    let mut scripts: Vec<std::path::PathBuf> = loft::file_access::read_dir("tests/scripts")
        .expect("tests/scripts")
        .into_iter()
        .map(|e| e.os_spelling())
        .filter(|p| loft::file_access::has_extension(p, "loft"))
        .collect();
    scripts.sort();
    scripts.extend(common::corpus::suite_programs());
    let (mut checked, mut with_types) = (0usize, 0usize);
    let (mut lost_null, mut lost_target, mut lost_group, mut lost_linked) = (0, 0, 0, 0);
    let mut failures: Vec<String> = Vec::new();
    for script in &scripts {
        let mut p = Parser::new();
        p.data = data.clone();
        p.database = db.clone();
        if !p.parse(&script.to_string_lossy(), false) {
            continue;
        }
        checked += 1;
        let roots = loft::layout_schema::program_roots(&p.data);
        if roots.is_empty() {
            continue;
        }
        with_types += 1;
        let live = p.database.layout_descriptor(&roots);
        let dump = p.database.layout_dump(&roots);
        let name = script.display();
        let back = match loft::database::LayoutDesc::from_dump(&dump) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        match (back.canonical(), live.as_dumped().canonical()) {
            (Ok(a), Ok(b)) if a == b => {}
            (Ok(a), Ok(b)) => {
                let first = b
                    .nodes
                    .iter()
                    .find(|(k, n)| a.nodes.get(k) != Some(n))
                    .map_or_else(String::new, |(k, n)| {
                        format!(
                            " — first at `{}`: live {n:?}, read back {:?}",
                            b.names[k],
                            a.nodes.get(k)
                        )
                    });
                failures.push(format!("{name}: reads back as another layout{first}"));
            }
            (Err(e), _) | (_, Err(e)) => failures.push(format!("{name}: {e}")),
        }
        // Version 2: the dump plus its facts read back into the WHOLE live description.
        let mut full = back.clone();
        match full.apply_facts(&live.render_facts()) {
            Ok(()) => {
                if full.canonical().ok() != live.canonical().ok() {
                    failures.push(format!("{name}: dump + facts read back as another layout"));
                }
            }
            Err(e) => failures.push(format!("{name}: facts do not apply: {e}")),
        }
        let fields = || {
            live.nodes.values().flat_map(|n| match n {
                LayoutNode::Record(fs) | LayoutNode::EnumValue(_, fs) => fs.clone(),
                _ => Vec::new(),
            })
        };
        lost_null += usize::from(fields().any(|f| f.nullable));
        lost_target += usize::from(fields().any(|f| f.target != u16::MAX));
        lost_group += usize::from(fields().any(|f| !f.group.is_empty()));
        // Linked shows in the dump wherever a `vector`/`sorted` over the type became an
        // `array`/`ordered`; it is lost only for a linked type nothing renders that way.
        let shown: std::collections::BTreeSet<u16> = live
            .nodes
            .values()
            .filter_map(|n| match n {
                LayoutNode::Array(e) => Some(*e),
                LayoutNode::Iterated(Iterated::Ordered { elem, .. }) => Some(*elem),
                _ => None,
            })
            .collect();
        lost_linked += usize::from(live.linked.iter().any(|t| {
            !shown.contains(t)
                && live.nodes.values().any(
                    |n| matches!(n, LayoutNode::Iterated(Iterated::Hash { elem, .. }) if elem == t),
                )
        }));
    }
    eprintln!(
        "@PLN196 step 0: {checked} programs parsed, {with_types} with types; the dump drops \
         nullability in {lost_null}, a reference's target in {lost_target}, an index group in \
         {lost_group}, a hash element's linkedness in {lost_linked}"
    );
    assert!(
        checked > 100,
        "expected many programs to parse, got {checked}"
    );
    assert!(
        failures.is_empty(),
        "{} program layouts do not read back from their dump:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ── loft#2096: a variant type's disambiguated name is part of the identity ──────

/// Two programs whose struct-enums share the variant name `Item`, declared in the opposite
/// order and with the field types swapped.  The second definer's variant type is named
/// `src1::Item`, so in one program `PV1` holds `Item { v: text }` and in the other
/// `src1::Item { v: u32 }` — the same set of type lines either way.  The enum line must say
/// which, or the identities are equal and `store_load` reads one program's text pointer as
/// the other's number (measured before the fix: `a.v=6`, `b.v=`).
#[test]
fn swapped_same_named_variants_have_different_identities() {
    let x = "enum PV1 { NoneA, Item { v: text } }\nenum PV2 { NoneB, Item { v: u32 } }\n\
             struct H { a: PV1, b: PV2 }\nstruct W { hs: vector<H> }\n";
    let y = "enum PV2 { NoneB, Item { v: text } }\nenum PV1 { NoneA, Item { v: u32 } }\n\
             struct H { a: PV1, b: PV2 }\nstruct W { hs: vector<H> }\n";
    let (px, py) = (parse(x), parse(y));
    let (wx, wy) = ([known(&px, "W")], [known(&py, "W")]);
    let (dx, dy) = (px.database.layout_dump(&wx), py.database.layout_dump(&wy));
    assert_ne!(dx, dy, "the variant swap must change the dump:\n{dx}");
    assert_ne!(
        px.database.layout_algo_hash(&wx),
        py.database.layout_algo_hash(&wy),
        "and the identity a store records"
    );
    // Each reads back into its own description, with each variant bound to its own type.
    for (p, roots, dump) in [(&px, &wx, &dx), (&py, &wy, &dy)] {
        let back = loft::database::LayoutDesc::from_dump(dump).expect("parses");
        assert_eq!(
            back.canonical().unwrap(),
            p.database
                .layout_descriptor(roots)
                .as_dumped()
                .canonical()
                .unwrap(),
            "{dump}"
        );
    }
}
