// Copyright (c) 2021-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

extern crate loft;

use loft::database::Stores;
use loft::hash;
#[cfg(feature = "random")]
use loft::keys;
use loft::keys::{Content, DbRef, Str};
use loft::{tree, vector};
#[cfg(feature = "random")]
use rand_core::{RngCore, SeedableRng};
#[cfg(feature = "random")]
use rand_pcg::Pcg64Mcg;

#[test]
pub fn record() {
    let mut stores = Stores::new();
    let e = stores.enumerate("Category");
    stores.value(e, "Daily", u16::MAX);
    stores.value(e, "Hourly", u16::MAX);
    stores.value(e, "Weekly", u16::MAX);
    let s = stores.structure("Data", 0);
    stores.field(s, "name", stores.name("text"));
    stores.field(s, "category", stores.name("Category"));
    stores.field(s, "size", stores.name("integer"));
    stores.field(s, "amount", stores.name("float"));
    stores.field(s, "percentage", stores.name("single"));
    stores.field(s, "calc", stores.name("long"));
    stores.finish();
    assert_eq!(stores.size(stores.name("Data")), 33);
    assert_eq!(stores.enum_val(e, 2), "Hourly");
    assert_eq!(stores.position(s, "size"), 0);
    assert_eq!(stores.position(s, "amount"), 8);
    assert_eq!(stores.position(s, "calc"), 16);
    assert_eq!(stores.position(s, "name"), 24);
    assert_eq!(stores.position(s, "percentage"), 28);
    assert_eq!(stores.position(s, "category"), 32);
    //stores.dump_types();
    let result = stores.database(1234);
    let test_string = "{ name: \"Hello World!\", category: Hourly, size: 12345, percentage: 0.15 }";
    stores.parse(test_string, s, &result);
    let mut check = String::new();
    stores.show(&mut check, &result, s, true);
    // loft#870 — `amount` and `calc` are non-null (nothing marked them nullable), so
    // the keys the input omits land as their type's ZERO, not the null sentinel, and
    // `show` prints them because 0 is a real value of a field that cannot hold null.
    // So parse-then-show NORMALISES rather than echoes: what it prints is the full
    // record, and THAT is what round-trips to itself (checked just below).
    let shown = "{ name: \"Hello World!\", category: Hourly, size: 12345, amount: 0, \
                  percentage: 0.15, calc: 0 }";
    assert_eq!(shown, check);
    let round = stores.database(1234);
    stores.parse(shown, s, &round);
    let mut again = String::new();
    stores.show(&mut again, &round, s, true);
    assert_eq!(shown, again);
    let pf = Stores::get_field(&result, stores.position(s, "percentage") as u32);
    assert_eq!(stores.store(&pf).get_single(pf.rec, pf.pos), 0.15);
    stores.store_mut(&pf).set_single(pf.rec, pf.pos, 0.125);
    check.clear();
    stores.show(&mut check, &result, s, true);
    assert_ne!(shown, check);
    // @P366: an unknown JSON key (`blame`) is now skipped (lenient-ignore),
    // not a parse error.  The object parses to an all-default struct instead of the
    // old `"line 1:7 path:blame"` strict-reject.  This aligns the typed
    // `text as <T>` cast with the dynamic `JsonValue` walker, which already
    // tolerates extra keys.  The defaults are visible (loft#870), and `name` joined
    // them with loft#875: a plain `text` the document omits is the EMPTY string, which
    // is a value `show` prints, not the null it used to hold and skip.  `category` is
    // an enum, whose absent value genuinely is nothing, so it stays unprinted.
    assert_eq!(
        stores.parse_message("{blame:\"nothing\"}", s),
        "{name:\"\",size:0,amount:0,percentage:0,calc:0}"
    );
    assert_eq!("/", stores.path(&result, s));
    assert_eq!(
        stores.parse_message("{name:\"a\",category: Daily}", s),
        "{name:\"a\",category:Daily,size:0,amount:0,percentage:0,calc:0}"
    );
}

/// P54-U phase 3 — leaf type-mismatch errors should report the
/// field's byte offset, not byte 0.  Before the `at`-threading fix,
/// passing `{count: "string"}` to a struct with `count: integer`
/// gave `"line 1:1 path:count"` (mismatch was reported at byte 0
/// because `walk_primitive_into` defaulted `at` to 0 for every
/// type-mismatch arm).  After the fix, the position is the field's
/// key offset (the byte just past `count`'s `:` would be ideal,
/// but the simpler invariant we lock here is "position is non-1").
#[test]
pub fn p54u_leaf_mismatch_reports_field_position() {
    let mut stores = Stores::new();
    let s = stores.structure("Bag", 0);
    stores.field(s, "count", stores.name("integer"));
    stores.finish();
    let msg = stores.parse_message("{count:\"oops\"}", s);
    // The error's column must be > 1 — the mismatch fires inside the
    // value position, not at the start of the input.  `walk_parsed_struct`
    // passes the key's byte offset to the recursion, which
    // `walk_primitive_into` reports on the WalkErr.
    assert!(
        msg.starts_with("line 1:") && !msg.starts_with("line 1:1 "),
        "expected non-byte-0 mismatch position, got: {msg}"
    );
    assert!(
        msg.ends_with("path:count"),
        "expected path:count, got: {msg}"
    );
}

