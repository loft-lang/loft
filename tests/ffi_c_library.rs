// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A library whose native part is C behind the loft-ffi ABI (`loft-ffi/include/loft_ffi.h`),
//! shipped PREBUILT for the host, answers the same on every backend, and nothing is compiled
//! to use it (@PLN205 L0a).  The fixture `tests/fixtures/ffi_c_library` is copied to a scratch
//! directory and its `native/ffi_c_library.c` built into `prebuilt/<host-triple>/` with the
//! loft-ffi fingerprint beside it, as a library's CI would publish it.  Then one program calls
//! every native: binary text both ways (a NUL and a 0xff byte), integers, a boolean, a float,
//! a `vector<u8>` argument, and heap answers — a vector in a store of its own, a vector in its
//! argument's store, a record.  Every expected line is fixed by hand.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

const PROGRAM: &str = r#"use ffi_c_library;

fn main() {
  b = ffi_c_library::make_bin();
  println("LEN {ffi_c_library::text_len(b)}");
  e = ffi_c_library::wrap(b);
  println("WRAP {ffi_c_library::text_len(e)} {ffi_c_library::byte(e, 0)} {ffi_c_library::byte(e, 2)} {ffi_c_library::byte(e, 3)} {ffi_c_library::byte(e, 5)}");
  println("ADD {ffi_c_library::add(40, 2)} {ffi_c_library::add(-5, 3)}");
  println("EVEN {ffi_c_library::is_even(10)} {ffi_c_library::is_even(7)}");
  v: vector<u8> = [1, 2, 250];
  println("SUM {ffi_c_library::sum_bytes(v)}");
  println("HALF {ffi_c_library::half(5.0)}");
  println("TEXT {ffi_c_library::wrap("hé")}");
  n = ffi_c_library::iota(3);
  println("IOTA {n.len()} {n}");
  z = ffi_c_library::iota(0);
  println("EMPTY {z.len()} {z}");
  for k in 0..50 { x = ffi_c_library::none_vec(); if k == 0 { w = x ?? [5]; println("NONE {x.len()} {w} {z ?? [6]}"); } }
  r = ffi_c_library::rev(v);
  println("REV {r.len()} {r} {v}");
  r += [9];
  println("GROW {r.len()} {r}");
  p = ffi_c_library::pair(-77, 123456789);
  println("PAIR {p.a} {p.b}");
  println("NARROW {ffi_c_library::narrow(21)} {ffi_c_library::narrow(-1)} {!ffi_c_library::narrow(-1)}");
  if write_bytes("mapped.bin", v) {
    m = file_map("mapped.bin") ?? [];
    t = 0;
    for k in 0..200 { t += ffi_c_library::sum_bytes(m); }
    println("MAPPED {m.len()} {ffi_c_library::sum_bytes(m)} {t}");
    mc = 0;
    for k in 0..100 { mc += ffi_c_library::cat(m, v).len(); }
    println("MCAT {ffi_c_library::cat(m, v)} {m} {v} {mc}");
    delete("mapped.bin");
  }
}
"#;

