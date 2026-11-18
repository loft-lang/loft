// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Two libraries that each declare a struct of one NAME, with other fields, each key their own
//! collections: a `hash<Nm[k]>`, a `sorted<Nm[n]>`, an `index<Nm[n]>`, a `trie<Nm[w]>` and a
//! `spatial<Pt[x, y]>`, as a local and as a struct field, answer the library's own records whichever library is loaded first.
//!
//! The schema registers the second library's struct source-qualified, and its collections
//! with it; `StoreTypeOf::keyed_type` looked a collection up by the element's DEFINITION name
//! and so found the first library's, whose key layout is another struct's: every hash lookup
//! of the second library missed, on both backends, with no diagnostic.
//!
//! @falsified-at: 4338b7a7a — by hand: `keyed_type` naming the element by its definition again.
//!   CHANNEL: value (the printed line against the hand-computed one), on both backends.
//!   ARMED: nothing to arm.
//!   WITNESS: loaded second, `rb` answers `-1 110 77 …` and `-1 55 66` (its hash reads miss);
//!   loaded second, `ra` answers `miss s10 i7 …` and `miss f5 f6`.  The trie and spatial cells
//!   were scored on installed main (6ee97714d), which holds the defect: loaded second, `tb`
//!   answers `-1 0/0 3 -1 0/0 3 | -1 -1` — every lookup and every range walk misses, the
//!   lengths hold.
//!   HOLDS: each library alone answers its line on both trees.

const RA: &str = r#"#package
struct Nm { k: text, n: integer, v: text }
struct Box { h: hash<Nm[k]>, s: sorted<Nm[n]>, i: index<Nm[n]> }
pub fn a_local() -> text {
  h: hash<Nm[k]> = [Nm { k: "x", n: 1, v: "vx" }, Nm { k: "y", n: 2, v: "vy" }];
  s: sorted<Nm[n]> = [Nm { k: "p", n: 30, v: "s30" }, Nm { k: "q", n: 10, v: "s10" }];
  i: index<Nm[n]> = [Nm { k: "p", n: 7, v: "i7" }, Nm { k: "q", n: 3, v: "i3" }];
  o = "";
  for e in s { o += e.v; }
  for e in i { o += e.v; }
  "{h["y"].v ?? "miss"} {s[10].v ?? "miss"} {i[7].v ?? "miss"} {len(h)}{len(s)}{len(i)} {o}"
}
pub fn a_field() -> text {
  b = Box { h: [Nm { k: "x", n: 1, v: "fx" }], s: [Nm { k: "p", n: 5, v: "f5" }], i: [Nm { k: "q", n: 6, v: "f6" }] };
  "{b.h["x"].v ?? "miss"} {b.s[5].v ?? "miss"} {b.i[6].v ?? "miss"}"
}
"#;

const RB: &str = r#"#package
struct Nm { n: integer, k: text, v: integer }
struct Box { h: hash<Nm[k]>, s: sorted<Nm[n]>, i: index<Nm[n]> }
pub fn b_local() -> text {
  h: hash<Nm[k]> = [Nm { n: 1, k: "x", v: 100 }, Nm { n: 2, k: "y", v: 200 }];
  s: sorted<Nm[n]> = [Nm { n: 30, k: "p", v: 330 }, Nm { n: 10, k: "q", v: 110 }];
  i: index<Nm[n]> = [Nm { n: 7, k: "p", v: 77 }, Nm { n: 3, k: "q", v: 33 }];
  t = 0;
  for e in s { t = t * 1000 + e.v; }
  for e in i { t = t * 100 + e.v; }
  "{h["y"].v ?? -1} {s[10].v ?? -1} {i[7].v ?? -1} {len(h)}{len(s)}{len(i)} {t}"
}
pub fn b_field() -> text {
  b = Box { h: [Nm { n: 1, k: "x", v: 11 }], s: [Nm { n: 5, k: "p", v: 55 }], i: [Nm { n: 6, k: "q", v: 66 }] };
  "{b.h["x"].v ?? -1} {b.s[5].v ?? -1} {b.i[6].v ?? -1}"
}
"#;

