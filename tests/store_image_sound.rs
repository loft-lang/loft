// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-S-Sound` — a store image is refused when a number INSIDE it does not hold.
//!
//! A save is a file people pass on, and every read after a load believes what the file says:
//! which record a field names, how many elements a vector has, how long a text is.  One
//! program writes an image of a struct holding a text and a vector of three records, each with
//! a text of its own.  The test then changes ONE number in a copy of that image, and a second
//! program loads the copy and reports what it sees, on the interpreter and on the C build.
//!
//! Expected, fixed in advance: the untouched image and a plain copy of it load and read
//! `name [seven77] len 3 walked 3 sum 60 labels a1b2c3` (10 + 20 + 30); every changed copy is
//! refused — `loaded false`, an empty value, and a `refusing` line on stderr.  Without the
//! check the C build walks 1000 elements out of a three-element record or dies, and the
//! interpreter reads other records' bytes as elements until it leaves the store.

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
  ok = store_load(l, env_variable("IMG") ?? "");
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

fn loft(dir: &Path, backend: &str, program: &str, image: &str) -> (String, String) {
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg(backend)
        .arg("--no-warnings")
        .arg(program)
        .current_dir(dir)
        .env("IMG", image)
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
    let (wrote, err) = loft(&dir, "--interpret", "write.loft", "");
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
            let (out, err) = loft(&dir, backend, "read.loft", &format!("{tag}.store"));
            assert_eq!(out, SOUND, "{backend} {tag}: a sound image loads\n{err}");
        }
        for (tag, _, _) in cells {
            let (out, err) = loft(&dir, backend, "read.loft", &format!("{tag}.store"));
            assert_eq!(out, REFUSED, "{backend} {tag}: the image is refused\n{err}");
            assert!(
                err.contains("store_load: refusing"),
                "{backend} {tag}: and the refusal says why\n{err}"
            );
        }
    }
    let _ = fa::remove_dir_all(&dir);
}
