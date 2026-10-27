// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A library whose native part is C behind the loft-ffi ABI (`loft-ffi/include/loft_ffi.h`),
//! shipped PREBUILT for the host, answers the same on every backend, and nothing is compiled
//! to use it (@PLN205 L0a).  The fixture `tests/fixtures/ffi_c_library` is copied to a scratch
//! directory and its `native/ffi_c_library.c` built into `prebuilt/<host-triple>/` with the
//! loft-ffi fingerprint beside it, as a library's CI would publish it.  Then one program calls
//! every native: binary text both ways (a NUL and a 0xff byte), integers, a boolean, a float,
//! a `vector<u8>` argument.  Every expected line is fixed by hand.

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
}
"#;

/// `make_bin` is `A \0 0xff B`; `wrap` puts `[` and `]` (91, 93) around its argument.
const WANT: [&str; 7] = [
    "LEN 4",
    "WRAP 6 91 0 255 93",
    "ADD 42 -2",
    "EVEN true false",
    "SUM 253",
    "HALF 2.5",
    "TEXT [hé]",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The fixture package, its native part built into `prebuilt/<host-triple>/`: the directory
/// to name with `--lib`.
fn published(tag: &str) -> PathBuf {
    let libs = std::env::temp_dir().join(format!("loft_ffi_c_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&libs);
    let pkg = libs.join("ffi_c_library");
    let fixture = root().join("tests/fixtures/ffi_c_library");
    fa::create_dir_all(pkg.join("src")).expect("the package directory");
    for f in ["loft.toml", "src/ffi_c_library.loft"] {
        fa::copy(fixture.join(f), pkg.join(f)).expect("copy the fixture");
    }
    let triple = pkg.join("prebuilt").join(loft::cache::host_triple());
    fa::create_dir_all(&triple).expect("the prebuilt directory");
    let lib = triple.join(loft::native_build::platform_lib_name("loft_ffi_c_library"));
    let out = loft::platform::process::harness_command("cc")
        .args([
            "-shared", "-fPIC", "-O2", "-Wall", "-Wextra", "-Werror", "-I",
        ])
        .arg(root().join("loft-ffi/include"))
        .arg(fixture.join("native/ffi_c_library.c"))
        .arg("-o")
        .arg(&lib)
        .output()
        .expect("cc runs");
    assert!(
        out.status.success(),
        "the C library builds:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    loft::cache::write_native_artifact_fingerprint(&triple, loft::cache::loft_ffi_fingerprint());
    libs
}

fn run(backend: &str) -> (String, String) {
    let libs = published(&backend[2..]);
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
        .output()
        .expect("loft runs");
    let _ = fa::remove_dir_all(&libs);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn check(backend: &str) {
    let (out, err) = run(backend);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, WANT, "{backend}\nstderr:\n{err}");
}

#[test]
fn a_prebuilt_c_library_answers_on_the_interpreter() {
    check("--interpret");
}

#[test]
fn a_prebuilt_c_library_answers_on_the_rust_writer() {
    check("--native-rust");
}

#[test]
fn a_prebuilt_c_library_answers_on_the_c() {
    if loft::platform::c_runtime_posix_host() {
        check("--native-c");
    }
}
