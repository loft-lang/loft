// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-S-Sound` — the one walk that says whether a store's numbers hold, asked for by the
//! program: `store_verify`, and the loader for a file from elsewhere.
//!
//! A save is a file people pass on, and every read after a load believes what the file says:
//! which record a field names, how many elements a vector has, how long a text is.  One
//! program writes an image of a struct holding a text and a vector of three records, each with
//! a text of its own.  The test then changes ONE number in a copy of that image, and a second
//! program opens the copy two ways, on the interpreter and on the C build.
//!
//! Expected, fixed in advance:
//! - `store_load_untrusted` reads the untouched image and a plain copy as
//!   `name [seven77] len 3 walked 3 sum 60 labels a1b2c3` (10 + 20 + 30), and refuses every
//!   changed copy — `loaded false`, an empty value, a line saying which number;
//! - `store_load` followed by `store_verify` answers `verify true` for the two sound files
//!   and `verify false` for every changed one.  A plain `store_load` runs no walk: the
//!   program that wants the answer asks.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

const WRITER: &str = r#"struct R { label: text, v: integer }
struct W { name: text, items: vector<R> }

fn main() {
  w = W { name: "seven77", items: [R { label: "a1", v: 10 }, R { label: "b2", v: 20 }, R { label: "c3", v: 30 }] };
  println("wrote {store_persist_copy(w, "img.store")}");
}
"#;

const READER: &str = r#"struct R { label: text, v: integer }
struct W { name: text, items: vector<R> }

fn main() {
  l = W { name: "", items: [] };
  p = env_variable("IMG") ?? "";
  if (env_variable("MODE") ?? "") == "verify" {
    println("loaded {store_load(l, p)} verify {store_verify(l)}");
    return;
  }
  ok = store_load_untrusted(l, p);
  s = 0;
  n = 0;
  labels = "";
  for r in l.items { s += r.v; n += 1; labels += r.label; }
  println("loaded {ok} name [{l.name}] len {len(l.items)} walked {n} sum {s} labels {labels}");
}
"#;

const SOUND: &str = "loaded true name [seven77] len 3 walked 3 sum 60 labels a1b2c3";
const REFUSED: &str = "loaded false name [] len 0 walked 0 sum 0 labels";

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_image_sound_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch");
    dir
}

