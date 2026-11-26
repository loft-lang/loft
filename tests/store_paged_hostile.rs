// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The working-set loaders stay inside the file they read, whatever its numbers say.
//!
//! `store_load_key`, `store_load_prefix`, `store_load_range`, `store_load_box` and a
//! collection bound with `store_bind_lazy` read a few pages of an image that may come from
//! elsewhere, and run no walk over it (`@FR-S-Sound` is for a whole image).  What keeps them
//! safe is the reader itself: every fetch is held inside the file, and a record is copied
//! field by field into the local store.  This is the guard for that.
//!
//! Four images are written — a hash of 50 tiles, a trie of 80 words, a sorted of 80 tiles, a
//! spatial of 80 points — and each cell changes ONE number of one of them: the collection's
//! record, a tree's root, a child that is its own node, a child that names no node or no
//! record, the counts, a vector's length, a text's handle and length.
//!
//! Expected, fixed in advance:
//! - the sound images answer `key true [tile-number-42] 3 129` (42 + 43 + 44),
//!   `prefix 80 80 3160` (the sum of 0..79), `range 10 10 5145` (510..519), `box 80 80` and
//!   `lazy true 3`, on both backends;
//! - every changed copy ENDS, with exit code 0 and one answer line of the same shape: a
//!   refused load, an absent entry, an empty field or a shorter list.  What the line says is
//!   not fixed — a file whose numbers are wrong has no right answer — only that the program
//!   is still running to print it, and that both backends print the same.

use loft::file_access as fa;
use std::path::Path;

const TYPES: &str = r"struct Tile { id: integer, label: text, cells: vector<integer> }
struct Word { w: text, n: integer }
struct Step { tkey: integer, label: text }
struct Pt { x: integer, y: integer, tag: text }
";

const WRITER: &str = r#"
fn main() {
  tiles: hash<Tile[id]> = [];
  for i in 0..50 { tiles += [Tile { id: 1000 + i, label: "tile-number-{i}", cells: [i, i + 1, i + 2] }]; }
  words: trie<Word[w]> = [];
  for i in 0..80 { words += [Word { w: "kerk{i % 9}/{i}", n: i }]; }
  steps: sorted<Step[tkey]> = [];
  for i in 0..80 { steps += [Step { tkey: 500 + i, label: "t{i}" }]; }
  pts: spatial<Pt[x, y]> = [];
  for i in 0..80 { pts += [Pt { x: (i * 37) % 50, y: (i * 11) % 40, tag: "p{i}" }]; }
  a = store_persist_copy(tiles, "hash.store");
  b = store_persist_copy(words, "trie.store");
  c = store_persist_copy(steps, "sorted.store");
  d = store_persist_copy(pts, "spatial.store");
  println("wrote {a} {b} {c} {d}");
}
"#;

const READER: &str = r#"
fn main() {
  p = env_variable("IMG") ?? "";
  m = env_variable("MODE") ?? "";
  if m == "key" {
    tiles: hash<Tile[id]> = [];
    ok = store_load_key(tiles, p, 1042);
    t = tiles[1042];
    if t == null { println("key {ok} absent"); } else {
      s = 0; n = 0;
      for c in t.cells { s += c; n += 1; if n > 100000 { break; } }
      println("key {ok} [{t.label}] {n} {s}");
    }
  } else if m == "prefix" {
    words: trie<Word[w]> = [];
    got = store_load_prefix(words, p, "kerk", -1);
    s = 0; for e in words { s += e.n; }
    println("prefix {got} {len(words)} {s}");
  } else if m == "range" {
    steps: sorted<Step[tkey]> = [];
    got = store_load_range(steps, p, 510, 519);
    s = 0; for t in steps { s += t.tkey; }
    println("range {got} {len(steps)} {s}");
  } else if m == "box" {
    pts: spatial<Pt[x, y]> = [];
    got = store_load_box(pts, p, [0, 0], [49, 39], -1);
    println("box {got} {len(pts)}");
  } else {
    words: trie<Word[w]> = [];
    ok = store_bind_lazy(words, p);
    e = words["kerk3/3"];
    println("lazy {ok} {e.n}");
  }
}
"#;

