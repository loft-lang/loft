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
        .env("LOFT_TIMEOUT", "240");
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