fn loft(dir: &Path, backend: &str, program: &str, image: &str, mode: &str) -> (String, String) {
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg(backend)
        .arg("--no-warnings")
        .arg(program)
        .current_dir(dir)
        .env("IMG", image)
        .env("MODE", mode)
        .env("LOFT_TIMEOUT", "300")
        .output()
        .expect("loft runs");
    (
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn u32_at(image: &[u8], byte: usize) -> u32 {
    u32::from_le_bytes(image[byte..byte + 4].try_into().expect("four bytes"))
}

#[test]
fn a_number_inside_an_image_that_does_not_hold_refuses_the_load() {
    let dir = scratch();
    fa::write(dir.join("write.loft"), WRITER).expect("writer");
    fa::write(dir.join("read.loft"), READER).expect("reader");
    let (wrote, err) = loft(&dir, "--interpret", "write.loft", "", "");
    assert_eq!(wrote, "wrote true", "{err}");
    let image = fa::read(dir.join("img.store")).expect("the image");
    let sidecar = fa::read(dir.join("img.store.dschema")).expect("its sidecar");

    // Where the numbers are, read off the image itself: the root's payload is at word 1
    // byte 8 (`W`: name at 0, items at 4); a text or vector record holds its length at
    // byte 4 and its content from byte 8; an element is 16 bytes (`R`: v at 0, label at 8).
    let name = u32_at(&image, 16) as usize;
    let items = u32_at(&image, 20) as usize;
    let label1_at = items * 8 + 8 + 16 + 8;
    let label1 = u32_at(&image, label1_at) as usize;
    assert_eq!(u32_at(&image, items * 8 + 4), 3, "the vector's length");
    assert_eq!(&image[name * 8 + 8..name * 8 + 15], b"seven77");
    assert_eq!(&image[label1 * 8 + 8..label1 * 8 + 10], b"b2");
    let far = 0x00F0_0000u32;
    let cells: [(&str, usize, u32); 9] = [
        ("vector_length", items * 8 + 4, 1000),
        ("vector_record_past_the_image", 20, far),
        ("vector_record_inside_a_block", 20, name as u32 + 1),
        ("vector_record_too_small", 20, name as u32),
        ("text_record_past_the_image", 16, far),
        ("text_length", name * 8 + 4, 100_000),
        ("element_text_record_past_the_image", label1_at, far),
        ("element_text_length", label1 * 8 + 4, 100_000),
        // 0xFF where the `s` of `seven77` was: bytes that are not UTF-8.
        ("text_not_utf8", name * 8 + 8, 0x6576_65FF),
    ];
    let write = |tag: &str, bytes: &[u8]| {
        fa::write(dir.join(format!("{tag}.store")), bytes).expect("an image");
        fa::write(dir.join(format!("{tag}.store.dschema")), &sidecar).expect("a sidecar");
    };
    write("copy", &image);
    for (tag, byte, value) in cells {
        let mut changed = image.clone();
        changed[byte..byte + 4].copy_from_slice(&value.to_le_bytes());
        write(tag, &changed);
    }

    let mut backends = vec!["--interpret"];
    if loft::platform::c_runtime_posix_host() {
        backends.push("--native-c");
    }
    for backend in backends {
        for tag in ["img", "copy"] {
            let file = format!("{tag}.store");
            let (out, err) = loft(&dir, backend, "read.loft", &file, "");
            assert_eq!(out, SOUND, "{backend} {tag}: a sound image loads\n{err}");
            let (out, err) = loft(&dir, backend, "read.loft", &file, "verify");
            assert_eq!(out, "loaded true verify true", "{backend} {tag}\n{err}");
        }
        for (tag, _, _) in cells {
            let file = format!("{tag}.store");
            let (out, err) = loft(&dir, backend, "read.loft", &file, "");
            assert_eq!(out, REFUSED, "{backend} {tag}: the image is refused\n{err}");
            assert!(
                err.contains("[cr-check]") && err.contains("store_load_untrusted: refusing"),
                "{backend} {tag}: and the refusal says which number\n{err}"
            );
            let (out, err) = loft(&dir, backend, "read.loft", &file, "verify");
            assert_eq!(out, "loaded true verify false", "{backend} {tag}\n{err}");
        }
    }
    let _ = fa::remove_dir_all(&dir);
}

const TREE_WRITER: &str = r#"struct R { label: text, v: integer }
struct T { key: text, n: integer }
struct W { name: text, by: index<R[v]>, pick: reference<R>? }
struct Churned { by: index<R[v]>, words: index<T[key]>, pick: reference<R>? }

fn main() {
  w = W { name: "seven77" };
  for i in 0..40 { w.by += [R { label: "r{i}", v: 100 + i }]; }
  w.pick = w.by[117];
  println("wrote {store_persist_copy(w, "tree.store")}");
  // Trees that lived: removals leave free nodes and new separators, a text key a prefix.
  c = Churned { };
  for i in 0..600 { c.by += [R { label: "r{i}", v: (i * 7919) % 1000 }]; }
  for i in 0..600 { if i % 3 != 0 { c.by[(i * 7919) % 1000] = null; } }
  for i in 0..90 { c.by += [R { label: "again{i}", v: 2000 + i }]; }
  for i in 0..300 { c.words += [T { key: "prefix/shared/key{i % 97}/{i}", n: i }]; }
  for i in 0..300 { if i % 4 == 1 { c.words["prefix/shared/key{i % 97}/{i}"] = null; } }
  c.pick = c.by[2005];
  println("wrote {store_persist_copy(c, "churned.store")}");
}
"#;

const TREE_READER: &str = r#"struct R { label: text, v: integer }
struct T { key: text, n: integer }
struct W { name: text, by: index<R[v]>, pick: reference<R>? }
struct Churned { by: index<R[v]>, words: index<T[key]>, pick: reference<R>? }

fn main() {
  p = env_variable("IMG") ?? "";
  if (env_variable("MODE") ?? "") == "churned" {
    c = Churned { };
    ok = store_load_untrusted(c, p);
    n = 0; for _ in c.by { n += 1; }
    m = 0; for _ in c.words { m += 1; }
    println("loaded {ok} by {n} words {m} pick {c.pick.label}");
    return;
  }
  l = W { name: "" };
  ok = store_load_untrusted(l, p);
  if !ok { println("loaded false name [{l.name}]"); return; }
  n = 0; s = 0;
  for r in l.by { n += 1; s += r.v; }
  println("loaded true by[105] {l.by[105].label} walked {n} sum {s} pick {l.pick.label}");
}
"#;

/// `@FR-S-Sound` — an index's tree and a stored reference are walked too.
///
/// One image holds an index of 40 records (keys 100..139: a root over four leaves) and a
/// reference to the record under 117; a second holds two trees that went through removals,
/// one keyed by text.  Each cell changes ONE number of the first image; the layout it is
/// found by is `btree.rs`'s (`@FR-S-Index`): the arena's header at byte 8 of its record,
/// node `n` at byte `8 + 296 n`, a node's count at 0, its kind at 4, its next leaf at 8, its
/// records at 144 and its children at 208.
///
/// Expected, fixed in advance: both sound images load through `store_load_untrusted` on both
/// backends — `by[105] r5 walked 40 sum 4780 pick r17` (the sum of 100..139), and
/// `by 290 words 225 pick again5` (600 − 400 + 90, and 300 − 75) — and every changed copy
/// is refused with a line saying which number.  Before the walk read a tree, the same copies
/// loaded and then stopped the interpreter on a store bound, answered with other records'
/// bytes, or never ended; a sound image holding a reference was refused, its reference still
/// naming the slot it was written from.
#[test]
fn a_tree_or_a_reference_that_does_not_hold_refuses_the_load() {
    // A directory of its own: `scratch()` empties the one the other test works in.
    let dir = std::env::temp_dir().join(format!("loft_image_tree_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch");
    fa::write(dir.join("write.loft"), TREE_WRITER).expect("writer");
    fa::write(dir.join("read.loft"), TREE_READER).expect("reader");
    let (wrote, err) = loft(&dir, "--interpret", "write.loft", "", "");
    assert_eq!(wrote, "wrote true\nwrote true", "{err}");
    let image = fa::read(dir.join("tree.store")).expect("the image");
    let sidecar = fa::read(dir.join("tree.store.dschema")).expect("its sidecar");

    // The root's payload is at byte 16: name, the index's arena record, then the reference
    // (store, record, byte).
    let name = u32_at(&image, 16);
    let arena = u32_at(&image, 20) as usize;
    let header = arena * 8 + 8;
    let node = |n: usize, field: usize| header + n * 296 + field;
    let root = u32_at(&image, header) as usize;
    let first = u32_at(&image, header + 24) as usize;
    assert_eq!(u32_at(&image, header + 4), 1, "a root over leaves");
    assert_eq!(u32_at(&image, header + 8), 40, "forty records");
    assert_eq!(
        u32_at(&image, node(root, 4)) & 0xFFFF,
        0,
        "the root is an inner node"
    );
    assert_eq!(
        u32_at(&image, node(first, 4)) & 0xFFFF,
        1,
        "the first leaf is a leaf"
    );
    let far = 0x00F0_0000u32;
    let cells: [(&str, usize, u32); 23] = [
        ("arena_past_the_image", 20, far),
        ("arena_too_small", 20, name),
        ("arena_inside_a_block", 20, arena as u32 + 1),
        ("header_root", header, 9999),
        ("header_height", header + 4, 1_000_000),
        ("header_length", header + 8, 5000),
        ("header_used", header + 12, 60_000),
        ("header_first_leaf", header + 24, 9999),
        ("header_last_leaf", header + 28, 9999),
        ("root_child", node(root, 208 + 4), 9999),
        ("root_child_is_the_root", node(root, 208 + 4), root as u32),
        ("root_count", node(root, 0), 1000),
        ("root_separator", node(root, 144 + 4), far),
        ("leaf_count", node(first, 0), 1000),
        ("leaf_record_past_the_image", node(first, 144), far),
        ("leaf_record_inside_a_block", node(first, 144), name + 1),
        ("leaf_next", node(first, 8), 9999),
        ("leaf_next_is_itself", node(first, 8), first as u32),
        ("leaf_marked_inner", node(first, 4), 0),
        ("reference_record_past_the_image", 28, far),
        ("reference_record_inside_a_block", 28, name + 1),
        ("reference_byte_past_its_record", 32, 100_000),
        ("reference_into_another_store", 24, 7),
    ];
    for (tag, byte, value) in cells {
        let mut changed = image.clone();
        changed[byte..byte + 4].copy_from_slice(&value.to_le_bytes());
        fa::write(dir.join(format!("{tag}.store")), &changed).expect("an image");
        fa::write(dir.join(format!("{tag}.store.dschema")), &sidecar).expect("a sidecar");
    }

    let mut backends = vec!["--interpret"];
    if loft::platform::c_runtime_posix_host() {
        backends.push("--native-c");
    }
    for backend in backends {
        let (out, err) = loft(&dir, backend, "read.loft", "tree.store", "");
        assert_eq!(
            out, "loaded true by[105] r5 walked 40 sum 4780 pick r17",
            "{backend}: a sound tree and reference load\n{err}"
        );
        let (out, err) = loft(&dir, backend, "read.loft", "churned.store", "churned");
        assert_eq!(
            out, "loaded true by 290 words 225 pick again5",
            "{backend}: trees that went through removals load\n{err}"
        );
        for (tag, _, _) in cells {
            let (out, err) = loft(&dir, backend, "read.loft", &format!("{tag}.store"), "");
            assert_eq!(
                out, "loaded false name []",
                "{backend} {tag}: the image is refused\n{err}"
            );
            assert!(
                err.contains("[cr-check]") && err.contains("store_load_untrusted: refusing"),
                "{backend} {tag}: and the refusal says which number\n{err}"
            );
        }
    }
    let _ = fa::remove_dir_all(&dir);
}

const KEYED_WRITER: &str = r#"struct P { x: integer, y: integer, tag: text }
struct K { w: text, n: integer }
struct W { name: text, at: spatial<P[x, y]>, words: trie<K[w]> }

fn main() {
  w = W { name: "seven77" };
  for i in 0..60 { w.at += [P { x: (i * 37) % 50, y: (i * 11) % 40, tag: "p{i}" }]; }
  for i in 0..60 { w.words += [K { w: "key/{i % 7}/{i}", n: i }]; }
  for i in 0..60 { if i % 3 == 1 { w.words["key/{i % 7}/{i}"] = null; } }
  for i in 0..10 { w.words += [K { w: "late{i}", n: 100 + i }]; }
  println("wrote {store_persist_copy(w, "keyed.store")}");
}
"#;

const KEYED_READER: &str = r#"struct P { x: integer, y: integer, tag: text }
struct K { w: text, n: integer }
struct W { name: text, at: spatial<P[x, y]>, words: trie<K[w]> }

fn main() {
  l = W { name: "" };
  ok = store_load_untrusted(l, env_variable("IMG") ?? "");
  if !ok { println("loaded false name [{l.name}]"); return; }
  n = 0; s = 0;
  for p in l.at { n += 1; s += p.x + p.y; }
  m = 0; k = 0;
  for e in l.words { m += 1; k += e.n; }
  println("loaded true points {n} sum {s} words {m} sum {k} has {l.words["late3"].n}");
}
"#;

/// `@FR-S-Sound` — the tree of a spatial and of a trie collection is walked too.
///
/// One image holds a spatial of 60 points and a trie that had 60 words, lost 20 and gained
/// 10.  Each cell changes ONE number of it; the layout is `radix_tree.rs`'s (`@FR-S-Tree`):
/// in the tree's record the root child at byte 4 (negative: a node), the record count at 8,
/// the nodes minted at 12, the trie's tagged null-key record at 16, the free list at 20, and
/// node `n` at byte `24 + 16 (n − 1)` as bit, parent, false child, true child.
///
/// Expected, fixed in advance: the sound image loads on both backends as
/// `points 60 sum 2640 words 50 sum 2225 has 103` — the sums of `37i mod 50 + 11i mod 40`
/// over 0..59, and of the 40 kept numbers (1770 − 590) plus 100..109 — and so does a copy
/// whose spare header word holds an UNTAGGED number, which names no record; every other
/// changed copy is refused with a line saying which number.
#[test]
fn a_spatial_or_trie_tree_that_does_not_hold_refuses_the_load() {
    let dir = std::env::temp_dir().join(format!("loft_image_keyed_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch");
    fa::write(dir.join("write.loft"), KEYED_WRITER).expect("writer");
    fa::write(dir.join("read.loft"), KEYED_READER).expect("reader");
    let (wrote, err) = loft(&dir, "--interpret", "write.loft", "", "");
    assert_eq!(wrote, "wrote true", "{err}");
    let image = fa::read(dir.join("keyed.store")).expect("the image");
    let sidecar = fa::read(dir.join("keyed.store.dschema")).expect("its sidecar");

    // The root's payload is at byte 16: name, the spatial's tree record, the trie's.
    let name = u32_at(&image, 16);
    let at = u32_at(&image, 20) as usize * 8;
    let words = u32_at(&image, 24) as usize * 8;
    let node = |tree: usize, n: usize, field: usize| tree + 24 + 16 * (n - 1) + field;
    let minus = |n: u32| n.wrapping_neg();
    assert_eq!(
        u32_at(&image, at + 4),
        minus(1),
        "the spatial's root child is node 1"
    );
    assert_eq!(u32_at(&image, at + 8), 60, "sixty points");
    assert_eq!(u32_at(&image, words + 8), 50, "fifty words");
    // A node of the spatial whose false child is a record: follow false children down.
    let mut low = 1usize;
    while (u32_at(&image, node(at, low, 8)) as i32) < 0 {
        low = (u32_at(&image, node(at, low, 8)) as i32).unsigned_abs() as usize;
    }
    let far = 0x00F0_0000u32;
    let cells: [(&str, usize, u32); 17] = [
        ("tree_past_the_image", 20, far),
        ("tree_too_small", 20, name),
        ("tree_inside_a_block", 20, u32_at(&image, 20) + 1),
        ("root_child_is_no_node", at + 4, minus(9999)),
        ("record_count", at + 8, 5000),
        ("nodes_minted", at + 12, 100_000),
        ("free_list_names_a_live_node", at + 20, 7),
        ("child_is_no_node", node(at, 1, 8), minus(9999)),
        ("child_is_its_own_node", node(at, 1, 8), minus(1)),
        ("child_missing", node(at, 1, 12), 0),
        ("bit_not_later_than_above", node(at, 2, 0), 0),
        ("parent_wrong", node(at, 2, 4), 7),
        ("leaf_record_past_the_image", node(at, low, 8), far),
        ("leaf_record_inside_a_block", node(at, low, 8), name + 1),
        ("trie_root_child_is_no_node", words + 4, minus(9999)),
        ("trie_null_key_record", words + 16, far | 0x8000_0000),
        ("trie_child_is_its_own_node", node(words, 1, 12), minus(1)),
    ];
    let write = |tag: &str, byte: usize, value: u32| {
        let mut changed = image.clone();
        changed[byte..byte + 4].copy_from_slice(&value.to_le_bytes());
        fa::write(dir.join(format!("{tag}.store")), &changed).expect("an image");
        fa::write(dir.join(format!("{tag}.store.dschema")), &sidecar).expect("a sidecar");
    };
    for (tag, byte, value) in cells {
        write(tag, byte, value);
    }
    write("trie_untagged_word", words + 16, far);

    let sound = "loaded true points 60 sum 2640 words 50 sum 2225 has 103";
    let mut backends = vec!["--interpret"];
    if loft::platform::c_runtime_posix_host() {
        backends.push("--native-c");
    }
    for backend in backends {
        for tag in ["keyed", "trie_untagged_word"] {
            let (out, err) = loft(&dir, backend, "read.loft", &format!("{tag}.store"), "");
            assert_eq!(out, sound, "{backend} {tag}: a sound image loads\n{err}");
        }
        for (tag, _, _) in cells {
            let (out, err) = loft(&dir, backend, "read.loft", &format!("{tag}.store"), "");
            assert_eq!(
                out, "loaded false name []",
                "{backend} {tag}: the image is refused\n{err}"
            );
            assert!(
                err.contains("[cr-check]") && err.contains("store_load_untrusted: refusing"),
                "{backend} {tag}: and the refusal says which number\n{err}"
            );
        }
    }
    let _ = fa::remove_dir_all(&dir);
}

// `@FR-R-Header` — a hoisted loop of the rustc writer takes a vector's length once and reads
// elements through a raw base; the length comes from the record's own word, so in a store
// loaded with a plain `store_load` (no walk) a word that claims more than the record holds
// read, and written, past it: 2,000 reads of a three-element vector whose word was changed
// to 1,000,000 walked 16 KB past an 8 KB image.  The header now holds the length to the
// record's claim, once, so the elements past what the record holds read as past the end.
// (The interpreter's accessors stop at the store's end, and the C build reads inside its
// store's allocation: a plain load of a changed file answers differently per backend, as
// `@FR-S-Sound` allows, but no backend reads outside its memory.)
const HOIST_WRITER: &str = r#"struct W { name: text, v: vector<integer> }
fn main() {
  w = W { name: "n63", v: [10, 20, 30] };
  println("wrote {store_persist_copy(w, "img.store")}");
}
"#;

const HOIST_READER: &str = r#"struct W { name: text, v: vector<integer> }
fn main() {
  l = W { name: "", v: [] };
  ok = store_load(l, env_variable("IMG") ?? "");
  n = 0;
  s = 0;
  for i in 0..2000 {
    x = l.v[i];
    if x != null { n += 1; s += x; }
  }
  println("loaded {ok} len {len(l.v)} present {n} sum {s}");
}
"#;

#[test]
fn a_hoisted_loop_reads_no_further_than_the_record_holds() {
    if !loft::platform::process::harness_command("rustc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("SKIP: no rustc for --native-rust");
        return;
    }
    // Its own directory: `scratch()` empties the one the other tests share.
    let dir = std::env::temp_dir().join(format!("loft_image_hoist_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch");
    fa::write(dir.join("write.loft"), HOIST_WRITER).expect("writer");
    fa::write(dir.join("read.loft"), HOIST_READER).expect("reader");
    let (wrote, err) = loft(&dir, "--interpret", "write.loft", "", "");
    assert_eq!(wrote, "wrote true", "{err}");
    let image = fa::read(dir.join("img.store")).expect("the image");
    let sidecar = fa::read(dir.join("img.store.dschema")).expect("its sidecar");
    // The root's payload at word 1 byte 8 (`W`: name at 0, v at 4); the vector record holds
    // its claim in words at byte 0 and its length at byte 4.
    let v = u32_at(&image, 20) as usize;
    assert_eq!(u32_at(&image, v * 8 + 4), 3, "the vector's length");
    let held = (u32_at(&image, v * 8) as usize * 8 - 8) / 8;
    assert!(
        (3..2000).contains(&held),
        "the record holds {held} elements"
    );
    let mut changed = image.clone();
    changed[v * 8 + 4..v * 8 + 8].copy_from_slice(&1_000_000u32.to_le_bytes());
    fa::write(dir.join("bad.store"), &changed).expect("an image");
    fa::write(dir.join("bad.store.dschema"), &sidecar).expect("a sidecar");

    let (out, err) = loft(&dir, "--native", "read.loft", "img.store", "");
    assert_eq!(
        out, "loaded true len 3 present 3 sum 60",
        "a sound image\n{err}"
    );
    // The record's own slack past element 3 is zero-filled, so it reads as present zeros.
    let (out, err) = loft(&dir, "--native", "read.loft", "bad.store", "");
    assert_eq!(
        out,
        format!("loaded true len 1000000 present {held} sum 60"),
        "the reads stop where the record does\n{err}"
    );
    let _ = fa::remove_dir_all(&dir);
}
