// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN196 — the store upgrade: a changed definition converts its data, or says why not.
//
// Step 2: the upgrade map, derived from two layouts and nothing else.  Each case is a pair of
// programs — the one that wrote the store and the one that reads it — with the verdict and the
// reason text the map must give.  No store is involved: the map is what `loft layout check`
// prints before any data exists.

mod common;
extern crate loft;

use common::cached_default;
use loft::database::{LayoutDesc, UpgradeMap};
use loft::parser::Parser;

fn parse(src: &str) -> Parser {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(src, "upgrade_test", false);
    assert_eq!(
        p.diagnostics.error_count(),
        0,
        "does not parse:\n{src}\n{:?}",
        p.diagnostics.lines()
    );
    p
}

fn known(p: &Parser, name: &str) -> u16 {
    let kt = p.data.def(p.data.def_nr(name)).known_type;
    assert!(kt != u16::MAX, "type `{name}` did not resolve");
    kt
}

fn id_of(d: &LayoutDesc, name: &str) -> u16 {
    *d.names
        .iter()
        .find(|(_, n)| n.as_str() == name)
        .unwrap_or_else(|| panic!("`{name}` is not in the description"))
        .0
}

/// The map from `old` to `new` for the store rooted at struct `root`, with the old layout
/// taken as a sidecar gives it: the dump text, read back.
fn map_of(old: &str, new: &str, root: &str) -> UpgradeMap {
    let (po, pn) = (parse(old), parse(new));
    let dump = po.database.layout_dump(&[known(&po, root)]);
    let od = LayoutDesc::from_dump(&dump).expect("the old dump reads back");
    let nr = known(&pn, root);
    let nd = pn.database.layout_descriptor(&[nr]);
    UpgradeMap::derive(&od, id_of(&od, root), &nd, nr)
}

/// The map's verdict as one text: its change lines, then its refusals.
fn verdict(old: &str, new: &str, root: &str) -> (bool, String) {
    let m = map_of(old, new, root);
    (m.refusals.is_empty(), m.describe())
}

#[track_caller]
fn admits(old: &str, new: &str, root: &str, expect: &[&str]) {
    let (ok, text) = verdict(old, new, root);
    assert!(ok, "expected an upgrade, got a refusal:\n{text}");
    for e in expect {
        assert!(text.contains(e), "missing `{e}` in:\n{text}");
    }
}

#[track_caller]
fn refuses(old: &str, new: &str, root: &str, expect: &str) {
    let (ok, text) = verdict(old, new, root);
    assert!(!ok, "expected a refusal, got an upgrade:\n{text}");
    assert!(text.contains(expect), "missing `{expect}` in:\n{text}");
}

#[test]
fn an_unchanged_layout_is_the_identity() {
    let s = "struct T { k: integer, n: text }\nstruct W { items: hash<T[k]>, c: integer }\n";
    let m = map_of(s, s, "W");
    assert!(m.is_identity(), "{}", m.describe());
    assert_eq!(m.describe(), "", "nothing to report");
}

#[test]
fn an_added_field_takes_its_default() {
    admits(
        "struct W { a: integer }\n",
        "struct W { a: integer, b: text, c: float = 1.5 }\n",
        "W",
        &["W.b added: filled with its default", "W.c added"],
    );
}

#[test]
fn a_removed_field_is_not_kept() {
    admits(
        "struct W { a: integer, b: text }\n",
        "struct W { a: integer }\n",
        "W",
        &["W.b removed: its values are not kept"],
    );
}

#[test]
fn a_removed_and_an_added_field_look_like_a_rename() {
    refuses(
        "struct W { a: integer, old_name: text }\n",
        "struct W { a: integer, new_name: text }\n",
        "W",
        "looks like a rename",
    );
}

#[test]
fn a_widened_number_is_converted() {
    admits(
        "struct W { a: u8, b: i16, c: single }\n",
        "struct W { a: integer, b: i32, c: float }\n",
        "W",
        &[
            "W.a: widened",
            "W.b: widened",
            "W.c: widened from single to float",
        ],
    );
}

#[test]
fn a_narrowed_number_is_refused() {
    refuses(
        "struct W { a: integer }\n",
        "struct W { a: u8 }\n",
        "W",
        "W.a — narrows",
    );
}

#[test]
fn a_shifted_range_of_the_same_width_is_refused() {
    // Both one byte; `u8` holds 0..255 and `i8` -128..127, so 200 has nowhere to go.
    refuses(
        "struct W { a: u8 }\n",
        "struct W { a: i8 }\n",
        "W",
        "narrows",
    );
}

#[test]
fn a_nullable_number_cannot_become_non_null() {
    refuses(
        "struct W { a: u8? }\n",
        "struct W { a: integer limit(0, 255) }\n",
        "W",
        "cannot hold null",
    );
}

#[test]
fn a_changed_kind_is_refused() {
    refuses(
        "struct W { a: text }\n",
        "struct W { a: integer }\n",
        "W",
        "changes from text to an integer",
    );
}

#[test]
fn a_field_holding_another_struct_is_refused() {
    refuses(
        "struct A { x: integer }\nstruct B { x: integer }\nstruct W { f: A }\n",
        "struct A { x: integer }\nstruct B { x: integer }\nstruct W { f: B }\n",
        "W",
        "now holds a `B` where the store holds a `A`",
    );
}

#[test]
fn a_nested_struct_change_is_found_through_its_field() {
    admits(
        "struct In { x: integer }\nstruct W { inner: In, items: vector<In> }\n",
        "struct In { x: integer, y: integer }\nstruct W { inner: In, items: vector<In> }\n",
        "W",
        &["In.y added"],
    );
}

#[test]
fn an_enum_variant_added_or_removed() {
    admits(
        "enum C { Red, Green }\nstruct W { c: C }\n",
        "enum C { Red, Green, Blue }\nstruct W { c: C }\n",
        "W",
        &["C.Blue added"],
    );
    admits(
        "enum C { Red, Green, Blue }\nstruct W { c: C? }\n",
        "enum C { Red, Blue }\nstruct W { c: C? }\n",
        "W",
        &["C.Green removed: such a value becomes null"],
    );
}

#[test]
fn a_changed_key_rebuilds_the_collection() {
    admits(
        "struct T { a: integer, b: text }\nstruct W { h: hash<T[a]> }\n",
        "struct T { a: integer, b: text }\nstruct W { h: hash<T[b]> }\n",
        "W",
        &["W.h: rebuilt under the key [b] (was [a])"],
    );
}

#[test]
fn a_changed_collection_kind_is_refused() {
    refuses(
        "struct T { a: integer }\nstruct W { h: hash<T[a]> }\n",
        "struct T { a: integer }\nstruct W { h: index<T[a]> }\n",
        "W",
        "changes from a hash to an index",
    );
}

/// The owner's case: a vector's element gains an index somewhere else in the program, so
/// every element moves into a record of its own — and the reverse.
#[test]
fn the_linked_flip_both_directions() {
    let inline = "struct T { k: integer, n: text }\nstruct W { items: vector<T> }\n";
    let linked = "struct T { k: integer, n: text }\nstruct W { items: vector<T> }\n\
                  struct X { by_k: index<T[k]> }\n";
    admits(
        inline,
        linked,
        "W",
        &["W.items: each `T` moves into a record of its own"],
    );
    admits(
        linked,
        inline,
        "W",
        &["W.items: each `T` moves back inline"],
    );
}

#[test]
fn a_reference_without_its_recorded_target_is_refused() {
    // The old layout as a version-1 sidecar gives it: the dump alone, no target.
    refuses(
        "struct T { k: integer }\nstruct W { items: vector<T>, r: reference<T>?, n: integer }\n",
        "struct T { k: integer }\nstruct W { items: vector<T>, r: reference<T>?, n: integer, m: integer }\n",
        "W",
        "does not say what `W.r` points at",
    );
}

#[test]
fn a_reference_with_its_recorded_target_is_carried() {
    // The old layout as a version-2 sidecar gives it: the dump and its facts.
    let old =
        "struct T { k: integer }\nstruct W { items: vector<T>, r: reference<T>?, n: integer }\n";
    let new = "struct T { k: integer }\nstruct W { items: vector<T>, r: reference<T>?, n: integer, m: integer }\n";
    let (po, pn) = (parse(old), parse(new));
    let root = [known(&po, "W")];
    let mut od = LayoutDesc::from_dump(&po.database.layout_dump(&root)).unwrap();
    od.apply_facts(&po.database.layout_descriptor(&root).render_facts())
        .unwrap();
    let nr = known(&pn, "W");
    let m = UpgradeMap::derive(
        &od,
        id_of(&od, "W"),
        &pn.database.layout_descriptor(&[nr]),
        nr,
    );
    assert!(m.refusals.is_empty(), "{}", m.describe());
    assert!(m.describe().contains("W.m added"), "{}", m.describe());
}

// ── The end-to-end cells: a store written by one program, upgraded, read by the next ─────────
//
// Each cell runs the CLI: program A writes `a.store`; `loft layout upgrade b.loft a.store`
// carries it to B's layout; B reads it back with `store_load`, on the interpreter and on
// `--native`, and its output must be the hand-computed text.  A refusing cell must leave the
// store and its sidecar byte for byte as A wrote them.

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn cell_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("loft_upgrade_{}_{name}", std::process::id()));
    let _ = loft::file_access::remove_dir_all(&d);
    loft::file_access::create_dir_all(&d).expect("scratch dir");
    d
}

