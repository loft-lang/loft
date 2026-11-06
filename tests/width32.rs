// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN204 — the 32-bit regime across a library boundary (WIDTH32.md).
//!
//! A library file is not parsed from a `parse_file` entry: the parser switches its lexer to
//! the file in place (`switch_to_dep`).  So the regime a whole-program `LOFT_WIDTH=32` gives a
//! library, and a library's own `#width` line, are read where that switch happens.  Each cell
//! is a library and a program in a scratch directory, run on the interpreter, with its output
//! computed by hand.  Guards `@FR-W-Regime` (the switch and a library's own `#width` line),
//! `@FR-W-Call` (narrowing on both sides of a call), `@FR-W-Op` (i32::MAX + 1 reads null),
//! `@FR-W-Store` (a 64-bit field into an `i32` slot) and `@FR-W-Count` (`sizeof(len(v))` is 4).

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

/// `@FR-W-Store`: a 64-bit library's `integer` field, read in a `#width 32` file into an
/// `integer` (an `i32` there), is narrowed at the store: 7 fits, 5000000000 reads null.  The
/// same program without the regime keeps 5000000000; a `u8` target is still refused
/// (`@FR-I-Narrow`).  Both backends.
#[test]
fn a_64_bit_field_stored_in_a_32_bit_slot_is_narrowed() {
    let lib = "#width 64
pub struct Big { pub v: integer }
pub fn mk(x: i64) -> Big { Big { v: x } }
";
    let main32 = "#width 32
use wlib::*;
fn main() {
  ka: integer = mk(7l).v;
  kb: integer = mk(5000000000l).v;
  println(\"{ka} {kb}\");
}
";
    let main64 = "use wlib::*;
fn main() {
  kb: integer = mk(5000000000l).v;
  println(\"{kb}\");
}
";
    let to_u8 = "#width 32
use wlib::*;
fn main() {
  z: u8 = mk(7l).v;
  println(\"{z}\");
}
";
    for backend in ["--interpret", "--native-c"] {
        assert_eq!(
            run_on(backend, "store32", lib, main32, None),
            "7 null\n",
            "{backend}"
        );
        assert_eq!(
            run_on(backend, "store64", lib, main64, None),
            "5000000000\n",
            "{backend}"
        );
        let refused = run_on(backend, "store-u8", lib, to_u8, None);
        assert!(
            refused.contains("cannot implicitly narrow"),
            "{backend}: {refused}"
        );
    }
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

/// The program for [`the_32_bit_regime_computes_in_32_bits`]: sizes, and the edges where a 32-bit
/// answer differs from a 64-bit one.
const CALC: &str = "struct S { a: integer, b: float }
struct W { a: i64, b: f64 }
fn main() {
  m = 2147483647;
  x = 46341;
  f = 16777216.0;
  t = 0.1;
  v = [1, 2, 3];
  j = json_parse(\"[2.5, 400]\");
  println(\"sizes {sizeof(integer)} {sizeof(float)} {sizeof(i64)} {sizeof(f64)} {sizeof(1)} {sizeof(1l)} {sizeof(2.5)} {sizeof(2.5d)}\");
  println(\"results {sizeof(m + m)} {sizeof(m + 1l)} {sizeof(f * f)} {sizeof(f + 1.0d)} {sizeof(v[0])} {sizeof(len(v))}\");
  println(\"stdlib {sizeof(PI)} {sizeof(sin(f))} {sizeof(abs(-3))} {sizeof(ticks())} {sizeof(j.item(0).as_number())} {sizeof(j.item(1).as_long())}\");
  println(\"layout {sizeof(S)} {sizeof(W)}\");
  println(\"integer {m + 1} {x * x} {m + 1l} {-m - 2}\");
  println(\"float {f + 1.0 == f} {t + 0.2} {f + 1.0d == f as f64}\");
}
";

/// The C the C backend writes for `main` under `width`.
fn emit_c(name: &str, main: &str, width: Option<&str>) -> String {
    let dir = std::env::temp_dir().join(format!("loft-width32-{}-{name}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    write(&dir.join("main.loft"), main);
    let out = dir.join("main.c");
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-c")
        .arg("--native-emit")
        .arg(&out)
        .arg(dir.join("main.loft"))
        .env_remove("LOFT_WIDTH")
        .env_remove("LOFT_NO_C_INT32")
        .env("LOFT_NO_CACHE", "1");
    if let Some(w) = width {
        cmd.env("LOFT_WIDTH", w);
    }
    let _ = cmd.output().expect("run loft");
    fa::read_to_string(&out).expect("the emitted C")
}

/// The C type `c` declares the local `name` with (`int32_t v0_m = …` → `int32_t`).
fn local_ctype<'a>(c: &'a str, name: &str) -> &'a str {
    let suffix = format!("_{name} = ");
    c.lines()
        .map(str::trim)
        .find(|l| l.contains(&suffix) && l.starts_with(|ch: char| ch.is_ascii_lowercase()))
        .and_then(|l| l.split(' ').next())
        .unwrap_or("")
}

/// @PLN217 — under `LOFT_WIDTH=32` the program, the standard library and the C computes in 32
/// bits, and only what is written `i64` / `f64` in 64.  Sizes: `integer`, `float`, a plain
/// literal, a 32-bit result, an element, a count, `PI`, `sin`, `abs` and a JSON number narrowed
/// at the call are 4 bytes; `i64`, `f64`, `1l`, `2.5d`, an operation with an `i64` / `f64`
/// operand, `ticks()` and `as_long()` stay 8; a struct of an `integer` and a `float` is 8 bytes.
/// Values: i32::MAX + 1 and 46341² = 2147488281 do not fit and read null, -i32::MAX - 2 too;
/// with an `i64` operand the sum is 2147483648.  A single cannot hold 2^24 + 1, so 16777216.0 +
/// 1.0 is 16777216.0, and 0.1 + 0.2 prints as the single 0.3; the `f64` sum keeps the 1.  The 64-bit
/// build answers 8 everywhere, the exact sums and the double's 0.30000000000000004.  The same on
/// both backends; and the C itself holds the 32-bit locals in `int32_t` / `float` and adds and
/// multiplies them with `loft_add32` / `loft_mul32`.
#[test]
fn the_32_bit_regime_computes_in_32_bits() {
    let want32 = "sizes 4 4 8 8 4 8 4 8
results 4 8 4 8 4 4
stdlib 4 4 4 8 4 8
layout 8 16
integer null null 2147483648 null
float true 0.3 false
";
    let want64 = "sizes 8 8 8 8 8 8 8 8
results 8 8 8 8 8 8
stdlib 8 8 8 8 8 8
layout 16 16
integer 2147483648 2147488281 2147483648 -2147483649
float false 0.30000000000000004 false
";
    for backend in ["--interpret", "--native-c"] {
        let b = &backend[2..5];
        assert_eq!(
            run_on(backend, &format!("calc{b}32"), "", CALC, Some("32")),
            want32,
            "{backend}"
        );
        assert_eq!(
            run_on(backend, &format!("calc{b}64"), "", CALC, None),
            want64,
            "{backend}"
        );
    }
    let c32 = emit_c("calc-c32", CALC, Some("32"));
    let c64 = emit_c("calc-c64", CALC, None);
    for (local, narrow, wide) in [
        ("m", "int32_t", "int64_t"),
        ("x", "int32_t", "int64_t"),
        ("f", "float", "double"),
    ] {
        assert_eq!(
            local_ctype(&c32, local),
            narrow,
            "`{local}` under the regime"
        );
        assert_eq!(
            local_ctype(&c64, local),
            wide,
            "`{local}` in the 64-bit build"
        );
    }
    assert!(
        c32.contains("loft_add32(") && c32.contains("loft_mul32("),
        "32-bit operations"
    );
}
