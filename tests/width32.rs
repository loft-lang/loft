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
    run_on("--interpret", name, lib, main, width)
}

/// [`run`] on `backend` (`--interpret` or `--native-c`).
fn run_on(backend: &str, name: &str, lib: &str, main: &str, width: Option<&str>) -> String {
    let dir = std::env::temp_dir().join(format!("loft-width32-{}-{name}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    write(&dir.join("lib").join("wlib.loft"), lib);
    write(&dir.join("main.loft"), main);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg(backend)
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

/// A 64-bit program calls into a `#width 32` library with the values it has.  An `integer`
/// variable for a 32-bit parameter is narrowed at the call (21 doubles to 42; 5000000000 does
/// not fit and reads null), a `float` for a `single` is rounded (3.0 / 2 = 1.5), a method's
/// argument and an `a | 1` expression (21) go the same way, and an `i64` parameter keeps all
/// 64 bits.  The library's `0xFF000000l` is a 64-bit literal: 4278190080.
#[test]
fn a_64_bit_caller_narrows_its_arguments_into_a_32_bit_library() {
    let lib = "#width 32
pub struct Box { pub v: integer }
pub fn twice(x: integer) -> integer { x * 2 }
pub fn half(x: float) -> float { x / 2.0 }
pub fn get(self: const Box, k: integer) -> integer { self.v + k }
pub fn keep(x: i64) -> i64 { x }
pub fn alpha() -> i64 { 0xFF000000l }
";
    let main = "use wlib;
fn big() -> integer { 21 }
fn huge() -> integer { 5000000000 }
fn main() {
  a = big();
  b = wlib::Box { v: 1 };
  println(\"{wlib::twice(a)} {wlib::twice(huge())} {wlib::half(3.0)} {b.get(a)} {wlib::twice(a | 1)} {wlib::keep(huge())} {wlib::alpha()}\");
}
";
    assert_eq!(
        run("boundary", lib, main, None),
        "42 null 1.5 22 42 5000000000 4278190080\n"
    );
}

/// @PLN217 — the standard library under the regime, where the runtime fills its records: a
/// body-less binding answers 64 bits narrowed at the call (`as_number`, a double, reads 2.5
/// rather than its bytes as a single), `File` keeps its 64-bit offsets (the path is found and
/// the size is this file's), a JSON integer keeps 64 bits (`as_long`), and a stack frame's
/// line, a type's size, a field's position and a variant's tag are written at the 4 bytes
/// their width-neutral fields take.  Two `i64` fields: 16 bytes at 0 and 8; tags count from 1;
/// `depth()` is called on line 12.  The same answers on both backends and both widths.
#[test]
fn the_standard_library_s_runtime_records_keep_their_layout() {
    let main = "struct P { a: i64, b: i64 }
enum E { One, Two }
fn depth() -> i64 {
  s = stack_trace();
  s[len(s) - 1].line
}
fn main() {
  f = file(\"main.loft\");
  v = json_parse(\"[400, 2.5, 5000000000]\");
  t = type_named(\"P\")!;
  e = type_named(\"E\")!;
  println(\"{f.size} {f.path} {v.item(0).as_long()} {v.item(1).as_number()} {v.item(2).as_long()} {depth()} {t.size} {t.fields[0].position} {t.fields[1].position} {e.variants[0].tag} {e.variants[1].tag}\");
}
";
    let want = format!(
        "{} main.loft 400 2.5 5000000000 12 16 0 8 1 2\n",
        main.len()
    );
    for backend in ["--interpret", "--native-c"] {
        for width in [None, Some("32")] {
            let name = format!("records{}{}", &backend[2..5], width.unwrap_or("64"));
            assert_eq!(
                run_on(backend, &name, "", main, width),
                want,
                "{backend} width {width:?}"
            );
        }
    }
}
