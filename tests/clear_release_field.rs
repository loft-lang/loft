// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-H-ClearRelease` — clearing a vector FIELD of a multi-field record releases what its
//! elements own, on both backends.
//!
//! `Stores::clear_vector_release` derived the element type only when the store's root was a
//! one-field `main_vector<T>` wrapper, so a rebind of a record's vector field (`h.entries =
//! kept` on a two-field `Timeline`) cleared the length and released nothing: every old
//! element's owned heap stayed claimed inside the store — 1.6 MB after one op of the
//! dryopea `truncate_to` row where 48 KB was live — and no store-granular leak check can
//! see it, because the store is freed whole at scope exit.  The type is now read off the
//! root's field at `db.pos`.  What the end-to-end run cannot show, the runtime's own trace
//! can: `LOFT_TRACE_CLEAR=1` prints, per clear, the element type it derived and whether it
//! owns heap, and the build before this fix printed `elem=65535 owns_heap=false` for the
//! field clear on both backends (measured 2026-09-24).  The two cells pin that line.
use std::path::PathBuf;
use std::process::Command;

const PROBE: &str = "\
struct Inner { v: vector<integer> }
struct T { es: vector<Inner>, n: integer }
fn main() {
  t = T { es: [], n: 0 };
  for i in 0..3 { t.es += [Inner { v: [i, i + 1] }]; }
  keep: vector<Inner> = [];
  keep += [t.es[0]?];
  t.es = keep;
  println(\"{len(t.es)} {t.es[0]?.v}\");
}
";

fn run(backend: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("loft_clear_release_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("f.loft");
    std::fs::write(&src, PROBE).unwrap();
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg(backend)
        .arg(&src)
        .env("LOFT_TRACE_CLEAR", "1")
        .env("LOFT_TIMEOUT", "300")
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "{backend} run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The `[clear]` line of the field rebind: the last clear the program makes.
fn field_clear(stderr: &str) -> String {
    stderr
        .lines()
        .rfind(|l| l.starts_with("[clear] store="))
        .unwrap_or_else(|| panic!("no [clear] trace line in:\n{stderr}"))
        .to_string()
}

fn check(backend: &str) {
    let (stdout, stderr) = run(backend);
    assert_eq!(
        stdout.trim(),
        "1 [0,1]",
        "{backend}: the program's own answer moved"
    );
    let line = field_clear(&stderr);
    assert!(
        line.contains(" rec=1 ") && line.contains("owns_heap=true") && !line.contains("elem=65535"),
        "{backend}: the field clear derived no element type — every old element's owned heap \
         stays claimed in the store:\n{line}"
    );
}

#[test]
fn a_vector_field_clear_names_its_element_type_on_the_interpreter() {
    check("--interpret");
}

#[test]
fn a_vector_field_clear_names_its_element_type_on_native() {
    check("--native");
}