/// Run `loft <args>` in `dir`; (exit code, stdout, stderr).
fn run_loft(dir: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let out = loft::platform::process::harness_command(loft_bin())
        .args(args)
        .current_dir(dir)
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("loft runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Write the store with `writer` (which must print `saved true`), and answer the cell's dir.
fn write_store(name: &str, writer: &str) -> std::path::PathBuf {
    let dir = cell_dir(name);
    loft::file_access::write(dir.join("a.loft"), writer).unwrap();
    let (code, out, err) = run_loft(&dir, &["a.loft"]);
    assert!(
        code == 0 && out.contains("saved true"),
        "{name}: the writer failed ({code}):\n{out}\n{err}"
    );
    dir
}

/// Run `b.loft` on both backends and require `expect` on stdout.
#[track_caller]
fn reads(name: &str, dir: &std::path::Path, expect: &str, when: &str) {
    for mode in [&[][..], &["--native"][..]] {
        let mut args: Vec<&str> = mode.to_vec();
        args.push("b.loft");
        let (code, out, err) = run_loft(dir, &args);
        assert_eq!(
            (code, out.as_str()),
            (0, expect),
            "{name} {mode:?} {when}: the reader saw\n{out}\n{err}"
        );
    }
}

/// The store file and its sidecar, to compare before and after.
fn store_bytes(dir: &std::path::Path) -> (Vec<u8>, Vec<u8>) {
    (
        loft::file_access::read(dir.join("a.store")).unwrap(),
        loft::file_access::read(dir.join("a.store.dschema")).unwrap(),
    )
}

/// An upgrade that must succeed: B reads `expect` on both backends — first from the OLD file,
/// which its `store_load` converts in memory and leaves as it was, then from the file
/// `loft layout upgrade` converted.
#[track_caller]
fn upgrades(name: &str, writer: &str, reader: &str, expect: &str) {
    let dir = write_store(name, writer);
    loft::file_access::write(dir.join("b.loft"), reader).unwrap();
    let before = store_bytes(&dir);
    reads(name, &dir, expect, "converting in memory");
    assert!(
        store_bytes(&dir) == before,
        "{name}: a converting store_load changed the file"
    );
    let (code, out, err) = run_loft(&dir, &["layout", "upgrade", "b.loft", "a.store"]);
    assert_eq!(code, 0, "{name}: the upgrade was refused:\n{out}\n{err}");
    assert!(
        loft::file_access::exists(dir.join("a.store.old")),
        "{name}: the old store is kept"
    );
    reads(name, &dir, expect, "after the upgrade");
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// An upgrade that must be refused with `reason`, leaving the store untouched.
#[track_caller]
fn upgrade_refused(name: &str, writer: &str, reader: &str, reason: &str) {
    let dir = write_store(name, writer);
    loft::file_access::write(dir.join("b.loft"), reader).unwrap();
    let before = store_bytes(&dir);
    // The reader's own `store_load` refuses it too, with the same reason.
    for mode in [&[][..], &["--native"][..]] {
        let mut args: Vec<&str> = mode.to_vec();
        args.push("b.loft");
        let (code, out, err) = run_loft(&dir, &args);
        assert!(
            code == 0 && out.starts_with("not loaded") && err.contains(reason),
            "{name} {mode:?}: store_load must refuse with `{reason}` ({code}):\n{out}\n{err}"
        );
    }
    let (code, out, err) = run_loft(&dir, &["layout", "upgrade", "b.loft", "a.store"]);
    assert_eq!(code, 1, "{name}: expected a refusal:\n{out}\n{err}");
    assert!(
        err.contains(reason),
        "{name}: missing `{reason}` in:\n{err}"
    );
    let after = store_bytes(&dir);
    assert!(
        before == after,
        "{name}: a refused upgrade changed the store"
    );
    assert!(
        !loft::file_access::exists(dir.join("a.store.old")),
        "{name}: nothing was swapped"
    );
    let _ = loft::file_access::remove_dir_all(&dir);
}

const T_W: &str = "struct T { k: integer, n: text }\nstruct W { a: integer, items: vector<T> }\n";

/// The writer every field cell shares: `a = 5`, three items `k = 1..3`, `n = "t0".."t2"`.
fn writer_w(types: &str) -> String {
    format!(
        "{types}fn main() {{\n  w = W {{ a: 5 }};\n  for i in 0..3 {{ w.items += [T {{ k: i + 1, n: \"t{{i}}\" }}]; }}\n  println(\"saved {{store_persist_copy(w, \\\"a.store\\\")}}\");\n}}\n"
    )
}

fn reader(types: &str, body: &str) -> String {
    format!(
        "{types}fn main() {{\n  w = W {{}};\n  if !store_load(w, \"a.store\") {{ println(\"not loaded\"); return; }}\n{body}}}\n"
    )
}

#[test]
fn cell_unchanged_needs_no_upgrade() {
    let dir = write_store("same", &writer_w(T_W));
    loft::file_access::write(dir.join("b.loft"), reader(T_W, "  println(\"{w.a}\");\n")).unwrap();
    let before = loft::file_access::read(dir.join("a.store")).unwrap();
    let (code, out, err) = run_loft(&dir, &["layout", "upgrade", "b.loft", "a.store"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (0, "", ""),
        "silent when current"
    );
    assert_eq!(
        loft::file_access::read(dir.join("a.store")).unwrap(),
        before,
        "untouched"
    );
    let _ = loft::file_access::remove_dir_all(&dir);
}

#[test]
fn cell_field_added_at_the_top() {
    // a=5 carried, b takes its declared default, c (integer, no default) its zero; the three
    // items arrive in order.
    upgrades(
        "add_top",
        &writer_w(T_W),
        &reader(
            "struct T { k: integer, n: text }\nstruct W { a: integer, items: vector<T>, b: text = \"dflt\", c: integer }\n",
            "  println(\"{w.a} {w.b} {w.c} {len(w.items)} {w.items[0].n} {w.items[2].n}\");\n",
        ),
        "5 dflt 0 3 t0 t2\n",
    );
}

#[test]
fn cell_field_added_in_a_vector_element() {
    // Each of the three items gains m = 7: the sum is 21, and k still sums to 1+2+3 = 6.
    upgrades(
        "add_elem",
        &writer_w(T_W),
        &reader(
            "struct T { k: integer, n: text, m: integer = 7 }\nstruct W { a: integer, items: vector<T> }\n",
            "  s = 0; ks = 0;\n  for t in w.items { s += t.m; ks += t.k; }\n  println(\"{s} {ks} {w.items[1].n}\");\n",
        ),
        "21 6 t1\n",
    );
}

#[test]
fn cell_field_removed() {
    upgrades(
        "remove",
        &writer_w(T_W),
        &reader(
            "struct T { k: integer }\nstruct W { items: vector<T> }\n",
            "  println(\"{len(w.items)} {w.items[2].k}\");\n",
        ),
        "3 3\n",
    );
}

#[test]
fn cell_empty_and_single_vectors() {
    // An empty container hides append against replace: the empty one stays empty, the single
    // one carries its one element.
    let writer = "struct T { k: integer, n: text }\nstruct W { a: integer, items: vector<T>, one: vector<T> }\nfn main() {\n  w = W { a: 1 };\n  w.one += [T { k: 42, n: \"x\" }];\n  println(\"saved {store_persist_copy(w, \\\"a.store\\\")}\");\n}\n";
    upgrades(
        "empty_one",
        writer,
        &reader(
            "struct T { k: integer, n: text, m: integer = 9 }\nstruct W { a: integer, items: vector<T>, one: vector<T> }\n",
            "  println(\"{len(w.items)} {len(w.one)} {w.one[0].k} {w.one[0].m}\");\n",
        ),
        "0 1 42 9\n",
    );
}

#[test]
fn cell_number_widened() {
    // u8 250, 251, 252 → integer: they sum to 753; a null u8? stays null.
    let writer = "struct T { k: u8, q: u8? }\nstruct W { items: vector<T> }\nfn main() {\n  w = W {};\n  for i in 0..3 { w.items += [T { k: (250 + i) & 255, q: if i == 1 { null } else { i & 255 } }]; }\n  println(\"saved {store_persist_copy(w, \\\"a.store\\\")}\");\n}\n";
    upgrades(
        "widen",
        writer,
        &reader(
            "struct T { k: integer, q: integer? }\nstruct W { items: vector<T> }\n",
            "  s = 0;\n  for t in w.items { s += t.k; }\n  println(\"{s} {w.items[0].q} {w.items[1].q ?? -1} {w.items[2].q}\");\n",
        ),
        "753 0 -1 2\n",
    );
}

#[test]
fn cell_enum_variant_added_and_removed() {
    let writer = "enum C { Red, Green, Blue }\nstruct W { cs: vector<C?> }\nfn main() {\n  w = W {};\n  w.cs += [Red, Green, Blue];\n  println(\"saved {store_persist_copy(w, \\\"a.store\\\")}\");\n}\n";
    // Green is gone: its value reads null; Red and Blue keep their names although Blue's
    // position moved, and the new variant Teal is unused.
    upgrades(
        "variants",
        writer,
        &reader(
            "enum C { Teal, Red, Blue }\nstruct W { cs: vector<C?> }\n",
            "  for c in w.cs { if c == null { println(\"null\"); } else { println(\"{c}\"); } }\n",
        ),
        "Red\nnull\nBlue\n",
    );
}

#[test]
fn cell_rename_in_one_step_is_refused() {
    upgrade_refused(
        "rename",
        &writer_w(T_W),
        &reader(
            "struct T { key: integer, n: text }\nstruct W { a: integer, items: vector<T> }\n",
            "  println(\"{len(w.items)}\");\n",
        ),
        "looks like a rename",
    );
}

#[test]
fn cell_narrowed_number_is_refused() {
    upgrade_refused(
        "narrow",
        &writer_w(T_W),
        &reader(
            "struct T { k: u8, n: text }\nstruct W { a: integer, items: vector<T> }\n",
            "  println(\"{len(w.items)}\");\n",
        ),
        "narrows",
    );
}

/// Every kind a store holds, in one record: the same text printed from the writer's live data
/// before it saves and from the reader's upgraded data after.  The writer's print is the
/// oracle — it is computed from the data as built, never from the converter.
const SINK_TYPES: &str = "\
enum Mood { Calm, Busy, Gone }
enum Shape { Dot, Circle { r: integer }, Box { w: u8, h: u8 } }
struct Inner { x: integer, label: text }
struct Item { id: integer, name: text, score: float, tags: vector<text>, small: u16? }
struct Pin { x: integer, y: integer, what: text }
struct Word { w: text, n: integer }
struct Shared { k: integer, s: text }
struct Sink {
  i: integer, f: float, sg: single, b: boolean, c: character, t: text, n8: u8, i16v: i16,
  mood: Mood, shape: Shape, inner: Inner,
  items: vector<Item>, texts: vector<text>, grid: vector<vector<integer>>,
  by_id: hash<Item[id]>, by_name: sorted<Item[name]>, ordered: index<Item[score, id]>,
  pins: spatial<Pin[x, y]>, words: trie<Word[w]>,
  sh: hash<Shared[k]>, sh_by_s: index<Shared[s]>
}
";

const SINK_DIGEST: &str = r#"fn digest(s: Sink) -> text {
  out = "{s.i} {s.f} {s.sg} {s.b} {s.c} {s.t} {s.n8} {s.i16v} {s.mood} {s.inner.x} {s.inner.label}";
  match s.shape { Circle { r } => { out += " circle {r}"; }, Box { w, h } => { out += " box {w}x{h}"; }, _ => { out += " dot"; } };
  out += " | items";
  for it in s.items { out += " {it.id}:{it.name}:{it.score}:{len(it.tags)}:{it.small ?? -1}"; }
  out += " | texts {s.texts}";
  out += " | grid";
  for row in s.grid { out += " {row}"; }
  out += " | by_id";
  for k in [3, 1, 2, 9] { if s.by_id[k] != null { out += " {k}={s.by_id[k].name}"; } else { out += " {k}=none"; } }
  out += " | by_name";
  for it in s.by_name { out += " {it.name}"; }
  out += " | ordered";
  for it in s.ordered { out += " {it.score}/{it.id}"; }
  out += " | pins";
  for p in s.pins { out += " {p.what}"; }
  out += " | words";
  for w in s.words { out += " {w.w}={w.n}"; }
  out += " | sh";
  for x in s.sh_by_s { out += " {x.s}={x.k}"; }
  out += " {len(s.sh)} {s.sh[20].s}";
  out
}
"#;

fn sink_writer() -> String {
    format!(
        "{SINK_TYPES}{SINK_DIGEST}fn main() {{
  s = Sink {{ i: -7, f: 2.5, sg: 1.25f, b: true, c: 'q', t: \"top\", n8: 200, i16v: -300,
    mood: Busy, shape: Box {{ w: 3, h: 4 }}, inner: Inner {{ x: 11, label: \"in\" }} }};
  for i in 0..3 {{
    it = Item {{ id: i + 1, name: \"n{{3 - i}}\", score: 0.5 * i, tags: [\"a{{i}}\", \"b\"], small: if i == 1 {{ null }} else {{ i * 1000 }} }};
    s.items += [it];
    s.by_id += [Item {{ id: i + 1, name: \"n{{3 - i}}\", score: 0.5 * i, tags: [], small: 7 }}];
    s.by_name += [Item {{ id: i + 1, name: \"n{{3 - i}}\", score: 0.5 * i, tags: [\"z\"], small: null }}];
    s.ordered += [Item {{ id: 10 - i, name: \"o\", score: 1.0 - 0.5 * (i % 2), tags: [], small: i }}];
    s.pins += [Pin {{ x: i * 10, y: 20 - i, what: \"p{{i}}\" }}];
    s.words += [Word {{ w: \"w{{2 - i}}\", n: i }}];
    s.sh += [Shared {{ k: (i + 1) * 10, s: \"s{{2 - i}}\" }}];
  }}
  s.texts += [\"x\", \"\", \"zz\"];
  s.grid += [[1, 2], [], [3]];
  println(digest(s));
  println(\"saved {{store_persist_copy(s, \\\"a.store\\\")}}\");
}}
"
    )
}

fn sink_reader(types: &str) -> String {
    format!(
        "{types}{SINK_DIGEST}fn main() {{
  s = Sink {{ mood: Calm, shape: Dot }};
  if !store_load(s, \"a.store\") {{ println(\"not loaded\"); return; }}
  println(digest(s));
}}
"
    )
}

/// Step 1 through the CLI: the kitchen-sink store, upgraded across one added top-level field,
/// reads back every value the writer printed.  The added field forces a real upgrade; every
/// type below it is unchanged, so this is the identity conversion of all of them.
#[test]
fn cell_every_kind_survives_an_upgrade() {
    let dir = write_store("sink", &sink_writer());
    let (_, out, _) = run_loft(&dir, &["a.loft"]);
    let expect = out
        .lines()
        .next()
        .expect("the writer printed its digest")
        .to_string()
        + "\n";
    let _ = loft::file_access::remove_dir_all(&dir);
    let reader_types = SINK_TYPES.replace(
        "  sh: hash<Shared[k]>",
        "  added: integer = 3,\n  sh: hash<Shared[k]>",
    );
    upgrades(
        "sink_up",
        &sink_writer(),
        &sink_reader(&reader_types),
        &expect,
    );
}

/// Step 1 proper: the identity conversion with whole-type copies OFF, so every record and
/// collection crosses through the old description, compared against the store it came from.
/// The rendering of the root value is the digest: equal text = every value, length and order
/// arrived.  With copies on (the fast path) it must agree too.
#[test]
fn the_identity_conversion_reproduces_the_store() {
    let dir = write_store("identity", &sink_writer());
    let bytes = loft::file_access::read(dir.join("a.store")).unwrap();
    let side = loft::file_access::read_to_string(dir.join("a.store.dschema")).unwrap();
    let id = loft::schema_sidecar::LayoutIdentity::from_sidecar(&side).expect("sidecar");
    let old = LayoutDesc::from_dump(&id.dump).expect("the sink layout reads back");
    let mut p = parse(SINK_TYPES);
    let root = known(&p, "Sink");
    let slot = p
        .database
        .adopt_store(loft::store::Store::from_bytes(&bytes).expect("an image"));
    let show = |db: &loft::database::Stores, s: u16| {
        let mut out = String::new();
        let r = loft::keys::DbRef {
            store_nr: s,
            rec: loft::store::PRIMARY,
            pos: 8,
        };
        db.show(&mut out, &r, root, false);
        out
    };
    let original = show(&p.database, slot);
    assert!(
        original.contains("top") && original.contains("w0"),
        "the digest sees the data: {original}"
    );
    for whole in [false, true] {
        let types_before = p.database.types_len();
        let new = p
            .database
            .upgrade_store(slot, &old, id_of(&old, "Sink"), root, id.slot, whole)
            .unwrap_or_else(|e| panic!("identity upgrade (whole={whole}) refused: {e}"));
        assert_eq!(
            p.database.types_len(),
            types_before,
            "the old rows are gone again"
        );
        assert_eq!(show(&p.database, new), original, "whole={whole}");
        assert_eq!(
            show(&p.database, slot),
            original,
            "the old store is untouched"
        );
        drop(p.database.take_store(new));
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}

// ── Step 4: keyed collections ──────────────────────────────────────────────────────────────

/// The writer the keyed cells share: four records `k = 1..4`, `n = "n3".."n0"` (so `k + n`'s
/// digit is always 4), in a hash and a sorted, both keyed on `k`.
const KEYED_A: &str = "struct T { k: integer, n: text }\nstruct W { h: hash<T[k]>, s: sorted<T[k]> }\nfn main() {\n  w = W {};\n  for i in 0..4 { w.h += [T { k: i + 1, n: \"n{3 - i}\" }]; w.s += [T { k: i + 1, n: \"n{3 - i}\" }]; }\n  println(\"saved {store_persist_copy(w, \\\"a.store\\\")}\");\n}\n";

#[test]
fn cell_changed_key_rebuilds_hash_and_sorted() {
    upgrades(
        "rekey",
        KEYED_A,
        &reader(
            "struct T { k: integer, n: text }\nstruct W { h: hash<T[n]>, s: sorted<T[n]> }\n",
            "  println(\"{len(w.h)} {w.h[\\\"n0\\\"].k} {w.h[\\\"n3\\\"].k} {w.h[\\\"n9\\\"] == null}\");\n  o = \"\"; for t in w.s { o += \" {t.n}:{t.k}\"; }\n  println(o);\n",
        ),
        "4 4 1 true\n n0:4 n1:3 n2:2 n3:1\n",
    );
}

#[test]
fn cell_duplicate_under_the_new_key_is_refused() {
    // n = "n0", "n1", "n0", "n1": the new key repeats.
    upgrade_refused(
        "dupkey",
        &KEYED_A.replace("\"n{3 - i}\"", "\"n{i % 2}\""),
        &reader(
            "struct T { k: integer, n: text }\nstruct W { h: hash<T[n]>, s: sorted<T[n]> }\n",
            "  println(\"{len(w.h)}\");\n",
        ),
        "W.h: two stored records share the key [\"n0\"]",
    );
}

const GROUP_A: &str = "struct T { k: integer, n: text }\nstruct W { h: hash<T[k]> }\nfn main() {\n  w = W {};\n  for i in 0..4 { w.h += [T { k: i + 1, n: \"n{3 - i}\" }]; }\n  println(\"saved {store_persist_copy(w, \\\"a.store\\\")}\");\n}\n";

#[test]
fn cell_index_added_over_records_already_stored() {
    // `by_n` joins `h` in one group: it is built from the four records `h` holds, in key
    // order, and a later insert reaches both.
    upgrades(
        "group_add",
        GROUP_A,
        &reader(
            "struct T { k: integer, n: text }\nstruct W { h: hash<T[k]>, by_n: index<T[n]> }\n",
            "  o = \"\"; for t in w.by_n { o += \" {t.n}:{t.k}\"; }\n  println(\"{len(w.h)}{o}\");\n  w.h += [T { k: 9, n: \"a\" }];\n  o = \"\"; for t in w.by_n { o += \" {t.n}\"; }\n  println(\"{len(w.h)}{o} {store_verify(w)}\");\n",
        ),
        "4 n0:4 n1:3 n2:2 n3:1\n5 a n0 n1 n2 n3 true\n",
    );
}

#[test]
fn cell_index_removed_from_a_group() {
    let writer = GROUP_A.replace(
        "struct W { h: hash<T[k]> }",
        "struct W { h: hash<T[k]>, by_n: index<T[n]> }",
    );
    upgrades(
        "group_remove",
        &writer,
        &reader(
            "struct T { k: integer, n: text }\nstruct W { h: hash<T[k]> }\n",
            "  println(\"{len(w.h)} {w.h[4].n} {w.h[1].n} {store_verify(w)}\");\n",
        ),
        "4 n0 n3 true\n",
    );
}

/// The owner's case: a vector's element gains an index elsewhere in the program, so each
/// element moves into a record of its own; and the reverse.  An append after the upgrade
/// shows the new layout is the one the program writes.
#[test]
fn cell_linked_flip_both_directions() {
    let inline = "struct T { k: integer, n: text }\nstruct W { a: integer, items: vector<T> }\n";
    let linked = "struct T { k: integer, n: text }\nstruct W { a: integer, items: vector<T> }\nstruct X { by_k: index<T[k]> }\n";
    let body = "  w.items += [T { k: 4, n: \"t3\" }];\n  o = \"\"; for t in w.items { o += \" {t.k}:{t.n}\"; }\n  println(\"{w.a}{o} {store_verify(w)}\");\n";
    upgrades(
        "flip_in",
        &writer_w(inline),
        &reader(linked, body),
        "5 1:t0 2:t1 3:t2 4:t3 true\n",
    );
    upgrades(
        "flip_out",
        &writer_w(linked),
        &reader(inline, body),
        "5 1:t0 2:t1 3:t2 4:t3 true\n",
    );
}

// ── loft#2100: a method on an enum is not one of its values ────────────────────────────────

/// Adding a method changes no byte of a store, so the store needs no upgrade at all: the
/// identity is the same and `store_load` reads it directly.  Measured before the fix: the
/// method was an entry of the enum's storage type and the load was refused ("reshaped: E").
#[test]
fn an_enum_method_leaves_the_layout_alone() {
    let writer = "enum E { A, B { x: integer } }\nstruct W { e: E, n: integer }\nfn main() {\n  w = W { e: B { x: 5 }, n: 1 };\n  println(\"saved {store_persist_copy(w, \\\"a.store\\\")}\");\n}\n";
    let dir = write_store("enum_method", writer);
    let reader = "enum E { A, B { x: integer } }\nfn label(self: E) -> text { match self { B { x } => \"b{x}\", _ => \"a\" } }\nstruct W { e: E, n: integer }\nfn main() {\n  w = W { e: A, n: 0 };\n  ok = store_load(w, \"a.store\");\n  println(\"{ok} {w.n} {w.e.label()}\");\n}\n";
    loft::file_access::write(dir.join("b.loft"), reader).unwrap();
    let (code, out, err) = run_loft(&dir, &["layout", "upgrade", "b.loft", "a.store"]);
    assert_eq!((code, out.as_str()), (0, ""), "nothing to upgrade:\n{err}");
    for mode in [&[][..], &["--native"][..]] {
        let mut args: Vec<&str> = mode.to_vec();
        args.push("b.loft");
        let (code, out, err) = run_loft(&dir, &args);
        assert_eq!((code, out.as_str()), (0, "true 1 b5\n"), "{mode:?}:\n{err}");
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// A method's name is not a variant: text naming it parses to null, as any non-variant does.
#[test]
fn an_enum_method_name_does_not_parse_as_a_variant() {
    let dir = cell_dir("enum_method_text");
    let src = "enum E { A, B }\nfn label(self: E) -> text { if self == A { \"a\" } else { \"b\" } }\nfn main() {\n  t = \"label\";\n  e = t as E?;\n  f = \"B\" as E;\n  println(\"{e == null} {f} {f.label()}\");\n}\n";
    loft::file_access::write(dir.join("p.loft"), src).unwrap();
    for mode in [&[][..], &["--native"][..]] {
        let mut args: Vec<&str> = mode.to_vec();
        args.push("p.loft");
        let (code, out, err) = run_loft(&dir, &args);
        assert_eq!((code, out.as_str()), (0, "true B b\n"), "{mode:?}:\n{err}");
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}

// ── loft#2099: a stored reference names its store by slot, and a load moves the slot ───────

const REF_TYPES: &str =
    "struct T { k: integer }\nstruct W { items: vector<T>, r: reference<T>?, n: integer }\n";

/// The reader allocates another store first, so the loaded one lands in a different slot
/// than it had when it was written.  Measured before the fix: `r.k=0` and `0` on both
/// backends (right only when the slots happened to coincide).
#[test]
fn a_stored_reference_survives_a_load_into_another_slot() {
    let writer = format!(
        "{REF_TYPES}fn main() {{\n  w = W {{ n: 1 }};\n  for i in 0..3 {{ w.items += [T {{ k: (i + 1) * 10 }}]; }}\n  w.r = w.items[1];\n  println(\"saved {{store_persist_bind(w, \\\"a.store\\\")}}\");\n}}\n"
    );
    let dir = write_store("ref_slot", &writer);
    let reader = format!(
        "{REF_TYPES}fn main() {{\n  pad = W {{ n: 9 }};\n  w = W {{ n: 0 }};\n  ok = store_load(w, \"a.store\");\n  println(\"{{ok}} {{pad.n}} {{w.r.k ?? -1}}\");\n  w.items[1].k = 77;\n  println(\"{{w.r.k ?? -1}}\");\n}}\n"
    );
    loft::file_access::write(dir.join("b.loft"), &reader).unwrap();
    for mode in [&[][..], &["--native"][..]] {
        let mut args: Vec<&str> = mode.to_vec();
        args.push("b.loft");
        let (code, out, err) = run_loft(&dir, &args);
        assert_eq!(
            (code, out.as_str()),
            (0, "true 9 20\n77\n"),
            "{mode:?}:\n{err}"
        );
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// A reference into a store that is not part of the file cannot be carried by it: the load
/// is refused with that reason, and the collection is left empty.
#[test]
fn a_reference_into_another_store_refuses_the_load() {
    let writer = format!(
        "{REF_TYPES}fn main() {{\n  other = W {{ n: 2 }};\n  other.items += [T {{ k: 5 }}];\n  w = W {{ n: 1 }};\n  w.r = other.items[0];\n  println(\"saved {{store_persist_bind(w, \\\"a.store\\\")}} {{other.n}}\");\n}}\n"
    );
    let dir = write_store("ref_foreign", &writer);
    let reader = format!(
        "{REF_TYPES}fn main() {{\n  w = W {{ n: 0 }};\n  ok = store_load(w, \"a.store\");\n  println(\"{{ok}} {{w.n}}\");\n}}\n"
    );
    loft::file_access::write(dir.join("b.loft"), &reader).unwrap();
    let (code, out, err) = run_loft(&dir, &["b.loft"]);
    assert_eq!((code, out.as_str()), (0, "false 0\n"), "{err}");
    assert!(err.contains("a reference into another store"), "{err}");
    let _ = loft::file_access::remove_dir_all(&dir);
}

// ── Step 6: references across an upgrade ───────────────────────────────────────────────────

fn ref_writer(types: &str) -> String {
    format!(
        "{types}fn main() {{\n  w = W {{ n: 1 }};\n  for i in 0..3 {{ w.items += [T {{ k: (i + 1) * 10 }}]; }}\n  w.r = w.items[1];\n  println(\"saved {{store_persist_bind(w, \\\"a.store\\\")}}\");\n}}\n"
    )
}

/// A reference into the store crosses an upgrade that changes both its own record and the
/// record it points at: it names the same element afterwards (k = 20), and a write through
/// the collection is seen through it.
#[test]
fn cell_a_reference_is_repointed() {
    upgrades(
        "ref_repoint",
        &ref_writer(REF_TYPES),
        &reader(
            "struct T { k: integer, tag: text = \"t\" }\nstruct W { items: vector<T>, r: reference<T>?, n: integer, m: integer = 4 }\n",
            "  println(\"{w.n} {w.m} {w.r.k ?? -1} {w.r.tag ?? \\\"-\\\"}\");\n  w.items[1].k = 77;\n  println(\"{w.r.k ?? -1} {store_verify(w)}\");\n",
        ),
        "1 4 20 t\n77 true\n",
    );
}

/// A reference whose target type changed cannot be carried: the stored records are of the
/// old type.
#[test]
fn cell_a_reference_to_another_type_is_refused() {
    upgrade_refused(
        "ref_retarget",
        &ref_writer(REF_TYPES),
        &reader(
            "struct T { k: integer }\nstruct U { k: integer }\nstruct W { items: vector<T>, r: reference<U>?, n: integer, us: vector<U> }\n",
            "  println(\"{w.n}\");\n",
        ),
        "now points at a `U`",
    );
}

/// A method's name used where a variant is expected is not a variant: the program does not
/// compile, rather than storing a value named after the method (measured before the fix:
/// `e: E = label` printed `[label]`).
#[test]
fn an_enum_method_name_is_not_a_bare_variant() {
    let dir = cell_dir("enum_method_bare");
    let src = "enum E { A, B }\nfn label(self: E) -> text { if self == A { \"a\" } else { \"b\" } }\nfn main() {\n  e: E = label;\n  println(\"[{e}]\");\n}\n";
    loft::file_access::write(dir.join("p.loft"), src).unwrap();
    let (code, out, _) = run_loft(&dir, &["--interpret", "p.loft"]);
    assert!(
        code != 0 && !out.contains("label"),
        "compiled to a value: {out}"
    );
    let _ = loft::file_access::remove_dir_all(&dir);
}

// ── loft#2102: a spatial and a trie field render as the list of their elements ─────────────

/// Measured before the fix: a panic on both backends (`Undefined iterate on non-collection
/// type: spatial<…>`, `not implemented: trie rendering`).  The trie renders in key order;
/// the spatial in its tree's Morton order, where (1,2) interleaves to 9 and (5,0) to 17.
#[test]
fn a_spatial_and_a_trie_field_render() {
    let dir = cell_dir("render_keyed");
    let src = "struct Pin { x: integer, y: integer }\nstruct Wd { w: text }\nstruct M { pins: spatial<Pin[x, y]>, t: trie<Wd[w]>, n: integer }\nfn main() {\n  m = M { n: 1 };\n  m.pins += [Pin { x: 5, y: 0 }, Pin { x: 1, y: 2 }];\n  m.t += [Wd { w: \"pear\" }, Wd { w: \"apple\" }];\n  println(\"{m}\");\n}\n";
    loft::file_access::write(dir.join("p.loft"), src).unwrap();
    for mode in [&["--interpret"][..], &["--native"][..]] {
        let mut args: Vec<&str> = mode.to_vec();
        args.push("p.loft");
        let (code, out, err) = run_loft(&dir, &args);
        assert_eq!(
            (code, out.as_str()),
            (
                0,
                "{pins:[{x:1,y:2},{x:5,y:0}],t:[{w:\"apple\"},{w:\"pear\"}],n:1}\n"
            ),
            "{mode:?}:\n{err}"
        );
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}

// ── Step 5: an added field is filled by its default, and a default may be an expression ───

/// The defaults these cells exercise: arithmetic, a default reading an earlier default, a
/// format string, a method and a concatenation, a vector literal, an `if` choosing a variant,
/// a nested struct literal, a nullable conditional, and a call to a user function.
const FILLED: &str = "\
enum Size { Small, Big }
struct P { x: integer, y: integer }
fn describe(l: text, p: integer) -> text { \"{l}@{p}\" }
struct Item {
  id: integer,
  label: text,
  price: integer,
  a: integer = $.id * 2,
  b: integer = $.a + 1,
  title: text = \"{$.id}: {$.label}\",
  shout: text = $.label.to_uppercase() + \"!\",
  tags: vector<text> = [$.label, \"x\"],
  size: Size = if $.price > 10 { Big } else { Small },
  at: P = P { x: $.id, y: $.price },
  half: integer? = if $.price % 2 == 0 { $.price / 2 } else { null },
  word: text = describe($.label, $.price)
}
fn line(x: Item) -> text {
  \"{x.a} {x.b} {x.title} {x.shout} {x.tags} {x.size} {x.at.x},{x.at.y} {x.half ?? -1} {x.word}\"
}
";

/// Hand-computed for (7, \"pen\", 12) and (2, \"ink\", 3).
const FILLED_PEN: &str = "14 15 7: pen PEN! [\"pen\",\"x\"] Big 7,12 6 pen@12";
const FILLED_INK: &str = "4 5 2: ink INK! [\"ink\",\"x\"] Small 2,3 -1 ink@3";

/// The construction half: every one of these defaults was refused or compiled before only in
/// part (*"both reads `$` and needs a temporary value"*).  A supplied field still beats its
/// default (`title: "given"`).
#[test]
fn dollar_defaults_construct_on_both_backends() {
    let dir = cell_dir("dollar_construct");
    let src = format!(
        "{FILLED}fn main() {{\n  i = Item {{ id: 7, label: \"pen\", price: 12 }};\n  j = Item {{ id: 2, label: \"ink\", price: 3, title: \"given\" }};\n  println(line(i));\n  println(line(j));\n}}\n"
    );
    loft::file_access::write(dir.join("p.loft"), &src).unwrap();
    let expect = format!("{FILLED_PEN}\n{}\n", FILLED_INK.replace("2: ink", "given"));
    for mode in [&["--interpret"][..], &["--native"][..]] {
        let mut args: Vec<&str> = mode.to_vec();
        args.push("p.loft");
        let (code, out, err) = run_loft(&dir, &args);
        assert_eq!(
            (code, out.as_str()),
            (0, expect.as_str()),
            "{mode:?}:\n{err}"
        );
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// The upgrade half: a store of items written before any of those fields existed, upgraded,
/// reads every added field as its default evaluated over the stored record.  `items` and `h`
/// are two collections over `Item` in one struct, so they are ONE record set
/// (`@FR-Col-Group`): the writer's four appends leave four records in `items` (pen, ink, pen,
/// ink) and one per key in `h` — the writer's own semantics, which the upgrade keeps.
#[test]
fn cell_expression_defaults_fill_an_upgrade() {
    let writer = "struct Item { id: integer, label: text, price: integer }\nstruct W { items: vector<Item>, h: hash<Item[id]> }\nfn main() {\n  w = W {};\n  w.items += [Item { id: 7, label: \"pen\", price: 12 }, Item { id: 2, label: \"ink\", price: 3 }];\n  w.h += [Item { id: 7, label: \"pen\", price: 12 }, Item { id: 2, label: \"ink\", price: 3 }];\n  println(\"saved {store_persist_copy(w, \\\"a.store\\\")}\");\n}\n";
    let reader = format!(
        "{FILLED}struct W {{ items: vector<Item>, h: hash<Item[id]> }}\nfn main() {{\n  w = W {{}};\n  if !store_load(w, \"a.store\") {{ println(\"not loaded\"); return; }}\n  for x in w.items {{ println(line(x)); }}\n  println(line(w.h[2]));\n  println(line(w.h[7]));\n}}\n"
    );
    upgrades(
        "dollar_upgrade",
        writer,
        &reader,
        &format!(
            "{FILLED_PEN}\n{FILLED_INK}\n{FILLED_PEN}\n{FILLED_INK}\n{FILLED_INK}\n{FILLED_PEN}\n"
        ),
    );
}

/// A rename is two upgrades: v2 keeps `label` and fills `name` from it, v3 drops `label`.
/// Skipping v2 is refused with the cure in the reason, and leaves the store as it was.
#[test]
fn cell_rename_in_two_upgrades() {
    let v1 = "struct T { k: integer, label: text }\nstruct W { items: vector<T> }\n";
    let v2 = "struct T { k: integer, label: text, name: text = $.label }\nstruct W { items: vector<T> }\n";
    let v3 = "struct T { k: integer, name: text }\nstruct W { items: vector<T> }\n";
    let writer = format!(
        "{v1}fn main() {{\n  w = W {{}};\n  w.items += [T {{ k: 1, label: \"one\" }}, T {{ k: 2, label: \"two\" }}];\n  println(\"saved {{store_persist_copy(w, \\\"a.store\\\")}}\");\n}}\n"
    );
    let body = "  o = \"\"; for t in w.items { o += \" {t.k}={t.name}\"; }\n  println(o);\n";
    // Skipping step 1 is refused.
    upgrade_refused(
        "rename_skip",
        &writer,
        &reader(v3, body),
        "give the new one a default that reads it",
    );
    // Step 1 then step 2.
    let dir = write_store("rename_two", &writer);
    loft::file_access::write(dir.join("b.loft"), reader(v2, body)).unwrap();
    let (code, out, err) = run_loft(&dir, &["layout", "upgrade", "b.loft", "a.store"]);
    assert_eq!(code, 0, "step 1:\n{out}\n{err}");
    loft::file_access::write(dir.join("c.loft"), reader(v3, body)).unwrap();
    let (code, out, err) = run_loft(&dir, &["layout", "upgrade", "c.loft", "a.store"]);
    assert_eq!(code, 0, "step 2:\n{out}\n{err}");
    assert!(out.contains("T.label removed"), "{out}");
    for mode in [&["--interpret"][..], &["--native"][..]] {
        let mut args: Vec<&str> = mode.to_vec();
        args.push("c.loft");
        let (code, out, err) = run_loft(&dir, &args);
        assert_eq!(
            (code, out.as_str()),
            (0, " 1=one 2=two\n"),
            "{mode:?}:\n{err}"
        );
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// The same fill for a hash on its own: its entries live in the hash's arena, not in records
/// of their own, and each one is filled in place.
#[test]
fn cell_expression_defaults_fill_hash_entries() {
    let writer = "struct Item { id: integer, label: text, price: integer }\nstruct H { h: hash<Item[id]> }\nfn main() {\n  w = H {};\n  w.h += [Item { id: 7, label: \"pen\", price: 12 }, Item { id: 2, label: \"ink\", price: 3 }];\n  println(\"saved {store_persist_copy(w, \\\"a.store\\\")}\");\n}\n";
    let reader = format!(
        "{FILLED}struct H {{ h: hash<Item[id]> }}\nfn main() {{\n  w = H {{}};\n  if !store_load(w, \"a.store\") {{ println(\"not loaded\"); return; }}\n  println(\"{{len(w.h)}}\");\n  for k in [2, 7] {{ if w.h[k] != null {{ println(line(w.h[k]?)); }} }}\n}}\n"
    );
    upgrades(
        "dollar_hash",
        writer,
        &reader,
        &format!("2\n{FILLED_INK}\n{FILLED_PEN}\n"),
    );
}

// @PLN196 step 8 — the loads that convert by themselves.

const ITEM_V1: &str = "struct Item { id: integer, name: text }\n";
const ITEM_V2: &str = "struct Item { id: integer, name: text, price: integer = 10, title: text = \"{$.id}: {$.name}\" }\n";
const ITEM_V3: &str = "struct Item { id: integer, name: text, price: integer = 10, title: text = \"{$.id}: {$.name}\", stock: integer = $.price * 3 }\n";

/// Version 1 binds `a.store` with four items, `id = 1..4`, `name = "n1".."n4"`.
fn item_writer() -> String {
    format!(
        "{ITEM_V1}fn main() {{\n  h: hash<Item[id]> = [];\n  for i in 1..5 {{ h += [Item {{ id: i, name: \"n{{i}}\" }}]; }}\n  println(\"saved {{store_persist_bind(h, \\\"a.store\\\")}}\");\n}}\n"
    )
}

/// `store_persist_bind` of an older file upgrades the FILE in place: the call answers true with
/// every field filled (a constant, a format string over `$`, an expression over another
/// default), the original is kept as `a.store.old`, and what the program writes afterwards is
/// in the file.  A second upgrade replaces that `.old` with the file it upgraded, so one
/// upgrade back stays on disk.  Interpreter for the first upgrade, native for the second.
#[test]
fn cell_bind_upgrades_the_file_in_place_and_keeps_one_old() {
    let dir = write_store("bind_in_place", &item_writer());
    let v1 = store_bytes(&dir);
    let binder = |types: &str, show: &str| {
        format!(
            "{types}fn main() {{\n  h: hash<Item[id]> = [];\n  if !store_persist_bind(h, \"a.store\") {{ println(\"not bound\"); return; }}\n  for e in h {{ println({show}); }}\n  h += [Item {{ id: 9, name: \"late\" }}];\n}}\n"
        )
    };
    loft::file_access::write(
        dir.join("b.loft"),
        binder(ITEM_V2, "\"{e.id} {e.price} [{e.title}]\""),
    )
    .unwrap();
    let (code, out, err) = run_loft(&dir, &["b.loft"]);
    assert_eq!(
        (code, out.as_str()),
        (
            0,
            "1 10 [1: n1]\n2 10 [2: n2]\n3 10 [3: n3]\n4 10 [4: n4]\n"
        ),
        "the first bind upgrades:\n{out}\n{err}"
    );
    let old = (
        loft::file_access::read(dir.join("a.store.old")).unwrap(),
        loft::file_access::read(dir.join("a.store.old.dschema")).unwrap(),
    );
    assert!(
        old == v1,
        "a.store.old is the version-1 file, byte for byte"
    );
    let v2 = store_bytes(&dir);
    loft::file_access::write(
        dir.join("c.loft"),
        binder(ITEM_V3, "\"{e.id} {e.price} [{e.title}] {e.stock}\""),
    )
    .unwrap();
    let (code, out, err) = run_loft(&dir, &["--native", "c.loft"]);
    assert_eq!(
        (code, out.as_str()),
        (
            0,
            "1 10 [1: n1] 30\n2 10 [2: n2] 30\n3 10 [3: n3] 30\n4 10 [4: n4] 30\n9 10 [9: late] 30\n"
        ),
        "the second bind upgrades the first one's file, with the record it added:\n{out}\n{err}"
    );
    let old = (
        loft::file_access::read(dir.join("a.store.old")).unwrap(),
        loft::file_access::read(dir.join("a.store.old.dschema")).unwrap(),
    );
    assert!(old == v2, "a.store.old is now the version-2 file: one .old");
    for gone in [
        "a.store.upgrade",
        "a.store.upgrade.dschema",
        "a.store.old.old",
    ] {
        assert!(
            !loft::file_access::exists(dir.join(gone)),
            "{gone}: no half-swapped file and no second .old"
        );
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// Two converting loads in one program: each fills its own records, and the second starts
/// with nothing left owed from the first.
#[test]
fn cell_two_converting_loads_in_one_program() {
    let dir = write_store("two_loads", &item_writer());
    let reader = format!(
        "{ITEM_V2}fn main() {{\n  a: hash<Item[id]> = [];\n  b: hash<Item[id]> = [];\n  if !store_load(a, \"a.store\") {{ println(\"not loaded\"); return; }}\n  a[2].price = 77;\n  if !store_load(b, \"a.store\") {{ println(\"not loaded\"); return; }}\n  println(\"{{a[2].price}} [{{a[3].title}}] {{b[2].price}} [{{b[4].title}}] {{len(a)}} {{len(b)}}\");\n}}\n"
    );
    loft::file_access::write(dir.join("b.loft"), reader).unwrap();
    let before = store_bytes(&dir);
    reads(
        "two_loads",
        &dir,
        "77 [3: n3] 10 [4: n4] 4 4\n",
        "two loads",
    );
    assert!(
        store_bytes(&dir) == before,
        "store_load leaves the file as it was"
    );
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// The other whole-image loaders convert an older layout as `store_load` does: the untrusted
/// local loader, the trusted URL loader and the pinned URL loader (whose pin is the OLD file's
/// hash, checked before anything is converted).  A `file://` URL keeps the cell off the
/// network.  A wrong pin still refuses, before any conversion.
#[test]
fn cell_every_whole_image_loader_converts() {
    let dir = write_store("other_loaders", &item_writer());
    let before = store_bytes(&dir);
    let url = format!("file://{}", dir.join("a.store").display());
    let pin = loft::integrity::sha256_hex(&before.0);
    let show = "  println(\"{h[1].price} [{h[2].title}] {len(h)}\");\n";
    for (name, call) in [
        (
            "untrusted",
            "store_load_untrusted(h, \"a.store\")".to_string(),
        ),
        (
            "url_trusted",
            format!("store_load_url_trusted(h, \"{url}\")"),
        ),
        (
            "url_pinned",
            format!("store_load_url(h, \"{url}\", \"{pin}\")"),
        ),
    ] {
        let reader = format!(
            "{ITEM_V2}fn main() {{\n  h: hash<Item[id]> = [];\n  if !{call} {{ println(\"not loaded\"); return; }}\n{show}}}\n"
        );
        loft::file_access::write(dir.join("b.loft"), reader).unwrap();
        reads(name, &dir, "10 [2: n2] 4\n", name);
        assert!(
            store_bytes(&dir) == before,
            "{name}: the file is left as it was"
        );
    }
    let wrong = "0".repeat(64);
    let reader = format!(
        "{ITEM_V2}fn main() {{\n  h: hash<Item[id]> = [];\n  if !store_load_url(h, \"{url}\", \"{wrong}\") {{ println(\"not loaded {{len(h)}}\"); return; }}\n{show}}}\n"
    );
    loft::file_access::write(dir.join("b.loft"), reader).unwrap();
    reads("url_wrong_pin", &dir, "not loaded 0\n", "a wrong pin");
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// The live build swap's restore (@PLN196 step 8, open question 4): `swap_world` takes the
/// previous build's world from `LOFT_RESUME` as a store image and converts it into this
/// build's types.  The world holds a hash and a REFERENCE into it, which survive the load into
/// another slot (`@FR-L-Ref-Slot`); both structs grew a field whose default is computed from
/// the record (`size = len($.name)` per item, `bonus = len($.items) * 10` on the world), filled
/// by the drain the parser puts after `swap_world`.  Both backends.  The writer binds its
/// world, which writes the live bytes and the slot exactly as the swap's snapshot does.
#[test]
fn cell_swap_world_restores_and_converts_a_world_with_a_reference() {
    let writer = "struct Item { id: integer, name: text }\nstruct World { items: hash<Item[id]>, best: reference<Item>? }\nfn main() {\n  w = World {};\n  w.items += [Item { id: 1, name: \"a\" }, Item { id: 2, name: \"bb\" }, Item { id: 3, name: \"ccc\" }];\n  w.best = w.items[2];\n  println(\"saved {store_persist_bind(w, \\\"a.store\\\")}\");\n}\n";
    let reader = "use engine_host;\nstruct Item { id: integer, name: text, size: integer = len($.name) }\nstruct World { items: hash<Item[id]>, best: reference<Item>?, bonus: integer = len($.items) * 10 }\nfn main() {\n  w = World {};\n  r = engine_host::swap_world(w);\n  println(\"{r} {len(w.items)} {w.best.id} {w.best.name} {w.best.size} {w.bonus} {w.items[3].size}\");\n}\n";
    let dir = write_store("swap_world_restore", writer);
    loft::file_access::write(dir.join("b.loft"), reader).unwrap();
    let lib = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib");
    for mode in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(loft_bin())
            .args([mode, "--lib"])
            .arg(&lib)
            .arg("b.loft")
            .current_dir(&dir)
            .env("LOFT_TIMEOUT", "120")
            .env("LOFT_RESUME", dir.join("a.store"))
            .output()
            .expect("loft runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            (out.status.code(), stdout.as_ref()),
            (Some(0), "true 3 2 bb 2 30 3\n"),
            "{mode}: the restored world:\n{stdout}\n{stderr}"
        );
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// A live rebuild decides at COMPILE time whether the next instance can take over the running
/// world (owner, 2026-10-21): the build is checked against the world's layout
/// (`LOFT_CHECK_WORLD`, written by `live_rebuild::spawn_build`), so a refused change fails the
/// BUILD and the swap is never offered.  An unchanged world checks silently, a grown one names
/// what converts, a refused one fails with the reason — under `--check --native` too, before
/// any code is generated.
#[test]
fn cell_a_rebuild_checks_the_running_world_before_it_compiles() {
    let writer = "struct W { events: integer, name: text }\nfn main() {\n  w = W { events: 3, name: \"a\" };\n  println(\"saved {store_persist_bind(w, \\\"a.store\\\")}\");\n}\n";
    let dir = write_store("rebuild_check", writer);
    let check = |name: &str, src: &str, native: bool| -> (i32, String, String) {
        loft::file_access::write(dir.join(format!("{name}.loft")), src).unwrap();
        let mut args = vec!["--check"];
        if native {
            args.push("--native");
        }
        let file = format!("{name}.loft");
        args.push(&file);
        let out = loft::platform::process::harness_command(loft_bin())
            .args(&args)
            .current_dir(&dir)
            .env("LOFT_TIMEOUT", "120")
            .env("LOFT_CHECK_WORLD", dir.join("a.store.dschema"))
            .output()
            .expect("loft runs");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    // Every version builds the world without naming `name`, whose type is what changes.
    let main = "fn main() {\n  w = W { events: 0 };\n  println(\"{w.events}\");\n}\n";
    let (code, out, err) = check(
        "same",
        &format!("struct W {{ events: integer, name: text }}\n{main}"),
        false,
    );
    assert_eq!((code, out.as_str()), (0, "ok\n"), "unchanged:\n{err}");
    assert!(
        !err.contains("loft-live"),
        "an unchanged world checks silently:\n{err}"
    );
    let (code, out, err) = check(
        "grown",
        &format!(
            "struct W {{ events: integer, name: text, bonus: integer = $.events * 10 }}\n{main}"
        ),
        false,
    );
    assert_eq!((code, out.as_str()), (0, "ok\n"), "grown:\n{err}");
    assert!(
        err.contains("the running world converts to this build") && err.contains("bonus"),
        "a grown world names what converts:\n{err}"
    );
    for native in [false, true] {
        let (code, out, err) = check(
            "refused",
            &format!("struct W {{ events: integer, name: integer }}\n{main}"),
            native,
        );
        assert!(
            code == 1
                && out.is_empty()
                && err.contains("this build cannot take over the running world")
                && err.contains("name"),
            "native={native}: a refused world fails the build with the reason ({code}):\n{out}\n{err}"
        );
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}

// ── Step 9: skipped versions ─────────────────────────────────────────────────────────────────
//
// The sidecar records the release that wrote the store (its `loft.toml` version).  A store
// older than the reading program goes through the `upgrade(from, old, item)` routines it wrote,
// after the fields carried by name; `data_compatible_with` is the oldest release it reads, and
// `data_bridge` the release a refusal names.  Each cell writes the store under one manifest and
// reads it under the next, in the same directory.  Rules: @FR-L-Release, @FR-L-Upgrade-Routine,
// @FR-L-Upgrade-Old (formal/layout.md § Releases a store skipped).

/// The manifest of a release: `version`, plus any extra `[package]` lines.
fn manifest(dir: &std::path::Path, version: &str, extra: &str) {
    loft::file_access::write(
        dir.join("loft.toml"),
        format!("[package]\nname = \"cell\"\nversion = \"{version}\"\n{extra}"),
    )
    .unwrap();
}

/// Write the store with `writer` as release `version` (none when empty).
fn write_store_as(name: &str, version: &str, writer: &str) -> std::path::PathBuf {
    let dir = cell_dir(name);
    if !version.is_empty() {
        manifest(&dir, version, "");
    }
    loft::file_access::write(dir.join("a.loft"), writer).unwrap();
    let (code, out, err) = run_loft(&dir, &["a.loft"]);
    assert!(
        code == 0 && out.contains("saved true"),
        "{name}: the writer failed ({code}):\n{out}\n{err}"
    );
    dir
}

/// v1: three items `id = 10, 20, 30`, `label = "x", "yyyy", "zzzzzzz"`, in a root that holds
/// `tag = "ab"`.
const V1: &str =
    "struct Item { id: integer, label: text }\nstruct S { tag: text, items: vector<Item> }\n";

fn writer_v1(types: &str) -> String {
    format!(
        "{types}fn main() {{\n  s = S {{ tag: \"ab\" }};\n  s.items += [Item {{ id: 10, label: \"x\" }}];\n  s.items += [Item {{ id: 20, label: \"yyyy\" }}];\n  s.items += [Item {{ id: 30, label: \"zzzzzzz\" }}];\n  println(\"saved {{store_persist_copy(s, \\\"a.store\\\")}}\");\n}}\n"
    )
}

/// v3: `label` and `tag` are gone, `count` and `size` arrived.  v2 (never run here) computed
/// `count = len($.label)`; a v1 store skipped it.
const V3: &str = "struct Item { id: integer, count: integer }\nstruct S { size: integer, items: vector<Item> }\n";

/// The routines a v3 program writes: one per converted type, an overload set.
const V3_ROUTINES: &str = "struct ItemBefore2 { label: text }\nstruct SBefore2 { tag: text }\nfn upgrade(from: Version, old: ItemBefore2, item: Item) {\n  if from < \"2.0\" { item.count = len(old.label) }\n}\nfn upgrade(from: Version, old: SBefore2, item: S) {\n  if from < \"2.0\" { item.size = len(old.tag) * 100 }\n}\n";

fn reader_s(types: &str) -> String {
    format!(
        "{types}fn main() {{\n  s = S {{}};\n  if !store_load(s, \"a.store\") {{ println(\"not loaded\"); return; }}\n  t = \"{{s.size}}\";\n  for i in s.items {{ t += \" {{i.id}}:{{i.count}}\"; }}\n  println(t);\n}}\n"
    )
}

/// Cases 1 and 8: a v1 store opened by v3 arrives with the value v2 would have moved, through
/// the routine — on every element and on the root, read by `store_load` from the old file and
/// again after `loft layout upgrade`, both backends.
#[test]
fn cell_a_skipped_version_runs_the_routine() {
    let dir = write_store_as("skip_routine", "1.0", &writer_v1(V1));
    manifest(&dir, "3.0", "");
    loft::file_access::write(dir.join("b.loft"), reader_s(&format!("{V3}{V3_ROUTINES}"))).unwrap();
    let side = loft::file_access::read_to_string(dir.join("a.store.dschema")).unwrap();
    assert!(
        side.contains("\nversion=1.0\n"),
        "the writer's release is recorded:\n{side}"
    );
    let before = store_bytes(&dir);
    // len("ab") * 100 = 200; len("x"), len("yyyy"), len("zzzzzzz") = 1, 4, 7.
    reads(
        "skip_routine",
        &dir,
        "200 10:1 20:4 30:7\n",
        "converting in memory",
    );
    assert!(
        store_bytes(&dir) == before,
        "a converting store_load changed the file"
    );
    let (code, out, err) = run_loft(&dir, &["layout", "upgrade", "b.loft", "a.store"]);
    assert_eq!(code, 0, "the upgrade was refused:\n{out}\n{err}");
    let side = loft::file_access::read_to_string(dir.join("a.store.dschema")).unwrap();
    assert!(
        side.contains("\nversion=3.0\n"),
        "the upgraded store records 3.0:\n{side}"
    );
    reads(
        "skip_routine",
        &dir,
        "200 10:1 20:4 30:7\n",
        "after the upgrade",
    );
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// Case 2: the same change with no routine is refused, naming the release that wrote the store,
/// and the store is left as it was.
#[test]
fn cell_a_skipped_version_without_a_routine_is_refused() {
    let dir = write_store_as("skip_refused", "1.0", &writer_v1(V1));
    manifest(&dir, "3.0", "");
    loft::file_access::write(dir.join("b.loft"), reader_s(V3)).unwrap();
    let before = store_bytes(&dir);
    for mode in [&[][..], &["--native"][..]] {
        let mut args: Vec<&str> = mode.to_vec();
        args.push("b.loft");
        let (code, out, err) = run_loft(&dir, &args);
        assert!(
            code == 0
                && out == "not loaded\n"
                && err.contains("the store was written by 1.0")
                && err.contains("looks like a rename")
                && err.contains("fn upgrade(from: Version"),
            "{mode:?}: ({code})\n{out}\n{err}"
        );
    }
    let (code, out, err) = run_loft(&dir, &["layout", "upgrade", "b.loft", "a.store"]);
    assert!(
        code == 1 && err.contains("the store was written by 1.0"),
        "({code})\n{out}\n{err}"
    );
    assert!(store_bytes(&dir) == before, "a refusal changed the store");
    assert!(
        !loft::file_access::exists(dir.join("a.store.old")),
        "nothing was swapped"
    );
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// Case 3: a v2 store already holds the moved value; the routine runs with `from = 2.0`, its
/// branch for older stores is skipped, and the stored counts (v2's `len * 100`) arrive.
#[test]
fn cell_a_routine_skips_the_steps_a_store_already_took() {
    let v2 = "struct Item { id: integer, label: text, count: integer = len($.label) * 100 }\nstruct S { tag: text, size: integer = 5, items: vector<Item> }\n";
    let dir = write_store_as("skip_v2", "2.0", &writer_v1(v2));
    manifest(&dir, "3.0", "");
    loft::file_access::write(dir.join("b.loft"), reader_s(&format!("{V3}{V3_ROUTINES}"))).unwrap();
    reads("skip_v2", &dir, "5 10:100 20:400 30:700\n", "from 2.0");
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// Case 9: a store a LATER release wrote is not older than this program, so no routine runs on
/// it (this one would add 1 to every count).
#[test]
fn cell_a_newer_store_runs_no_routine() {
    let v2 = "struct Item { id: integer, label: text, count: integer = len($.label) * 100 }\nstruct S { tag: text, size: integer = 5, items: vector<Item> }\n";
    let dir = write_store_as("skip_newer", "4.0", &writer_v1(v2));
    manifest(&dir, "3.0", "");
    let routine = "struct ItemOld { label: text }\nfn upgrade(from: Version, old: ItemOld, item: Item) {\n  item.count += 1;\n}\n";
    loft::file_access::write(dir.join("b.loft"), reader_s(&format!("{V3}{routine}"))).unwrap();
    reads("skip_newer", &dir, "5 10:100 20:400 30:700\n", "from 4.0");
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// Case 6: a field the routine's `old` declares that the stored record lacks takes the value a
/// constructor gives an omitted field — `""` for a text — never another field's value, so a
/// routine tells the releases apart by `from`.
#[test]
fn cell_a_routine_field_the_store_lacks_is_absent() {
    let dir = write_store_as("skip_absent", "1.0", &writer_v1(V1));
    manifest(&dir, "3.0", "");
    let routine = "struct ItemOld { label: text, note: text }\nfn upgrade(from: Version, old: ItemOld, item: Item) {\n  item.count = len(old.label) + len(old.note) * 1000;\n}\nfn upgrade(from: Version, old: ItemOld, item: S) { item.size = 9 }\n";
    // `ItemOld` reads `label` from a stored `S` too: S has no `label`, so nothing there.
    let types = V3.to_string();
    loft::file_access::write(dir.join("b.loft"), reader_s(&format!("{types}{routine}"))).unwrap();
    reads(
        "skip_absent",
        &dir,
        "9 10:1 20:4 30:7\n",
        "absent old field",
    );
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// Case 7: the routine lives in a separate upgrade module, so the program's normal build
/// carries no code for it: `loft layout upgrade up.loft` brings the store forward once, and the
/// program, which has no routine, then reads it as its own.
#[test]
fn cell_a_routine_in_a_separate_upgrade_module() {
    let dir = write_store_as("skip_module", "1.0", &writer_v1(V1));
    manifest(&dir, "3.0", "");
    loft::file_access::write(dir.join("b.loft"), reader_s(V3)).unwrap();
    loft::file_access::write(
        dir.join("up.loft"),
        format!("{V3}{V3_ROUTINES}fn main() {{}}\n"),
    )
    .unwrap();
    let (code, out, err) = run_loft(&dir, &["layout", "upgrade", "up.loft", "a.store"]);
    assert_eq!(code, 0, "the upgrade module was refused:\n{out}\n{err}");
    reads(
        "skip_module",
        &dir,
        "200 10:1 20:4 30:7\n",
        "after the upgrade module ran",
    );
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// Case 4: a store older than the oldest release the program reads is refused naming the
/// release that bridges the gap — whether or not its layout changed — and is left byte for
/// byte as it was.
#[test]
fn cell_a_store_below_the_oldest_release_is_refused() {
    for (cell, reader_types) in [("below_same", V1), ("below_changed", V3)] {
        let dir = write_store_as(cell, "1.4", &writer_v1(V1));
        manifest(
            &dir,
            "3.0",
            "data_compatible_with = \"2.0\"\ndata_bridge = \"2.3\"\n",
        );
        let body = "  println(\"{len(s.items)}\");\n";
        loft::file_access::write(
            dir.join("b.loft"),
            format!(
                "{reader_types}{}fn main() {{\n  s = S {{}};\n  if !store_load(s, \"a.store\") {{ println(\"not loaded\"); return; }}\n{body}}}\n",
                if reader_types == V3 { V3_ROUTINES } else { "" }
            ),
        )
        .unwrap();
        let before = store_bytes(&dir);
        let reason = "this store was written by 1.4, and this program reads stores from 2.0 on; open it once with 2.3, then with this one";
        for mode in [&[][..], &["--native"][..]] {
            let mut args: Vec<&str> = mode.to_vec();
            args.push("b.loft");
            let (code, out, err) = run_loft(&dir, &args);
            assert!(
                code == 0 && out == "not loaded\n" && err.contains(reason),
                "{cell} {mode:?}: ({code})\n{out}\n{err}"
            );
        }
        let (code, out, err) = run_loft(&dir, &["layout", "upgrade", "b.loft", "a.store"]);
        assert!(
            code == 1 && err.contains(reason),
            "{cell}: ({code})\n{out}\n{err}"
        );
        assert!(
            store_bytes(&dir) == before,
            "{cell}: a refusal changed the store"
        );
        assert!(
            !loft::file_access::exists(dir.join("a.store.old")),
            "{cell}: nothing swapped"
        );
        let _ = loft::file_access::remove_dir_all(&dir);
    }
}

/// Case 5: a store that records no release predates the recording and is read, as a sidecar
/// without a `schema=` line is — an existing store is never newly refused by a floor.
#[test]
fn cell_a_store_without_a_recorded_release_passes_the_floor() {
    let dir = write_store_as("below_unrecorded", "", &writer_v1(V1));
    let side = loft::file_access::read_to_string(dir.join("a.store.dschema")).unwrap();
    assert!(
        !side.contains("version="),
        "no manifest, no release:\n{side}"
    );
    manifest(&dir, "3.0", "data_compatible_with = \"2.0\"\n");
    loft::file_access::write(
        dir.join("b.loft"),
        format!("{V1}fn main() {{\n  s = S {{}};\n  if !store_load(s, \"a.store\") {{ println(\"not loaded\"); return; }}\n  println(\"{{len(s.items)}} {{s.tag}}\");\n}}\n"),
    )
    .unwrap();
    reads("below_unrecorded", &dir, "3 ab\n", "unrecorded release");
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// Case 10: `Version` orders releases by number and compares against the text of one.
#[test]
fn version_orders_releases_by_number() {
    let dir = cell_dir("version_order");
    loft::file_access::write(
        dir.join("b.loft"),
        "fn main() {\n  a = \"10.0\" as Version;\n  b = \"9.2.7\" as Version;\n  println(\"{a > b} {a < \\\"9.0\\\"} {b < \\\"9.10\\\"} {\\\"2\\\" as Version == \\\"2.0.0\\\" as Version} {b}\");\n}\n",
    )
    .unwrap();
    reads("version_order", &dir, "true false true true 9.2.7\n", "");
    let _ = loft::file_access::remove_dir_all(&dir);
}

/// A conversion that stops part way — here on a duplicate under the new key, after the root
/// already owed `b` its computed default — leaves nothing owed: the drain after the refused
/// `store_load` must not fill a record of the store the conversion released.  Before, both
/// backends wrote into freed memory and died at exit (`unaligned tcache chunk detected`).
#[test]
fn cell_a_conversion_refused_part_way_leaves_nothing_owed() {
    let writer = "struct T { k: integer, n: text }\nstruct W { a: integer, h: hash<T[k]> }\nfn main() {\n  w = W { a: 21 };\n  for i in 0..4 { w.h += [T { k: i + 1, n: \"n{i % 2}\" }]; }\n  println(\"saved {store_persist_copy(w, \\\"a.store\\\")}\");\n}\n";
    let dir = write_store("refused_part_way", writer);
    loft::file_access::write(
        dir.join("b.loft"),
        "struct T { k: integer, n: text, m: integer = $.k * 1000 }\nstruct W { a: integer, b: integer = $.a * 2, h: hash<T[n]> }\nfn main() {\n  w = W {};\n  ok = store_load(w, \"a.store\");\n  println(\"ok={ok} a={w.a} b={w.b} len={len(w.h)}\");\n}\n",
    )
    .unwrap();
    for mode in [&[][..], &["--native"][..]] {
        let mut args: Vec<&str> = mode.to_vec();
        args.push("b.loft");
        let (code, out, err) = run_loft(&dir, &args);
        assert!(
            code == 0
                && out == "ok=false a=0 b=0 len=0\n"
                && err.contains("share the key [\"n0\"]")
                && !err.contains("tcache"),
            "{mode:?}: ({code})\n{out}\n{err}"
        );
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}