/// What `ra` answers: `h["y"]`, `s[10]`, `i[7]`, the three lengths, the two walks in key order;
/// then the same reads through a struct's fields.
const A: &str = "vy s10 i7 222 s10s30i3i7 | fx f5 f6";
/// What `rb` answers: 200, 110, 77, the lengths, and the walks folded into one number —
/// `110`, then `110 * 1000 + 330`, then `* 100 + 33`, then `* 100 + 77`.
const B: &str = "200 110 77 222 1103303377 | 11 55 66";

const CALL: &str = "fn main() { println(\"{ra::a_local()} | {ra::a_field()} | {rb::b_local()} | {rb::b_field()}\"); }\n";

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn have_cc() -> bool {
    loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

const TA: &str = r#"#package
struct Nm { w: text, v: text }
struct Pt { x: integer, y: integer, n: integer, v: text }
struct Box { t: trie<Nm[w]>, s: spatial<Pt[x, y]> }
fn walk(t: trie<Nm[w]>, pre: text) -> text { o = ""; for e in t[pre..] { o += e.v; } o }
fn inbox(s: spatial<Pt[x, y]>) -> text { c = 0; sum = 0; for p in s[(0, 0)..(7, 7)] { c += 1; sum += p.n; } "{c}/{sum}" }
pub fn a_local() -> text {
  t: trie<Nm[w]> = [Nm { w: "apple", v: "va" }, Nm { w: "apex", v: "vx" }, Nm { w: "bob", v: "vb" }];
  s: spatial<Pt[x, y]> = [Pt { x: 1, y: 2, n: 10, v: "p12" }, Pt { x: 5, y: 6, n: 20, v: "p56" }, Pt { x: 9, y: 9, n: 40, v: "p99" }];
  "{t["apex"].v ?? "miss"} {walk(t, "ap")} {len(t)} {s[5, 6].v ?? "miss"} {inbox(s)} {len(s)}"
}
pub fn a_field() -> text {
  b = Box { t: [Nm { w: "cat", v: "fc" }], s: [Pt { x: 3, y: 4, n: 7, v: "f34" }] };
  "{b.t["cat"].v ?? "miss"} {b.s[3, 4].v ?? "miss"}"
}
"#;

const TB: &str = r#"#package
struct Nm { v: integer, w: text }
struct Pt { n: integer, y: integer, x: integer }
struct Box { t: trie<Nm[w]>, s: spatial<Pt[x, y]> }
fn walk(t: trie<Nm[w]>, pre: text) -> text { c = 0; sum = 0; for e in t[pre..] { c += 1; sum += e.v; } "{c}/{sum}" }
fn inbox(s: spatial<Pt[x, y]>) -> text { c = 0; sum = 0; for p in s[(0, 0)..(7, 7)] { c += 1; sum += p.n; } "{c}/{sum}" }
pub fn b_local() -> text {
  t: trie<Nm[w]> = [Nm { v: 1, w: "apple" }, Nm { v: 2, w: "apex" }, Nm { v: 4, w: "bob" }];
  s: spatial<Pt[x, y]> = [Pt { n: 100, y: 2, x: 1 }, Pt { n: 200, y: 6, x: 5 }, Pt { n: 400, y: 9, x: 9 }];
  "{t["apex"].v ?? -1} {walk(t, "ap")} {len(t)} {s[5, 6].n ?? -1} {inbox(s)} {len(s)}"
}
pub fn b_field() -> text {
  b = Box { t: [Nm { v: 8, w: "cat" }], s: [Pt { n: 70, y: 4, x: 3 }] };
  "{b.t["cat"].v ?? -1} {b.s[3, 4].n ?? -1}"
}
"#;

/// What `ta` answers: `t["apex"]`, the `ap` prefix walk in key order, the trie's length, the
/// point at (5, 6), the count and sum of the points in the box (0, 0)..(7, 7), the length;
/// then an exact read of each through a struct's fields.
const TRIE_A: &str = "vx vxva 3 p56 2/30 3 | fc f34";
/// What `tb` answers: the same reads over its own layouts (the prefix walk as count/sum).
const TRIE_B: &str = "2 2/3 3 200 2/300 3 | 8 70";

/// `first` and `second` as libraries `a` and `b`, loaded in both orders on both backends: the
/// line `call` prints is `want` each time.
fn both_orders(tag: &str, libs: [(&str, &str); 2], call: &str, want: &str) {
    let dir = std::env::temp_dir().join(format!("loft-keyed-{tag}-{}", std::process::id()));
    let lib = dir.join("lib");
    loft::file_access::create_dir_all(&lib).unwrap();
    for (name, src) in libs {
        loft::file_access::write(lib.join(format!("{name}.loft")), src).unwrap();
    }
    let (a, b) = (libs[0].0, libs[1].0);
    for (name, uses) in [
        ("ab", format!("use {a};\nuse {b};\n")),
        ("ba", format!("use {b};\nuse {a};\n")),
    ] {
        let prog = dir.join(format!("{name}.loft"));
        loft::file_access::write(&prog, format!("{uses}{call}")).unwrap();
        for mode in ["--interpret", "--native-c"] {
            if mode == "--native-c" && !have_cc() {
                continue;
            }
            let o = loft::platform::process::harness_command(loft_bin())
                .args([mode, "--no-warnings", "--lib"])
                .arg(&lib)
                .arg(&prog)
                .env("LOFT_NATIVE_LEAK_CHECK", "1")
                .output()
                .expect("loft runs");
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout),
                format!("{want}\n"),
                "{tag} {name} {mode}: {err}"
            );
            assert!(
                o.status.success() && !err.contains("stores not freed"),
                "{tag} {name} {mode}: {err}"
            );
        }
    }
}

