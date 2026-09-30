// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @C91 — `==` compares content for every type (`@FR-E-Eq`).  The Rust-level guards of the
//! build (@PLN175 § C91): the identity census the flips are measured by.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// The census trace (`LOFT_TRACE_EQ_IDENTITY=1`) of a `--check` over `src`, one line per site.
fn identity_sites(name: &str, src: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("loft-c91-census-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(name);
    std::fs::write(&path, src).expect("write probe");
    let out = Command::new(loft_bin())
        .arg("--check")
        .arg(&path)
        .env("LOFT_TRACE_EQ_IDENTITY", "1")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run loft --check");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "the probe must compile: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter_map(|l| l.strip_prefix("[eq-identity] "))
        // The probe's path is the host's: a Windows temp dir separates with `\`.
        .map(|l| {
            l.rsplit_once(['/', '\\'])
                .map_or(l, |(_, tail)| tail)
                .to_string()
        })
        .collect()
}

/// The census names each `==` / `!=` that still compares identity — by line, kind and operand
/// types — and is silent where the answer does not depend on identity: a `null` test, a
/// scalar, and (since the C91 flips) a struct, a struct-enum and a keyed collection, which
/// compare by content.  Two records of DIFFERENT struct types are the identity lowering left,
/// so the trace must still name that one: a trace that went silent everywhere would report a
/// future flip as changing nothing.
#[test]
fn the_census_names_identity_compares_and_nothing_else() {
    let sites = identity_sites(
        "census.loft",
        "struct P { x: integer }
struct Q { x: integer }
value struct V { x: integer }
enum S { Circle { r: integer }, Square { s: integer } }
struct E { k: integer, v: integer }
fn main() {
  a = P { x: 1 };
  b = P { x: 1 };
  q = Q { x: 1 };
  n: P? = null;
  m = P { x: 2 };
  v = V { x: 1 };
  w = V { x: 1 };
  c = Circle { r: 1 };
  d = Circle { r: 1 };
  h: hash<E[k]> = [];
  g: hash<E[k]> = [];
  assert(a == b, \"struct\");
  assert(!(a != b), \"struct ne\");
  assert(n == null, \"null test\");
  assert(!(m == null), \"null test on a non-optional struct\");
  assert(v == w, \"value struct\");
  assert(1 == 1, \"scalar\");
  assert(c == d, \"struct-enum\");
  assert(h == g, \"collection\");
  assert(!(a == q), \"two struct types\");
}
",
    );
    assert_eq!(
        sites,
        vec!["census.loft:26  struct  P == Q"],
        "the identity census"
    );
}

// ── `Stores::eq_content` over hand-built stores (@PLN175 § C91 step 4) ──────────────────────
//
// Each value is parsed from a literal into its own store, so two equal values never share a
// record and the answer can only come from their content.  Every expected answer is worked
// out by hand from `(E-Eq)`.

use loft::database::Stores;
use loft::keys::DbRef;

/// A record of type `tp` in a fresh store, filled from the loft-data literal `text`.
fn value(stores: &mut Stores, tp: u16, text: &str) -> DbRef {
    let db = stores.database(u32::from(stores.size(tp)).div_ceil(8) + 1);
    stores.set_default_value(tp, &db);
    stores.parse(text, tp, &db);
    db
}

#[test]
fn eq_content_scalars_and_text() {
    let mut stores = Stores::new();
    let s = stores.structure("Rec", 0);
    stores.field(s, "i", stores.name("integer"));
    stores.field(s, "l", stores.name("long"));
    stores.field(s, "f", stores.name("float"));
    stores.field(s, "g", stores.name("single"));
    stores.field(s, "b", stores.name("boolean"));
    stores.field(s, "t", stores.name("text"));
    stores.finish();
    let base = "{i:1,l:2,f:1.5,g:0.25,b:true,t:\"x\"}";
    let a = value(&mut stores, s, base);
    let b = value(&mut stores, s, base);
    assert!(stores.eq_content(&a, &b, s), "equal content in two stores");
    assert!(stores.eq_content(&a, &a, s), "one record is itself");
    for (field, other) in [
        ("i", "{i:9,l:2,f:1.5,g:0.25,b:true,t:\"x\"}"),
        ("l", "{i:1,l:9,f:1.5,g:0.25,b:true,t:\"x\"}"),
        ("f", "{i:1,l:2,f:9.5,g:0.25,b:true,t:\"x\"}"),
        ("g", "{i:1,l:2,f:1.5,g:9.25,b:true,t:\"x\"}"),
        ("b", "{i:1,l:2,f:1.5,g:0.25,b:false,t:\"x\"}"),
        ("t", "{i:1,l:2,f:1.5,g:0.25,b:true,t:\"y\"}"),
    ] {
        let c = value(&mut stores, s, other);
        assert!(
            !stores.eq_content(&a, &c, s),
            "a differing `{field}` differs"
        );
        assert!(!stores.eq_content(&c, &a, s), "in either order (`{field}`)");
    }
    // `0.0 == -0.0`, in a float and in a single field.
    let z = value(&mut stores, s, "{i:1,l:2,f:0.0,g:0.0,b:true,t:\"x\"}");
    let nz = value(&mut stores, s, "{i:1,l:2,f:0.0,g:0.0,b:true,t:\"x\"}");
    let f = u32::from(stores.position(s, "f"));
    let g = u32::from(stores.position(s, "g"));
    stores.store_mut(&nz).set_float(nz.rec, nz.pos + f, -0.0);
    stores.store_mut(&nz).set_single(nz.rec, nz.pos + g, -0.0);
    assert!(stores.eq_content(&z, &nz, s), "0.0 == -0.0");
    // Null equals null, and differs from every value: a float's null is its NaN.
    let n1 = value(&mut stores, s, base);
    let n2 = value(&mut stores, s, base);
    stores
        .store_mut(&n1)
        .set_float(n1.rec, n1.pos + f, f64::NAN);
    assert!(
        !stores.eq_content(&n1, &n2, s),
        "a null float differs from 1.5"
    );
    stores
        .store_mut(&n2)
        .set_float(n2.rec, n2.pos + f, f64::NAN);
    assert!(stores.eq_content(&n1, &n2, s), "two null floats are equal");
    // A text by its characters: "" is a value, and differs from "x".
    let e = value(&mut stores, s, "{i:1,l:2,f:1.5,g:0.25,b:true,t:\"\"}");
    assert!(!stores.eq_content(&a, &e, s), "\"\" differs from \"x\"");
}

#[test]
fn eq_content_nested_struct_and_vectors() {
    let mut stores = Stores::new();
    let elm = stores.structure("Elm", 0);
    stores.field(elm, "n", stores.name("text"));
    stores.field(elm, "c", stores.name("integer"));
    let ints = stores.vector(stores.name("integer"));
    let elms = stores.vector(elm);
    let m = stores.structure("Main", 0);
    stores.field(m, "inner", elm);
    stores.field(m, "xs", ints);
    stores.field(m, "es", elms);
    stores.finish();
    let base = "{inner:{n:\"a\",c:1},xs:[1,2,3],es:[{n:\"p\",c:1},{n:\"q\",c:2}]}";
    let a = value(&mut stores, m, base);
    let b = value(&mut stores, m, base);
    assert!(stores.eq_content(&a, &b, m), "equal nested content");
    for (what, other) in [
        (
            "an inline struct's field",
            "{inner:{n:\"a\",c:9},xs:[1,2,3],es:[{n:\"p\",c:1},{n:\"q\",c:2}]}",
        ),
        (
            "a vector element",
            "{inner:{n:\"a\",c:1},xs:[1,2,4],es:[{n:\"p\",c:1},{n:\"q\",c:2}]}",
        ),
        (
            "a shorter vector",
            "{inner:{n:\"a\",c:1},xs:[1,2],es:[{n:\"p\",c:1},{n:\"q\",c:2}]}",
        ),
        (
            "a longer vector",
            "{inner:{n:\"a\",c:1},xs:[1,2,3,4],es:[{n:\"p\",c:1},{n:\"q\",c:2}]}",
        ),
        (
            "a deep field of the last element",
            "{inner:{n:\"a\",c:1},xs:[1,2,3],es:[{n:\"p\",c:1},{n:\"r\",c:2}]}",
        ),
        (
            "the element order",
            "{inner:{n:\"a\",c:1},xs:[3,2,1],es:[{n:\"p\",c:1},{n:\"q\",c:2}]}",
        ),
    ] {
        let c = value(&mut stores, m, other);
        assert!(!stores.eq_content(&a, &c, m), "{what} differs");
        assert!(
            !stores.eq_content(&c, &a, m),
            "{what} differs, in either order"
        );
    }
    let e1 = value(&mut stores, m, "{inner:{n:\"a\",c:1},xs:[],es:[]}");
    let e2 = value(&mut stores, m, "{inner:{n:\"a\",c:1},xs:[],es:[]}");
    assert!(
        stores.eq_content(&e1, &e2, m),
        "two empty vectors are equal"
    );
    assert!(
        !stores.eq_content(&a, &e1, m),
        "an empty vector differs from a full one"
    );
}

#[test]
fn eq_content_keyed_collections_compare_by_content_not_insertion() {
    let mut stores = Stores::new();
    let elm = stores.structure("Elm", 0);
    stores.field(elm, "n", stores.name("text"));
    stores.field(elm, "c", stores.name("integer"));
    let h = stores.hash(elm, &["n".to_string()]);
    let hm = stores.structure("HMain", 0);
    stores.field(hm, "h", h);
    let elm2 = stores.structure("Elm2", 0);
    stores.field(elm2, "n", stores.name("text"));
    stores.field(elm2, "c", stores.name("integer"));
    let srt = stores.sorted(elm2, &[("n".to_string(), true)]);
    let sm = stores.structure("SMain", 0);
    stores.field(sm, "s", srt);
    let elm3 = stores.structure("Elm3", 0);
    stores.field(elm3, "n", stores.name("text"));
    stores.field(elm3, "c", stores.name("integer"));
    let idx = stores.index(elm3, &[("n".to_string(), true)]);
    let im = stores.structure("IMain", 0);
    stores.field(im, "i", idx);
    let elm4 = stores.structure("Elm4", 0);
    stores.field(elm4, "n", stores.name("text"));
    stores.field(elm4, "c", stores.name("integer"));
    let trie = stores.trie(elm4, "n");
    let tm = stores.structure("TMain", 0);
    stores.field(tm, "t", trie);
    stores.finish();
    for (tp, field) in [(hm, "h"), (sm, "s"), (im, "i"), (tm, "t")] {
        let lit = |body: &str| format!("{{{field}:[{body}]}}");
        let a = value(&mut stores, tp, &lit("{n:\"p\",c:1},{n:\"q\",c:2}"));
        let b = value(&mut stores, tp, &lit("{n:\"q\",c:2},{n:\"p\",c:1}"));
        assert!(
            stores.eq_content(&a, &b, tp),
            "`{field}`: the same records inserted in another order are equal"
        );
        let c = value(&mut stores, tp, &lit("{n:\"p\",c:1},{n:\"q\",c:3}"));
        assert!(
            !stores.eq_content(&a, &c, tp),
            "`{field}`: a differing value under one key differs"
        );
        let d = value(&mut stores, tp, &lit("{n:\"p\",c:1}"));
        assert!(
            !stores.eq_content(&a, &d, tp),
            "`{field}`: one record fewer differs"
        );
        assert!(
            !stores.eq_content(&d, &a, tp),
            "`{field}`: one record more differs"
        );
    }
}

#[test]
fn eq_content_enum_compares_variant_then_fields() {
    let mut stores = Stores::new();
    let e = stores.enumerate("Shape");
    // A variant's first field is its tag, `enum`, as the parser lays it out.
    let circle = stores.structure("Circle", 1);
    stores.field(circle, "enum", e);
    stores.field(circle, "r", stores.name("integer"));
    let square = stores.structure("Square", 2);
    stores.field(square, "enum", e);
    stores.field(square, "s", stores.name("integer"));
    stores.value(e, "Circle", circle);
    stores.value(e, "Square", square);
    let m = stores.structure("Holder", 0);
    stores.field(m, "shape", e);
    stores.finish();
    let c1 = value(&mut stores, m, "{shape:Circle {r:1}}");
    let c1b = value(&mut stores, m, "{shape:Circle {r:1}}");
    let c2 = value(&mut stores, m, "{shape:Circle {r:2}}");
    let s1 = value(&mut stores, m, "{shape:Square {s:1}}");
    let mut shown = String::new();
    stores.show(&mut shown, &c1, m, false);
    assert!(
        shown.contains("Circle"),
        "the literal parsed as a Circle: {shown}"
    );
    let mut sq = String::new();
    stores.show(&mut sq, &s1, m, false);
    assert!(
        sq.contains("Square"),
        "the literal parsed as a Square: {sq}"
    );
    assert!(stores.eq_content(&c1, &c1b, m), "same variant, same fields");
    assert!(
        !stores.eq_content(&c1, &c2, m),
        "same variant, another field value"
    );
    assert!(
        !stores.eq_content(&c1, &s1, m),
        "another variant, even with the same field value"
    );
}
