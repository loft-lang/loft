// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-S-Guard` — the guard beside a saved store says whether the store is the one that was
//! written, and nothing about who may change it.
//!
//! Each backend writes an image of a struct holding a text and a vector of three records
//! (10 + 20 + 30 = 60); each backend then loads these, as files beside it:
//!
//! | the files | answer |
//! |---|---|
//! | as written | loads, sum 60 |
//! | the 20 changed to 21, guard left | refused: not the file the guard was written for |
//! | the same, guard removed | loads, sum 61 — a save changed on purpose is a save |
//! | a newline added to the sidecar | refused: not the sidecar the guard was written for |
//! | the guard naming another image | refused |
//! | the guard with a line a later writer added | loads, sum 60 |
//! | no guard at all | loads, sum 60 — a store written before guards existed |
//!
//! The changed 20 is a VALUE: no number in the image points anywhere it should not, so the
//! soundness walk (`@FR-S-Sound`) has nothing to say and only the guard can.  Without the
//! check the second and fourth rows load.

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
  for r in l.items { s += r.v; }
  println("loaded {ok} name [{l.name}] len {len(l.items)} sum {s}");
}
"#;

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
        let (wrote, err) = loft(&dir, writer, "write.loft", "");
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
        let mut longer = sidecar.clone();
        longer.push(b'\n');
        let another = format!("LOFT-GUARD v1\nimage={}\n", "0".repeat(64));
        let later = format!("{guard}pages=a line a later writer added\n");

        // (name, image, sidecar, guard, the reader's line, what stderr must say)
        let refused = "loaded false name [] len 0 sum 0".to_string();
        let cells: [(&str, &[u8], &[u8], Option<&str>, String, &str); 7] = [
            ("written", &image, &sidecar, Some(&guard), sum(60), ""),
            (
                "edited",
                &edited,
                &sidecar,
                Some(&guard),
                refused.clone(),
                "not the file its guard",
            ),
            ("edited_unguarded", &edited, &sidecar, None, sum(61), ""),
            (
                "sidecar",
                &image,
                &longer,
                Some(&guard),
                refused.clone(),
                "layout sidecar is not",
            ),
            (
                "another",
                &image,
                &sidecar,
                Some(&another),
                refused.clone(),
                "not the file its guard",
            ),
            ("later", &image, &sidecar, Some(&later), sum(60), ""),
            ("unguarded", &image, &sidecar, None, sum(60), ""),
        ];
        for (name, image, sidecar, guard, _, _) in &cells {
            fa::write(dir.join(format!("{name}.store")), image).expect("an image");
            fa::write(dir.join(format!("{name}.store.dschema")), sidecar).expect("a sidecar");
            if let Some(guard) = guard {
                fa::write(dir.join(format!("{name}.store.guard")), guard).expect("a guard");
            }
        }
        for reader in &backends {
            for (name, _, _, _, want, says) in &cells {
                let (out, err) = loft(&dir, reader, "read.loft", &format!("{name}.store"));
                assert_eq!(
                    &out, want,
                    "written by {writer}, read by {reader}, {name}\n{err}"
                );
                assert!(
                    err.contains(says),
                    "written by {writer}, read by {reader}, {name}: stderr says `{says}`\n{err}"
                );
            }
        }
        let _ = fa::remove_dir_all(&dir);
    }
}
