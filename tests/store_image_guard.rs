// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-S-Guard` — the guard beside a saved store says whether the store is the one that was
//! written, and nothing about who may change it.
//!
//! Each backend writes an image of a struct holding a text and a vector of three records
//! (10 + 20 + 30 = 60); each backend then loads these, as files beside it, with `store_load`
//! and with `store_load_untrusted`:
//!
//! | the files | `store_load` | `store_load_untrusted` |
//! |---|---|---|
//! | as written | sum 60 | sum 60 |
//! | the 20 changed to 21, guard left | sum 61, and a warning | refused |
//! | the same, guard removed | sum 61 | sum 61 |
//! | the guard naming another image | sum 60, and a warning | refused |
//! | the guard naming another sidecar | sum 60, and a warning | refused |
//! | the guard with a line a later writer added | sum 60 | sum 60 |
//! | no guard at all | sum 60 | sum 60 |
//!
//! The changed 20 is a VALUE: no number in the image points anywhere it should not, so the
//! soundness walk (`@FR-S-Sound`) has nothing to say and only the guard can.  A save changed on
//! purpose is a save, and a guard some other writer left must not cost a good one: the
//! ordinary load says what it saw and goes on, and the loader for a file from elsewhere
//! refuses.  A deleted store takes its guard with it.

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
  ok = if (env_variable("LOADER") ?? "") == "untrusted" { store_load_untrusted(l, p) } else { store_load(l, p) };
  s = 0;
  for r in l.items { s += r.v; }
  println("loaded {ok} name [{l.name}] len {len(l.items)} sum {s}");
}
"#;

const DELETER: &str = r#"struct W { name: text }

fn main() {
  w = W { name: "gone" };
  wrote = store_persist_copy(w, "gone.store");
  println("wrote {wrote} deleted {delete("gone.store")}");
}
"#;

fn loft(dir: &Path, backend: &str, program: &str, image: &str, loader: &str) -> (String, String) {
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg(backend)
        .arg("--no-warnings")
        .arg(program)
        .current_dir(dir)
        .env("IMG", image)
        .env("LOADER", loader)
        .env("LOFT_TIMEOUT", "300")
        .output()
        .expect("loft runs");
    (
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn sum(n: u32) -> String {
    format!("loaded true name [seven77] len 3 sum {n}")
}

#[test]
fn the_guard_says_whether_a_store_is_the_one_that_was_written() {
    let mut backends = vec!["--interpret"];
    if loft::platform::c_runtime_posix_host() {
        backends.push("--native-c");
    }
    for writer in &backends {
        let dir: PathBuf = std::env::temp_dir().join(format!(
            "loft_image_guard_{}_{}",
            std::process::id(),
            &writer[2..]
        ));
        let _ = fa::remove_dir_all(&dir);
        fa::create_dir_all(&dir).expect("scratch");
        fa::write(dir.join("write.loft"), WRITER).expect("writer");
        fa::write(dir.join("read.loft"), READER).expect("reader");
        let (wrote, err) = loft(&dir, writer, "write.loft", "", "");
        assert_eq!(wrote, "wrote true", "{writer}: {err}");
        let image = fa::read(dir.join("img.store")).expect("the image");
        let sidecar = fa::read(dir.join("img.store.dschema")).expect("its sidecar");
        let guard = fa::read_to_string(dir.join("img.store.guard"))
            .unwrap_or_else(|_| panic!("{writer}: the write leaves a guard beside the image"));
        assert!(guard.starts_with("LOFT-GUARD v1\nimage="), "{guard}");

        // The second element's `v` (`R`: v at 0), read off the image: the root's vector
        // record at word 1 byte 12, its elements from byte 8, 16 bytes each.
        let items = u32::from_le_bytes(image[20..24].try_into().unwrap()) as usize;
        let v1 = items * 8 + 8 + 16;
        assert_eq!(image[v1], 20, "the value the cells change");
        let mut edited = image.clone();
        edited[v1] = 21;
        let hash_of = |line: &str| {
            guard
                .lines()
                .find_map(|l| l.strip_prefix(line))
                .expect("the guard's line")
                .to_string()
        };
        let zeros = "0".repeat(64);
        let another = format!(
            "LOFT-GUARD v1\nimage={zeros}\nsidecar={}\n",
            hash_of("sidecar=")
        );
        let other_side = format!(
            "LOFT-GUARD v1\nimage={}\nsidecar={zeros}\n",
            hash_of("image=")
        );
        let later = format!("{guard}pages=a line a later writer added\n");

        // (name, image, guard, the sum a load reads, what a mismatch says — "" for none)
        let cells: [(&str, &[u8], Option<&str>, u32, &str); 7] = [
            ("written", &image, Some(&guard), 60, ""),
            (
                "edited",
                &edited,
                Some(&guard),
                61,
                "not the file its guard",
            ),
            ("edited_unguarded", &edited, None, 61, ""),
            (
                "another",
                &image,
                Some(&another),
                60,
                "not the file its guard",
            ),
            (
                "other_side",
                &image,
                Some(&other_side),
                60,
                "layout sidecar is not",
            ),
            ("later", &image, Some(&later), 60, ""),
            ("unguarded", &image, None, 60, ""),
        ];
        for (name, image, guard, _, _) in &cells {
            fa::write(dir.join(format!("{name}.store")), image).expect("an image");
            fa::write(dir.join(format!("{name}.store.dschema")), &sidecar).expect("a sidecar");
            if let Some(guard) = guard {
                fa::write(dir.join(format!("{name}.store.guard")), guard).expect("a guard");
            }
        }
        for reader in &backends {
            for (name, _, _, read, says) in &cells {
                let file = format!("{name}.store");
                let at = format!("written by {writer}, read by {reader}, {name}");
                let (out, err) = loft(&dir, reader, "read.loft", &file, "");
                assert_eq!(out, sum(*read), "{at}: store_load reads the file\n{err}");
                assert!(err.contains(says), "{at}: store_load says `{says}`\n{err}");
                assert_eq!(
                    says.is_empty(),
                    err.is_empty(),
                    "{at}: and nothing else\n{err}"
                );
                let (out, err) = loft(&dir, reader, "read.loft", &file, "untrusted");
                let want = if says.is_empty() {
                    sum(*read)
                } else {
                    "loaded false name [] len 0 sum 0".to_string()
                };
                assert_eq!(out, want, "{at}: store_load_untrusted\n{err}");
                assert!(
                    err.contains(says),
                    "{at}: store_load_untrusted says `{says}`\n{err}"
                );
            }
        }
        // A deleted store takes its guard with it; its sidecar is the program's to delete.
        fa::write(dir.join("delete.loft"), DELETER).expect("deleter");
        let (out, err) = loft(&dir, writer, "delete.loft", "", "");
        assert_eq!(out, "wrote true deleted Ok", "{writer}: {err}");
        assert!(
            fa::exists(dir.join("gone.store.dschema")) && !fa::exists(dir.join("gone.store.guard")),
            "{writer}: the guard went with the store, the sidecar stayed"
        );
        let _ = fa::remove_dir_all(&dir);
    }
}
