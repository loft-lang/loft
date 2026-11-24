// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! A vector held in a store of its own is wrapped in a `main_vector<T>` record, and every site
//! finds that wrapper by name through `Data::vector_wrapper_name`.  A type's name is not
//! unique: a program may declare a `File` beside the standard library's.  The first
//! definition of a name keeps the plain wrapper; a later one of the same name gets its own,
//! `main_vector<File>#<def>`.  Before, the program's `File` took the standard library's
//! wrapper, so its store was typed as holding `File` records of the library's layout: freeing
//! it closed "descriptors" read from the program's records, and MSVC's runtime ended the
//! program with 0xC0000409 after its last line (the loft-suite attic cell 1882 on the nightly
//! C sweep).  On other platforms the values read right, so this pins the wrapper the IR names.
use std::path::{Path, PathBuf};

const SHADOW: &str = r#"
fn count(v: vector<File>) -> integer { len(v) }
struct File { name: text, size: integer }
fn main() {
  assert(count([File { name: "a", size: 1 }, File { name: "b", size: 2 }]) == 2, "count");
  f = file("Cargo.toml");
  assert(f.path == "Cargo.toml", "the library's File still reads");
  println("all ok");
}
"#;

fn loft(args: &[&str], src: &Path) -> std::process::Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .arg(src)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1");
    cmd.output().expect("spawn loft")
}

/// The program written to a file of its own per test: the tests run on threads of one process.
fn program(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_vector_wrapper_{}", std::process::id()));
    loft::file_access::create_dir_all(&dir).expect("temp dir");
    let src = dir.join(format!("shadow_{tag}.loft"));
    loft::file_access::write(&src, SHADOW).expect("write the program");
    src
}

// The literal passed to `count` is built in a wrapper of the program's `File`, not in the
// standard library's `main_vector<File>`.
#[test]
fn a_type_named_like_a_library_type_has_a_wrapper_of_its_own() {
    let src = program("ir");
    let out = loft(&["introspect"], &src);
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    let main = &text[text.find("fn n_main(").expect("main's IR")..];
    let line = main
        .lines()
        .find(|l| l.contains("__vdb_1(1):ref(main_vector<File"))
        .expect("the literal's wrapper");
    assert!(
        line.contains("main_vector<File>#"),
        "the program's File takes the library's wrapper: {line}"
    );
}

#[test]
fn the_program_answers_on_both_backends() {
    let src = program("run");
    for mode in ["--interpret", "--native"] {
        let out = loft(&[mode], &src);
        assert!(
            out.status.success() && String::from_utf8_lossy(&out.stdout).contains("all ok"),
            "{mode}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