fn loft(
    dir: &Path,
    backend: &str,
    program: &str,
    image: &str,
    mode: &str,
) -> (Option<i32>, String, String) {
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg(backend)
        .arg("--no-warnings")
        .arg(program)
        .current_dir(dir)
        .env("IMG", image)
        .env("MODE", mode)
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("loft runs");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn u32_at(image: &[u8], byte: usize) -> u32 {
    u32::from_le_bytes(image[byte..byte + 4].try_into().expect("four bytes"))
}

/// The byte where `needle` starts in `image`.
fn find(image: &[u8], needle: &[u8]) -> usize {
    image
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("the text is in the image")
}

#[test]
fn a_working_set_loader_stays_inside_a_file_whose_numbers_are_wrong() {
    let dir = std::env::temp_dir().join(format!("loft_paged_hostile_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch");
    fa::write(dir.join("write.loft"), format!("{TYPES}{WRITER}")).expect("writer");
    fa::write(dir.join("read.loft"), format!("{TYPES}{READER}")).expect("reader");
    let (_, wrote, err) = loft(&dir, "--interpret", "write.loft", "", "");
    assert_eq!(wrote, "wrote true true true true", "{err}");

    let far = 0x00F0_0000u32;
    let minus = |n: u32| n.wrapping_neg();
    // (the image, the loaders that read it, the cells: a name, a byte, a value)
    let mut jobs: Vec<(String, &str)> = Vec::new();
    let mut write = |base: &str, modes: &[&'static str], cells: &[(&str, usize, u32)]| {
        let image = fa::read(dir.join(format!("{base}.store"))).expect("the image");
        let sidecar = fa::read(dir.join(format!("{base}.store.dschema"))).expect("its sidecar");
        for (tag, byte, value) in cells {
            let mut changed = image.clone();
            changed[*byte..*byte + 4].copy_from_slice(&value.to_le_bytes());
            let name = format!("{base}_{tag}");
            fa::write(dir.join(format!("{name}.store")), &changed).expect("an image");
            fa::write(dir.join(format!("{name}.store.dschema")), &sidecar).expect("a sidecar");
            for mode in modes {
                jobs.push((name.clone(), mode));
            }
        }
    };

    // The hash: the collection's record is named at byte 16; entry 1042's text is found by
    // its bytes, the handle to it by its number, and its vector is the word after the handle.
    let hash = fa::read(dir.join("hash.store")).expect("the image");
    let text = (find(&hash, b"tile-number-42") - 8) / 8;
    let handle = (0..hash.len() - 4)
        .step_by(4)
        .find(|b| u32_at(&hash, *b) as usize == text)
        .expect("the entry names its text");
    let cells_rec = u32_at(&hash, handle + 4) as usize;
    let table = u32_at(&hash, 16) as usize * 8;
    write(
        "hash",
        &["key"],
        &[
            ("collection_record", 16, far),
            ("table_word_4", table + 4, 0x1000_0000),
            ("table_word_8", table + 8, 0x1000_0000),
            ("table_word_12", table + 12, 0x1000_0000),
            ("text_handle", handle, far),
            ("text_length", text * 8 + 4, 100_000),
            ("text_length_max", text * 8 + 4, 0x7FFF_FFFF),
            ("vector_record", handle + 4, far),
            ("vector_length", cells_rec * 8 + 4, 100_000),
            ("vector_length_2_30", cells_rec * 8 + 4, 0x4000_0000),
        ],
    );
    // A tree: its record at byte 16; in it the root child at 4, the count at 8, the nodes
    // minted at 12, node n at 24 + 16 (n - 1) as bit, parent, false child, true child.
    for (base, modes) in [("trie", &["prefix", "lazy"][..]), ("spatial", &["box"][..])] {
        let image = fa::read(dir.join(format!("{base}.store"))).expect("the image");
        let tree = u32_at(&image, 16) as usize * 8;
        write(
            base,
            modes,
            &[
                ("tree_record", 16, far),
                ("root_is_no_node", tree + 4, minus(9999)),
                ("false_child_is_its_node", tree + 24 + 8, minus(1)),
                ("true_child_is_its_node", tree + 24 + 12, minus(1)),
                ("child_is_no_record", tree + 24 + 8, far),
                ("child_is_no_node", tree + 24 + 12, minus(900_000)),
                ("record_count", tree + 8, 0x7FFF_FFFF),
                ("nodes_minted", tree + 12, 0x7FFF_FFFF),
            ],
        );
    }
    let sorted = fa::read(dir.join("sorted.store")).expect("the image");
    let vector = u32_at(&sorted, 16) as usize * 8;
    write(
        "sorted",
        &["range"],
        &[
            ("vector_record", 16, far),
            ("length", vector + 4, 100_000),
            ("length_2_30", vector + 4, 0x4000_0000),
            ("length_max", vector + 4, 0xFFFF_FFFF),
        ],
    );

    let sound = [
        ("hash", "key", "key true [tile-number-42] 3 129"),
        ("trie", "prefix", "prefix 80 80 3160"),
        ("sorted", "range", "range 10 10 5145"),
        ("spatial", "box", "box 80 80"),
        ("trie", "lazy", "lazy true 3"),
    ];
    let c_build = loft::platform::c_runtime_posix_host();
    for (image, mode, want) in sound {
        let file = format!("{image}.store");
        let (_, out, err) = loft(&dir, "--interpret", "read.loft", &file, mode);
        assert_eq!(out, want, "--interpret {image} {mode}\n{err}");
        if c_build {
            let (_, out, err) = loft(&dir, "--native-c", "read.loft", &file, mode);
            assert_eq!(out, want, "--native-c {image} {mode}\n{err}");
        }
    }
    assert_eq!(
        jobs.len(),
        38,
        "ten hash, sixteen trie, eight spatial and four sorted cells"
    );
    for (name, mode) in &jobs {
        let file = format!("{name}.store");
        let (code, out, err) = loft(&dir, "--interpret", "read.loft", &file, mode);
        assert_eq!(
            code,
            Some(0),
            "--interpret {name} {mode}: the program ends\n{out}\n{err}"
        );
        assert!(
            out.starts_with(mode) && out.lines().count() == 1,
            "--interpret {name} {mode}: and answers in one line\n{out}\n{err}"
        );
        if c_build {
            let (code, c_out, err) = loft(&dir, "--native-c", "read.loft", &file, mode);
            assert_eq!(
                code,
                Some(0),
                "--native-c {name} {mode}: the program ends\n{c_out}\n{err}"
            );
            assert_eq!(
                c_out, out,
                "{name} {mode}: both backends answer alike\n{err}"
            );
        }
    }
    let _ = fa::remove_dir_all(&dir);
}

/// A paged read trusts its server (REMOTE_STORES.md § Trust), so the first plain-`http://` read
/// of another machine in a run says so on stderr — once, on both backends — and a read of this
/// machine or over `https://` says nothing.  The hosts do not exist; every load answers `false`.
///
/// @falsified-at: 61391784d + this change — by hand: with the note's call removed from
/// `check_sidecar` (Rust) and `loft_pg_begin` (C) no line is printed; with the once-guard
/// removed the line is printed twice.
#[test]
fn a_plain_http_read_of_another_machine_says_so_once() {
    let dir = std::env::temp_dir().join(format!("loft_plain_http_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("dir");
    let program = dir.join("p.loft");
    fa::write(
        &program,
        "struct Tile { tkey: integer not null, name: text not null }\n\
         fn main() {\n\
         \x20 t: hash<Tile[tkey]> = [];\n\
         \x20 a = store_load_key(t, \"http://127.0.0.1:9/x.store\", 1);\n\
         \x20 b = store_load_key(t, \"https://example.invalid/x.store\", 1);\n\
         \x20 c = store_load_key(t, \"http://example.invalid/x.store\", 1);\n\
         \x20 d = store_load_key(t, \"http://other.invalid:8080/y.store\", 2);\n\
         \x20 println(\"done {a} {b} {c} {d}\");\n\
         }\n",
    )
    .expect("program");
    for backend in ["--interpret", "--native"] {
        let (code, stdout, stderr) = loft(&dir, backend, program.to_str().unwrap(), "", "");
        assert_eq!(code, Some(0), "{backend}: {stderr}");
        assert_eq!(stdout, "done false false false false", "{backend}");
        let lines: Vec<&str> = stderr
            .lines()
            .filter(|l| l.contains("plain http"))
            .collect();
        assert_eq!(
            lines,
            [
                "loft: example.invalid is read over plain http: anyone on the network path can \
              change what is read; serve it over https"
            ],
            "{backend}: one line, for the first remote host\n{stderr}"
        );
    }
    let _ = fa::remove_dir_all(&dir);
}
