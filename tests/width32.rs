// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN204 — the 32-bit regime across a library boundary (WIDTH32.md).
//!
//! A library file is not parsed from a `parse_file` entry: the parser switches its lexer to
//! the file in place (`switch_to_dep`).  So the regime a whole-program `LOFT_WIDTH=32` gives a
//! library, and a library's own `#width` line, are read where that switch happens.  Each cell
//! is a library and a program in a scratch directory, run on the interpreter, with its output
//! computed by hand.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn write(path: &Path, body: &str) {
    fa::create_dir_all(path.parent().unwrap()).expect("mkdir");
    fa::write(path, body).expect("write");
}

/// Run `main` with `lib` as `wlib.loft` on the lib path, `width` as `LOFT_WIDTH` (or unset).
fn run(name: &str, lib: &str, main: &str, width: Option<&str>) -> String {
    let dir = std::env::temp_dir().join(format!("loft-width32-{}-{name}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    write(&dir.join("lib").join("wlib.loft"), lib);
    write(&dir.join("main.loft"), main);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg("--lib")
        .arg(dir.join("lib"))
        .arg(dir.join("main.loft"))
        .env_remove("LOFT_WIDTH")
        .env("LOFT_NO_CACHE", "1");
    if let Some(w) = width {
        cmd.env("LOFT_WIDTH", w);
    }
    let out = cmd.output().expect("run loft");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const LIB: &str = "pub struct R {
  pub w: float,
  pub h: float,
}
pub fn mk() -> R { R { w: 2.5, h: 4.0 } }
pub fn top() -> integer { 2147483647 }
";

const MAIN: &str = "use wlib;
fn main() {
  r = wlib::mk();
  t = 0.0;
  t += r.w * r.h;
  b = wlib::top() + 1;
  println(\"{t} {b}\");
}
";

/// `LOFT_WIDTH=32` reaches the library: its `float` fields are singles (a `single` sum takes
/// their product, 2.5 * 4.0 = 10) and its `integer` is an `i32`, so i32::MAX + 1 is null.
/// Without the switch the same program answers the 64-bit sum.
#[test]
fn the_whole_program_switch_reaches_a_library() {
    assert_eq!(run("all", LIB, MAIN, Some("32")), "10 null\n");
    assert_eq!(run("none", LIB, MAIN, None), "10 2147483648\n");
}

/// A library that begins `#width 64` stays 64-bit under `LOFT_WIDTH=32`: it computes
/// i32::MAX + 1 = 2147483648 itself.  Its plain `integer` answer is narrowed where the 32-bit
/// program receives it (null); its `i64` answer is not.
#[test]
fn a_library_keeps_64_bits_with_its_own_width_line() {
    let lib = "#width 64
pub fn plain() -> integer { 2147483647 + 1 }
pub fn wide() -> i64 { 2147483647 + 1 }
";
    let main = "use wlib;
fn main() {
  p = wlib::plain();
  w = wlib::wide();
  println(\"{p} {w}\");
}
";
    assert_eq!(run("opt-out", lib, main, Some("32")), "null 2147483648\n");
}

/// A library that begins `#width 32` is 32-bit inside a 64-bit program: its sum of two
/// `i32` values overflows at i32::MAX, while the program's own sum does not.
#[test]
fn a_library_takes_32_bits_with_its_own_width_line() {
    let lib = "#width 32
pub fn add(a: integer, b: integer) -> integer { a + b }
";
    let main = "use wlib;
fn main() {
  s = wlib::add(2147483647, 1);
  u = 2147483647 + 1;
  println(\"{s} {u}\");
}
";
    assert_eq!(run("opt-in", lib, main, None), "null 2147483648\n");
}