#[test]
pub fn vector() {
    let mut stores = Stores::new();
    let vec = stores.vector(stores.name("integer"));
    let v = stores.structure("Vector", 0);
    stores.field(v, "numbers", vec);
    stores.finish();
    //stores.dump_types();
    let db = stores.database(2);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 8,
    };
    stores.set_default_value(vec, &into);
    let test_string = "{ numbers: [ 1, 2, 55, 11, 22 ]\n}";
    stores.parse(test_string, v, &db);
    let mut check = String::new();
    stores.show(&mut check, &db, v, true);
    assert_eq!(test_string, check);
}

#[test]
pub fn vector_record() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "n", stores.name("text"));
    stores.field(s, "c", stores.name("integer"));
    let v = stores.vector(s);
    stores.finish();
    // stores.dump_types();
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(v, &into);
    let test_string = "[ { n: \"hi\", c: 10 },\n  { n: \"world\", c: 2 } ]";
    stores.parse(test_string, v, &into);
    let mut check = String::new();
    stores.show(&mut check, &into, v, true);
    assert_eq!(test_string, check);
}

#[test]
pub fn sorted_vector() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "cat", stores.name("integer"));
    stores.field(s, "name", stores.name("text"));
    stores.field(s, "value", stores.name("float"));
    let v = stores.sorted(s, &[("cat".to_string(), false), ("name".to_string(), true)]);
    stores.finish();
    let size = stores.size(s);
    //stores.dump_types();
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(v, &into);
    let data = "[
        {cat:1, name:\"first\",value:1.23},
        {cat:1, name:\"second\",value:1.34},
        {cat:1, name:\"third\",value:1.45},
        {cat:2, name:\"first\",value:1.56},
        {cat:2, name:\"second\",value:1.67},
        {cat:2, name:\"third\",value:1.78},
        {cat:3, name:\"first\",value:1.89}
    ]";
    stores.parse(data, v, &into);
    let mut check = String::new();
    stores.show(&mut check, &into, v, true);
    assert_eq!(
        "[ { cat: 3, name: \"first\", value: 1.89 },
  { cat: 2, name: \"first\", value: 1.56 },
  { cat: 2, name: \"second\", value: 1.67 },
  { cat: 2, name: \"third\", value: 1.78 },
  { cat: 1, name: \"first\", value: 1.23 },
  { cat: 1, name: \"second\", value: 1.34 },
  { cat: 1, name: \"third\", value: 1.45 } ]",
        check
    );
    let a = &stores.allocations;
    assert_eq!(
        vector::sorted_find(&into, true, size, a, stores.keys(v), &[]),
        (0, true),
        "First element"
    );
    assert_eq!(
        vector::sorted_find(&into, false, size, a, stores.keys(v), &[]),
        (7, true),
        "Last element"
    );
    assert_eq!(
        vector::sorted_find(&into, false, size, a, stores.keys(v), &[Content::Long(2)]),
        (4, true),
        "Last 2"
    );
    assert_eq!(
        vector::sorted_find(&into, true, size, a, stores.keys(v), &[Content::Long(2)]),
        (1, true),
        "First 2"
    );
    assert_eq!(
        vector::sorted_find(&into, false, size, a, stores.keys(v), &[Content::Long(4)]),
        (0, false),
        "Last 4"
    );
    assert_eq!(
        vector::sorted_find(&into, true, size, a, stores.keys(v), &[Content::Long(0)]),
        (7, false),
        "First 0"
    );
}

#[test]
pub fn hash() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "name", stores.name("text"));
    stores.field(s, "cat", stores.name("integer"));
    stores.field(s, "value", stores.name("float"));
    let m = stores.structure("Main", 0);
    let v = stores.hash(s, &["name".to_string(), "cat".to_string()]);
    stores.field(m, "data", v);
    stores.finish();
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(v, &into);
    let data = "[
        {cat:1, name:\"first\",value:1.23},
        {cat:1, name:\"second\",value:1.34},
        {cat:1, name:\"third\",value:1.45},
        {cat:2, name:\"first\",value:1.56},
        {cat:2, name:\"second\",value:1.67},
        {cat:2, name:\"third\",value:1.78},
        {cat:3, name:\"first\",value:1.89}
    ]";
    stores.parse(data, v, &into);
    let key = [Content::Str(Str::new("second")), Content::Long(2)];
    let mut check = String::new();
    stores.show(
        &mut check,
        &hash::find(&into, &stores.allocations, stores.keys(v), &key),
        s,
        false,
    );
    assert_eq!(check, "{name:\"second\",cat:2,value:1.67}");
    let key = [Content::Str(Str::new("third")), Content::Long(2)];
    let rec = hash::find(&into, &stores.allocations, stores.keys(v), &key);
    assert_eq!("/data[third,2]", stores.path(&rec, s));
    // Unknown key
    let key = [Content::Str(Str::new("first")), Content::Long(4)];
    let rec = hash::find(&into, &stores.allocations, stores.keys(v), &key);
    assert_eq!(rec.rec, 0, "Null result");
}

/// C60 Step 1a: `hash::records` walks all live records in a hash and
/// returns their record numbers.  Order is internal bucket order
/// (unsorted, unspecified).  Callers that need sorted iteration wrap
/// this via Step 2's `records_sorted`.
#[test]
pub fn hash_records_walk() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "name", stores.name("text"));
    stores.field(s, "cat", stores.name("integer"));
    stores.field(s, "value", stores.name("float"));
    let m = stores.structure("Main", 0);
    let v = stores.hash(s, &["name".to_string(), "cat".to_string()]);
    stores.field(m, "data", v);
    stores.finish();
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(v, &into);
    let data = "[
        {cat:1, name:\"first\",value:1.23},
        {cat:1, name:\"second\",value:1.34},
        {cat:2, name:\"third\",value:1.78}
    ]";
    stores.parse(data, v, &into);
    let recs = hash::records(&into, &stores.allocations);
    assert_eq!(
        recs.len(),
        3,
        "hash::records must return every live record: got {recs:?}"
    );
    // Each returned entry must resolve to a non-zero record pointer.
    for entry in &recs {
        assert_ne!(
            entry.rec, 0,
            "record 0 is the null sentinel — should be skipped"
        );
    }
    // The three entries are distinct.  @PLN135 arc H — entries of one hash SHARE a
    // chunk record, so identity is `(rec, pos)`; comparing record numbers alone would
    // now call three separate slots of one chunk a duplicate.
    let mut sorted: Vec<(u32, u32)> = recs.iter().map(|r| (r.rec, r.pos)).collect();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), recs.len(), "duplicate entry returned");
}

#[test]
pub fn hash_records_empty() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "name", stores.name("text"));
    let m = stores.structure("Main", 0);
    let v = stores.hash(s, &["name".to_string()]);
    stores.field(m, "data", v);
    stores.finish();
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(v, &into);
    // Empty hash — no entries added.
    let recs = hash::records(&into, &stores.allocations);
    assert_eq!(
        recs.len(),
        0,
        "empty hash must yield no records: got {recs:?}"
    );
}

/// C60 Step 2: `hash::records_sorted` returns records in ascending
/// key order — single-field key case.
#[test]
pub fn hash_records_sorted_single_field() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "name", stores.name("text"));
    stores.field(s, "value", stores.name("integer"));
    let m = stores.structure("Main", 0);
    let v = stores.hash(s, &["name".to_string()]);
    stores.field(m, "data", v);
    stores.finish();
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(v, &into);
    let data = "[
        {name:\"zebra\",value:1},
        {name:\"apple\",value:5},
        {name:\"mango\",value:3}
    ]";
    stores.parse(data, v, &into);
    let keys = stores.keys(v).to_vec();
    let recs = hash::records_sorted(&into, &stores.allocations, &keys);
    // Resolve each rec-nr to its `name` field for an observable order.
    let name_off = u32::from(stores.position(s, "name"));
    let names: Vec<String> = recs
        .iter()
        .map(|&rec| {
            let name_pos =
                stores.allocations[rec.store_nr as usize].get_u32_raw(rec.rec, rec.pos + name_off);
            stores.allocations[rec.store_nr as usize]
                .get_str(name_pos)
                .to_string()
        })
        .collect();
    assert_eq!(names, vec!["apple", "mango", "zebra"]);
}

/// C60 Step 6: multi-field key, lexicographic order.  Hash keys are
/// ascending-only at the schema level (the `-` descending prefix is a
/// `sorted` / `index` feature that the parser rejects on hash with
/// "Structure doesn't support descending fields" — see
/// `src/parser/definitions.rs:1198`), so the whole key space is
/// lexicographic ascending.
#[test]
pub fn hash_records_sorted_multi_field() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "region", stores.name("text"));
    stores.field(s, "score", stores.name("integer"));
    let m = stores.structure("Main", 0);
    let v = stores.hash(s, &["region".to_string(), "score".to_string()]);
    stores.field(m, "data", v);
    stores.finish();
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(v, &into);
    let data = "[
        {region:\"east\",score:10},
        {region:\"west\",score:30},
        {region:\"east\",score:50},
        {region:\"west\",score:20}
    ]";
    stores.parse(data, v, &into);
    let keys = stores.keys(v).to_vec();
    let recs = hash::records_sorted(&into, &stores.allocations, &keys);
    let region_off = u32::from(stores.position(s, "region"));
    let score_off = u32::from(stores.position(s, "score"));
    let pairs: Vec<(String, i64)> = recs
        .iter()
        .map(|&rec| {
            let store = &stores.allocations[rec.store_nr as usize];
            let region_pos = store.get_u32_raw(rec.rec, rec.pos + region_off);
            let region = store.get_str(region_pos).to_string();
            let score = store.get_int(rec.rec, rec.pos + score_off);
            (region, score)
        })
        .collect();
    // Expected: lexicographic (region ASC, score ASC within region).
    assert_eq!(
        pairs,
        vec![
            ("east".to_string(), 10),
            ("east".to_string(), 50),
            ("west".to_string(), 20),
            ("west".to_string(), 30),
        ]
    );
}

/// C60 Step 3 (path 2c, piece 1): `Stores::build_hash_sorted_vec`
/// emits a u32 rec-nr vector at 4-byte stride — the layout that the
/// on=4 iterate/step arm will walk.  The returned DbRef points at a
/// header whose offset-4 word is the data record; offset 4 of the
/// data record is the element count; offset 8..+4*n holds u32 rec-nrs.
#[test]
pub fn hash_sorted_vec_u32_layout() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "name", stores.name("text"));
    stores.field(s, "value", stores.name("integer"));
    let m = stores.structure("Main", 0);
    let v = stores.hash(s, &["name".to_string()]);
    stores.field(m, "data", v);
    stores.finish();
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(v, &into);
    let data = "[
        {name:\"zebra\",value:1},
        {name:\"apple\",value:5},
        {name:\"mango\",value:3}
    ]";
    stores.parse(data, v, &into);
    let result = stores.build_hash_sorted_vec(&into, v);
    // C60 piece 3 edit A: scratch shares `store_nr` with the hash.
    // This is what lets Ordered (on=3) iteration yield valid hash
    // record refs — the yielded DbRef's store_nr comes from the
    // scratch's store, which is now the same as the hash's store.
    assert_eq!(
        result.store_nr, into.store_nr,
        "scratch must be allocated in the hash's store"
    );
    // Header: offset 4 holds the data-record number.
    let data_rec = stores.allocations[result.store_nr as usize].get_u32_raw(result.rec, 4);
    assert_ne!(data_rec, 0, "header must point at a nonzero data record");
    // Data record: offset 4 = count.
    let count = stores.allocations[result.store_nr as usize].get_u32_raw(data_rec, 4);
    assert_eq!(count, 3, "expected 3 elements, got {count}");
    // @PLN135 arc H — an entry is a SLOT in a chunked arena, so an element of this
    // scratch is a `(record, offset)` PAIR at 8-byte stride, not a bare record number
    // whose payload starts at byte 8.  The header's third word (offset 12) carries
    // that width so `vector::step_ordered` decodes it without being told; assert it
    // here, because a scratch that says 4 while holding pairs reads every second
    // element as an offset and yields records that do not exist.  The same word
    // carries the marker that lets a RELEASED scratch be told from whatever claims
    // its block next — see `a_released_iteration_scratch_is_not_acted_on_twice`.
    let tag = stores.allocations[result.store_nr as usize].get_u32_raw(result.rec, 12);
    assert_eq!(
        tag,
        loft::vector::scratch_tag(8),
        "the hash scratch header must carry the marker and its 8-byte pair stride"
    );
    // Read each pair, resolve its `name` field, verify ascending order.
    // Post-2c layout for Elm{name:text, value:integer}: value at 0 (8B), name at 8 (4B).
    let name_pos = u32::from(stores.position(s, "name"));
    let mut names = Vec::new();
    for i in 0..3u32 {
        let base = 8 + i * 8;
        let store = &stores.allocations[result.store_nr as usize];
        let rec_nr = store.get_u32_raw(data_rec, base);
        let elem_pos = store.get_u32_raw(data_rec, base + 4);
        assert_ne!(rec_nr, 0, "element {i} rec-nr should be nonzero");
        let rec = DbRef {
            store_nr: into.store_nr,
            rec: rec_nr,
            pos: elem_pos,
        };
        let store = &stores.allocations[rec.store_nr as usize];
        let name_off = store.get_u32_raw(rec.rec, rec.pos + name_pos);
        names.push(store.get_str(name_off).to_string());
    }
    assert_eq!(names, vec!["apple", "mango", "zebra"]);
}

#[test]
pub fn array_record() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "n", stores.name("text"));
    stores.field(s, "c", stores.name("integer"));
    let v = stores.vector(s);
    let h = stores.hash(s, &["n".to_string()]);
    let m = stores.structure("Main", 0);
    stores.field(m, "list", v);
    stores.field(m, "search", h);
    stores.finish();
    assert_eq!(
        stores.dump_type("Elm"),
        "Elm[12/8]: parents [Main 10]{n:text[8], c:integer[0]}"
    );
    // `other [65535, 0]`: the leading 65535 marks `search` as a VIEW of records
    // `list` also holds; the `0` is the back-link to `list` itself. Both
    // directions are recorded since loft#843, so an insert maintains the sibling
    // whichever field it is spelled through — it used to be earlier-declared →
    // later-declared only, which made that depend on declaration order.
    assert_eq!(
        stores.dump_type("Main"),
        "Main[8/4]:{list:array<Elm>[0] other [1], search:hash<Elm[n]>[4] other [65535, 0]}"
    );
    let mut into = stores.database(2);
    stores.set_default_value(m, &into);
    let test_string = "{list:[{n:\"hello\",c:10},{n:\"world\",c:2}]}";
    stores.parse(test_string, m, &into);
    let mut check = String::new();
    stores.show(&mut check, &into, m, false);
    assert_eq!(test_string, check);
    let mut check = String::new();
    into.pos = 12; // record base=8, hash_field=4
    let keys = stores.keys(h).to_vec();
    hash::validate(&into, &stores.allocations, &keys);
    let key = [Content::Str(Str::new("hello"))];
    let rec = hash::find(&into, &stores.allocations, &keys, &key);
    stores.show(&mut check, &rec, s, false);
    assert_eq!(check, "{n:\"hello\",c:10}");
}

#[test]
pub fn ordered_record() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "n", stores.name("text"));
    stores.field(s, "c", stores.name("integer"));
    let v = stores.sorted(s, &[("n".to_string(), true)]);
    let h = stores.hash(s, &["n".to_string()]);
    let m = stores.structure("Main", 0);
    stores.field(m, "list", v);
    stores.field(m, "search", h);
    stores.finish();
    assert_eq!(
        stores.dump_type("Elm"),
        "Elm[12/8]: parents [Main 10]{n:text[8], c:integer[0]}"
    );
    // `other [65535, 0]`: the leading 65535 marks `search` as a VIEW of records
    // `list` also holds; the `0` is the back-link to `list` itself. Both
    // directions are recorded since loft#843, so an insert maintains the sibling
    // whichever field it is spelled through — it used to be earlier-declared →
    // later-declared only, which made that depend on declaration order.
    assert_eq!(
        stores.dump_type("Main"),
        "Main[8/4]:{list:ordered<Elm[n]>[0] other [1], search:hash<Elm[n]>[4] other [65535, 0]}"
    );
    let db = stores.database(2);
    let mut into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 8,
    };
    stores.set_default_value(m, &into);
    let test_string = "{list:[{n:\"hello\",c:10},{n:\"world\",c:2}]}";
    stores.parse(test_string, m, &into);
    let mut check = String::new();
    stores.show(&mut check, &into, m, false);
    assert_eq!(test_string, check);
    let mut check = String::new();
    let key = [Content::Str(Str::new("world"))];
    into.pos = 12; // base 8 + hash field 4
    stores.show(
        &mut check,
        &hash::find(&into, &stores.allocations, stores.keys(h), &key),
        s,
        false,
    );
    assert_eq!(check, "{n:\"world\",c:2}");
}

#[test]
pub fn index() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "n", stores.name("text"));
    stores.field(s, "c", stores.name("integer"));
    let v = stores.index(s, &[("n".to_string(), true)]);
    let m = stores.structure("Main", 0);
    stores.field(m, "index", v);
    stores.finish();
    // P191: bookkeeping fields are 4-byte int<0,false> (was 8-byte
    // integer) so tree::add's hardcoded RB_LEFT=0/RB_RIGHT=4/RB_FLAG=8
    // offsets line up with the actual field positions.  Saves 8 bytes
    // per indexed record.
    assert_eq!(
        stores.dump_type("Elm"),
        "Elm[21/8]: parents [Main 10]{n:text[8], c:integer[0], #left_1:int<0,false>[12], #right_1:int<0,false>[16], #color_1:boolean[20]}"
    );
    assert_eq!(
        stores.dump_type("Main"),
        "Main[4/4]:{index:index<Elm[n]>[0]}"
    );
    let db = stores.database(2);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 8,
    };
    stores.set_default_value(m, &into);
    let test_string = "{index:[{n:\"one\",c:1},{n:\"two\",c:2},{n:\"three\",c:3},
{n:\"four\",c:4},{n:\"five\",c:5},{n:\"six\",c:6},{n:\"seven\",c:7},{n:\"eight\",c:8},
{n:\"nine\",c:9},{n:\"ten\",c:10}]}";
    let ordered = "{index:[{n:\"eight\",c:8},{n:\"five\",c:5},{n:\"four\",c:4},\
{n:\"nine\",c:9},{n:\"one\",c:1},{n:\"seven\",c:7},{n:\"six\",c:6},{n:\"ten\",c:10},\
{n:\"three\",c:3},{n:\"two\",c:2}]}";
    stores.parse(test_string, m, &into);
    let mut check = String::new();
    stores.show(&mut check, &into, m, false);
    assert_eq!(ordered, check);
    let mut check = String::new();
    let key = [Content::Str(Str::new("four"))];
    let rec = DbRef {
        store_nr: into.store_nr,
        rec: tree::find(
            &into,
            true,
            stores.fields(v),
            &stores.allocations,
            stores.keys(v),
            &key,
        ),
        pos: 8,
    };
    stores.show(&mut check, &rec, s, false);
    assert_eq!(check, "{n:\"five\",c:5}");
}

#[cfg(feature = "random")]
#[test]
pub fn index_deletions() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "k", stores.name("integer"));
    stores.field(s, "c", stores.name("integer"));
    let v = stores.index(s, &[("k".to_string(), true)]);
    let m = stores.structure("Main", 0);
    stores.field(m, "index", v);
    stores.finish();
    // P191: bookkeeping shrunk to 4-byte int<0,false> (was 8-byte
    // integer); record size 33→25 user bytes.
    assert_eq!(
        stores.dump_type("Elm"),
        "Elm[25/8]: parents [Main 10]{k:integer[0], c:integer[8], #left_1:int<0,false>[16], #right_1:int<0,false>[20], #color_1:boolean[24]}"
    );
    let db = stores.database(2);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 8,
    };
    stores.set_default_value(m, &into);
    let mut recs = vec![];
    let mut rng = Pcg64Mcg::seed_from_u64(42);
    let keys = stores.keys(v).to_vec();
    let elms = 100;
    // Post-2c Elm layout: k:integer[0] c:integer[8] #left[16] #right[24] #color[32].
    // User data starts at rec.pos=8; tree pointers at absolute byte rec.pos+16=24.
    // Record needs 33 user bytes + 8 header = 41 bytes = 6 words.
    for i in 0..elms {
        let rec = stores.claim(&db, 6);
        let s = keys::mut_store(&rec, &mut stores.allocations);
        let key = rng.next_u32();
        // Write k at rec.pos+0=8 (8 bytes), c at rec.pos+8=16 (8 bytes).
        s.set_int(rec.rec, 8, i64::from(key));
        s.set_int(rec.rec, 16, i64::from(i));
        tree::add(&into, &rec, 24, &mut stores.allocations, &keys);
        tree::validate(&into, 24, &stores.allocations, &keys);
        recs.push(rec);
    }
    for d in 0..500 {
        let i = rng.next_u64() % recs.len() as u64;
        let rec = recs[i as usize];
        tree::remove(&into, &rec, 24, &mut stores.allocations, &keys);
        tree::validate(&into, 24, &stores.allocations, &keys);
        let s = keys::mut_store(&rec, &mut stores.allocations);
        let key = rng.next_u32();
        s.set_int(rec.rec, 8, i64::from(key));
        s.set_int(rec.rec, 16, i64::from(100 + d));
        tree::add(&into, &rec, 24, &mut stores.allocations, &keys);
        tree::validate(&into, 24, &stores.allocations, &keys);
    }
}

#[test]
pub fn index_find() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "cat", stores.name("integer"));
    stores.field(s, "name", stores.name("text"));
    stores.field(s, "value", stores.name("float"));
    let v = stores.index(s, &[("cat".to_string(), true), ("name".to_string(), true)]);
    stores.finish();
    // P191: bookkeeping shrunk to 4-byte int<0,false>.
    assert_eq!(
        stores.dump_type("Elm"),
        "Elm[29/8]:{cat:integer[0], name:text[16], value:float[8], #left_1:int<0,false>[20], #right_1:int<0,false>[24], #color_1:boolean[28]}"
    );
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: db.pos,
    };
    stores.set_default_value(v, &into);
    let data = "[ { cat: 1, name: \"first\", value: 1.23 },
  { cat: 1, name: \"second\", value: 1.34 },
  { cat: 1, name: \"third\", value: 1.45 },
  { cat: 2, name: \"first\", value: 1.56 },
  { cat: 2, name: \"second\", value: 1.67 },
  { cat: 2, name: \"third\", value: 1.78 },
  { cat: 3, name: \"first\", value: 1.89 } ]";
    stores.parse(data, v, &into);
    let mut out = String::new();
    stores.show(&mut out, &into, v, true);
    assert_eq!(data, out);
    assert_eq!(
        find_rec(2, true, s, v, &into, &stores),
        "{cat:1,name:\"third\",value:1.45}"
    );
    assert_eq!(
        find_rec(2, false, s, v, &into, &stores),
        "{cat:3,name:\"first\",value:1.89}"
    );
}

#[test]
pub fn hash_load_factor_threshold() {
    // The initial hash has room=9, elms=16.
    // Old threshold: rehash at length=10 (≈62.5% of elms).
    // New threshold: rehash at length=12 (75% of elms).
    // This test adds 12 items and verifies the table stays valid throughout,
    // exercising the range between old and new thresholds.
    let mut stores = Stores::new();
    let s = stores.structure("Item", 0);
    stores.field(s, "key", stores.name("integer"));
    stores.field(s, "val", stores.name("text"));
    let h = stores.hash(s, &["key".to_string()]);
    let m = stores.structure("Container", 0);
    stores.field(m, "data", h);
    stores.finish();
    let db = stores.database(8);
    // "data: hash" is the first user field at byte offset 4 (after the 4-byte type header).
    let hash_ref = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(h, &hash_ref);
    // Build and insert 12 items one at a time, validating after each batch
    let data: String = (1..=12)
        .map(|i| format!("{{key:{i},val:\"item{i}\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    stores.parse(&format!("[{data}]"), h, &hash_ref);
    hash::validate(&hash_ref, &stores.allocations, stores.keys(h));
    // Verify every item is findable
    let keys_list = stores.keys(h).to_vec();
    for i in 1i64..=12 {
        let key = [Content::Long(i)];
        let rec = hash::find(&hash_ref, &stores.allocations, &keys_list, &key);
        assert_ne!(rec.rec, 0, "item {i} not found after load-factor rehash");
    }
}

fn find_rec(key: u8, before: bool, s: u16, v: u16, data: &DbRef, stores: &Stores) -> String {
    let rec = DbRef {
        store_nr: data.store_nr,
        rec: tree::find(
            data,
            before,
            stores.fields(v),
            &stores.allocations,
            stores.keys(v),
            &[Content::Long(key as i64)],
        ),
        pos: 8,
    };
    stores.rec(&rec, s)
}

/// @PLN135 — a bucket table that is no longer the hash's table is given back.
///
/// `add`'s growth path and `reserve` on a non-empty hash both claim a new table, rehash
/// into it and repoint the field. Neither returned the old claim — `hash.rs` never called
/// `Store::delete` at all — so every doubling stranded its predecessor as a block that is
/// CLAIMED and unreachable. `store_reclaim` cannot recover such a block (it is not free),
/// and `store_persist_bind` snapshots the store verbatim, so the dead tables went to disk:
/// a grown 1M-entry hash persisted 49.3 MB where the identical content pre-sized persisted
/// 33.0 MB.
///
/// The assertion is a COUNT, not a size, because a count cannot be argued with: the store's
/// claimed-record total must not depend on how many times the table was rebuilt. Comparing
/// two fills cancels the fixed overhead (the root record, the one live table), so what is
/// left is exactly one claim per entry. Before the fix the larger fill carried one extra
/// claim per doubling it had been through.
#[test]
pub fn hash_growth_frees_the_table_it_replaces() {
    fn claims_for(n: u32) -> (u32, u32) {
        let mut stores = Stores::new();
        let s = stores.structure("Elm", 0);
        stores.field(s, "key", stores.name("integer"));
        let m = stores.structure("Main", 0);
        let v = stores.hash(s, &["key".to_string()]);
        stores.field(m, "data", v);
        stores.finish();
        let db = stores.database(8);
        let into = DbRef {
            store_nr: db.store_nr,
            rec: db.rec,
            pos: 4,
        };
        stores.set_default_value(v, &into);
        let body: Vec<String> = (0..n).map(|i| format!("{{key:{i}}}")).collect();
        stores.parse(&format!("[{}]", body.join(",")), v, &into);
        // Every key must still resolve — freeing a block someone reaches is worse than
        // leaking it, so the count below only means something beside a correctness check.
        for i in 0..n {
            let key = [Content::Long(i64::from(i))];
            let rec = hash::find(&into, &stores.allocations, stores.keys(v), &key);
            assert_ne!(rec.rec, 0, "key {i} of {n} lost across the rebuilds");
        }
        let u = stores.allocations[db.store_nr as usize].usage();
        (u.claimed_count, n)
    }

    // @PLN135 arc H — entries are SLOTS in a chunked arena, so a hash's claimed
    // records no longer scale with the entry count at all.  The whole of what it
    // claims is now hand-computable, which is a stronger statement than the
    // differential this used to make: the store's own root record, the bucket table,
    // the arena's chunk directory, and one record per CHUNK.
    //
    // Chunk `k` holds `arena::BASE << k` slots, so the chunk count for `n` entries is
    // `arena::chunk_of(n - 1) + 1`.  Stated as an exact expectation rather than a
    // formula shared with the code: a closed form that agrees with itself proves
    // nothing, and a stranded table or a leaked chunk shows up here as one extra.
    let expect = |n: u32| -> u32 { 1 + 1 + 1 + loft::arena::chunk_of(n - 1) + 1 };

    // 8 entries stay inside the first table (16 slots, resize at 12) and inside chunk 0.
    let (small, small_n) = claims_for(8);
    // 2000 entries cross ~8 table doublings and six chunks: pre-fix this carried ~8
    // stranded tables, and one record per entry on top.
    let (large, large_n) = claims_for(2000);

    assert_eq!(
        small,
        expect(small_n),
        "{small_n} entries should claim root + table + directory + 1 chunk, got {small}"
    );
    assert_eq!(
        large,
        expect(large_n),
        "{large_n} entries should claim root + table + directory + {} chunks, got {large} \
         — a table it replaced, or a chunk, was never given back",
        loft::arena::chunk_of(large_n - 1) + 1
    );
    // The point of the pair: growing to 2000 costs SIX more records than 8 does, not
    // 1992.  A per-entry claim would put 2000 here.
    assert_eq!(
        large - small,
        5,
        "growing from {small_n} to {large_n} entries must cost only the extra chunks"
    );
}

/// A released iteration scratch must not be acted on again once its block has been
/// re-claimed — and above all must not take the STORE with it.
///
/// `free_iteration_scratch` reads the header's fields to decide what to release, and
/// one of those decisions is *"the elements live in another store, so this scratch has
/// a dedicated one — free the whole store."*  It guarded that with
/// `is_claimed_record`, which answers whether the BLOCK is live, not whether the block
/// is still ours.  A scratch that has already been released leaves its block on the
/// free list; the next claim takes it; every field read after that is somebody else's
/// bytes.
///
/// Nothing re-claimed such a block promptly enough to matter until @PLN135 arc H made
/// a hash's first insert claim an arena CHUNK — which is exactly the size that lands
/// there.  The stale header then read a chunk's payload as `elem_store`, decided it
/// differed, and freed the store the collection itself lives in: every entry gone
/// between one call and the next, no error anywhere.  It surfaced as
/// `multiplayer_v2::v2_two_clients_with_spectator_routing` hanging, because the server
/// keeps its client table in a struct a closure captures and every lookup after the
/// first iteration missed.
///
/// The sequence below is that one, deterministic and without a socket in sight.
#[test]
pub fn a_released_iteration_scratch_is_not_acted_on_twice() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "key", stores.name("integer"));
    let m = stores.structure("Main", 0);
    let v = stores.hash(s, &["key".to_string()]);
    stores.field(m, "data", v);
    stores.finish();
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(v, &into);
    stores.parse("[{key:1},{key:2},{key:3}]", v, &into);

    let scratch = stores.build_hash_unsorted_vec(&into, v);
    assert_eq!(
        scratch.store_nr, into.store_nr,
        "the scratch must share the collection's store, or this proves nothing"
    );
    // Release it once — the normal end of a loop.
    stores.free_iteration_scratch(&scratch);
    // Something else takes the block back.  In the real case that is a hash's first
    // arena chunk; here it is a claim of the header's own size, which is what makes
    // the reuse certain rather than incidental.
    // The free released two blocks — the element vector and the header — so take
    // them back in turn until the HEADER's block is the one re-claimed.
    let store = &mut stores.allocations[scratch.store_nr as usize];
    let mut taker = 0;
    for _ in 0..4 {
        let got = store.claim(2);
        if got == scratch.rec {
            taker = got;
            break;
        }
    }
    // The CONTROL.  If nothing landed on the header's block, the stale reference
    // below still points at its own freed header and the test proves nothing.
    assert_eq!(
        taker, scratch.rec,
        "the re-claim must take the released scratch's block, or this proves nothing"
    );
    // The new tenant's bytes. Offset 8 is where the free path reads the element
    // store; anything that is not this store's number sends it down the
    // free-the-whole-store branch.
    store.set_u32_raw(taker, 4, 0xDEAD);
    store.set_u32_raw(taker, 8, 0xBEEF);
    store.set_u32_raw(taker, 12, 0xF00D);

    // The stale reference, freed a second time.  This is the call that used to read
    // the new tenant's bytes and free the store.
    stores.free_iteration_scratch(&scratch);

    // A freed store keeps its buffer until the slot is reused, so reading a value
    // back proves nothing about its lifetime — ask the store.
    assert!(
        !stores.allocations[into.store_nr as usize].is_free(),
        "a stale scratch reference freed the whole store the collection lives in"
    );
    // And the collection is intact.
    assert_eq!(
        hash::count(&into, &stores.allocations),
        3,
        "the collection lost entries to a stale scratch free"
    );
    for i in 1..=3i64 {
        let key = [Content::Long(i)];
        assert_ne!(
            hash::find(&into, &stores.allocations, stores.keys(v), &key).rec,
            0,
            "key {i} is gone"
        );
    }
}

/// One hash can hold BOTH kinds of entry, and each must be told apart by its slot.
///
/// Entries a collection is asked to create come from its arena (@PLN135 arc H).
/// Entries handed to it already built belong to whoever built them — a sibling
/// field's `other_indexes` makes one collection a second view of another's records,
/// and neither may move or free what the other owns. The loft parser's own `Data`
/// does exactly this: `def_names` receives records the definition list allocated,
/// alongside entries of its own.
///
/// The discriminator was per TABLE first — the recorded stride, on the theory that a
/// table either allocates its entries or borrows them. The parser falsified that in
/// the debug-assertions gate: a hash with a real stride was handed a foreign record,
/// `arena::index_of` answered 0, and the entry went into a slot whose value means
/// EMPTY, so it could never be found again. The discriminator is now the slot's high
/// bit (`hash::SLOT_RECORD`), which can describe a mixed table because it describes
/// one entry at a time.
#[test]
pub fn a_hash_holds_arena_entries_and_borrowed_records_at_once() {
    let mut stores = Stores::new();
    let s = stores.structure("Elm", 0);
    stores.field(s, "key", stores.name("integer"));
    let m = stores.structure("Main", 0);
    let v = stores.hash(s, &["key".to_string()]);
    stores.field(m, "data", v);
    stores.finish();
    let db = stores.database(8);
    let into = DbRef {
        store_nr: db.store_nr,
        rec: db.rec,
        pos: 4,
    };
    stores.set_default_value(v, &into);
    // Its OWN entries, through the normal path: these come from the arena.
    stores.parse("[{key:1},{key:2}]", v, &into);

    // A record somebody else allocated, inserted as a sibling view would insert it.
    let size = u32::from(stores.size(s));
    let foreign = stores.allocations[into.store_nr as usize].claim(1 + size.div_ceil(8));
    stores.allocations[into.store_nr as usize].zero_fill(foreign);
    let elsewhere = DbRef {
        store_nr: into.store_nr,
        rec: foreign,
        pos: 8,
    };
    stores.allocations[into.store_nr as usize].set_int(foreign, 8, 99);
    let keys = stores.keys(v).to_vec();
    hash::add(&into, &elsewhere, &mut stores.allocations, &keys);

    // All three resolve — the borrowed one included, which is the case that used to
    // land in a slot meaning EMPTY.
    for k in [1i64, 2, 99] {
        let key = [Content::Long(k)];
        let got = hash::find(&into, &stores.allocations, stores.keys(v), &key);
        assert_ne!(got.rec, 0, "key {k} is not findable");
        assert_eq!(
            stores.allocations[got.store_nr as usize].get_int(got.rec, got.pos),
            k,
            "key {k} resolved to the wrong entry"
        );
    }
    assert_eq!(hash::count(&into, &stores.allocations), 3);

    // And the walk says WHICH are the table's own, because the teardown frees the
    // arena's entries and must not touch the borrowed record.
    let entries = hash::entries(&into, &stores.allocations);
    let ours = entries.iter().filter(|(_, o)| *o).count();
    let borrowed: Vec<&DbRef> = entries
        .iter()
        .filter(|(_, o)| !*o)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(ours, 2, "the two parsed entries came from the arena");
    assert_eq!(borrowed.len(), 1, "the handed-in record is borrowed");
    assert_eq!(
        borrowed[0].rec, foreign,
        "a borrowed entry must decode back to the record it was given"
    );
}

/// The heap facts of the base types: `text` is the one that owns a heap record.  `character`
/// (base type 6) is an inline codepoint, and a type answering "owns heap" for it sent every
/// copy and free of a `vector<character>` — and of any record with a `character` field —
/// through the per-element claims walk to find nothing, and kept such records off every
/// native rewrite gated on a record of scalars.  No value differs either way, so no `.loft`
/// cell can see it; this reads the fact.
#[test]
pub fn only_text_among_the_base_types_owns_heap() {
    let mut stores = Stores::new();
    let s = stores.structure("Glyph", 0);
    stores.field(s, "ch", stores.name("character"));
    stores.field(s, "width", stores.name("integer"));
    let chars = stores.vector(stores.name("character"));
    stores.finish();
    for name in ["integer", "long", "single", "float", "boolean", "character"] {
        assert!(!stores.owns_heap(stores.name(name)), "{name} owns no heap");
    }
    assert!(
        stores.owns_heap(stores.name("text")),
        "text owns its string"
    );
    assert!(
        !stores.owns_heap(s),
        "a record of a character and an integer owns no heap"
    );
    assert!(stores.owns_heap(chars), "a vector owns its element block");
}