/// `make_bin` is `A \0 0xff B`; `wrap` puts `[` and `]` (91, 93) around its argument;
/// `iota(n)` is `n*10+1 ..= n*10+n`; `none_vec` answers the null ref from a store loft minted
/// for it (no heap argument): the declared `vector<integer>` reads it as the empty vector on
/// every backend, 50 calls release 50 minted stores (STORE_MODEL.md D19i: the interpreter
/// claimed a header in the store the null named — store 0, its stack — and the Rust writer
/// answered a ref into store 0; both kept the minted store); `rev` reverses and appends a 7,
/// leaving its argument be.
/// `narrow` doubles a non-negative and answers `INT32_MIN` — the narrow null — for a negative:
/// every backend reads it as null (`loft_ffi::widen_i32`; the C and the Rust writer widened it
/// to -2147483648).  A MAPPED file handed to `sum_bytes` is copied into the handed store for the
/// call (`Stores::bridge_args`; the C handed the foreign record id, read out of bounds): 3 bytes,
/// 1 + 2 + 250 = 253, and 200 calls 50 600, each copy released.  `cat(m, v)` hands the
/// library the mapped bytes copied into `v`'s store and copies the answer out of that store
/// (loft#2227 beside the mapped copy): `[1,2,250,1,2,250]`, both arguments as they were, 100
/// calls 600.
const WANT: [&str; 16] = [
    "LEN 4",
    "WRAP 6 91 0 255 93",
    "ADD 42 -2",
    "EVEN true false",
    "SUM 253",
    "HALF 2.5",
    "TEXT [hé]",
    "IOTA 3 [31,32,33]",
    "EMPTY 0 []",
    "NONE 0 [] []",
    "REV 4 [250,2,1,7] [1,2,250]",
    "GROW 5 [250,2,1,7,9]",
    "PAIR -77 123456789",
    "NARROW 42 null true",
    "MAPPED 3 253 50600",
    "MCAT [1,2,250,1,2,250] [1,2,250] [1,2,250] 600",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Where the fixture's native part is published: in the package, as a library's CI would, or
/// as a component loft ships beside its binary (the package then says only `[native]
/// component`).
#[derive(Clone, Copy)]
enum Shipped {
    InPackage,
    AsComponent,
}

/// Build the fixture's C into `dir` as `loft_ffi_c_library`, stamped with the loft-ffi fingerprint.
fn build_library(dir: &Path) {
    fa::create_dir_all(dir).expect("the library directory");
    let lib = dir.join(loft::native_build::platform_lib_name("loft_ffi_c_library"));
    let out = loft::platform::process::harness_command("cc")
        .args([
            "-shared", "-fPIC", "-O2", "-Wall", "-Wextra", "-Werror", "-I",
        ])
        .arg(root().join("loft-ffi/include"))
        .arg(root().join("tests/fixtures/ffi_c_library/native/ffi_c_library.c"))
        .arg("-o")
        .arg(&lib)
        .output()
        .expect("cc runs");
    assert!(
        out.status.success(),
        "the C library builds:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    loft::cache::write_native_artifact_fingerprint(dir, loft::cache::loft_ffi_fingerprint());
}

/// The fixture package with its native part published as `how` says: the directory to name
/// with `--lib`, and the components directory when there is one.
fn published(tag: &str, how: Shipped) -> (PathBuf, Option<PathBuf>) {
    let libs = std::env::temp_dir().join(format!("loft_ffi_c_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&libs);
    let pkg = libs.join("ffi_c_library");
    let fixture = root().join("tests/fixtures/ffi_c_library");
    fa::create_dir_all(pkg.join("src")).expect("the package directory");
    fa::copy(
        fixture.join("src/ffi_c_library.loft"),
        pkg.join("src/ffi_c_library.loft"),
    )
    .expect("copy the fixture");
    match how {
        Shipped::InPackage => {
            fa::copy(fixture.join("loft.toml"), pkg.join("loft.toml")).expect("copy the manifest");
            build_library(&pkg.join("prebuilt").join(loft::cache::host_triple()));
            (libs, None)
        }
        Shipped::AsComponent => {
            fa::write(
                pkg.join("loft.toml"),
                "[package]\nname = \"ffi_c_library\"\nversion = \"0.1.0\"\nloft = \">=2026.10.0\"\n\n\
                 [library]\nentry = \"src/ffi_c_library.loft\"\n\n[native]\ncomponent = \"ffi_c_library\"\n",
            )
            .expect("write the manifest");
            let components = libs.join("components");
            build_library(&components);
            (libs, Some(components))
        }
    }
}

fn run(backend: &str, how: Shipped) -> (String, String) {
    let tag = match how {
        Shipped::InPackage => "pkg",
        Shipped::AsComponent => "comp",
    };
    let (libs, components) = published(&format!("{tag}_{}", &backend[2..]), how);
    let file = libs.join("prog.loft");
    fa::write(&file, PROGRAM).expect("write the program");
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
    cmd.arg(backend)
        .arg("--no-warnings")
        .arg("--lib")
        .arg(&libs)
        .arg(&file)
        .current_dir(&libs)
        .env("LOFT_TIMEOUT", "240")
        .env("LOFT_STRICT_STORES", "1")
        .env("LOFT_NATIVE_LEAK_CHECK", "1");
    match &components {
        Some(dir) => cmd.env("LOFT_COMPONENTS", dir),
        None => cmd.env_remove("LOFT_COMPONENTS"),
    };
    let out = cmd.output().expect("loft runs");
    let _ = fa::remove_dir_all(&libs);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn check(backend: &str, how: Shipped) {
    let (out, err) = run(backend, how);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, WANT, "{backend}\nstderr:\n{err}");
    // STORE_MODEL.md D19i — every store the calls mint is released, a null answer's included.
    assert!(
        !err.contains("strict-store") && !err.contains("not freed"),
        "{backend}: a store outlived the program\nstderr:\n{err}"
    );
}

/// The backends a C library is called on: the interpreter, the Rust writer, and the C runtime
/// where it has its POSIX host layer (Windows is @PLN203 C8's).
fn backends() -> Vec<&'static str> {
    let mut b = vec!["--interpret", "--native-rust"];
    if loft::platform::c_runtime_posix_host() {
        b.push("--native-c");
    }
    b
}

#[test]
fn a_prebuilt_c_library_answers_on_the_interpreter() {
    check("--interpret", Shipped::InPackage);
}

#[test]
fn a_prebuilt_c_library_answers_on_the_rust_writer() {
    check("--native-rust", Shipped::InPackage);
}

#[test]
fn a_prebuilt_c_library_answers_on_the_c() {
    if loft::platform::c_runtime_posix_host() {
        check("--native-c", Shipped::InPackage);
    }
}

/// The same library as a component loft ships (@PLN205 L1.1): the package names it with
/// `[native] component` and holds no library of its own.
#[test]
fn a_component_answers_on_every_backend() {
    for backend in backends() {
        check(backend, Shipped::AsComponent);
    }
}

/// A component that is not installed is refused by name, and nothing is built in its place.
#[test]
fn a_missing_component_is_refused_by_name() {
    for backend in backends() {
        let (libs, components) =
            published(&format!("missing_{}", &backend[2..]), Shipped::AsComponent);
        let components = components.expect("a components directory");
        let _ = fa::remove_dir_all(&components);
        fa::create_dir_all(&components).expect("an empty components directory");
        let file = libs.join("prog.loft");
        fa::write(&file, PROGRAM).expect("write the program");
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libs)
            .arg(&file)
            .current_dir(&libs)
            .env("LOFT_TIMEOUT", "240")
            .env("LOFT_COMPONENTS", &components)
            .output()
            .expect("loft runs");
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        let _ = fa::remove_dir_all(&libs);
        assert!(
            !out.status.success(),
            "{backend}: a missing component fails the run"
        );
        assert!(
            err.contains("component `ffi_c_library`") && !err.contains("cargo"),
            "{backend}: the refusal names the component and builds nothing\n{err}"
        );
    }
}

/// `loft <flags> --lib <libs> <file>` as these tests run it, from `libs`.
fn loft_in(libs: &Path, flags: &[&str], file: &Path) -> std::process::Command {
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
    cmd.args(flags)
        .arg("--no-warnings")
        .arg("--lib")
        .arg(libs)
        .arg(file)
        .current_dir(libs)
        .env("LOFT_TIMEOUT", "240")
        .env_remove("LOFT_COMPONENTS");
    cmd
}

/// Every heap answer goes with the value that holds it: twenty rounds of each answer built in a
/// store of its own, under the test runner's leak check, end with no store left behind on any
/// backend.  `rev`, whose answer is built in its argument's store, stays out of the loop: that
/// answer's temporary frees the argument's store (loft#2227).
#[test]
fn heap_answers_leave_no_store_behind() {
    const TESTS: &str = r#"use ffi_c_library;

fn test_heap_answers() {
  for i in 0..20 {
    n = ffi_c_library::iota(5);
    assert(n.len() == 5 && n[0] == 51 && n[4] == 55, "iota {n}");
    z = ffi_c_library::iota(0);
    assert(z.len() == 0, "empty {z}");
    p = ffi_c_library::pair(i, 1000 - i);
    assert(p.a == i && p.b == 1000 - i, "pair {p.a} {p.b}");
  }
}
"#;
    for backend in backends() {
        let (libs, _) = published(&format!("leak_{}", &backend[2..]), Shipped::InPackage);
        let file = libs.join("heap_test.loft");
        fa::write(&file, TESTS).expect("write the tests");
        let out = loft_in(&libs, &["--tests", backend], &file)
            .env("LOFT_LEAK_CHECK", "1")
            .output()
            .expect("loft runs");
        let (stdout, stderr) = (
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        );
        let _ = fa::remove_dir_all(&libs);
        assert!(
            out.status.success() && !stderr.contains("not freed") && !stdout.contains("not freed"),
            "{backend}: the heap answers pass and leak nothing\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

/// loft#2227, `@FR-F-Ret` — a call's answer is FRESH, also when the native built it in its
/// ARGUMENT's store (the bridge hands it that one store): the answer is its own value, the
/// argument is untouched, and nothing the argument holds is freed with the answer.  Each cell
/// fixes its values by hand; `LOFT_STRICT_STORES=1` names a read of a freed store on the
/// interpreter and the Rust writer, and the C is checked by value and leak.
#[test]
fn an_answer_built_in_its_arguments_store_is_its_own() {
    const TESTS: &str = r#"use ffi_c_library;

fn test_rev_leaves_its_argument_and_a_minted_neighbour_be() {
  v: vector<u8> = [3, 4];
  for i in 0..20 {
    r = ffi_c_library::rev(v);
    p = ffi_c_library::pair(i, 1000 - i);
    assert(r.len() == 3 && r[0] == 4 && r[1] == 3 && r[2] == 7, "rev {r}");
    assert(v.len() == 2 && v[0] == 3 && v[1] == 4, "the argument {v}");
    assert(p.a == i && p.b == 1000 - i, "pair {p.a} {p.b}");
  }
}

fn test_the_answer_is_written_apart_from_the_argument() {
  v: vector<u8> = [3, 4];
  r = ffi_c_library::rev(v);
  r += [9];
  r[0] = 100;
  assert("{r}" == "[100,3,7,9]", "the answer {r}");
  assert("{v}" == "[3,4]", "the argument {v}");
}

fn test_an_answer_that_is_its_argument_is_a_copy() {
  v: vector<u8> = [5, 6];
  s = ffi_c_library::same(v);
  s += [1];
  s[0] = 50;
  assert("{s}" == "[50,6,1]", "the answer {s}");
  assert("{v}" == "[5,6]", "the argument {v}");
}

fn test_a_field_of_a_record_argument_is_a_copy() {
  b = ffi_c_library::Bag { items: [1, 2, 3] };
  it = ffi_c_library::bag_items(b);
  it += [4];
  assert("{it}" == "[1,2,3,4]", "the answer {it}");
  assert("{b.items}" == "[1,2,3]", "the argument {b.items}");
}

fn test_a_record_answer_holding_its_arguments_text_owns_it() {
  a = ffi_c_library::Named { k: 1, n: "abc" };
  m = ffi_c_library::named_k(a, 7);
  assert(m.k == 7 && m.n == "abc", "the answer {m.k} {m.n}");
  m.n = "zz";
  assert(a.k == 1 && a.n == "abc", "the argument {a.k} {a.n}");
  assert(m.n == "zz", "the answer took the write: {m.n}");
}

fn worker_rev(i: integer) -> integer {
  v: vector<u8> = [i & 255, 2];
  r = ffi_c_library::rev(v);
  r.len() + r[0] + v[0]
}

fn test_a_par_worker_owns_its_answer() {
  t = 0;
  for i in 0..100 par(n = worker_rev(i), 4) { t += n; }
  assert(t == 5450, "100 × (3 + 2) + (0 + … + 99): {t}");
}

fn test_a_thousand_calls_and_an_answer_passed_straight_on() {
  v: vector<u8> = [1, 2, 3];
  t = 0;
  for i in 0..1000 {
    r = ffi_c_library::rev(v);
    t += r.len() + r[0];
    t += ffi_c_library::sum_bytes(ffi_c_library::rev(v));
  }
  assert(t == 20000, "1000 × (4 + 3 + 13): {t}");
  assert("{v}" == "[1,2,3]", "the argument {v}");
}
"#;
    for backend in backends() {
        let (libs, _) = published(&format!("own_{}", &backend[2..]), Shipped::InPackage);
        let file = libs.join("own_test.loft");
        fa::write(&file, TESTS).expect("write the tests");
        let out = loft_in(&libs, &["--tests", backend], &file)
            .env("LOFT_LEAK_CHECK", "1")
            .env("LOFT_STRICT_STORES", "1")
            .output()
            .expect("loft runs");
        let (stdout, stderr) = (
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        );
        let _ = fa::remove_dir_all(&libs);
        let bad = |t: &str| t.contains("not freed") || t.contains("USE AFTER FREE");
        assert!(
            out.status.success() && !bad(&stdout) && !bad(&stderr),
            "{backend}: every answer is its own\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

/// A `par` worker holds its parent's stores READ-ONLY (@PLN108), and a native called there on
/// the parent's data is pinned to that store (`Stores::bridge_args`): its answer cannot be built
/// in it.  The call is handed a writable copy of the store instead, its answer is copied out
/// (loft#2227's `own_bridge_answer`), and the copy goes — so the worker answers, the parent is
/// unchanged, and nothing is left behind.  Values by hand, on every backend.
#[test]
fn a_native_called_on_a_parents_data_in_a_par_worker_answers() {
    const TESTS: &str = r#"use ffi_c_library;

fn head_of_rev(x: vector<u8>) -> integer {
  r = ffi_c_library::rev(x);
  r.len() * 1000 + r[0]
}

fn items_of(b: ffi_c_library::Bag) -> integer {
  it = ffi_c_library::bag_items(b);
  it.len() * 1000 + it[0]
}

fn renamed(p: ffi_c_library::Named) -> integer {
  m = ffi_c_library::named_k(p, p.k + 10);
  m.k * 100 + len(m.n)
}

fn summed(x: vector<u8>) -> integer {
  ffi_c_library::sum_bytes(x)
}

fn test_a_parents_vector() {
  vv: vector<vector<u8>> = [[1, 2], [3, 4, 5], [6]];
  t = 0;
  for x in vv par(n = head_of_rev(x), 2) { t += n; }
  assert(t == 9013, "3002 + 4005 + 2006: {t}");
  assert("{vv}" == "[[1,2],[3,4,5],[6]]", "the parent {vv}");
}

fn test_a_parents_record_answering_its_field() {
  bags = [ffi_c_library::Bag { items: [1, 2] }, ffi_c_library::Bag { items: [3, 4, 5] }];
  t = 0;
  for b in bags par(n = items_of(b), 2) { t += n; }
  assert(t == 5004, "2001 + 3003: {t}");
  assert("{bags[1].items}" == "[3,4,5]", "the parent {bags[1].items}");
}

fn test_a_parents_record_holding_text() {
  ps = [ffi_c_library::Named { k: 1, n: "ab" }, ffi_c_library::Named { k: 2, n: "cde" }];
  t = 0;
  for p in ps par(n = renamed(p), 2) { t += n; }
  assert(t == 2305, "1102 + 1203: {t}");
  assert(ps[0].k == 1 && ps[0].n == "ab" && ps[1].n == "cde", "the parent {ps[0].k} {ps[0].n} {ps[1].n}");
}

fn test_a_scalar_answer_needs_no_copy() {
  vv: vector<vector<u8>> = [[1, 2], [3, 4, 5], [6]];
  t = 0;
  for x in vv par(n = summed(x), 2) { t += n; }
  assert(t == 21, "3 + 12 + 6: {t}");
}

fn test_many_rounds_leave_nothing_behind() {
  vv: vector<vector<u8>> = [[1, 2], [3, 4, 5], [6]];
  t = 0;
  for round in 0..50 {
    for x in vv par(n = head_of_rev(x), 3) { t += n; }
  }
  assert(t == 50 * 9013, "50 rounds of 9013: {t}");
}
"#;
    for backend in backends() {
        let (libs, _) = published(&format!("par_{}", &backend[2..]), Shipped::InPackage);
        let file = libs.join("par_test.loft");
        fa::write(&file, TESTS).expect("write the tests");
        let out = loft_in(&libs, &["--tests", backend], &file)
            .env("LOFT_LEAK_CHECK", "1")
            .env("LOFT_STRICT_STORES", "1")
            .output()
            .expect("loft runs");
        let (stdout, stderr) = (
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        );
        let _ = fa::remove_dir_all(&libs);
        let bad = |t: &str| t.contains("not freed") || t.contains("USE AFTER FREE");
        assert!(
            out.status.success() && !bad(&stdout) && !bad(&stderr),
            "{backend}: a worker's native answers on its parent's data\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

/// A program written out with `--out` carries the library beside it and finds it there by
/// loft's search order once the package it was built from is gone — never by the path this
/// machine built it at.
#[test]
fn a_shipped_program_finds_its_library_beside_it() {
    let mut compiled = vec!["--native-rust"];
    if loft::platform::c_runtime_posix_host() {
        compiled.push("--native-c");
    }
    for backend in compiled {
        let (libs, _) = published(&format!("out_{}", &backend[2..]), Shipped::InPackage);
        let file = libs.join("prog.loft");
        fa::write(
            &file,
            "use ffi_c_library;\nfn main() { println(\"ADD {ffi_c_library::add(40, 2)}\"); }\n",
        )
        .expect("write the program");
        let shipped = std::env::temp_dir().join(format!(
            "loft_ffi_c_shipped_{}_{}",
            &backend[2..],
            std::process::id()
        ));
        let _ = fa::remove_dir_all(&shipped);
        let out_dir = shipped.to_str().expect("a UTF-8 scratch path");
        let built = loft_in(&libs, &[backend, "--out", out_dir], &file)
            .output()
            .expect("loft runs");
        assert!(
            built.status.success(),
            "{backend}: the program is written out\n{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let _ = fa::remove_dir_all(&libs);
        let exe = shipped.join(format!("prog{}", std::env::consts::EXE_SUFFIX));
        let out = loft::platform::process::harness_command(&exe)
            .current_dir(std::env::temp_dir())
            .env("LOFT_TIMEOUT", "60")
            .env_remove("LOFT_LIB_PATH")
            .output()
            .expect("the shipped program runs");
        let _ = fa::remove_dir_all(&shipped);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "ADD 42\n",
            "{backend}: the shipped program answers from the library beside it\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// A library shipped prebuilt per host that is absent for this one, or built against another
/// loft-ffi, is refused naming what is missing: there is no crate to build, so no backend
/// tells the user to run `cargo`.
#[test]
fn a_missing_prebuilt_library_is_refused_without_build_advice() {
    for backend in backends() {
        for (how, want) in [
            ("absent", "ships no native library for"),
            ("stale", "was built against another loft-ffi"),
        ] {
            let (libs, _) = published(&format!("{how}_{}", &backend[2..]), Shipped::InPackage);
            let prebuilt = libs.join("ffi_c_library").join("prebuilt");
            if how == "absent" {
                let _ = fa::remove_dir_all(&prebuilt);
            } else {
                loft::cache::write_native_artifact_fingerprint(
                    &prebuilt.join(loft::cache::host_triple()),
                    loft::cache::loft_ffi_fingerprint() ^ 1,
                );
            }
            let file = libs.join("prog.loft");
            fa::write(&file, PROGRAM).expect("write the program");
            let out = loft_in(&libs, &[backend], &file)
                .output()
                .expect("loft runs");
            let err = String::from_utf8_lossy(&out.stderr).to_string();
            let _ = fa::remove_dir_all(&libs);
            assert!(!out.status.success(), "{backend} {how}: the run fails");
            assert!(
                err.contains(want) && err.contains("ffi_c_library") && !err.contains("cargo"),
                "{backend} {how}: the refusal says what is missing and builds nothing\n{err}"
            );
        }
    }
}