#[test]
fn each_library_keys_its_own_struct_of_a_shared_name() {
    both_orders(
        "same-name",
        [("ra", RA), ("rb", RB)],
        CALL,
        &format!("{A} | {B}"),
    );
}

#[test]
fn a_trie_and_a_spatial_key_their_own_struct_of_a_shared_name() {
    let call = "fn main() { println(\"{ta::a_local()} | {ta::a_field()} | {tb::b_local()} | {tb::b_field()}\"); }\n";
    both_orders(
        "trie-spatial",
        [("ta", TA), ("tb", TB)],
        call,
        &format!("{TRIE_A} | {TRIE_B}"),
    );
}

const WA: &str = r#"#package
struct Nm { k: text, v: text }
fn mk(x: integer) -> hash<Nm[k]> {
  h: hash<Nm[k]> = [];
  for i in 0..x { h += [Nm { k: "k{i}", v: "a{x}{i}" }]; }
  h
}
pub fn a_par() -> text {
  o = "";
  for x in [2, 3] par(r = mk(x), 2) { o += "{len(r)}:{r["k1"].v ?? "miss"};"; }
  o
}
"#;

const WB: &str = r#"#package
struct Nm { v: integer, pad: integer, k: text }
fn mk(x: integer) -> hash<Nm[k]> {
  h: hash<Nm[k]> = [];
  for i in 0..x { h += [Nm { v: x * 10 + i, pad: 7, k: "k{i}" }]; }
  h
}
pub fn b_par() -> text {
  o = "";
  for x in [2, 3] par(r = mk(x), 2) { o += "{len(r)}:{r["k1"].v ?? -1};"; }
  o
}
"#;

/// A `par` worker that answers its library's own `hash<Nm[k]>`: each result's length and its
/// `k1` entry.  The worker's result is sized by the same lookup (`heap_ref_kt` in the rustc
/// writer), which was scored by hand on `--native-rust` as well: main answers `2:-1;3:-1;`
/// for the library loaded second on all three backends.
#[test]
fn a_par_worker_answers_its_own_keyed_struct_of_a_shared_name() {
    let call = "fn main() { println(\"{wa::a_par()} | {wb::b_par()}\"); }\n";
    both_orders(
        "par",
        [("wa", WA), ("wb", WB)],
        call,
        "2:a21;3:a31; | 2:21;3:31;",
    );
}
